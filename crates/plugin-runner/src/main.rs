use anyhow::Result;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::json;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{info, warn};

const MQTT_HOST: &str = "localhost";
const MQTT_PORT: u16 = 1883;
const VSCODE_TOPIC: &str = "agent/commands/vscode";
const TERMUX_TOPIC: &str = "agent/commands/termux";
const RESPONSES_TOPIC: &str = "agent/responses";
const STATUS_TOPIC: &str = "plugins/status/runner";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut mqttoptions = MqttOptions::new("plugin-runner", MQTT_HOST, MQTT_PORT);
    mqttoptions.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    client.subscribe(VSCODE_TOPIC, QoS::AtMostOnce).await?;
    client.subscribe(TERMUX_TOPIC, QoS::AtMostOnce).await?;

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
                        .publish(RESPONSES_TOPIC, QoS::AtLeastOnce, false, response.to_string())
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
