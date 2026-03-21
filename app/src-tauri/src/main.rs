// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mqtt;
mod stt;

use mqtt::MqttClient;
use stt::SttEngine;
use std::sync::Arc;
use tauri::Manager;
use tracing::info;

struct AppState {
    mqtt: Arc<MqttClient>,
    stt: Arc<SttEngine>,
}

const DEFAULT_MQTT_HOST: &str = "localhost";
const DEFAULT_MQTT_PORT: u16 = 1884;

fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}

#[tauri::command]
async fn start_listening(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .stt
        .start(app, state.mqtt.clone())
        .await
        .map_err(|e| e.to_string())?;
    Ok("listening".to_string())
}

#[tauri::command]
async fn stop_listening(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state.stt.stop().await;
    Ok("stopped".to_string())
}

#[tauri::command]
async fn connect_mqtt(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    port: Option<u16>,
) -> Result<String, String> {
    let host = host.unwrap_or_else(|| DEFAULT_MQTT_HOST.to_string());
    let port = port.unwrap_or_else(mqtt_port);
    state
        .mqtt
        .connect(&host, port)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("connected to {host}:{port}"))
}

#[tauri::command]
async fn send_transcription(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<String, String> {
    state
        .mqtt
        .publish_transcription(&text, true)
        .await
        .map_err(|e| e.to_string())?;
    Ok("sent".to_string())
}

#[tauri::command]
async fn get_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (model_exists, model_path) = stt::model_status();
    Ok(serde_json::json!({
        "listening": state.stt.is_listening(),
        "mqtt_connected": state.mqtt.is_connected().await,
        "model_exists": model_exists,
        "model_path": model_path,
    }))
}

fn main() {
    tracing_subscriber::fmt::init();

    let mqtt = Arc::new(MqttClient::new());
    let stt = Arc::new(SttEngine::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            let state = AppState {
                mqtt: mqtt.clone(),
                stt: stt.clone(),
            };

            // Auto-connect MQTT on startup
            let mqtt_for_connect = mqtt.clone();
            let port = mqtt_port();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = mqtt_for_connect.connect(DEFAULT_MQTT_HOST, port).await {
                    info!("MQTT auto-connect failed (will retry on demand): {e}");
                }
            });

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_listening,
            stop_listening,
            connect_mqtt,
            send_transcription,
            get_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
