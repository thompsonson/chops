use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use tracing::{info, warn};

use crate::tunnel::TunnelImpl;

// ---------------------------------------------------------------------------
// DesktopTunnel — ssh -NL via std::process::Command
// ---------------------------------------------------------------------------

pub(crate) struct DesktopTunnel {
    child: Option<Child>,
    socket_path: PathBuf,
}

impl DesktopTunnel {
    pub fn open(hostname: &str, remote_path: &str, socket_path: &Path) -> Result<Self, String> {
        info!("Spawning ssh tunnel to {hostname} -> {}", socket_path.display());

        let mut child = Command::new("ssh")
            .args([
                "-N",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-L",
                &format!("{}:{}", socket_path.display(), remote_path),
                hostname,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn SSH tunnel to {hostname}: {e}"))?;

        // Forward the ssh client's own stderr (auth prompts, host key
        // warnings, connection errors) into our log instead of discarding it.
        if let Some(stderr) = child.stderr.take() {
            let host = hostname.to_string();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    warn!("ssh[{host}]: {line}");
                }
            });
        }

        // Brief wait for the socket to appear
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !socket_path.exists() {
            warn!("ssh tunnel to {hostname} did not create socket in time, killing");
            let _ = child.kill();
            return Err(format!(
                "SSH tunnel to {hostname} did not create socket (check auth)"
            ));
        }

        info!("ssh tunnel to {hostname} ready at {}", socket_path.display());

        Ok(Self {
            child: Some(child),
            socket_path: socket_path.to_path_buf(),
        })
    }
}

impl TunnelImpl for DesktopTunnel {
    fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    warn!("ssh tunnel process exited: {status}");
                    false
                }
                Err(e) => {
                    warn!("ssh tunnel process wait failed: {e}");
                    false
                }
            },
            None => false,
        }
    }

    fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn kill(self: Box<Self>) {
        info!("Killing ssh tunnel at {}", self.socket_path.display());
        if let Some(mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}
