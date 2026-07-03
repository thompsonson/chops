use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chops_dev_client::{DevClient, Error as DevError};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::info;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn json_response(
    status: StatusCode,
    body: String,
) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    (status, [("content-type", "application/json")], body)
}

fn json_ok(body: String) -> JsonResponse {
    json_response(StatusCode::OK, body)
}

fn json_bad(msg: &str) -> JsonResponse {
    json_response(StatusCode::BAD_REQUEST, format!(r#"{{"error":"{}"}}"#, msg))
}

/// Map a DevClient error to an appropriate HTTP response.
fn dev_err(e: DevError) -> JsonResponse {
    match &e {
        DevError::Connect { .. } => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(r#"{{"error":"dev daemon unreachable","detail":"{}"}}"#, e),
        ),
        DevError::DaemonError { status, body } => json_response(
            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY),
            body.clone(),
        ),
        _ => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(r#"{{"error":"{}"}}"#, e),
        ),
    }
}

type JsonResponse = (StatusCode, [(&'static str, &'static str); 1], String);

fn validate_name(name: &str) -> Result<String, JsonResponse> {
    let name = name.trim().to_string();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(json_bad("invalid session name"));
    }
    Ok(name)
}

/// GET /api/sessions — list active sessions and available projects.
async fn api_sessions() -> impl IntoResponse {
    let client = DevClient::from_env();
    match client.list().await {
        Ok(listing) => match serde_json::to_string(&listing) {
            Ok(body) => json_ok(body),
            Err(e) => dev_err(DevError::Json(e)),
        },
        Err(e) => dev_err(e),
    }
}

#[derive(Deserialize)]
struct SessionParams {
    session: String,
}

#[derive(Deserialize)]
struct StartParams {
    project: String,
    layout: Option<String>,
}

/// POST /api/sessions/switch?session=<name> — switch ttyd to a different session.
async fn api_switch_session(Query(params): Query<SessionParams>) -> impl IntoResponse {
    let session = match validate_name(&params.session) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let home = dirs::home_dir().unwrap_or_default();
    let state_file = home.join(".config/chops/ttyd-session");

    match tokio::fs::write(&state_file, &session).await {
        Ok(_) => {
            info!("Switched ttyd session to: {session}");
            json_ok(format!(r#"{{"session":"{}"}}"#, session))
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(r#"{{"error":"{}"}}"#, e),
        ),
    }
}

/// POST /api/sessions/start?project=<name>&layout=<layout> — start a new session.
async fn api_start_session(Query(params): Query<StartParams>) -> impl IntoResponse {
    let project = match validate_name(&params.project) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let client = DevClient::from_env();
    match client.start(&project, params.layout.as_deref()).await {
        Ok(output) => {
            info!("Started session: {project}");
            json_ok(format!(
                r#"{{"project":"{}","output":"{}"}}"#,
                project,
                output.trim()
            ))
        }
        Err(e) => dev_err(e),
    }
}

/// POST /api/sessions/stop?session=<name> — stop (kill) a session.
async fn api_stop_session(Query(params): Query<SessionParams>) -> impl IntoResponse {
    let session = match validate_name(&params.session) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let client = DevClient::from_env();
    match client.stop(&session).await {
        Ok(()) => {
            info!("Stopped session: {session}");
            json_ok(format!(r#"{{"session":"{}","output":"stopped"}}"#, session))
        }
        Err(e) => dev_err(e),
    }
}

#[derive(Deserialize)]
struct KeysPayload {
    keys: String,
}

/// POST /api/sessions/:name/panes/:pane/keys — send keystrokes to a tmux pane.
async fn api_send_keys(
    Path((session, pane)): Path<(String, String)>,
    Json(payload): Json<KeysPayload>,
) -> impl IntoResponse {
    let session = match validate_name(&session) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let client = DevClient::from_env();
    match client.send_keys(&session, &pane, &payload.keys).await {
        Ok(body) => {
            info!("Sent keys to {session}:{pane}");
            json_ok(body)
        }
        Err(e) => dev_err(e),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let web_dir = env_or("CHOPS_WEB_DIR", "web");
    let cert_path = env_or(
        "CHOPS_TLS_CERT",
        &dirs::home_dir()
            .unwrap_or_default()
            .join(".config/tailscale-certs/pop-mini.monkey-ladon.ts.net.crt")
            .to_string_lossy(),
    );
    let key_path = env_or(
        "CHOPS_TLS_KEY",
        &dirs::home_dir()
            .unwrap_or_default()
            .join(".config/tailscale-certs/pop-mini.monkey-ladon.ts.net.key")
            .to_string_lossy(),
    );

    // CORS for Tauri app (Android/desktop WebView), browser, and dev
    let cors = CorsLayer::permissive();

    let app = Router::new()
        .route("/api/sessions", get(api_sessions))
        .route("/api/sessions/switch", post(api_switch_session))
        .route("/api/sessions/start", post(api_start_session))
        .route("/api/sessions/stop", post(api_stop_session))
        .route(
            "/api/sessions/{name}/panes/{pane}/keys",
            post(api_send_keys),
        )
        .fallback_service(ServeDir::new(&web_dir))
        .layer(cors);

    let has_tls = PathBuf::from(&cert_path).exists() && PathBuf::from(&key_path).exists();

    if has_tls {
        let port: u16 = env_or("CHOPS_WEB_PORT", "8443").parse().unwrap_or(8443);
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!("Serving {web_dir} on https://0.0.0.0:{port}");

        let tls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .expect("Failed to load TLS cert/key");

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .expect("Server failed");
    } else {
        let port: u16 = env_or("CHOPS_WEB_PORT", "8080").parse().unwrap_or(8080);
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        info!("No TLS certs found, serving {web_dir} on http://0.0.0.0:{port}");

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.expect("Server failed");
    }
}
