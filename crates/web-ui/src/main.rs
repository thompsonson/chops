use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::process::Command;
use tower_http::services::ServeDir;
use tracing::info;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Run `dev <args>` and return (success, stdout/stderr).
async fn run_dev(args: &[&str]) -> Result<String, String> {
    let home = dirs::home_dir().unwrap_or_default();
    let dev_bin = home.join(".local/bin/dev");

    let result = Command::new(dev_bin)
        .args(args)
        .env("TMUX_TMPDIR", "/tmp")
        .env("HOME", &home)
        .env(
            "PATH",
            format!(
                "{}/.local/bin:{}",
                home.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&result.stderr).trim().to_string();
        Err(err)
    }
}

fn json_ok(body: String) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    (StatusCode::OK, [("content-type", "application/json")], body)
}

fn json_err(msg: &str) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [("content-type", "application/json")],
        serde_json::json!({"error": msg}).to_string(),
    )
}

fn json_bad(msg: &str) -> (StatusCode, [(&'static str, &'static str); 1], String) {
    (
        StatusCode::BAD_REQUEST,
        [("content-type", "application/json")],
        serde_json::json!({"error": msg}).to_string(),
    )
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
    match run_dev(&["list"]).await {
        Ok(body) => json_ok(body),
        Err(e) => json_err(&e),
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
            json_ok(serde_json::json!({"session": session}).to_string())
        }
        Err(e) => json_err(&e.to_string()),
    }
}

/// POST /api/sessions/start?project=<name>&layout=<layout> — start a new session.
async fn api_start_session(Query(params): Query<StartParams>) -> impl IntoResponse {
    let project = match validate_name(&params.project) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let mut args = vec!["start", &project];
    let layout_str;
    if let Some(ref layout) = params.layout {
        layout_str = layout.clone();
        args.push(&layout_str);
    }

    match run_dev(&args).await {
        Ok(output) => {
            info!("Started session: {project}");
            json_ok(serde_json::json!({"project": project, "output": output.trim()}).to_string())
        }
        Err(e) => json_err(&e),
    }
}

/// POST /api/sessions/stop?session=<name> — stop (kill) a session.
async fn api_stop_session(Query(params): Query<SessionParams>) -> impl IntoResponse {
    let session = match validate_name(&params.session) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match run_dev(&["stop", &session]).await {
        Ok(output) => {
            info!("Stopped session: {session}");
            json_ok(serde_json::json!({"session": session, "output": output.trim()}).to_string())
        }
        Err(e) => json_err(&e),
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

    let app = Router::new()
        .route("/api/sessions", get(api_sessions))
        .route("/api/sessions/switch", post(api_switch_session))
        .route("/api/sessions/start", post(api_start_session))
        .route("/api/sessions/stop", post(api_stop_session))
        .fallback_service(ServeDir::new(&web_dir));

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
