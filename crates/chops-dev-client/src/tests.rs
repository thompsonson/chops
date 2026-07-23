use super::*;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

// ---------------------------------------------------------------------------
// Helpers — fake daemon
// ---------------------------------------------------------------------------

/// Atomic counter for unique socket paths per test.
static TEST_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Spawn a fake UDS server that replies with a canned HTTP response.
/// Returns the socket path (in a temp dir) and a join handle.
async fn fake_daemon(response_body: &str, status: u16) -> (PathBuf, tokio::task::JoinHandle<()>) {
    let id = TEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "chops-dev-client-test-{}-{}",
        std::process::id(),
        id
    ));
    let _ = std::fs::create_dir_all(&dir);
    let sock = dir.join("dev.sock");
    // Remove stale socket from previous test run.
    let _ = std::fs::remove_file(&sock);

    let listener = UnixListener::bind(&sock).unwrap();
    let body = response_body.to_string();

    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read request (we don't parse it in the fake).
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();

        let resp = format!(
            "HTTP/1.1 {status} OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    (sock, handle)
}

// ---------------------------------------------------------------------------
// Unit tests against fake daemon
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_parses_sessions_and_projects() {
    let body = serde_json::json!({
        "sessions": [{
            "name": "chops",
            "pane_count": 2,
            "attached": true,
            "last_activity": 1775983903,
            "layout": "claude",
            "agent": "claude",
            "active_command": "claude",
            "agent_running": true,
            "agent_session_id": "dev-abc123",
            "responsibility": "maintain CI pipelines",
            "project_path": "/home/mt/Projects/chops",
            "repository": "github.com/thompsonson/chops",
            "host": "pop-mini"
        }],
        "projects": [{
            "name": "manta-deploy",
            "path": "/home/mt/Projects/manta/manta-deploy",
            "layout": "default",
            "host": null
        }]
    });

    let (sock, handle) = fake_daemon(&body.to_string(), 200).await;
    let client = DevClient::new(sock);

    let listing = client.list().await.unwrap();
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].name, "chops");
    assert_eq!(listing.sessions[0].pane_count, 2);
    assert!(listing.sessions[0].attached);
    assert_eq!(listing.sessions[0].layout, "claude");
    assert_eq!(listing.sessions[0].agent.as_deref(), Some("claude"));
    assert_eq!(listing.sessions[0].active_command.as_deref(), Some("claude"));
    assert!(listing.sessions[0].agent_running);
    assert_eq!(
        listing.sessions[0].agent_session_id.as_deref(),
        Some("dev-abc123")
    );
    assert_eq!(
        listing.sessions[0].responsibility.as_deref(),
        Some("maintain CI pipelines")
    );
    assert_eq!(
        listing.sessions[0].project_path.as_deref(),
        Some("/home/mt/Projects/chops")
    );
    assert_eq!(
        listing.sessions[0].repository.as_deref(),
        Some("github.com/thompsonson/chops")
    );
    assert_eq!(
        listing.sessions[0].host.as_deref(),
        Some("pop-mini")
    );

    assert_eq!(listing.projects.len(), 1);
    assert_eq!(listing.projects[0].name, "manta-deploy");
    assert!(listing.projects[0].host.is_none());

    handle.await.unwrap();
}

#[tokio::test]
async fn test_list_handles_missing_agent_fields() {
    let body = serde_json::json!({
        "sessions": [{
            "name": "chops",
            "pane_count": 2,
            "attached": true,
            "last_activity": 1775983903,
            "layout": "claude"
        }],
        "projects": []
    });

    let (sock, handle) = fake_daemon(&body.to_string(), 200).await;
    let client = DevClient::new(sock);

    let listing = client.list().await.unwrap();
    assert_eq!(listing.sessions.len(), 1);
    assert_eq!(listing.sessions[0].name, "chops");
    assert!(listing.sessions[0].agent.is_none());
    assert!(listing.sessions[0].active_command.is_none());
    assert!(!listing.sessions[0].agent_running);
    assert!(listing.sessions[0].agent_session_id.is_none());
    assert!(listing.sessions[0].responsibility.is_none());
    assert!(listing.sessions[0].project_path.is_none());
    assert!(listing.sessions[0].repository.is_none());
    assert!(listing.sessions[0].host.is_none());

    handle.await.unwrap();
}

#[tokio::test]
async fn test_start_returns_response_body() {
    let body = serde_json::json!({"status": "started", "session": "myproj"});
    let (sock, handle) = fake_daemon(&body.to_string(), 200).await;
    let client = DevClient::new(sock);

    let resp = client.start("myproj", Some("claude")).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(v["session"], "myproj");

    handle.await.unwrap();
}

#[tokio::test]
async fn test_stop_succeeds_on_200() {
    let (sock, handle) = fake_daemon("{}", 200).await;
    let client = DevClient::new(sock);

    client.stop("myproj").await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn test_send_keys_succeeds() {
    let body = r#"{"sent":11}"#;
    let (sock, handle) = fake_daemon(body, 202).await;
    let client = DevClient::new(sock);

    let resp = client
        .send_keys("chops", "1.1", "echo hello")
        .await
        .unwrap();
    assert!(resp.contains("sent"));
    handle.await.unwrap();
}

#[tokio::test]
async fn test_daemon_error_propagates() {
    let body = r#"{"error":"session not found"}"#;
    let (sock, handle) = fake_daemon(body, 404).await;
    let client = DevClient::new(sock);

    let err = client.stop("nonexistent").await.unwrap_err();
    match err {
        Error::DaemonError { status, body } => {
            assert_eq!(status, 404);
            assert!(body.contains("session not found"));
        }
        other => panic!("expected DaemonError, got: {other}"),
    }

    handle.await.unwrap();
}

#[tokio::test]
async fn test_connect_failure() {
    let client = DevClient::new(PathBuf::from("/tmp/nonexistent-chops-test.sock"));
    let err = client.list().await.unwrap_err();
    assert!(matches!(err, Error::Connect { .. }));
}

#[tokio::test]
async fn test_from_env_uses_xdg_runtime_dir() {
    // Temporarily override the env var.
    let prev = std::env::var_os("XDG_RUNTIME_DIR");
    std::env::set_var("XDG_RUNTIME_DIR", "/tmp/fake-xdg");
    let client = DevClient::from_env();
    assert_eq!(
        client.socket_path(),
        &PathBuf::from("/tmp/fake-xdg/dev.sock")
    );
    // Restore.
    match prev {
        Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
        None => std::env::remove_var("XDG_RUNTIME_DIR"),
    }
}

// ---------------------------------------------------------------------------
// Integration test — live daemon (skipped if socket missing)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_daemon_list() {
    let client = DevClient::from_env();
    if !Path::new(client.socket_path()).exists() {
        eprintln!("skipping live daemon test: socket not found");
        return;
    }

    let listing = client.list().await.unwrap();
    // The live daemon should always return the two fields.
    // We can't assert exact contents, but the shape must parse.
    eprintln!(
        "live daemon: {} sessions, {} projects",
        listing.sessions.len(),
        listing.projects.len()
    );
    assert!(!listing.sessions.is_empty() || !listing.projects.is_empty());
}
