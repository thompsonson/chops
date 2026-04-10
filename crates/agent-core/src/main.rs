use anyhow::Result;
use chops_common::{
    self, DEFAULT_MQTT_HOST, MQTT_KEEP_ALIVE_SECS, MQTT_QUEUE_CAPACITY, MQTT_RECONNECT_DELAY_SECS,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tracing::{info, warn};

const TRANSCRIPTION_TOPIC: &str = "voice/transcriptions";
const INTENT_REQUEST_TOPIC: &str = "agent/intent/request";
const INTENT_RESPONSE_TOPIC: &str = "agent/intent/response";
const WORKFLOW_EVENTS_TOPIC: &str = "agent/workflow/events";
const WORKFLOW_ESCALATION_TOPIC: &str = "agent/workflow/escalation";

#[derive(Deserialize)]
struct TranscriptionMessage {
    text: String,
    is_final: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut mqttoptions =
        MqttOptions::new("agent-core", DEFAULT_MQTT_HOST, chops_common::mqtt_port());
    mqttoptions.set_keep_alive(Duration::from_secs(MQTT_KEEP_ALIVE_SECS));
    let (client, mut eventloop) = AsyncClient::new(mqttoptions, MQTT_QUEUE_CAPACITY);

    client
        .subscribe(TRANSCRIPTION_TOPIC, QoS::AtMostOnce)
        .await?;
    client
        .subscribe(INTENT_RESPONSE_TOPIC, QoS::AtLeastOnce)
        .await?;
    client
        .subscribe(WORKFLOW_EVENTS_TOPIC, QoS::AtMostOnce)
        .await?;
    client
        .subscribe(WORKFLOW_ESCALATION_TOPIC, QoS::AtLeastOnce)
        .await?;
    info!("Agent-core started — relay mode (all transcriptions → AtomicGuard)");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(msg))) => {
                let payload = std::str::from_utf8(&msg.payload)?;
                match msg.topic.as_str() {
                    TRANSCRIPTION_TOPIC => {
                        if let Ok(transcription) =
                            serde_json::from_str::<TranscriptionMessage>(payload)
                        {
                            if transcription.is_final {
                                let text = transcription.text.trim();
                                if text.is_empty() {
                                    continue;
                                }
                                info!("Forwarding to AtomicGuard: '{text}'");
                                if let Err(e) = client
                                    .publish(INTENT_REQUEST_TOPIC, QoS::AtLeastOnce, false, text)
                                    .await
                                {
                                    warn!("Failed to forward to AtomicGuard: {e}");
                                }
                            }
                        } else {
                            warn!("Failed to deserialize transcription: {payload}");
                        }
                    }
                    INTENT_RESPONSE_TOPIC => {
                        handle_intent_response(&client, payload).await;
                    }
                    WORKFLOW_EVENTS_TOPIC => {
                        handle_workflow_event(&client, payload).await;
                    }
                    WORKFLOW_ESCALATION_TOPIC => {
                        handle_workflow_escalation(&client, payload).await;
                    }
                    _ => {}
                }
            }
            Err(e) => {
                warn!("MQTT error: {e}. Reconnecting...");
                tokio::time::sleep(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS)).await;
            }
            _ => {}
        }
    }
}

async fn publish_response(client: &AsyncClient, type_: &str, level: &str, message: &str) {
    let response = json!({
        "source": "agent",
        "type": type_,
        "level": level,
        "message": message,
    });
    let _ = client
        .publish(
            "agent/responses",
            QoS::AtMostOnce,
            false,
            response.to_string(),
        )
        .await;
}

async fn handle_intent_response(client: &AsyncClient, payload: &str) {
    let response: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => {
            warn!("Invalid intent response JSON: {e}");
            return;
        }
    };
    match response["status"].as_str() {
        Some("success") => {
            let workflow = response["intent"]["workflow"].as_str().unwrap_or("unknown");
            info!("AtomicGuard running workflow: {workflow}");
            publish_response(client, "toast", "info", &format!("Running {workflow}...")).await;
        }
        Some("failed") => {
            let error = response["error"].as_str().unwrap_or("unknown error");
            warn!("AtomicGuard could not parse intent: {error}");
            publish_response(client, "toast", "warn", "I didn't understand that").await;
        }
        Some("escalated") => {
            let error = response["error"].as_str().unwrap_or("unknown");
            warn!("AtomicGuard rejected command: {error}");
            publish_response(client, "toast", "error", &format!("Rejected: {error}")).await;
        }
        _ => {
            warn!("Unknown intent response status: {payload}");
        }
    }
}

async fn handle_workflow_event(client: &AsyncClient, payload: &str) {
    let event: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => {
            warn!("Invalid workflow event JSON: {e}");
            return;
        }
    };
    let wf = event["workflow"].as_str().unwrap_or("?");
    let etype = event["type"].as_str().unwrap_or("?");
    let step = event["step"].as_str().unwrap_or("?");
    info!("Workflow event: {wf} {etype} {step}");

    let response = json!({
        "source": "atomicguard",
        "type": "workflow_event",
        "level": "info",
        "message": payload,
    });
    let _ = client
        .publish(
            "agent/responses",
            QoS::AtMostOnce,
            false,
            response.to_string(),
        )
        .await;
}

async fn handle_workflow_escalation(client: &AsyncClient, payload: &str) {
    let esc: serde_json::Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => {
            warn!("Invalid escalation JSON: {e}");
            return;
        }
    };
    let wf = esc["workflow"].as_str().unwrap_or("?");
    let step = esc["step"].as_str().unwrap_or("?");
    let feedback = esc["feedback"].as_str().unwrap_or("unknown");
    warn!("ESCALATION: {wf} step {step} — {feedback}");

    publish_response(
        client,
        "toast",
        "error",
        &format!("ESCALATION: {wf}/{step} — {feedback}"),
    )
    .await;

    let response = json!({
        "source": "atomicguard",
        "type": "escalation",
        "level": "error",
        "message": payload,
    });
    let _ = client
        .publish(
            "agent/responses",
            QoS::AtMostOnce,
            false,
            response.to_string(),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_transcription_final() {
        let json = r#"{"text":"hello world","is_final":true}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, "hello world");
        assert!(msg.is_final);
    }

    #[test]
    fn deserialize_transcription_partial() {
        let json = r#"{"text":"hel","is_final":false}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert!(!msg.is_final);
    }

    #[test]
    fn deserialize_ignores_extra_fields() {
        let json = r#"{"text":"hello","is_final":true,"timestamp":"2026-01-01T00:00:00Z"}"#;
        let msg: TranscriptionMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, "hello");
    }

    #[test]
    fn deserialize_intent_response_success() {
        let json = r#"{"status":"success","intent":{"intent":"workflow","workflow":"health_check"},"attempts":1}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["status"], "success");
        assert_eq!(v["intent"]["workflow"], "health_check");
    }

    #[test]
    fn deserialize_intent_response_failed() {
        let json = r#"{"status":"failed","error":"Missing or unknown intent type"}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["status"], "failed");
        assert!(v["error"].as_str().unwrap().contains("unknown intent"));
    }

    #[test]
    fn deserialize_intent_response_escalated() {
        let json = r#"{"status":"escalated","error":"Command rejected by safety check"}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["status"], "escalated");
    }

    #[test]
    fn deserialize_workflow_event_step_complete() {
        let json = r#"{"type":"step_complete","workflow":"health_check","workflow_id":"abc","step":"health","passed":true}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["type"], "step_complete");
        assert_eq!(v["passed"], true);
        assert_eq!(v["workflow"], "health_check");
    }

    #[test]
    fn deserialize_workflow_complete() {
        let json = r#"{"type":"workflow_complete","workflow":"health_check","workflow_id":"abc","status":"success","summary":"health: PASS"}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["type"], "workflow_complete");
        assert_eq!(v["status"], "success");
    }

    #[test]
    fn deserialize_escalation() {
        let json = r#"{"type":"escalation","workflow":"restart_service","step":"restart","reason":"Invariant E3","feedback":"Unit not found"}"#;
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["type"], "escalation");
        assert_eq!(v["feedback"], "Unit not found");
        assert_eq!(v["workflow"], "restart_service");
    }
}
