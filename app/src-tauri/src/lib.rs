mod mqtt;
mod stt;
mod tunnel;

use chops_dev_client::DevClient;
use mqtt::MqttClient;
use stt::SttEngine;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_fs::FsExt;
use tracing::info;
use tunnel::TunnelManagerHandle;

#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt;

pub struct AppState {
    pub mqtt: Arc<MqttClient>,
    stt: Arc<SttEngine>,
    pub dev_client: Arc<DevClient>,
    pub tunnel_mgr: TunnelManagerHandle,
}

impl AppState {
    pub async fn client_for_host(&self, host: Option<&str>) -> Result<DevClient, String> {
        match host {
            None | Some("") | Some("local") => Ok((*self.dev_client).clone()),
            Some(host) => {
                let path = self.tunnel_mgr.0.lock().await.ensure_tunnel(host)?;
                Ok(DevClient::new(path))
            }
        }
    }
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
async fn transcribe_audio(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    samples: Vec<f32>,
) -> Result<String, String> {
    let stt = state.stt.clone();
    let text = tokio::task::spawn_blocking(move || {
        stt.transcribe(&app, &samples)
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(text)
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

fn generate_conversation_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    format!("conv-{ts:x}")
}

#[tauri::command]
async fn send_transcription(
    state: tauri::State<'_, AppState>,
    text: String,
    conversation_id: Option<String>,
) -> Result<String, String> {
    let conv_id = conversation_id.unwrap_or_else(generate_conversation_id);
    state
        .mqtt
        .publish_transcription(&text, true, &conv_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(conv_id)
}

#[tauri::command]
async fn mqtt_ping(state: tauri::State<'_, AppState>) -> Result<u64, String> {
    state
        .mqtt
        .publish_ping()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn escalation_respond(
    state: tauri::State<'_, AppState>,
    workflow: String,
    workflow_id: String,
    step: String,
    passed: bool,
    feedback: Option<String>,
    specification: Option<String>,
    conversation_id: String,
) -> Result<String, String> {
    state
        .mqtt
        .publish_escalation_response(
            &workflow,
            &workflow_id,
            &step,
            passed,
            feedback.as_deref(),
            specification.as_deref(),
            &conversation_id,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(if passed {
        "approved".to_string()
    } else {
        "rejected".to_string()
    })
}

#[tauri::command]
async fn get_status(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (model_exists, model_path) = stt::model_status(&app);
    Ok(serde_json::json!({
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

// --- Updater commands (desktop only, stubs on mobile) ---

fn updater_endpoint(channel: &str) -> &'static str {
    match channel {
        "dev" => "https://github.com/thompsonson/chops/releases/download/dev-desktop/latest.json",
        _ => "https://github.com/thompsonson/chops/releases/latest/download/latest.json",
    }
}

#[cfg(desktop)]
#[tauri::command]
async fn check_for_update(
    app: tauri::AppHandle,
    channel: Option<String>,
) -> Result<serde_json::Value, String> {
    let ch = channel.as_deref().unwrap_or("stable");
    let endpoint = updater_endpoint(ch);
    let url: url::Url = endpoint.parse().map_err(|e: url::ParseError| e.to_string())?;

    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(u) => Ok(serde_json::json!({
            "available": true,
            "version": u.version,
            "body": u.body,
        })),
        None => Ok(serde_json::json!({
            "available": false,
        })),
    }
}

#[cfg(desktop)]
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    channel: Option<String>,
) -> Result<String, String> {
    use tauri::Emitter;

    let ch = channel.as_deref().unwrap_or("stable");
    let endpoint = updater_endpoint(ch);
    let url: url::Url = endpoint.parse().map_err(|e: url::ParseError| e.to_string())?;

    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;

    let app_handle = app.clone();
    let mut downloaded: u64 = 0;

    update
        .download_and_install(
            move |chunk, _total| {
                downloaded += chunk as u64;
                let _ = app_handle.emit("update-progress", serde_json::json!({
                    "downloaded": downloaded,
                    "total": _total,
                }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    app.restart();
}

#[cfg(mobile)]
#[tauri::command]
async fn check_for_update(
    _app: tauri::AppHandle,
    _channel: Option<String>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "available": false, "message": "Updates not available on mobile" }))
}

#[cfg(mobile)]
#[tauri::command]
async fn install_update(
    _app: tauri::AppHandle,
    _channel: Option<String>,
) -> Result<String, String> {
    Err("Updates not available on mobile".to_string())
}

// -- Dev session commands ---------------------------------------------------

#[tauri::command]
async fn list_sessions(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
) -> Result<chops_dev_client::Listing, String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.list().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn inspect_session(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    name: String,
    lines: Option<u32>,
) -> Result<serde_json::Value, String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.inspect(&name, lines, None).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn pane_content(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    name: String,
    pane: String,
    lines: Option<u32>,
) -> Result<serde_json::Value, String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.pane_content(&name, &pane, lines).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_keys(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    name: String,
    pane: String,
    keys: String,
) -> Result<String, String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.send_keys(&name, &pane, &keys).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_session(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    project: String,
    layout: Option<String>,
) -> Result<String, String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.start(&project, layout.as_deref()).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_session(
    state: tauri::State<'_, AppState>,
    host: Option<String>,
    name: String,
) -> Result<(), String> {
    let client = state.client_for_host(host.as_deref()).await?;
    client.stop(&name).await.map_err(|e| e.to_string())
}

// -- Host management -------------------------------------------------------

#[tauri::command]
async fn tunnel_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<tunnel::TunnelStatus>, String> {
    Ok(state.tunnel_mgr.0.lock().await.status())
}

#[tauri::command]
async fn list_hosts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let mut mgr = state.tunnel_mgr.0.lock().await;
    Ok(mgr.status().into_iter().map(|s| s.host).collect())
}

// -- Android SSH key management (provisioning) --------------------------------

#[cfg(target_os = "android")]
#[tauri::command]
async fn ssh_generate_key(
    app: tauri::AppHandle,
    host: String,
) -> Result<String, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let (priv_key, pub_key) = tunnel::generate_ssh_key()?;
    let alias = host.replace('.', "_");
    tunnel::store_ssh_key(&app_data, &alias, &priv_key)?;
    Ok(pub_key)
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn ssh_key_status(
    app: tauri::AppHandle,
    host: String,
) -> Result<bool, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let alias = host.replace('.', "_");
    Ok(tunnel::has_ssh_key(&app_data, &alias))
}

#[cfg(target_os = "android")]
#[tauri::command]
async fn ssh_remove_key(
    app: tauri::AppHandle,
    host: String,
) -> Result<(), String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let alias = host.replace('.', "_");
    tunnel::delete_ssh_key(&app_data, &alias);
    Ok(())
}

pub fn run() {
    tracing_subscriber::fmt::init();

    let mqtt = Arc::new(MqttClient::new());
    let stt = Arc::new(SttEngine::new());
    let dev_client = Arc::new(DevClient::from_env());
    let tunnel_mgr = TunnelManagerHandle::new();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .setup(move |app| {
            let state = AppState {
                mqtt: mqtt.clone(),
                stt: stt.clone(),
                dev_client: dev_client.clone(),
                tunnel_mgr: tunnel_mgr.clone(),
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

            #[cfg(target_os = "android")]
            {
                let app_data = app.path().app_data_dir().ok();
                if let Some(path) = app_data {
                    let mut mgr = tunnel_mgr.0.lock().await;
                    mgr.set_app_data(path);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            transcribe_audio,
            connect_mqtt,
            send_transcription,
            mqtt_ping,
            escalation_respond,
            get_status,
            set_model_path,
            get_model_path,
            import_model,
            check_for_update,
            install_update,
            list_sessions,
            inspect_session,
            pane_content,
            send_keys,
            start_session,
            stop_session,
            tunnel_status,
            list_hosts,
            #[cfg(target_os = "android")]
            ssh_generate_key,
            #[cfg(target_os = "android")]
            ssh_key_status,
            #[cfg(target_os = "android")]
            ssh_remove_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn mobile_entry() {
    run();
}
