mod intent;

use anyhow::Result;
use intent::{discover_projects, parse_intent, Intent, ParseContext};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

const MQTT_HOST: &str = "localhost";
const DEFAULT_MQTT_PORT: u16 = 1884;
const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";

fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}

#[derive(Deserialize)]
struct TranscriptionMessage {
    text: String,
    is_final: bool,
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

    let mut mqttoptions = MqttOptions::new("agent-core", MQTT_HOST, mqtt_port());
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    client
        .subscribe(TRANSCRIPTION_TOPIC, QoS::AtMostOnce)
        .await?;
    info!("Agent subscribed to {TRANSCRIPTION_TOPIC}");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(msg))) => {
                let payload = std::str::from_utf8(&msg.payload)?;
                if let Ok(transcription) = serde_json::from_str::<TranscriptionMessage>(payload) {
                    if transcription.is_final {
                        route_intent(&transcription.text, &client, &ctx).await;
                    }
                } else {
                    warn!("Failed to deserialize transcription: {payload}");
                }
            }
            Err(e) => {
                warn!("MQTT error: {e}. Reconnecting...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            _ => {}
        }
    }
}

async fn route_intent(text: &str, client: &AsyncClient, ctx: &ParseContext) {
    info!("Routing intent for: {text}");

    match parse_intent(text, ctx) {
        Some(m) => {
            info!(
                "Matched intent (confidence: {:.2}): {:?}",
                m.confidence, m.intent
            );
            match m.intent {
                Intent::Tmux(cmd) => {
                    let payload = serde_json::to_string(&cmd).unwrap();
                    dispatch(client, "agent/commands/tmux", &payload).await;
                }
                Intent::Vscode(payload) => {
                    dispatch(client, "agent/commands/vscode", &payload).await;
                }
                Intent::Termux(payload) => {
                    dispatch(client, "agent/commands/termux", &payload).await;
                }
            }
        }
        None => {
            warn!("Unhandled intent: {text}");
            // TODO: slow path — query local LLM (Ollama) here once pipeline is validated.
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
