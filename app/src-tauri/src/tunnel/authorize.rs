use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client;
use russh::keys::key;
use russh::{ChannelMsg, Disconnect};
use tracing::{debug, error, info, warn};

use crate::tunnel::sh_quote;

struct ClientHandler {
    hostname: String,
    port: u16,
    app_data: PathBuf,
}

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
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

fn build_install_command(public_key: &str) -> String {
    format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && \
         (grep -qF {key} ~/.ssh/authorized_keys 2>/dev/null || \
          echo {key} >> ~/.ssh/authorized_keys) && echo OK",
        key = sh_quote(public_key)
    )
}

pub async fn authorize_key(
    app_data: &Path,
    hostname: &str,
    port: u16,
    username: &str,
    password: &str,
    public_key: &str,
) -> Result<String, String> {
    let config = Arc::new(client::Config::default());
    let handler = ClientHandler {
        hostname: hostname.to_string(),
        port,
        app_data: app_data.to_path_buf(),
    };

    let mut handle = client::connect(config, (hostname, port), handler)
        .await
        .map_err(|e| format!("Connection to {hostname}:{port} failed: {e}"))?;

    info!("SSH connected to {hostname}:{port}");

    let auth_result = handle
        .authenticate_password(username, password)
        .await
        .map_err(|e| format!("Auth failed: {e}"))?;

    if !auth_result {
        error!("Password auth rejected for {username}@{hostname}");
        return Err("Authentication rejected by server".to_string());
    }

    info!("Password auth ok for {username}@{hostname}");

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("Session open failed: {e}"))?;

    info!("Executing key install command on {hostname}");

    let cmd = build_install_command(public_key);

    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| format!("Exec failed: {e}"))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;

    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => {
                stdout.extend_from_slice(&data);
            }
            Some(ChannelMsg::ExtendedData { data, .. }) => {
                stderr.extend_from_slice(&data);
            }
            Some(ChannelMsg::ExitStatus { exit_status: s }) => {
                exit_status = Some(s);
            }
            Some(ChannelMsg::Eof) | None => break,
            _ => {}
        }
    }

    debug!("Key install stdout: {:?}", String::from_utf8_lossy(&stdout));

    match exit_status {
        Some(0) => {}
        None => {
            warn!("Key install exit status not received (likely OK, key was installed)");
        }
        Some(code) => {
            let err_msg = String::from_utf8_lossy(&stderr);
            error!("Key install failed (exit={code}): {err_msg}");
            return Err(format!(
                "Remote command failed (exit={code}): {err_msg}",
            ));
        }
    }

    info!("SSH key authorized for {username}@{hostname}");

    handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await
        .map_err(|e| format!("Disconnect failed: {e}"))?;

    Ok(format!("SSH key authorized for {}@{}", username, hostname))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_install_command_contains_echo_ok() {
        let cmd = build_install_command("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test-key");
        assert!(cmd.ends_with("&& echo OK"), "command should produce stdout: {cmd}");
    }

    #[test]
    fn test_key_install_command_parens_wrap_grep_echo() {
        let cmd = build_install_command("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test-key");
        assert!(
            cmd.contains("(grep -qF"),
            "grep should be wrapped in parens: {cmd}"
        );
        assert!(
            cmd.contains(">> ~/.ssh/authorized_keys)"),
            "echo should be closed in parens: {cmd}"
        );
    }
}
