use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::tunnel::secure_key;
use crate::tunnel::TunnelImpl;

// ---------------------------------------------------------------------------
// AndroidTunnel — bundled SSH binary + temp key file
// Uses the same ssh -NL approach as desktop, but with a bundled SSH binary
// ---------------------------------------------------------------------------

pub(crate) struct AndroidTunnel {
    child: Option<Child>,
    socket_path: PathBuf,
    key_path: PathBuf,
}

impl AndroidTunnel {
    pub fn open(
        app_data: &Path,
        hostname: &str,
        remote_path: &str,
        socket_path: &Path,
    ) -> Result<Self, String> {
        let alias = hostname.replace('.', "_");
        let app_data = app_data.to_path_buf();

        // Decrypt the stored SSH key
        let key_bytes = secure_key::load_ssh_key(&app_data, &alias)
            .map_err(|e| format!("Key load failed: {e}"))?;

        // Write key to temp file in app data dir (protected by FBE)
        let key_path = app_data.join(format!("ssh-key-{alias}"));
        fs::write(&key_path, &key_bytes)
            .map_err(|e| format!("Cannot write key file: {e}"))?;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Cannot set key permissions: {e}"))?;

        // Lazy cleanup of stale sockets from crashed sessions
        let _ = fs::remove_file(socket_path);

        // Find SSH binary: bundled path > system PATH
        let ssh_path = find_ssh_binary(&app_data);

        let child = Command::new(&ssh_path)
            .args([
                "-N",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-i",
                &key_path.to_string_lossy(),
                "-L",
                &format!("{}:{}", socket_path.display(), remote_path),
                hostname,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("SSH binary at {ssh_path}: {e}"))?;

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
                "SSH tunnel to {hostname} did not create socket (check auth and that socat is on remote)"
            ));
        }

        Ok(Self {
            child: Some(child),
            socket_path: socket_path.to_path_buf(),
            key_path,
        })
    }
}

impl TunnelImpl for AndroidTunnel {
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
        let _ = fs::remove_file(&self.key_path);
    }
}

/// Find the SSH binary. Tries bundled location first, then system PATH.
fn find_ssh_binary(app_data: &Path) -> String {
    let bundled = app_data.join("ssh").join("ssh");
    if bundled.exists() {
        return bundled.to_string_lossy().to_string();
    }
    // Fallback for Termux or custom setups
    for path in &[
        "/data/data/com.termux/files/usr/bin/ssh",
        "/system/bin/ssh",
        "/usr/bin/ssh",
    ] {
        if Path::new(path).exists() {
            return path.to_string();
        }
    }
    "ssh".to_string() // hope it's on PATH
}
