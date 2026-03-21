use anyhow::Result;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};

const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";

pub struct MqttClient {
    client: Arc<Mutex<Option<AsyncClient>>>,
}

impl MqttClient {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn connect(&self, host: &str, port: u16) -> Result<()> {
        let client_id = format!(
            "chops-app-{}",
            &uuid_simple()
        );
        let mut opts = MqttOptions::new(&client_id, host, port);
        opts.set_keep_alive(Duration::from_secs(30));

        let (client, eventloop) = AsyncClient::new(opts, 10);
        *self.client.lock().await = Some(client);

        // Drive event loop in background
        tokio::spawn(drive_eventloop(eventloop));

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

async fn drive_eventloop(mut eventloop: EventLoop) {
    loop {
        match eventloop.poll().await {
            Ok(_) => {}
            Err(e) => {
                warn!("MQTT event loop error: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
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
