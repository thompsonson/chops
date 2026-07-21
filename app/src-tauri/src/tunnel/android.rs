use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tunnel::secure_key;
use crate::tunnel::TunnelImpl;
use russh::client;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------------
// SSH client handler — accepts any server key
// ---------------------------------------------------------------------------

struct TunnelHandler;

#[async_trait::async_trait]
impl client::Handler for TunnelHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
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

        let sock = socket_path.to_path_buf();
        let host = hostname.to_string();
        let remote = remote_path.to_string();

        // Clean up stale socket from crashed sessions
        let _ = std::fs::remove_file(&sock);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("AndroidTunnel tokio runtime");

            rt.block_on(async move {
                if let Err(e) = run_tunnel(&key_bytes, &host, &remote, &sock, shutdown_rx).await {
                    // Logged via tracing if available, otherwise silently dropped
                    let _ = e;
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
        let _ = drop(self.thread);
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

// ---------------------------------------------------------------------------
// Tunnel runtime — lives on a dedicated tokio runtime in a background thread
// ---------------------------------------------------------------------------

async fn run_tunnel(
    key_bytes: &[u8],
    hostname: &str,
    remote_path: &str,
    socket_path: &Path,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let key_str =
        std::str::from_utf8(key_bytes).map_err(|_| "Invalid private key UTF-8".to_string())?;

    let private_key = russh_keys::key::PrivateKey::from_openssh(key_str)
        .map_err(|e| format!("Failed to parse private key: {e}"))?;
    let key_pair: russh_keys::key::KeyPair = private_key.into();

    let config = Arc::new(client::Config::default());
    let handler = TunnelHandler;

    let mut handle = client::connect(config, (hostname, 22u16), handler)
        .await
        .map_err(|e| format!("SSH connect to {hostname}:22 failed: {e}"))?;

    let auth_ok = handle
        .authenticate_publickey("mt", Arc::new(key_pair))
        .await
        .map_err(|e| format!("Auth failed: {e}"))?;

    if !auth_ok {
        return Err("Public key authentication rejected by server".to_string());
    }

    let listener = tokio::net::UnixListener::bind(socket_path)
        .map_err(|e| format!("Cannot bind {socket_path:?}: {e}"))?;

    loop {
        tokio::select! {
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let h = handle.clone();
                        let rp = remote_path.to_string();
                        tokio::spawn(async move {
                            if let Err(e) = forward_connection(h, stream, &rp).await {
                                let _ = e;
                            }
                        });
                    }
                    Err(_) => break,
                }
            }
            _ = &mut shutdown => break,
        }
    }

    let _ = handle.close().await;
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
    let mut session = handle
        .open_session()
        .await
        .map_err(|e| format!("Open session: {e}"))?;

    let cmd = format!("socat STDIO UNIX-CONNECT:{}", sh_quote(remote_path));
    session
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
                        if session.data(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
            event = session.wait() => {
                match event {
                    Some(client::SessionEvent::Stdout { data }) => {
                        if local_writer.write_all(&data).await.is_err() {
                            break;
                        }
                    }
                    Some(client::SessionEvent::Eof) | None => break,
                    _ => {}
                }
            }
        }
    }

    let _ = session.close().await;
    Ok(())
}

fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
