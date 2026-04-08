# AtomicGuard Workflow Integration Specification

This document specifies how chops integrates with AtomicGuard's workflow engine for voice-triggered sysadmin operations. It is written for the chops side — what chops needs to publish, subscribe to, and display.

**AtomicGuard repo:** `/home/mt/Projects/thompsonson/atomicguard/`
**Design doc:** `docs/design/notes/hierarchical_workflow_dispatch.md`
**Working examples:** `examples/sysadmin/` (run_full_pipeline.py, mqtt_intent_listener.py)

---

## Architecture Overview

```
┌─────────────┐    voice/transcriptions     ┌──────────────┐
│ stt-publisher│ ─────────────────────────→  │  agent-core  │
│ (whisper)    │                              │              │
└─────────────┘                              │  1. Try regex │
                                             │  2. If no    │
                                             │     match →  │
┌─────────────┐    agent/commands/tmux       │     publish  │
│plugin-runner │ ←─────────────────────────  │     to AG    │
│(tmux/vscode) │                             └──────┬───────┘
└─────────────┘                                     │
                                                    │ agent/intent/request
                                                    ▼
                                             ┌──────────────┐
                                             │  AtomicGuard  │
                                             │  MQTT Listener│
                                             │              │
                                             │ intent AP →  │
                                             │ dispatch →   │
                                             │ child wf →   │
                                             │ real commands │
                                             └──────┬───────┘
                                                    │
                              ┌──────────────────────┼──────────────────────┐
                              │                      │                      │
                              ▼                      ▼                      ▼
                    agent/intent/response   agent/workflow/events   agent/workflow/escalation
                              │                      │                      │
                              ▼                      ▼                      ▼
                       ┌──────────────┐       ┌──────────┐          ┌──────────┐
                       │  agent-core  │       │  web-ui   │          │ agent-core│
                       │  (dispatch   │       │  (live    │          │ (notify   │
                       │   result)    │       │   status) │          │  user)    │
                       └──────────────┘       └──────────┘          └──────────┘
```

---

## MQTT Topics

### Topics chops publishes TO AtomicGuard

#### `agent/intent/request` (QoS 1)

Published by **agent-core** when regex parsing fails (the LLM fallback path from `llm-intent-parsing.md`).

**Payload:** Plain text — the finalised whisper transcript.

```
check the system
```

```
restart chops web
```

```
what's using memory
```

**When to publish:** When `parse_intent()` returns `None` (no regex match). This is the long-tail handler — natural phrasings, novel commands, workflow triggers that don't match existing patterns.

**Implementation in agent-core:**

```rust
// In the main dispatch loop, after parse_intent() fails:
match parse_intent(&text, &known_projects) {
    Some(intent_match) => {
        // Existing dispatch: tmux, vscode, termux
        dispatch_intent(client, intent_match).await;
    }
    None => {
        // LLM fallback: send to AtomicGuard for intent extraction + workflow dispatch
        info!("No regex match for '{}', forwarding to AtomicGuard", text);
        client.publish(
            "agent/intent/request",
            QoS::AtLeastOnce,
            false,
            text.as_bytes(),
        ).await?;
    }
}
```

### Topics chops subscribes TO from AtomicGuard

#### `agent/intent/response` (QoS 1)

The parsed intent result. Chops uses this to confirm what AtomicGuard understood and optionally speak/display it.

**Payload:** JSON

```json
{
  "status": "success",
  "intent": {
    "intent": "workflow",
    "workflow": "health_check"
  },
  "attempts": 1
}
```

```json
{
  "status": "failed",
  "error": "Missing or unknown intent type: 'unknown'. Valid: query, termux, tmux, vscode, workflow"
}
```

```json
{
  "status": "escalated",
  "error": "Command rejected by safety check: contains 'sudo '"
}
```

**What chops does with it:**
- `status: "success"` → display/speak "Running health check..." (or whatever the workflow is)
- `status: "failed"` → speak "I didn't understand that"
- `status: "escalated"` → speak "That command was rejected for safety reasons"

#### `agent/workflow/events` (QoS 0)

Step-level progress updates. Lossy is fine — these are for live display, not guaranteed delivery.

**Payload:** JSON

```json
{
  "type": "step_start",
  "workflow": "health_check",
  "workflow_id": "uuid",
  "step": "health",
  "attempt": 1,
  "timestamp": "2026-04-08T21:00:00"
}
```

```json
{
  "type": "step_complete",
  "workflow": "health_check",
  "workflow_id": "uuid",
  "step": "health",
  "attempt": 1,
  "passed": true,
  "feedback": null,
  "output_preview": "Host: pop-mini, Uptime: 8d, Load: 0.16...",
  "timestamp": "2026-04-08T21:00:01"
}
```

```json
{
  "type": "workflow_complete",
  "workflow": "health_check",
  "workflow_id": "uuid",
  "status": "success",
  "summary": "health: PASS, warnings: PASS, services: PASS",
  "timestamp": "2026-04-08T21:00:03"
}
```

```json
{
  "type": "workflow_complete",
  "workflow": "restart_service",
  "workflow_id": "uuid",
  "status": "failed",
  "summary": "restart: FAIL (exit code 5)",
  "failed_step": "restart",
  "timestamp": "2026-04-08T21:00:05"
}
```

**What chops does with it:**
- `step_start` → update web-ui progress indicator
- `step_complete` → update step status (green tick / red cross)
- `workflow_complete` + `success` → speak "Health check passed. All systems normal."
- `workflow_complete` + `failed` → speak "Health check failed at step: warnings"

#### `agent/workflow/escalation` (QoS 1)

Must-deliver alerts requiring human attention. These are serious — a service failed to restart, an irreversible action went wrong.

**Payload:** JSON

```json
{
  "type": "escalation",
  "workflow": "restart_service",
  "workflow_id": "uuid",
  "step": "restart",
  "reason": "Irreversible effector failed post-guard (Invariant E3)",
  "feedback": "Unit nonexistent.service not found",
  "attempts": 1,
  "timestamp": "2026-04-08T21:00:05"
}
```

**What chops does with it:**
- **Always notify** — toast notification + audible alert
- Speak: "Attention: restart service escalated. The service could not be found."
- Display full feedback in web-ui escalation panel
- Do NOT auto-retry — escalation means "a human needs to decide"

---

## Available Workflows

These are the pre-compiled task workflows in AtomicGuard's catalogue. Chops does not need to know their internal structure — just the names and what voice commands trigger them.

| Workflow | Voice triggers | What it does |
|----------|---------------|-------------|
| `health_check` | "check the system", "system status", "any warnings" | Runs `sysmon status`, `sysmon warn`, `systemctl --user list-units` |
| `disk_check` | "check disk", "disk space", "how's the disk" | Runs `sysmon disk` |
| `resource_monitor` | "what's using memory", "top processes" | Runs `sysmon mem`, `sysmon proc` |
| `restart_service` | "restart chops web", "restart the web server" | Restarts a systemd user service (non-idempotent, has undo) |

New workflows are added to AtomicGuard's catalogue without any changes to chops. The LLM intent parser learns to route to them via the prompt template (which lists known workflows).

---

## Integration Steps for agent-core

### 1. Add MQTT subscriptions

Subscribe to three new topics on startup:

```rust
client.subscribe("agent/intent/response", QoS::AtLeastOnce).await?;
client.subscribe("agent/workflow/events", QoS::AtMostOnce).await?;
client.subscribe("agent/workflow/escalation", QoS::AtLeastOnce).await?;
```

### 2. Add fallback dispatch in the message handler

When `parse_intent()` returns `None`, publish to `agent/intent/request`:

```rust
None => {
    client.publish(
        "agent/intent/request",
        QoS::AtLeastOnce,
        false,
        text.as_bytes(),
    ).await?;
}
```

### 3. Handle incoming events

In the MQTT event loop, add handlers for the new topics:

```rust
"agent/intent/response" => {
    let response: serde_json::Value = serde_json::from_slice(&payload)?;
    match response["status"].as_str() {
        Some("success") => {
            let workflow = response["intent"]["workflow"].as_str().unwrap_or("unknown");
            info!("AtomicGuard running workflow: {}", workflow);
            // Update UI / speak confirmation
        }
        Some("failed") => {
            warn!("AtomicGuard could not parse intent: {}", response["error"]);
            // Speak "I didn't understand that"
        }
        Some("escalated") => {
            warn!("AtomicGuard rejected command: {}", response["error"]);
            // Speak safety rejection
        }
        _ => {}
    }
}

"agent/workflow/events" => {
    let event: serde_json::Value = serde_json::from_slice(&payload)?;
    // Forward to web-ui via existing response topic or handle directly
    info!("Workflow event: {} {} {}", 
        event["workflow"], event["type"], event["step"]);
}

"agent/workflow/escalation" => {
    let esc: serde_json::Value = serde_json::from_slice(&payload)?;
    error!("ESCALATION: {} step {} — {}", 
        esc["workflow"], esc["step"], esc["feedback"]);
    // Audible alert + toast notification
}
```

---

## Integration Steps for web-ui

### 1. Subscribe to workflow events

In the MQTT.js connection (web PWA), subscribe to:

```javascript
client.subscribe('agent/workflow/events');
client.subscribe('agent/workflow/escalation');
client.subscribe('agent/intent/response');
```

### 2. Display workflow progress

On `agent/workflow/events`:
- `step_start` → add spinner to step in workflow panel
- `step_complete` → replace spinner with ✓ (green) or ✗ (red)
- `workflow_complete` → show summary banner

### 3. Display escalations

On `agent/workflow/escalation`:
- Show red alert banner with full feedback text
- Play notification sound
- Keep visible until dismissed

---

## Testing

### Quick test (no code changes needed)

AtomicGuard's MQTT listener is already built. Test the round-trip:

```bash
# Terminal 1: Start AtomicGuard listener
cd /home/mt/Projects/thompsonson/atomicguard
source .env
uv run python -m examples.sysadmin.mqtt_intent_listener --port 1884

# Terminal 2: Simulate voice command
mosquitto_pub -h localhost -p 1884 -t agent/intent/request -m "check the system"

# Terminal 3: Watch responses
mosquitto_sub -h localhost -p 1884 -t "agent/#" -v
```

### Full pipeline test (with real whisper)

1. Start AtomicGuard MQTT listener
2. Start chops services (`dev chops`)
3. Speak a command that doesn't match regex (e.g., "can you check the system please")
4. Observe: whisper → agent-core → MQTT → AtomicGuard → workflow → events back to chops

---

## What AtomicGuard Handles (chops does NOT need to implement)

- Intent extraction (LLM + IntentGuard with retry)
- Workflow selection and parameter extraction
- Workflow execution (command generation, effector execution, guard validation)
- Retry logic (stagnation detection, backtracking)
- Undo/compensation for failed non-idempotent operations
- Artifact persistence (full audit trail in filesystem DAG)

Chops is the **voice interface and notification layer**. AtomicGuard is the **workflow engine**.
