use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("dev daemon unreachable at {path}: {source}")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed HTTP response from daemon")]
    MalformedResponse,

    #[error("daemon returned HTTP {status}: {body}")]
    DaemonError { status: u16, body: String },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// DTOs — mirrors the dev daemon JSON wire format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub name: String,
    pub pane_count: u32,
    pub attached: bool,
    pub last_activity: u64,
    pub layout: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub layout: String,
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listing {
    pub sessions: Vec<SessionInfo>,
    pub projects: Vec<ProjectInfo>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DevClient {
    socket_path: PathBuf,
}

impl DevClient {
    /// Create a client with an explicit socket path.
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Create a client using the default socket path:
    /// `$XDG_RUNTIME_DIR/dev.sock`, fallback `~/.local/run/dev.sock`.
    pub fn from_env() -> Self {
        let path = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| PathBuf::from(d).join("dev.sock"))
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                PathBuf::from(home).join(".local/run/dev.sock")
            });
        Self::new(path)
    }

    /// Socket path this client will connect to.
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    // -- public API ---------------------------------------------------------

    /// List active sessions and dormant projects.
    pub async fn list(&self) -> Result<Listing> {
        let body = self.request("GET", "/sessions", None).await?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Start a session for the given project. Returns daemon response body.
    pub async fn start(
        &self,
        project: &str,
        layout: Option<&str>,
    ) -> Result<String> {
        let payload = match layout {
            Some(l) => serde_json::json!({ "project": project, "layout": l }),
            None => serde_json::json!({ "project": project }),
        };
        self.request("POST", "/sessions", Some(&payload)).await
    }

    /// Stop (kill) a session by name.
    pub async fn stop(&self, session: &str) -> Result<()> {
        self.request("DELETE", &format!("/sessions/{session}"), None)
            .await?;
        Ok(())
    }

    /// Send keystrokes to a tmux pane.
    pub async fn send_keys(
        &self,
        session: &str,
        pane: &str,
        keys: &str,
    ) -> Result<String> {
        let payload = serde_json::json!({ "keys": keys });
        self.request(
            "POST",
            &format!("/sessions/{session}/panes/{pane}/keys"),
            Some(&payload),
        )
        .await
    }

    // -- transport ----------------------------------------------------------

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<String> {
        let mut stream =
            UnixStream::connect(&self.socket_path)
                .await
                .map_err(|e| Error::Connect {
                    path: self.socket_path.clone(),
                    source: e,
                })?;

        let body_bytes = match body {
            Some(v) => serde_json::to_vec(v)?,
            None => Vec::new(),
        };

        let header = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body_bytes.len()
        );

        stream.write_all(header.as_bytes()).await?;
        if !body_bytes.is_empty() {
            stream.write_all(&body_bytes).await?;
        }
        stream.flush().await?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;

        // Split headers from body on CRLFCRLF boundary.
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or(Error::MalformedResponse)?;

        let headers = std::str::from_utf8(&raw[..split]).unwrap_or("");
        let status = parse_status(headers)?;
        let body_str =
            String::from_utf8_lossy(&raw[split + 4..]).into_owned();

        if (200..300).contains(&status) {
            Ok(body_str)
        } else {
            Err(Error::DaemonError {
                status,
                body: body_str,
            })
        }
    }
}

/// Extract status code from the HTTP/1.1 status line.
fn parse_status(headers: &str) -> Result<u16> {
    // e.g. "HTTP/1.1 200 OK"
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or(Error::MalformedResponse)
}

#[cfg(test)]
mod tests;
