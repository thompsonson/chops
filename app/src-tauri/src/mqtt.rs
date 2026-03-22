use anyhow::Result;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;
use tracing::{info, warn};

const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";

const SUB_RESPONSES: &str = "agent/responses";
const SUB_COMMANDS: &str = "agent/commands/#";
const SUB_STATUS: &str = "plugins/status/#";

#[derive(Clone, Serialize)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: String,
}

pub struct MqttClient {
    client: Arc<Mutex<Option<AsyncClient>>>,
}

impl MqttClient {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn connect(&self, host: &str, port: u16, app: AppHandle) -> Result<()> {
        // Disconnect existing connection if any
        if let Some(old) = self.client.lock().await.take() {
            let _ = old.disconnect().await;
        }

        let client_id = format!("chops-app-{}", &uuid_simple());
        let mut opts = MqttOptions::new(&client_id, host, port);
        opts.set_keep_alive(Duration::from_secs(30));

        let (client, eventloop) = AsyncClient::new(opts, 10);
        *self.client.lock().await = Some(client);

        // Drive event loop in background with app handle for emitting events
        tokio::spawn(drive_eventloop(eventloop, app));

        info!("MQTT connected to {host}:{port}");
        Ok(())
    }

    pub async fn publish_transcription(&self, text: &str, is_final: bool) -> Result<()> {
        let guard = self.client.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MQTT not connected"))?;

        let payload = json!({
            "text": text,
            "is_final": is_final,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "source": "tauri-app",
        });

        client
            .publish(
                TRANSCRIPTION_TOPIC,
                QoS::AtMostOnce,
                false,
                payload.to_string(),
            )
            .await?;

        if is_final {
            info!("Published transcription: {text}");
        }
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        self.client.lock().await.is_some()
    }

    pub async fn disconnect(&self) {
        if let Some(client) = self.client.lock().await.take() {
            let _ = client.disconnect().await;
            info!("MQTT disconnected");
        }
    }
}

async fn drive_eventloop(mut eventloop: EventLoop, app: AppHandle) {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                info!("MQTT connected, subscribing to topics");
                // Subscribe via the eventloop's client handle
                if let Err(e) = subscribe_topics(&app).await {
                    warn!("Failed to subscribe: {e}");
                }
                // Notify frontend of connection
                let _ = app.emit("mqtt-status", "connected");
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let topic = publish.topic.clone();
                let payload = String::from_utf8_lossy(&publish.payload).to_string();

                // Fire native notification for toast messages on agent/responses
                if topic == SUB_RESPONSES {
                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&payload) {
                        if msg.get("type").and_then(|t| t.as_str()) == Some("toast") {
                            let message = msg
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("notification");
                            let source = msg
                                .get("source")
                                .and_then(|s| s.as_str())
                                .unwrap_or("chops");
                            fire_notification(&app, source, message);
                        }
                    }
                }

                // Forward all messages to frontend
                let _ = app.emit(
                    "mqtt-message",
                    MqttMessage {
                        topic,
                        payload,
                    },
                );
            }
            Ok(Event::Incoming(Packet::Disconnect)) => {
                let _ = app.emit("mqtt-status", "disconnected");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("MQTT event loop error: {e}");
                let _ = app.emit("mqtt-status", "disconnected");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn subscribe_topics(app: &AppHandle) -> Result<()> {
    let state = app.state::<crate::AppState>();
    let guard = state.mqtt.client.lock().await;
    if let Some(client) = guard.as_ref() {
        client.subscribe(SUB_RESPONSES, QoS::AtMostOnce).await?;
        client.subscribe(SUB_COMMANDS, QoS::AtMostOnce).await?;
        client.subscribe(SUB_STATUS, QoS::AtMostOnce).await?;
        info!("Subscribed to {SUB_RESPONSES}, {SUB_COMMANDS}, {SUB_STATUS}");
    }
    Ok(())
}

fn fire_notification(app: &AppHandle, source: &str, message: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app
        .notification()
        .builder()
        .title(format!("chops — {source}"))
        .body(message)
        .show()
    {
        warn!("Failed to show notification: {e}");
    }
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{t:x}")
}
