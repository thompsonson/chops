use anyhow::Result;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
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

/// Structured command for the tmux plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TmuxCommand {
    /// Target tmux session name (e.g., "chops"). None = active session.
    session: Option<String>,
    /// Target pane: "shell" (right, :1.2) or "claude" (left, :1.1).
    pane: String,
    /// The command text to send via send-keys.
    command: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

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

/// Routing decision returned by parse_intent.
#[derive(Debug, Clone, PartialEq)]
enum Intent {
    Vscode(String),
    Termux(String),
    Tmux(TmuxCommand),
}

/// Parse intent from text. Pure function — no side effects, fully testable.
///
/// Supported patterns:
///   "in <project> run <command>"  → tmux send-keys to project's shell pane
///   "in <project> tell claude <msg>" → tmux send-keys to project's claude pane
///   "run <command>"               → tmux send-keys to active session's shell pane
///   "open vscode <file>"          → vscode
///   "termux/terminal ..."         → termux
fn parse_intent(text: &str) -> Option<Intent> {
    let text_lower = text.to_lowercase();
    let words: Vec<&str> = text_lower.split_whitespace().collect();

    // Pattern: "in <project> run <command>"
    // Pattern: "in <project> tell claude <message>"
    if words.first() == Some(&"in") && words.len() >= 4 {
        let project = words[1].to_string();

        if words[2] == "tell" && words[3] == "claude" && words.len() > 4 {
            // Reconstruct original-case command after "tell claude".
            let prefix_len = find_nth_word_end(text, 4);
            let command = text[prefix_len..].trim().to_string();
            return Some(Intent::Tmux(TmuxCommand {
                session: Some(project),
                pane: "claude".to_string(),
                command,
            }));
        }

        if words[2] == "run" && words.len() > 3 {
            let prefix_len = find_nth_word_end(text, 3);
            let command = text[prefix_len..].trim().to_string();
            return Some(Intent::Tmux(TmuxCommand {
                session: Some(project),
                pane: "shell".to_string(),
                command,
            }));
        }
    }

    // Pattern: "run <command>" (no project → active session)
    if words.first() == Some(&"run") && words.len() > 1 {
        let prefix_len = find_nth_word_end(text, 1);
        let command = text[prefix_len..].trim().to_string();
        return Some(Intent::Tmux(TmuxCommand {
            session: None,
            pane: "shell".to_string(),
            command,
        }));
    }

    // Legacy patterns
    if text_lower.contains("open") && text_lower.contains("vscode") {
        return Some(Intent::Vscode(text.to_string()));
    }
    if text_lower.contains("termux") || text_lower.contains("terminal") {
        return Some(Intent::Termux(text.to_string()));
    }

    None
}

/// Find the byte offset just past the first `n` whitespace-delimited words.
/// E.g., find_nth_word_end("in chops run cargo test", 3) returns the index
/// of the space before "cargo", so text[result..].trim() == "cargo test".
fn find_nth_word_end(text: &str, n: usize) -> usize {
    let mut words_seen = 0;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if in_word {
                words_seen += 1;
                if words_seen == n {
                    return i;
                }
            }
            in_word = false;
        } else {
            in_word = true;
        }
    }
    text.len()
}

async fn route_intent(text: &str, client: &AsyncClient) {
    info!("Routing intent for: {text}");

    match parse_intent(text) {
        Some(Intent::Tmux(cmd)) => {
            let payload = serde_json::to_string(&cmd).unwrap();
            dispatch(client, "agent/commands/tmux", &payload).await;
        }
        Some(Intent::Vscode(payload)) => {
            dispatch(client, "agent/commands/vscode", &payload).await;
        }
        Some(Intent::Termux(payload)) => {
            dispatch(client, "agent/commands/termux", &payload).await;
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

    // --- Tmux routing: "in <project> run <command>" ---

    #[test]
    fn routes_in_project_run_command() {
        assert_eq!(
            parse_intent("in chops run cargo test"),
            Some(Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test".into(),
            }))
        );
    }

    #[test]
    fn routes_in_project_run_preserves_case() {
        assert_eq!(
            parse_intent("In Chops Run cargo test --release"),
            Some(Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "shell".into(),
                command: "cargo test --release".into(),
            }))
        );
    }

    #[test]
    fn routes_in_project_tell_claude() {
        assert_eq!(
            parse_intent("in chops tell claude fix the tests"),
            Some(Intent::Tmux(TmuxCommand {
                session: Some("chops".into()),
                pane: "claude".into(),
                command: "fix the tests".into(),
            }))
        );
    }

    // --- Tmux routing: "run <command>" (active session) ---

    #[test]
    fn routes_run_to_active_session() {
        assert_eq!(
            parse_intent("run cargo build"),
            Some(Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".into(),
                command: "cargo build".into(),
            }))
        );
    }

    #[test]
    fn run_preserves_full_command() {
        assert_eq!(
            parse_intent("run git log --oneline -10"),
            Some(Intent::Tmux(TmuxCommand {
                session: None,
                pane: "shell".into(),
                command: "git log --oneline -10".into(),
            }))
        );
    }

    // --- Legacy: vscode ---

    #[test]
    fn routes_vscode_open() {
        assert!(matches!(
            parse_intent("open vscode README.md"),
            Some(Intent::Vscode(_))
        ));
    }

    #[test]
    fn routes_vscode_case_insensitive() {
        assert!(matches!(
            parse_intent("Open VSCode main.rs"),
            Some(Intent::Vscode(_))
        ));
    }

    // --- Legacy: termux ---

    #[test]
    fn routes_terminal() {
        assert!(matches!(
            parse_intent("open terminal"),
            Some(Intent::Termux(_))
        ));
    }

    // --- Unhandled ---

    #[test]
    fn unhandled_returns_none() {
        assert_eq!(parse_intent("what time is it"), None);
        assert_eq!(parse_intent("play music"), None);
    }

    // --- Deserialization ---

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

    // --- TmuxCommand serialization round-trip ---

    #[test]
    fn tmux_command_serializes() {
        let cmd = TmuxCommand {
            session: Some("chops".into()),
            pane: "shell".into(),
            command: "cargo test".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: TmuxCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cmd);
    }
}
