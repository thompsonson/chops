mod intent;

use anyhow::Result;
use chops_common::{
    self, DEFAULT_MQTT_HOST, MQTT_KEEP_ALIVE_SECS, MQTT_QUEUE_CAPACITY, MQTT_RECONNECT_DELAY_SECS,
};
use intent::{
    discover_projects, has_terminator, parse_intent, strip_terminator, Intent, IntentMatch,
    ParseContext, TmuxCommand,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";

/// Maximum time to wait for a terminator before auto-flushing an accumulated message.
const ACCUMULATOR_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct TranscriptionMessage {
    text: String,
    is_final: bool,
}

/// Accumulates multi-segment utterances for Claude messages.
/// Buffers text until a terminator keyword ("over", "done", etc.) is heard.
struct Accumulator {
    /// The target project and pane from the initial "tell claude" command.
    session: Option<String>,
    confidence: f64,
    /// Accumulated message segments.
    segments: Vec<String>,
    /// When accumulation started.
    started: Instant,
}

impl Accumulator {
    fn new(session: Option<String>, initial_message: String, confidence: f64) -> Self {
        info!(
            "Accumulating claude message for session {:?}: \"{}\"",
            session, initial_message
        );
        Self {
            session,
            confidence,
            segments: vec![initial_message],
            started: Instant::now(),
        }
    }

    fn append(&mut self, text: &str) {
        info!("Appending to accumulated message: \"{}\"", text);
        self.segments.push(text.to_string());
    }

    fn is_expired(&self) -> bool {
        self.started.elapsed() > ACCUMULATOR_TIMEOUT
    }

    fn build_command(self) -> (TmuxCommand, f64) {
        let message = self.segments.join(" ");
        info!("Finalized accumulated message: \"{}\"", message);
        (
            TmuxCommand {
                session: self.session,
                pane: "claude".to_string(),
                command: message,
            },
            self.confidence,
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    // Discover known projects for fuzzy matching.
    let projects_dir = dirs::home_dir()
        .map(|h| h.join("Projects"))
        .unwrap_or_default();
    let known_projects = discover_projects(&projects_dir);
    info!(
        "Discovered {} projects: {:?}",
        known_projects.len(),
        known_projects
    );
    let ctx = ParseContext { known_projects };

    let mut mqttoptions =
        MqttOptions::new("agent-core", DEFAULT_MQTT_HOST, chops_common::mqtt_port());
    mqttoptions.set_keep_alive(Duration::from_secs(MQTT_KEEP_ALIVE_SECS));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, MQTT_QUEUE_CAPACITY);

    client
        .subscribe(TRANSCRIPTION_TOPIC, QoS::AtMostOnce)
        .await?;
    info!("Agent subscribed to {TRANSCRIPTION_TOPIC}");

    let mut accumulator: Option<Accumulator> = None;

    loop {
        tokio::select! {
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::Publish(msg))) => {
                        let payload = std::str::from_utf8(&msg.payload)?;
                        if let Ok(transcription) = serde_json::from_str::<TranscriptionMessage>(payload) {
                            if transcription.is_final {
                                accumulator = handle_transcription(
                                    &transcription.text,
                                    &client,
                                    &ctx,
                                    accumulator,
                                ).await;
                            }
                        } else {
                            warn!("Failed to deserialize transcription: {payload}");
                        }
                    }
                    Err(e) => {
                        warn!("MQTT error: {e}. Reconnecting...");
                        tokio::time::sleep(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS)).await;
                    }
                    _ => {}
                }
            }
            // Check accumulator timeout every second.
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if let Some(ref acc) = accumulator {
                    if acc.is_expired() {
                        warn!("Accumulator timed out after {}s, flushing", ACCUMULATOR_TIMEOUT.as_secs());
                        let acc = accumulator.take().unwrap();
                        let (cmd, _confidence) = acc.build_command();
                        let payload = serde_json::to_string(&cmd).unwrap();
                        dispatch(&client, "agent/commands/tmux", &payload).await;
                    }
                }
            }
        }
    }
}

/// Handle a finalized transcription. Returns the new accumulator state.
async fn handle_transcription(
    text: &str,
    client: &AsyncClient,
    ctx: &ParseContext,
    accumulator: Option<Accumulator>,
) -> Option<Accumulator> {
    // If we're accumulating and this segment has a terminator, finalize.
    if let Some(mut acc) = accumulator {
        if let Some(stripped) = strip_terminator(text) {
            // Append any content before the terminator.
            if !stripped.is_empty() {
                acc.append(&stripped);
            }
            let (cmd, _confidence) = acc.build_command();
            let payload = serde_json::to_string(&cmd).unwrap();
            dispatch(client, "agent/commands/tmux", &payload).await;
            return None;
        }

        // No terminator — check if this is a new command (not a continuation).
        if let Some(m) = parse_intent(text, ctx) {
            // New command detected while accumulating — flush the accumulated
            // message first, then handle the new command.
            warn!("New command while accumulating — flushing accumulated message");
            let (cmd, _confidence) = acc.build_command();
            let payload = serde_json::to_string(&cmd).unwrap();
            dispatch(client, "agent/commands/tmux", &payload).await;

            // Handle the new command (may start a new accumulator).
            return handle_new_intent(m, client).await;
        }

        // No terminator, no new command — append as continuation.
        // Strip filler/punctuation from the continuation segment.
        let cleaned = text
            .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
            .to_string();
        if !cleaned.is_empty() {
            acc.append(&cleaned);
        }
        return Some(acc);
    }

    // Not accumulating — check if this is a terminator word by itself (ignore it).
    if has_terminator(text) && text.split_whitespace().count() <= 2 {
        info!("Ignoring stray terminator: {text}");
        return None;
    }

    // Normal path: parse intent and route.
    match parse_intent(text, ctx) {
        Some(m) => handle_new_intent(m, client).await,
        None => {
            warn!("Unhandled intent: {text}");
            // TODO: slow path — query local LLM (Ollama) here.
            None
        }
    }
}

/// Handle a newly parsed intent. For "tell claude" intents, start accumulating.
/// For everything else, dispatch immediately.
async fn handle_new_intent(m: IntentMatch, client: &AsyncClient) -> Option<Accumulator> {
    info!(
        "Matched intent (confidence: {:.2}): {:?}",
        m.confidence, m.intent
    );

    match m.intent {
        Intent::Tmux(cmd) if cmd.pane == "claude" => {
            // Check if the message itself already contains a terminator.
            if let Some(stripped) = strip_terminator(&cmd.command) {
                let final_cmd = TmuxCommand {
                    command: if stripped.is_empty() {
                        cmd.command // terminator-only message, send as-is
                    } else {
                        stripped
                    },
                    ..cmd
                };
                let payload = serde_json::to_string(&final_cmd).unwrap();
                dispatch(client, "agent/commands/tmux", &payload).await;
                None
            } else {
                // Start accumulating — wait for "over".
                Some(Accumulator::new(cmd.session, cmd.command, m.confidence))
            }
        }
        Intent::Tmux(cmd) => {
            let payload = serde_json::to_string(&cmd).unwrap();
            dispatch(client, "agent/commands/tmux", &payload).await;
            None
        }
        Intent::Vscode(payload) => {
            dispatch(client, "agent/commands/vscode", &payload).await;
            None
        }
        Intent::Termux(payload) => {
            dispatch(client, "agent/commands/termux", &payload).await;
            None
        }
    }
}

async fn dispatch(client: &AsyncClient, topic: &str, payload: &str) {
    info!("Dispatching to {topic}: {payload}");
    if let Err(e) = client.publish(topic, QoS::AtMostOnce, false, payload).await {
        warn!("Failed to dispatch to {topic}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_transcription_final() {
        let json = r#"{"text":"hello world","is_final":true}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, "hello world");
        assert!(msg.is_final);
    }

    #[test]
    fn deserialize_transcription_partial() {
        let json = r#"{"text":"hel","is_final":false}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert!(!msg.is_final);
    }

    #[test]
    fn deserialize_ignores_extra_fields() {
        let json = r#"{"text":"hello","is_final":true,"timestamp":"2026-01-01T00:00:00Z"}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, "hello");
    }
}
