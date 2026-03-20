use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::ServeDir;
use tracing::info;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
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

    let app = Router::new().fallback_service(ServeDir::new(&web_dir));

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
