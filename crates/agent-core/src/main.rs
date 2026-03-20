use anyhow::Result;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use std::time::Duration;
use tracing::{info, warn};

const MQTT_HOST: &str = "localhost";
const MQTT_PORT: u16 = 1883;
const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";

#[derive(Deserialize)]
struct TranscriptionMessage {
    text: String,
    is_final: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut mqttoptions = MqttOptions::new("agent-core", MQTT_HOST, MQTT_PORT);
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
                        route_intent(&transcription.text, &client).await;
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

async fn route_intent(text: &str, client: &AsyncClient) {
    let text_lower = text.to_lowercase();
    info!("Routing intent for: {text}");

    // Fast path: keyword matching.
    // Extend this match block to add new intent rules.
    if text_lower.contains("open") && text_lower.contains("vscode") {
        dispatch(client, "agent/commands/vscode", text).await;
    } else if text_lower.contains("termux") || text_lower.contains("terminal") {
        dispatch(client, "agent/commands/termux", text).await;
    } else {
        // Unhandled — log for future LLM training data or rule additions.
        warn!("Unhandled intent: {text}");
        // TODO: slow path — query local LLM (Ollama) here once pipeline is validated.
    }
}

async fn dispatch(client: &AsyncClient, topic: &str, payload: &str) {
    info!("Dispatching to {topic}: {payload}");
    if let Err(e) = client
        .publish(topic, QoS::AtMostOnce, false, payload)
        .await
    {
        warn!("Failed to dispatch to {topic}: {e}");
    }
}
