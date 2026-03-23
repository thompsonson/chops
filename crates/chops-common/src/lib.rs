use serde::{Deserialize, Serialize};

pub const DEFAULT_MQTT_HOST: &str = "localhost";
pub const DEFAULT_MQTT_PORT: u16 = 1884;
pub const MQTT_KEEP_ALIVE_SECS: u64 = 30;
pub const MQTT_RECONNECT_DELAY_SECS: u64 = 2;
pub const MQTT_QUEUE_CAPACITY: usize = 10;

pub fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}

/// Target pane in a tmux session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pane {
    Claude,
    Shell,
}

impl std::fmt::Display for Pane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pane::Claude => write!(f, "claude"),
            Pane::Shell => write!(f, "shell"),
        }
    }
}

/// Structured command for the tmux plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TmuxCommand {
    /// Target tmux session name (e.g., "chops"). None = active session.
    pub session: Option<String>,
    /// Target pane: Claude (left, :1.1) or Shell (right, :1.2).
    pub pane: Pane,
    /// The command text to send via send-keys.
    pub command: String,
}
