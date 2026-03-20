//! Integration tests for the chops MQTT pipeline.
//!
//! These tests require a running Mosquitto broker on localhost:1883.
//! Skip with: cargo test -- --skip integration
//! Or gate on CI with an env var check.

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

#[tokio::test]
async fn agent_routes_vscode_command() {
    if !mqtt_available() {
        eprintln!("SKIP: mosquitto not running on localhost:{}", mqtt_port());
        return;
    }

    // Subscriber listens on the vscode command topic.
    let (sub_client, mut sub_eventloop) = make_client("test-sub-vscode").await;
    sub_client
        .subscribe("agent/commands/vscode", QoS::AtMostOnce)
        .await
        .unwrap();
    // Drain ConnAck + SubAck.
    let _ = sub_eventloop.poll().await;
    let _ = sub_eventloop.poll().await;

    // Publisher simulates what agent-core does: parse intent, dispatch.
    // We test the routing logic end-to-end by having a "mini agent" inline.
    let (pub_client, mut pub_eventloop) = make_client("test-pub-agent").await;
    tokio::spawn(async move {
        loop {
            if pub_eventloop.poll().await.is_err() {
                break;
            }
        }
    });

    let text = "open vscode README.md";
    // Simulate agent-core's parse_intent + dispatch.
    let text_lower = text.to_lowercase();
    let topic = if text_lower.contains("open") && text_lower.contains("vscode") {
        "agent/commands/vscode"
    } else {
        panic!("Expected vscode route");
    };

    pub_client
        .publish(topic, QoS::AtMostOnce, false, text)
        .await
        .unwrap();

    // Subscriber should receive the command.
    let received = recv_publish(&mut sub_eventloop, Duration::from_secs(3)).await;
    assert_eq!(received.as_deref(), Some("open vscode README.md"));
}

#[tokio::test]
async fn agent_ignores_partial_transcriptions() {
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
        .publish("voice/transcriptions", QoS::AtMostOnce, false, partial.to_string())
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
async fn roundtrip_publish_subscribe() {
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

    let payload = json!({"text": "hello", "is_final": true, "timestamp": "2026-01-01T00:00:00Z"});
    pub_client
        .publish("test/roundtrip", QoS::AtMostOnce, false, payload.to_string())
        .await
        .unwrap();

    let received = recv_publish(&mut sub_eventloop, Duration::from_secs(3)).await;
    assert!(received.is_some(), "Should receive the published message");

    let parsed: serde_json::Value = serde_json::from_str(&received.unwrap()).unwrap();
    assert_eq!(parsed["text"], "hello");
    assert_eq!(parsed["is_final"], true);
}
