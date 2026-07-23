use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::tunnel::secure_key;
use crate::tunnel::sh_quote;
use crate::tunnel::TunnelImpl;
use russh::client;
use russh::keys::key;
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error, info, warn};

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
    error: Arc<Mutex<Option<String>>>,
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
        let sock_clone = sock.clone();
        let host = hostname.to_string();
        let remote = remote_path.to_string();
        let app_data = app_data.to_path_buf();

        let _ = std::fs::remove_file(&sock);

        let error = Arc::new(Mutex::new(None));
        let error_clone = error.clone();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("AndroidTunnel tokio runtime");

            rt.block_on(async move {
                if let Err(e) = run_tunnel(
                    app_data,
                    &key_bytes,
                    &username,
                    &host,
                    port,
                    &remote,
                    &sock_clone,
                    shutdown_rx,
                )
                .await
                {
                    if let Ok(mut err) = error_clone.lock() {
                        *err = Some(e.clone());
                    }
                    error!("Android tunnel to {host}:{port} failed: {e}");
                }
            });
        });

        // Poll briefly for the socket to appear (like DesktopTunnel)
        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !sock.exists() {
            let err = error.lock().ok()
                .and_then(|mut e| e.take())
                .unwrap_or_else(|| {
                    if thread.is_finished() {
                        "SSH tunnel failed (check host, credentials, and SSH server)".to_string()
                    } else {
                        "SSH tunnel not ready yet (try again)".to_string()
                    }
                });
            return Err(err);
        }

        Ok(Self {
            socket_path: sock,
            thread: Some(thread),
            shutdown: Some(shutdown_tx),
            error,
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

    let listener = tokio::net::UnixListener::bind(socket_path)
        .map_err(|e| format!("Cannot bind {socket_path:?}: {e}"))?;

    info!("Tunnel socket bound at {socket_path:?}");

    let config = Arc::new(client::Config::default());
    let handler = TunnelHandler {
        hostname: hostname.to_string(),
        port,
        app_data,
    };

    let mut handle = client::connect(config, (hostname, port), handler)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(socket_path);
            format!("SSH connect to {hostname}:{port} failed: {e}")
        })?;

    info!("Android tunnel SSH connected to {hostname}:{port}");

    let auth_ok = handle
        .authenticate_publickey(username, Arc::new(key_pair))
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(socket_path);
            format!("Auth failed: {e}")
        })?;

    if !auth_ok {
        let _ = std::fs::remove_file(socket_path);
        error!("Tunnel key auth rejected for {username}@{hostname}");
        return Err("Public key authentication rejected by server".to_string());
    }

    info!("Tunnel key auth ok for {username}@{hostname}");

    let handle = Arc::new(handle);

    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        info!("tunnel: accepted connection, spawning forward");
                        let h = Arc::clone(&handle);
                        let rp = remote_path.to_string();
                        tokio::spawn(async move {
                            match forward_connection(h, stream, &rp).await {
                                Ok(()) => info!("tunnel: forward completed ok"),
                                Err(e) => error!("tunnel: forward failed: {e}"),
                            }
                        });
                    }
                    Err(e) => {
                        error!("Tunnel accept error: {e}");
                        break;
                    }
                }
            }
            _ = &mut shutdown => {
                info!("tunnel: received shutdown signal");
                break;
            }
        }
    }

    info!("Tunnel shutting down: {hostname}");
    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-connection forwarding:  local Unix socket ↔ SSH exec'd socat
// ---------------------------------------------------------------------------

async fn forward_connection(
    handle: Arc<client::Handle<TunnelHandler>>,
    mut local: tokio::net::UnixStream,
    remote_path: &str,
) -> Result<(), String> {
    info!("forward: start connection to {remote_path}");

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("Open session: {e}"))?;
    info!("forward: SSH channel opened");

    let cmd = format!("socat STDIO UNIX-CONNECT:{}", sh_quote(remote_path));
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| format!("Exec failed: {e}"))?;
    info!("forward: socat exec'd");

    let (mut local_reader, mut local_writer) = local.split();
    let mut buf = vec![0u8; 16384];

    let reason: String = loop {
        tokio::select! {
            result = local_reader.read(&mut buf) => {
                match result {
                    Ok(0) => break "local_eof".into(),
                    Err(e) => break format!("local_read_err: {e}"),
                    Ok(n) => {
                        if channel.data(&buf[..n]).await.is_err() {
                            break "channel_write_err".into();
                        }
                    }
                }
            }
            event = channel.wait() => {
                match event {
                    Some(ChannelMsg::Data { ref data }) => {
                        if local_writer.write_all(data).await.is_err() {
                            break "local_write_err".into();
                        }
                    }
                    Some(ChannelMsg::Eof) => break "remote_eof".into(),
                    None => break "channel_closed".into(),
                    _ => {}
                }
            }
        }
    };

    info!("forward: done — reason={reason}");

    info!("forward: shutting down local writer");
    let _ = local_writer.shutdown().await;

    // Drain any bytes the client may have sent after we stopped reading.
    // Bounded: the client isn't guaranteed to close its write side just
    // because we closed ours, so an unbounded read here can hang the task
    // (and leak its fd) indefinitely.
    let mut final_buf = [0u8; 1024];
    match tokio::time::timeout(
        std::time::Duration::from_millis(200),
        local_reader.read(&mut final_buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => info!("forward: drained {n} bytes after shutdown"),
        Ok(Ok(_)) => {}
        Ok(Err(e)) => debug!("forward: drain read error (ignored): {e}"),
        Err(_) => warn!("forward: drain read timed out after 200ms, client kept connection open"),
    }

    info!("forward: closing SSH channel");
    let _ = channel.close().await;
    info!("forward: complete");
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
