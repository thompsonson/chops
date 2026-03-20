use anyhow::Result;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde_json::json;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{info, warn};

const SILENCE_THRESHOLD_MS: u64 = 800;
const MQTT_HOST: &str = "localhost";
const DEFAULT_MQTT_PORT: u16 = 1884;
const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";
const WHISPER_MODEL: &str = "models/ggml-base.en.bin";
const WHISPER_STEP_MS: &str = "1000";
const WHISPER_LENGTH_MS: &str = "3000";

fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}

struct TranscriptionBuffer {
    pending: String,
    last_update: Instant,
    silence_threshold: Duration,
}

impl TranscriptionBuffer {
    fn new() -> Self {
        Self {
            pending: String::new(),
            last_update: Instant::now(),
            silence_threshold: Duration::from_millis(SILENCE_THRESHOLD_MS),
        }
    }

    /// Overwrite pending text with the latest partial from whisper.cpp.
    fn update(&mut self, text: &str) {
        self.pending = text.trim().to_string();
        self.last_update = Instant::now();
    }

    /// True when silence has exceeded the threshold — treat as finalized.
    fn is_finalized(&self) -> bool {
        !self.pending.is_empty() && self.last_update.elapsed() > self.silence_threshold
    }

    /// Drain the buffer and return the finalized text.
    fn take(&mut self) -> String {
        let text = self.pending.clone();
        self.pending.clear();
        text
    }

    /// Heuristic: punctuation at end of line suggests whisper considers the
    /// segment complete. Use as a secondary (weak) signal only.
    fn looks_like_segment_end(text: &str) -> bool {
        matches!(text.chars().last(), Some('.' | '!' | '?'))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut mqttoptions = MqttOptions::new("stt-publisher", MQTT_HOST, mqtt_port());
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    // Drive the MQTT event loop in the background.
    tokio::spawn(async move {
        loop {
            if let Err(e) = eventloop.poll().await {
                warn!("MQTT event loop error: {e}");
            }
        }
    });

    // Supervisor loop: restart whisper.cpp if it crashes.
    loop {
        info!("Starting whisper.cpp stream...");
        if let Err(e) = run_whisper(&client).await {
            warn!("whisper.cpp exited with error: {e}. Restarting in 2s...");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

async fn run_whisper(client: &AsyncClient) -> Result<()> {
    let mut child = Command::new("./whisper.cpp/stream")
        .args([
            "-m",
            WHISPER_MODEL,
            "--step",
            WHISPER_STEP_MS,
            "--length",
            WHISPER_LENGTH_MS,
        ])
        .stdout(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut buffer = TranscriptionBuffer::new();

    // Poll lines and check silence timeout concurrently.
    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line? {
                    Some(text) if !text.trim().is_empty() => {
                        let text = text.trim().to_string();
                        // Skip whisper.cpp timestamp lines like "[00:00:00.000 --> ...]"
                        if text.starts_with('[') && text.contains("-->") {
                            continue;
                        }
                        let is_segment_end = TranscriptionBuffer::looks_like_segment_end(&text);
                        buffer.update(&text);
                        if is_segment_end {
                            publish_final(client, buffer.take()).await;
                        } else {
                            // Publish partial for optional UI feedback.
                            publish_partial(client, &buffer.pending).await;
                        }
                    }
                    None => break, // stdout closed
                    _ => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if buffer.is_finalized() {
                    publish_final(client, buffer.take()).await;
                }
            }
        }
    }

    child.wait().await?;
    Ok(())
}

async fn publish_final(client: &AsyncClient, text: String) {
    if text.is_empty() {
        return;
    }
    info!("Finalized: {text}");
    let payload = json!({
        "text": text,
        "is_final": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = client
        .publish(
            TRANSCRIPTION_TOPIC,
            QoS::AtMostOnce,
            false,
            payload.to_string(),
        )
        .await
    {
        warn!("Failed to publish final transcription: {e}");
    }
}

async fn publish_partial(client: &AsyncClient, text: &str) {
    let payload = json!({
        "text": text,
        "is_final": false,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = client
        .publish(
            TRANSCRIPTION_TOPIC,
            QoS::AtMostOnce,
            false,
            payload.to_string(),
        )
        .await
    {
        warn!("Failed to publish partial: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_starts_empty() {
        let buf = TranscriptionBuffer::new();
        assert!(buf.pending.is_empty());
        assert!(!buf.is_finalized());
    }

    #[test]
    fn buffer_update_replaces_pending() {
        let mut buf = TranscriptionBuffer::new();
        buf.update("hello");
        assert_eq!(buf.pending, "hello");
        buf.update("hello world");
        assert_eq!(buf.pending, "hello world");
    }

    #[test]
    fn buffer_update_trims_whitespace() {
        let mut buf = TranscriptionBuffer::new();
        buf.update("  hello world  ");
        assert_eq!(buf.pending, "hello world");
    }

    #[test]
    fn buffer_take_drains() {
        let mut buf = TranscriptionBuffer::new();
        buf.update("hello");
        let text = buf.take();
        assert_eq!(text, "hello");
        assert!(buf.pending.is_empty());
    }

    #[test]
    fn buffer_not_finalized_immediately() {
        let mut buf = TranscriptionBuffer::new();
        buf.update("hello");
        // Just updated — should not be finalized yet.
        assert!(!buf.is_finalized());
    }

    #[test]
    fn buffer_finalized_after_silence() {
        let buf = TranscriptionBuffer {
            pending: "hello".to_string(),
            last_update: Instant::now() - Duration::from_secs(2),
            silence_threshold: Duration::from_millis(SILENCE_THRESHOLD_MS),
        };
        assert!(buf.is_finalized());
    }

    #[test]
    fn buffer_empty_never_finalized() {
        let buf = TranscriptionBuffer {
            pending: String::new(),
            last_update: Instant::now() - Duration::from_secs(10),
            silence_threshold: Duration::from_millis(SILENCE_THRESHOLD_MS),
        };
        assert!(!buf.is_finalized());
    }

    #[test]
    fn segment_end_detection() {
        assert!(TranscriptionBuffer::looks_like_segment_end("hello."));
        assert!(TranscriptionBuffer::looks_like_segment_end("what?"));
        assert!(TranscriptionBuffer::looks_like_segment_end("wow!"));
        assert!(!TranscriptionBuffer::looks_like_segment_end("hello"));
        assert!(!TranscriptionBuffer::looks_like_segment_end("hello,"));
        assert!(!TranscriptionBuffer::looks_like_segment_end(""));
    }
}
