use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(target_os = "android")]
fn ssh_binary() -> &'static str {
    "/data/data/com.termux/files/usr/bin/ssh"
}

#[cfg(not(target_os = "android"))]
fn ssh_binary() -> &'static str {
    "ssh"
}

fn default_socket_path(host: &str) -> PathBuf {
    let safe = host.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '_', "-");
    PathBuf::from("/tmp").join(format!("dev-{safe}.sock"))
}

fn remote_socket_path() -> &'static str {
    "~/.local/run/dev.sock"
}

// ---------------------------------------------------------------------------
// TunnelManager
// ---------------------------------------------------------------------------

pub struct TunnelManager {
    tunnels: HashMap<String, SshTunnel>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            tunnels: HashMap::new(),
        }
    }

    /// Ensure a tunnel exists for the given host. Returns the local socket path.
    pub fn ensure_tunnel(&mut self, host: &str) -> Result<PathBuf, String> {
        if let Some(tunnel) = self.tunnels.get_mut(host) {
            if tunnel.is_alive() {
                return Ok(tunnel.socket_path.clone());
            }
            // Stale — tear down and recreate
            self.stop(host);
        }

        let socket_path = default_socket_path(host);
        let remote = remote_socket_path();

        let mut child = Command::new(ssh_binary())
            .args([
                "-N",
                "-L",
                &format!("{}:{}", socket_path.display(), remote),
                host,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn SSH tunnel to {host}: {e}"))?;

        // Brief wait for the socket to appear
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !socket_path.exists() {
            let _ = child.kill();
            return Err(format!("SSH tunnel to {host} did not create socket (check auth)"));
        }

        self.tunnels.insert(
            host.to_string(),
            SshTunnel {
                _host: host.to_string(),
                child: Some(child),
                socket_path: socket_path.clone(),
            },
        );

        Ok(socket_path)
    }

    /// Tear down a tunnel for a specific host.
    pub fn stop(&mut self, host: &str) {
        if let Some(mut tunnel) = self.tunnels.remove(host) {
            tunnel.kill();
        }
    }

    /// Tear down all tunnels.
    pub fn stop_all(&mut self) {
        for (_, mut tunnel) in self.tunnels.drain() {
            tunnel.kill();
        }
    }

    /// Check liveness of all tunnels. Returns status per host.
    pub fn status(&mut self) -> Vec<TunnelStatus> {
        let mut results = Vec::new();
        self.tunnels.retain(|host, tunnel| {
            let alive = tunnel.is_alive();
            results.push(TunnelStatus {
                host: host.clone(),
                alive,
                socket_path: tunnel.socket_path.clone(),
            });
            alive
        });
        results
    }
}

// ---------------------------------------------------------------------------
// SshTunnel
// ---------------------------------------------------------------------------

struct SshTunnel {
    _host: String,
    child: Option<Child>,
    socket_path: PathBuf,
}

impl SshTunnel {
    fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                _ => false,
            },
            None => false,
        }
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

// ---------------------------------------------------------------------------
// TunnelStatus DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct TunnelStatus {
    pub host: String,
    pub alive: bool,
    pub socket_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Managed wrapper for AppState
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TunnelManagerHandle(pub Arc<Mutex<TunnelManager>>);

impl TunnelManagerHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(TunnelManager::new())))
    }
}
