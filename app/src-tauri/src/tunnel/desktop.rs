use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::fs;

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
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn SSH tunnel to {hostname}: {e}"))?;

        // Brief wait for the socket to appear
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if !socket_path.exists() {
            let _ = child.kill();
            return Err(format!(
                "SSH tunnel to {hostname} did not create socket (check auth)"
            ));
        }

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
                _ => false,
            },
            None => false,
        }
    }

    fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn kill(self: Box<Self>) {
        if let Some(mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}
