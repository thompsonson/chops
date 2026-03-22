mod mqtt;
mod stt;

use mqtt::MqttClient;
use stt::SttEngine;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_fs::FsExt;
use tracing::info;

pub struct AppState {
    pub mqtt: Arc<MqttClient>,
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
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    port: Option<u16>,
) -> Result<String, String> {
    let host = host.unwrap_or_else(|| DEFAULT_MQTT_HOST.to_string());
    let port = port.unwrap_or_else(mqtt_port);
    state
        .mqtt
        .connect(&host, port, app)
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
async fn get_status(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (model_exists, model_path) = stt::model_status(&app);
    Ok(serde_json::json!({
        "listening": state.stt.is_listening(),
        "mqtt_connected": state.mqtt.is_connected().await,
        "model_exists": model_exists,
        "model_path": model_path,
    }))
}

#[tauri::command]
async fn set_model_path(app: tauri::AppHandle, path: String) -> Result<String, String> {
    stt::set_model_path_config(&app, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Import a model file from a content:// URI or filesystem path into the app data directory.
#[tauri::command]
async fn import_model(app: tauri::AppHandle, uri: String) -> Result<String, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let dest = data_dir.join("ggml-base.en.bin");

    // Parse URI and read via tauri-plugin-fs (handles content:// on Android)
    let file_path: tauri_plugin_fs::FilePath = uri
        .parse()
        .map_err(|e: std::convert::Infallible| e.to_string())?;
    let contents = app
        .fs()
        .read(file_path)
        .map_err(|e| format!("Failed to read model: {e}"))?;

    std::fs::write(&dest, &contents).map_err(|e| format!("Failed to write model: {e}"))?;

    // Clear custom path config so it uses the default data dir location
    stt::set_model_path_config(&app, "").map_err(|e| e.to_string())?;

    info!(
        "Imported model ({} bytes) to {}",
        contents.len(),
        dest.display()
    );
    Ok(dest.display().to_string())
}

#[tauri::command]
async fn get_model_path(app: tauri::AppHandle) -> Result<String, String> {
    Ok(stt::get_model_path_config(&app))
}

// --- Session management commands (Phase 4) ---

#[tauri::command]
async fn get_sessions() -> Result<String, String> {
    let output = tokio::process::Command::new("dev")
        .arg("list")
        .output()
        .await
        .map_err(|e| format!("Failed to run dev list: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dev list failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
async fn start_session(project: String) -> Result<String, String> {
    let output = tokio::process::Command::new("dev")
        .args(["start", &project])
        .output()
        .await
        .map_err(|e| format!("Failed to run dev start: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dev start failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
async fn stop_session(session: String) -> Result<String, String> {
    let output = tokio::process::Command::new("dev")
        .args(["stop", &session])
        .output()
        .await
        .map_err(|e| format!("Failed to run dev stop: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("dev stop failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn run() {
    tracing_subscriber::fmt::init();

    let mqtt = Arc::new(MqttClient::new());
    let stt = Arc::new(SttEngine::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let state = AppState {
                mqtt: mqtt.clone(),
                stt: stt.clone(),
            };

            // Auto-connect MQTT on startup (skip on mobile — no local broker)
            if !cfg!(mobile) {
                let mqtt_for_connect = mqtt.clone();
                let port = mqtt_port();
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = mqtt_for_connect.connect(DEFAULT_MQTT_HOST, port, app_handle).await {
                        info!("MQTT auto-connect failed (will retry on demand): {e}");
                    }
                });
            }

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_listening,
            stop_listening,
            connect_mqtt,
            send_transcription,
            get_status,
            set_model_path,
            get_model_path,
            import_model,
            get_sessions,
            start_session,
            stop_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn mobile_entry() {
    run();
}
