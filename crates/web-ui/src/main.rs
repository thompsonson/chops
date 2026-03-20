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

/// Call `dev list` and return its JSON output.
async fn api_sessions() -> impl IntoResponse {
    let home = dirs::home_dir().unwrap_or_default();
    let dev_bin = home.join(".local/bin/dev");

    let result = Command::new(dev_bin)
        .arg("list")
        .env("TMUX_TMPDIR", "/tmp")
        .env("HOME", &home)
        .env(
            "PATH",
            format!(
                "{}/.local/bin:/home/linuxbrew/.linuxbrew/bin:/usr/local/bin:/usr/bin:/bin",
                home.display()
            ),
        )
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            let body = String::from_utf8_lossy(&output.stdout).to_string();
            (StatusCode::OK, [("content-type", "application/json")], body)
        }
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                format!(r#"{{"error":"{}"}}"#, err.trim()),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "application/json")],
            format!(r#"{{"error":"{}"}}"#, e),
        ),
    }
}

#[derive(Deserialize)]
struct SwitchParams {
    session: String,
}

/// Write the session name to the ttyd state file, so the next browser
/// connection to ttyd attaches to this session.
async fn api_switch_session(Query(params): Query<SwitchParams>) -> impl IntoResponse {
    let home = dirs::home_dir().unwrap_or_default();
    let state_file = home.join(".config/chops/ttyd-session");

    // Sanitize: only allow alphanumeric, dash, underscore, dot
    let session = params.session.trim().to_string();
    if session.is_empty()
        || !session
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            r#"{"error":"invalid session name"}"#.to_string(),
        );
    }

    match tokio::fs::write(&state_file, &session).await {
        Ok(_) => {
            info!("Switched ttyd session to: {session}");
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                format!(r#"{{"session":"{}"}}"#, session),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "application/json")],
            format!(r#"{{"error":"{}"}}"#, e),
        ),
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
