use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tunnel::secure_key;
use crate::tunnel::sh_quote;
use crate::tunnel::TunnelImpl;
use russh::client;
use russh::keys::key;
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info};

// ---------------------------------------------------------------------------
// SSH client handler
// ---------------------------------------------------------------------------

struct TunnelHandler {
    hostname: String,
    port: u16,
    app_data: PathBuf,
}

#[async_trait::async_trait]
impl client::Handler for TunnelHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let known_hosts = self.app_data.join("known_hosts");
        if russh::keys::check_known_hosts_path(
            &self.hostname,
            self.port,
            server_public_key,
            &known_hosts,
        )? {
            return Ok(true);
        }
        russh::keys::learn_known_hosts_path(
            &self.hostname,
            self.port,
            server_public_key,
            &known_hosts,
        )?;
        info!("Learned host key for {}:{}", self.hostname, self.port);
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// AndroidTunnel — in-process SSH tunnel via russh
// ---------------------------------------------------------------------------

pub(crate) struct AndroidTunnel {
    socket_path: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl AndroidTunnel {
    pub fn open(
        app_data: &Path,
        hostname: &str,
        remote_path: &str,
        socket_path: &Path,
    ) -> Result<Self, String> {
        let alias = hostname.replace('.', "_");
        let key_bytes = secure_key::load_ssh_key(app_data, &alias)?;
        let username = secure_key::load_ssh_username(app_data, &alias);
        let port = secure_key::load_ssh_port(app_data, &alias);

        let sock = socket_path.to_path_buf();
        let host = hostname.to_string();
        let remote = remote_path.to_string();
        let app_data = app_data.to_path_buf();

        let _ = std::fs::remove_file(&sock);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("AndroidTunnel tokio runtime");

            rt.block_on(async move {
                if let Err(e) =
                    run_tunnel(app_data, &key_bytes, &username, &host, port, &remote, &sock, shutdown_rx).await
                {
                    error!("Android tunnel to {host}:{port} failed: {e}");
                }
            });
        });

        Ok(Self {
            socket_path: sock,
            thread: Some(thread),
            shutdown: Some(shutdown_tx),
        })
    }
}

impl TunnelImpl for AndroidTunnel {
    fn is_alive(&mut self) -> bool {
        match &self.thread {
            Some(t) => !t.is_finished(),
            None => false,
        }
    }

    fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn kill(self: Box<Self>) {
        if let Some(tx) = self.shutdown {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread {
            std::thread::spawn(move || drop(t.join()));
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

// ---------------------------------------------------------------------------
// Tunnel runtime — lives on a dedicated tokio runtime in a background thread
// ---------------------------------------------------------------------------

async fn run_tunnel(
    app_data: PathBuf,
    key_bytes: &[u8],
    username: &str,
    hostname: &str,
    port: u16,
    remote_path: &str,
    socket_path: &Path,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let key_str =
        std::str::from_utf8(key_bytes).map_err(|_| "Invalid private key UTF-8".to_string())?;
    let key_pair = russh::keys::decode_secret_key(key_str, None)
        .map_err(|e| format!("Failed to parse private key: {e}"))?;
    info!("Loaded SSH key for {username}@{hostname}");

    let config = Arc::new(client::Config::default());
    let handler = TunnelHandler {
        hostname: hostname.to_string(),
        port,
        app_data,
    };

    let mut handle = client::connect(config, (hostname, port), handler)
        .await
        .map_err(|e| format!("SSH connect to {hostname}:{port} failed: {e}"))?;

    info!("Android tunnel connected to {hostname}:{port}");

    let auth_ok = handle
        .authenticate_publickey(username, Arc::new(key_pair))
        .await
        .map_err(|e| format!("Auth failed: {e}"))?;

    if !auth_ok {
        error!("Tunnel key auth rejected for {username}@{hostname}");
        return Err("Public key authentication rejected by server".to_string());
    }

    info!("Tunnel key auth ok for {username}@{hostname}");

    let listener = tokio::net::UnixListener::bind(socket_path)
        .map_err(|e| format!("Cannot bind {socket_path:?}: {e}"))?;

    info!("Android tunnel listening on {socket_path:?}");

    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let h = handle.clone();
                        let rp = remote_path.to_string();
                        tokio::spawn(async move {
                            if let Err(e) = forward_connection(h, stream, &rp).await {
                                debug!("Tunnel forward error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        error!("Tunnel accept error: {e}");
                        break;
                    }
                }
            }
            _ = &mut shutdown => break,
        }
    }

    info!("Tunnel shutting down: {hostname}");
    let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-connection forwarding:  local Unix socket ↔ SSH exec'd socat
// ---------------------------------------------------------------------------

async fn forward_connection(
    mut handle: client::Handle<TunnelHandler>,
    mut local: tokio::net::UnixStream,
    remote_path: &str,
) -> Result<(), String> {
    debug!("Forwarding connection to {remote_path}");

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("Open session: {e}"))?;

    let cmd = format!("socat STDIO UNIX-CONNECT:{}", sh_quote(remote_path));
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| format!("Exec failed: {e}"))?;

    let (mut local_reader, mut local_writer) = local.split();
    let mut buf = vec![0u8; 16384];

    loop {
        tokio::select! {
            result = local_reader.read(&mut buf) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if channel.data(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
            event = channel.wait() => {
                match event {
                    Some(ChannelMsg::Data { ref data }) => {
                        if local_writer.write_all(data).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::Eof) | None => break,
                    _ => {}
                }
            }
        }
    }

    let _ = channel.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_secret_key_valid_ed25519() {
        let (key_bytes, _) = crate::tunnel::secure_key::generate_ssh_key().unwrap();
        let key_str = std::str::from_utf8(&key_bytes).unwrap();
        let result = russh::keys::decode_secret_key(key_str, None);
        assert!(result.is_ok(), "valid ED25519 key should parse");
    }

    #[test]
    fn test_decode_secret_key_garbage() {
        let result = russh::keys::decode_secret_key("not-a-valid-key", None);
        assert!(result.is_err(), "garbage should not parse");
    }
}
