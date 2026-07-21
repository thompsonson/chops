use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client;
use russh::keys::key;
use russh::{ChannelMsg, Disconnect};
use tracing::{error, info};

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

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("Session open failed: {e}"))?;

    let cmd = format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && \
         grep -qF {key} ~/.ssh/authorized_keys 2>/dev/null || \
         echo {key} >> ~/.ssh/authorized_keys",
        key = sh_quote(public_key)
    );

    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| format!("Exec failed: {e}"))?;

    let mut stderr = Vec::new();
    let mut exit_status = None;

    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { .. }) => {}
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

    if exit_status != Some(0) {
        let err_msg = String::from_utf8_lossy(&stderr);
        error!("Key install failed (exit={:?}): {}", exit_status, err_msg);
        return Err(format!(
            "Remote command failed (exit={:?}): {}",
            exit_status, err_msg
        ));
    }

    info!("SSH key authorized for {username}@{hostname}");

    handle
        .disconnect(Disconnect::ByApplication, "", "English")
        .await
        .map_err(|e| format!("Disconnect failed: {e}"))?;

    Ok(format!("SSH key authorized for {}@{}", username, hostname))
}
