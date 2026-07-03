use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod desktop;
mod secure_key;
#[allow(unused_imports)]
pub use secure_key::*;

// ---------------------------------------------------------------------------
// TunnelImpl trait — platform-specific tunnel backend
// ---------------------------------------------------------------------------

pub(crate) trait TunnelImpl: Send {
    fn is_alive(&mut self) -> bool;
    fn socket_path(&self) -> &Path;
    fn kill(self: Box<Self>);
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn default_socket_path(host: &str) -> PathBuf {
    let safe = host.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '_', "-");
    PathBuf::from("/tmp").join(format!("dev-{safe}.sock"))
}

fn default_remote_socket_path() -> String {
    "/run/user/1000/dev.sock".to_string()
}

/// Parse a host string into (hostname, remote_socket_path).
/// Format: `hostname` or `hostname:/path/to/socket`
fn parse_host(host: &str) -> Result<(&str, String), String> {
    if let Some(idx) = host.find(':') {
        let hostname = &host[..idx];
        let remote = host[idx + 1..].to_string();
        if !remote.is_empty() {
            if !remote.starts_with('/') {
                return Err(format!("Remote socket path must be absolute: {remote}"));
            }
            return Ok((hostname, remote));
        }
    }
    Ok((host, default_remote_socket_path()))
}

// ---------------------------------------------------------------------------
// TunnelManager
// ---------------------------------------------------------------------------

pub struct TunnelManager {
    tunnels: HashMap<String, Box<dyn TunnelImpl>>,
    #[cfg(target_os = "android")]
    app_data: PathBuf,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            tunnels: HashMap::new(),
            #[cfg(target_os = "android")]
            app_data: PathBuf::new(),
        }
    }

    #[cfg(target_os = "android")]
    pub fn set_app_data(&mut self, path: PathBuf) {
        self.app_data = path;
    }

    /// Ensure a tunnel exists for the given host. Returns the local socket path.
    pub fn ensure_tunnel(&mut self, host: &str) -> Result<PathBuf, String> {
        let (hostname, remote_path) = parse_host(host)?;

        if let Some(tunnel) = self.tunnels.get_mut(host) {
            if tunnel.is_alive() {
                return Ok(tunnel.socket_path().to_path_buf());
            }
            // Stale — tear down and recreate
            self.stop(host);
        }

        let socket_path = default_socket_path(hostname);

        // Lazy cleanup of stale sockets from crashed sessions
        let _ = std::fs::remove_file(&socket_path);

        #[cfg(not(target_os = "android"))]
        let tunnel: Box<dyn TunnelImpl> = Box::new(desktop::DesktopTunnel::open(
            hostname, &remote_path, &socket_path,
        )?);

        #[cfg(target_os = "android")]
        let tunnel: Box<dyn TunnelImpl> = Box::new(android::AndroidTunnel::open(
            &self.app_data, hostname, &remote_path, &socket_path,
        )?);

        self.tunnels.insert(host.to_string(), tunnel);
        Ok(socket_path)
    }

    pub fn stop(&mut self, host: &str) {
        if let Some(tunnel) = self.tunnels.remove(host) {
            tunnel.kill();
        }
    }

    pub fn stop_all(&mut self) {
        for (_, tunnel) in self.tunnels.drain() {
            tunnel.kill();
        }
    }

    pub fn status(&mut self) -> Vec<TunnelStatus> {
        let mut results = Vec::new();
        self.tunnels.retain(|host, tunnel| {
            let alive = tunnel.is_alive();
            results.push(TunnelStatus {
                host: host.clone(),
                alive,
                socket_path: tunnel.socket_path().to_path_buf(),
            });
            alive
        });
        results
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
