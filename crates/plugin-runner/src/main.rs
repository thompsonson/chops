use anyhow::Result;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{info, warn};

const MQTT_HOST: &str = "localhost";
const DEFAULT_MQTT_PORT: u16 = 1884;

fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}
const VSCODE_TOPIC: &str = "agent/commands/vscode";
const TERMUX_TOPIC: &str = "agent/commands/termux";
const TMUX_TOPIC: &str = "agent/commands/tmux";
const RESPONSES_TOPIC: &str = "agent/responses";
const STATUS_TOPIC: &str = "plugins/status/runner";

/// Matches the TmuxCommand struct from agent-core.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TmuxCommand {
    session: Option<String>,
    pane: String,
    command: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut mqttoptions = MqttOptions::new("plugin-runner", MQTT_HOST, mqtt_port());
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    client.subscribe(VSCODE_TOPIC, QoS::AtMostOnce).await?;
    client.subscribe(TERMUX_TOPIC, QoS::AtMostOnce).await?;
    client.subscribe(TMUX_TOPIC, QoS::AtMostOnce).await?;

    // Announce availability.
    client
        .publish(STATUS_TOPIC, QoS::AtMostOnce, false, "online")
        .await?;
    info!("Plugin runner online");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(msg))) => {
                let topic = msg.topic.clone();
                let payload = std::str::from_utf8(&msg.payload)?.to_string();
                let client_clone = client.clone();

                // Spawn each plugin execution as a separate task to avoid blocking.
                tokio::spawn(async move {
                    let result = match topic.as_str() {
                        VSCODE_TOPIC => handle_vscode(&payload).await,
                        TERMUX_TOPIC => handle_termux(&payload).await,
                        TMUX_TOPIC => handle_tmux(&payload).await,
                        _ => Err(anyhow::anyhow!("Unknown topic: {topic}")),
                    };

                    let response = match result {
                        Ok(output) => json!({
                            "topic": topic,
                            "status": "ok",
                            "output": output,
                        }),
                        Err(e) => json!({
                            "topic": topic,
                            "status": "error",
                            "error": e.to_string(),
                        }),
                    };

                    if let Err(e) = client_clone
                        .publish(
                            RESPONSES_TOPIC,
                            QoS::AtLeastOnce,
                            false,
                            response.to_string(),
                        )
                        .await
                    {
                        warn!("Failed to publish response: {e}");
                    }
                });
            }
            Err(e) => {
                warn!("MQTT error: {e}. Reconnecting...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            _ => {}
        }
    }
}

/// Send keys to a tmux session pane.
///
/// Pane mapping (matches `dev` command layouts):
///   "claude" → session:1.1 (left pane)
///   "shell"  → session:1.2 (right pane in claude layout, or 1.1 in default)
///
/// If no session is specified, queries tmux for the currently attached session.
async fn handle_tmux(payload: &str) -> Result<String> {
    let cmd: TmuxCommand = serde_json::from_str(payload)?;
    info!(
        "Tmux command: session={:?} pane={} cmd={}",
        cmd.session, cmd.pane, cmd.command
    );

    let session = match cmd.session {
        Some(s) => s,
        None => get_active_tmux_session().await?,
    };

    // Verify the session exists.
    let check = run_command("tmux", &["has-session", "-t", &session]).await;
    if check.is_err() {
        return Err(anyhow::anyhow!("tmux session '{}' not found", session));
    }

    // Count panes in the session to determine layout.
    let pane_count = run_command(
        "tmux",
        &[
            "list-panes",
            "-t",
            &format!("{}:1", session),
            "-F",
            "#{pane_index}",
        ],
    )
    .await
    .map(|out| out.lines().count())
    .unwrap_or(1);

    // Map pane name to tmux target.
    // In a 2-pane (claude) layout: claude=1, shell=2
    // In a 1-pane (default) layout: everything goes to pane 1
    let pane_index = if pane_count == 1 {
        "1"
    } else {
        match cmd.pane.as_str() {
            "claude" => "1",
            _ => "2",
        }
    };

    let target = format!("{}:1.{}", session, pane_index);

    let output = run_command("tmux", &["send-keys", "-t", &target, &cmd.command, "Enter"]).await?;

    Ok(format!("Sent to {}: {}", target, output))
}

/// Get the currently attached tmux session name.
async fn get_active_tmux_session() -> Result<String> {
    let output = run_command("tmux", &["display-message", "-p", "#{session_name}"]).await?;
    let name = output.trim().to_string();
    if name.is_empty() {
        Err(anyhow::anyhow!("No active tmux session"))
    } else {
        Ok(name)
    }
}

async fn handle_vscode(command: &str) -> Result<String> {
    info!("VSCode command: {command}");
    // Extract a file argument from the command text if present.
    // Example: "open vscode README.md" → open README.md in VSCode.
    let parts: Vec<&str> = command.split_whitespace().collect();
    let file_arg = parts.last().copied().unwrap_or(".");

    let output = run_command("code", &[file_arg]).await?;
    Ok(output)
}

async fn handle_termux(command: &str) -> Result<String> {
    info!("Termux command: {command}");
    // Shell out to bash. Extend with termux-api calls for Android.
    let output = run_command("bash", &["-c", command]).await?;
    Ok(output)
}

/// Run a process, capture stdout/stderr, enforce a timeout.
async fn run_command(program: &str, args: &[&str]) -> Result<String> {
    let timeout = Duration::from_secs(10);

    let child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if output.status.success() {
                Ok(stdout)
            } else {
                Err(anyhow::anyhow!("Process failed: {stderr}"))
            }
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Process error: {e}")),
        Err(_) => Err(anyhow::anyhow!("Process timed out after {timeout:?}")),
    }
}
