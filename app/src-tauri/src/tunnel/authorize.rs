use russh::client;
use std::sync::Arc;

struct ClientHandler;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub async fn authorize_key(
    hostname: &str,
    port: u16,
    username: &str,
    password: &str,
    public_key: &str,
) -> Result<String, String> {
    let config = Arc::new(client::Config::default());
    let handler = ClientHandler;

    let mut handle = client::connect(config, (hostname, port), handler)
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    let auth_result = handle
        .authenticate_password(username, password)
        .await
        .map_err(|e| format!("Auth failed: {e}"))?;

    if !auth_result {
        return Err("Authentication rejected by server".to_string());
    }

    let mut session = handle
        .open_session()
        .await
        .map_err(|e| format!("Session open failed: {e}"))?;

    let cmd = format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys && \
         grep -qF {key} ~/.ssh/authorized_keys 2>/dev/null || \
         echo {key} >> ~/.ssh/authorized_keys",
        key = sh_quote(public_key)
    );

    session
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| format!("Exec failed: {e}"))?;

    let mut stderr = Vec::new();
    let mut exit_status = None;

    loop {
        match session.wait().await {
            Some(client::SessionEvent::Stdout { data }) => {
                stdout.extend_from_slice(&data);
            }
            Some(client::SessionEvent::Stderr { data }) => {
                stderr.extend_from_slice(&data);
            }
            Some(client::SessionEvent::ExitStatus { status }) => {
                exit_status = Some(status);
            }
            Some(client::SessionEvent::Eof) | None => break,
        }
    }

    if exit_status != Some(0) {
        let err_msg = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "Remote command failed (exit={:?}): {}",
            exit_status, err_msg
        ));
    }

    handle
        .close()
        .await
        .map_err(|e| format!("Close failed: {e}"))?;

    Ok(format!("SSH key authorized for {}@{}", username, hostname))
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
