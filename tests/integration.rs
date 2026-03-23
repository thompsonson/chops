//! Integration tests for the chops MQTT pipeline.
//!
//! These tests require a running Mosquitto broker on the configured port.
//! Tests auto-skip if the broker is unavailable.

use agent_core::intent::{parse_intent, Intent, ParseContext};
use chops_common::{Pane, TmuxCommand};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::json;
use std::time::Duration;

const DEFAULT_MQTT_PORT: u16 = 1884;

fn mqtt_port() -> u16 {
    std::env::var("CHOPS_MQTT_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MQTT_PORT)
}

fn mqtt_available() -> bool {
    std::net::TcpStream::connect(("127.0.0.1", mqtt_port())).is_ok()
}

fn test_ctx() -> ParseContext {
    ParseContext {
        known_projects: vec![
            "chops".into(),
            "manta-deploy".into(),
            "atomicguard".into(),
            "dotfiles".into(),
        ],
    }
}

/// Helper: create an MQTT client with a unique ID.
async fn make_client(id: &str) -> (AsyncClient, rumqttc::EventLoop) {
    let mut opts = MqttOptions::new(id, "localhost", mqtt_port());
    opts.set_keep_alive(Duration::from_secs(10));
    AsyncClient::new(opts, 10)
}

/// Wait for the first Publish message on the event loop, with a timeout.
async fn recv_publish(eventloop: &mut rumqttc::EventLoop, timeout: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, eventloop.poll()).await {
            Ok(Ok(Event::Incoming(Incoming::Publish(msg)))) => {
                return Some(String::from_utf8_lossy(&msg.payload).to_string());
            }
            Ok(Ok(_)) => continue, // SubAck, ConnAck, etc.
            Ok(Err(_)) | Err(_) => return None,
        }
    }
}

/// Helper: publish a transcription and collect the routed command from the expected topic.
async fn publish_and_route(text: &str, expected_topic: &str) -> Option<String> {
    // Subscriber listens on the expected command topic.
    let (sub_client, mut sub_eventloop) =
        make_client(&format!("test-sub-{}", text.len())).await;
    sub_client
        .subscribe(expected_topic, QoS::AtMostOnce)
        .await
        .unwrap();
    let _ = sub_eventloop.poll().await;
    let _ = sub_eventloop.poll().await;

    // Publisher simulates agent-core: parse intent, dispatch to topic.
    let (pub_client, mut pub_eventloop) =
        make_client(&format!("test-pub-{}", text.len())).await;
    tokio::spawn(async move {
        loop {
            if pub_eventloop.poll().await.is_err() {
                break;
            }
        }
    });

    let ctx = test_ctx();
    if let Some(m) = parse_intent(text, &ctx) {
        let (topic, payload) = match m.intent {
            Intent::Tmux(ref cmd) => {
                ("agent/commands/tmux", serde_json::to_string(cmd).unwrap())
            }
            Intent::Vscode(ref f) => ("agent/commands/vscode", f.clone()),
            Intent::Termux(ref c) => ("agent/commands/termux", c.clone()),
        };

        if topic != expected_topic {
            return None;
        }

        pub_client
            .publish(topic, QoS::AtMostOnce, false, &payload)
            .await
            .unwrap();

        recv_publish(&mut sub_eventloop, Duration::from_secs(3)).await
    } else {
        None
    }
}

// --- Pipeline: intent parsing → MQTT routing ---

#[tokio::test]
async fn pipeline_routes_tmux_run_command() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    let received = publish_and_route("in chops run cargo test", "agent/commands/tmux").await;
    assert!(received.is_some());
    let cmd: TmuxCommand = serde_json::from_str(&received.unwrap()).unwrap();
    assert_eq!(cmd.session, Some("chops".into()));
    assert_eq!(cmd.pane, Pane::Shell);
    assert_eq!(cmd.command, "cargo test");
}

#[tokio::test]
async fn pipeline_routes_tmux_tell_claude() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    let received =
        publish_and_route("in chops tell claude fix the tests", "agent/commands/tmux").await;
    assert!(received.is_some());
    let cmd: TmuxCommand = serde_json::from_str(&received.unwrap()).unwrap();
    assert_eq!(cmd.session, Some("chops".into()));
    assert_eq!(cmd.pane, Pane::Claude);
    assert_eq!(cmd.command, "fix the tests");
}

#[tokio::test]
async fn pipeline_routes_vscode_command() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    let received = publish_and_route("open vscode README.md", "agent/commands/vscode").await;
    assert_eq!(received.as_deref(), Some("readme.md"));
}

#[tokio::test]
async fn pipeline_routes_noisy_whisper_input() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    // Filler + punctuation + synonym + project typo
    let received = publish_and_route(
        "Uh, okay, please in chop execute cargo test.",
        "agent/commands/tmux",
    )
    .await;
    assert!(received.is_some());
    let cmd: TmuxCommand = serde_json::from_str(&received.unwrap()).unwrap();
    assert_eq!(cmd.session, Some("chops".into())); // fuzzy matched
    assert_eq!(cmd.pane, Pane::Shell);
    assert_eq!(cmd.command, "cargo test");
}

#[tokio::test]
async fn pipeline_ignores_partial_transcriptions() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    // Subscribe to all agent command topics.
    let (sub_client, mut sub_eventloop) = make_client("test-sub-partial").await;
    sub_client
        .subscribe("agent/commands/#", QoS::AtMostOnce)
        .await
        .unwrap();
    let _ = sub_eventloop.poll().await;
    let _ = sub_eventloop.poll().await;

    // Publish a partial transcription (is_final: false).
    let (pub_client, mut pub_eventloop) = make_client("test-pub-partial").await;
    tokio::spawn(async move {
        loop {
            if pub_eventloop.poll().await.is_err() {
                break;
            }
        }
    });

    let partial = json!({
        "text": "open vs",
        "is_final": false,
        "timestamp": "2026-01-01T00:00:00Z"
    });
    pub_client
        .publish(
            "voice/transcriptions",
            QoS::AtMostOnce,
            false,
            partial.to_string(),
        )
        .await
        .unwrap();

    // Nothing should arrive on command topics within a reasonable window.
    let received = recv_publish(&mut sub_eventloop, Duration::from_secs(1)).await;
    assert!(
        received.is_none(),
        "Partial transcription should not produce a command, got: {received:?}"
    );
}

#[tokio::test]
async fn pipeline_roundtrip_publish_subscribe() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    let (sub_client, mut sub_eventloop) = make_client("test-sub-roundtrip").await;
    sub_client
        .subscribe("test/roundtrip", QoS::AtMostOnce)
        .await
        .unwrap();
    let _ = sub_eventloop.poll().await;
    let _ = sub_eventloop.poll().await;

    let (pub_client, mut pub_eventloop) = make_client("test-pub-roundtrip").await;
    tokio::spawn(async move {
        loop {
            if pub_eventloop.poll().await.is_err() {
                break;
            }
        }
    });

    let payload =
        json!({"text": "hello", "is_final": true, "timestamp": "2026-01-01T00:00:00Z"});
    pub_client
        .publish(
            "test/roundtrip",
            QoS::AtMostOnce,
            false,
            payload.to_string(),
        )
        .await
        .unwrap();

    let received = recv_publish(&mut sub_eventloop, Duration::from_secs(3)).await;
    assert!(received.is_some(), "Should receive the published message");

    let parsed: serde_json::Value = serde_json::from_str(&received.unwrap()).unwrap();
    assert_eq!(parsed["text"], "hello");
    assert_eq!(parsed["is_final"], true);
}
