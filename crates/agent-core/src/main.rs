mod intent;

use anyhow::Result;
use chops_common::{
    self, Pane, TmuxCommand, DEFAULT_MQTT_HOST, MQTT_KEEP_ALIVE_SECS, MQTT_QUEUE_CAPACITY,
    MQTT_RECONNECT_DELAY_SECS,
};
use intent::{
    discover_projects, has_terminator, parse_intent, strip_terminator, Intent, IntentMatch,
    ParseContext,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use serde_json::json;
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

/// What the accumulator decided after receiving new text.
enum AccumulatorAction {
    /// Finalized — dispatch this command, accumulator is consumed.
    Dispatch(TmuxCommand),
    /// Finalized current buffer AND a new intent was detected — dispatch both.
    DispatchAndNewIntent(TmuxCommand, IntentMatch),
    /// Still accumulating — caller should keep the accumulator.
    Continue(Accumulator),
}

/// Accumulates multi-segment utterances for Claude messages.
/// Buffers text until a terminator keyword ("over", "done", etc.) is heard.
struct Accumulator {
    /// The target project and pane from the initial "tell claude" command.
    session: Option<String>,
    /// Accumulated message segments.
    segments: Vec<String>,
    /// When accumulation started.
    started: Instant,
}

impl Accumulator {
    fn new(session: Option<String>, initial_message: String) -> Self {
        info!(
            "Accumulating claude message for session {:?}: \"{}\"",
            session, initial_message
        );
        Self {
            session,
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

    fn build_command(self) -> TmuxCommand {
        let message = self.segments.join(" ");
        info!("Finalized accumulated message: \"{}\"", message);
        TmuxCommand {
            session: self.session,
            pane: Pane::Claude,
            command: message,
        }
    }

    /// Process incoming text and decide what to do.
    /// This is a pure state machine — no MQTT, no side effects.
    fn receive(mut self, text: &str, ctx: &ParseContext) -> AccumulatorAction {
        // Terminator found — finalize.
        if let Some(stripped) = strip_terminator(text) {
            if !stripped.is_empty() {
                self.append(&stripped);
            }
            return AccumulatorAction::Dispatch(self.build_command());
        }

        // New command detected — flush accumulated, return new intent.
        if let Some(m) = parse_intent(text, ctx) {
            warn!("New command while accumulating — flushing accumulated message");
            return AccumulatorAction::DispatchAndNewIntent(self.build_command(), m);
        }

        // Continuation — append cleaned text.
        let cleaned = text
            .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
            .to_string();
        if !cleaned.is_empty() {
            self.append(&cleaned);
        }
        AccumulatorAction::Continue(self)
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
                        let cmd = acc.build_command();
                        let payload = serde_json::to_string(&cmd).expect("serialize TmuxCommand");
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
    // If we're accumulating, delegate to the state machine.
    if let Some(acc) = accumulator {
        return match acc.receive(text, ctx) {
            AccumulatorAction::Dispatch(cmd) => {
                let payload = serde_json::to_string(&cmd).expect("serialize TmuxCommand");
                dispatch(client, "agent/commands/tmux", &payload).await;
                None
            }
            AccumulatorAction::DispatchAndNewIntent(cmd, m) => {
                let payload = serde_json::to_string(&cmd).expect("serialize TmuxCommand");
                dispatch(client, "agent/commands/tmux", &payload).await;
                handle_new_intent(m, client).await
            }
            AccumulatorAction::Continue(acc) => Some(acc),
        };
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
            publish_response(client, "toast", "warn", &format!("Unknown command: {text}")).await;
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
        Intent::Tmux(cmd) if cmd.pane == Pane::Claude => {
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
                let payload = serde_json::to_string(&final_cmd).expect("serialize TmuxCommand");
                dispatch(client, "agent/commands/tmux", &payload).await;
                None
            } else {
                // Start accumulating — wait for "over".
                Some(Accumulator::new(cmd.session, cmd.command))
            }
        }
        Intent::Tmux(cmd) => {
            let payload = serde_json::to_string(&cmd).expect("serialize TmuxCommand");
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

async fn publish_response(client: &AsyncClient, type_: &str, level: &str, message: &str) {
    let response = json!({
        "source": "agent",
        "type": type_,
        "level": level,
        "message": message,
    });
    let _ = client
        .publish(
            "agent/responses",
            QoS::AtMostOnce,
            false,
            response.to_string(),
        )
        .await;
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
