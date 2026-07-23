use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod authorize;
#[cfg(not(target_os = "android"))]
mod desktop;
mod secure_key;
#[cfg(target_os = "android")]
pub use authorize::*;
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
    default_socket_path_in(host, Path::new("/tmp"))
}

fn default_socket_path_in(host: &str, base_dir: &Path) -> PathBuf {
    let safe = host.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '_', "-");
    base_dir.join(format!("dev-{safe}.sock"))
}

fn default_remote_socket_path() -> String {
    "/run/user/1000/dev.sock".to_string()
}

/// Parse a host string into (hostname, remote_socket_path).
/// Format: `hostname`, `user@hostname`, or `hostname:/path/to/socket`
fn parse_host(host: &str) -> Result<(&str, String), String> {
    // Strip optional user@ prefix
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);

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

/// Shell-quote a string for use in a remote command: wraps in single quotes,
/// escaping any internal single quotes.
pub(crate) fn sh_quote(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_plain() {
        let (host, remote) = parse_host("pop-mini").unwrap();
        assert_eq!(host, "pop-mini");
        assert_eq!(remote, "/run/user/1000/dev.sock");
    }

    #[test]
    fn test_parse_host_with_path() {
        let (host, remote) = parse_host("pop-mini:/custom/sock").unwrap();
        assert_eq!(host, "pop-mini");
        assert_eq!(remote, "/custom/sock");
    }

    #[test]
    fn test_parse_host_user_at() {
        let (host, remote) = parse_host("mt@pop-mini").unwrap();
        assert_eq!(host, "pop-mini");
        assert_eq!(remote, "/run/user/1000/dev.sock");
    }

    #[test]
    fn test_parse_host_user_at_with_path() {
        let (host, remote) = parse_host("mt@pop-mini:/custom/sock").unwrap();
        assert_eq!(host, "pop-mini");
        assert_eq!(remote, "/custom/sock");
    }

    #[test]
    fn test_parse_host_bad_relative_path() {
        assert!(parse_host("pop-mini:relative/path").is_err());
    }

    #[test]
    fn test_sh_quote_simple() {
        assert_eq!(sh_quote("hello"), "'hello'");
    }

    #[test]
    fn test_sh_quote_with_apostrophe() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_sh_quote_empty() {
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn test_sh_quote_path() {
        assert_eq!(
            sh_quote("/run/user/1000/dev.sock"),
            "'/run/user/1000/dev.sock'"
        );
    }

    #[test]
    fn test_default_socket_path_in_uses_base_dir() {
        let path = default_socket_path_in("pop-mini", Path::new("/data/data/com.app/files"));
        assert!(path.starts_with("/data/data/com.app/files"));
        assert!(path.to_string_lossy().contains("pop-mini"));
        assert!(path.to_string_lossy().ends_with(".sock"));
    }

    #[test]
    fn test_default_socket_path_in_sanitizes_host() {
        let path = default_socket_path_in("my host!@#", Path::new("/tmp"));
        let name = path.file_name().unwrap().to_string_lossy();
        // Dots and alphanumeric are kept; other chars become '-'
        assert!(name.starts_with("dev-"));
        assert!(!name.contains('!'));
        assert!(!name.contains('@'));
        assert!(!name.contains('#'));
        assert!(name.ends_with(".sock"));
    }
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

    #[cfg(target_os = "android")]
    fn socket_path_for(&self, hostname: &str) -> PathBuf {
        if self.app_data.as_os_str().is_empty() {
            default_socket_path(hostname)
        } else {
            default_socket_path_in(hostname, &self.app_data)
        }
    }

    #[cfg(not(target_os = "android"))]
    fn socket_path_for(&self, hostname: &str) -> PathBuf {
        default_socket_path(hostname)
    }

    /// Ensure a tunnel exists for the given host. Returns the local socket path.
    pub fn ensure_tunnel(&mut self, host: &str) -> Result<PathBuf, String> {
        let (hostname, remote_path) = parse_host(host)?;

        if let Some(tunnel) = self.tunnels.get_mut(host) {
            if tunnel.is_alive() {
                debug!("Reusing live tunnel for {host}");
                return Ok(tunnel.socket_path().to_path_buf());
            }
            // Stale — tear down and recreate
            warn!("Tunnel for {host} is dead, tearing down and recreating");
            self.stop(host);
        }

        let socket_path = self.socket_path_for(hostname);

        info!("Creating tunnel for {host} at {:?}", socket_path);

        // Lazy cleanup of stale sockets from crashed sessions
        let _ = std::fs::remove_file(&socket_path);

        #[cfg(not(target_os = "android"))]
        let tunnel: Box<dyn TunnelImpl> = Box::new(desktop::DesktopTunnel::open(
            hostname,
            &remote_path,
            &socket_path,
        )?);

        #[cfg(target_os = "android")]
        let tunnel: Box<dyn TunnelImpl> = Box::new(android::AndroidTunnel::open(
            &self.app_data,
            hostname,
            &remote_path,
            &socket_path,
        )?);

        self.tunnels.insert(host.to_string(), tunnel);
        Ok(socket_path)
    }

    pub fn stop(&mut self, host: &str) {
        if let Some(tunnel) = self.tunnels.remove(host) {
            info!("Stopping tunnel for {host}");
            tunnel.kill();
        }
    }

    pub fn stop_all(&mut self) {
        let count = self.tunnels.len();
        if count > 0 {
            info!("Stopping all {count} tunnel(s)");
        }
        for (_, tunnel) in self.tunnels.drain() {
            tunnel.kill();
        }
    }

    pub fn status(&mut self) -> Vec<TunnelStatus> {
        let mut results = Vec::new();
        self.tunnels.retain(|host, tunnel| {
            let alive = tunnel.is_alive();
            if !alive {
                warn!("Tunnel for {host} found dead during status check, dropping");
            }
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
