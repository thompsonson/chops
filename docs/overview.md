# chops — System Overview

## What is chops?

chops (chat ops) is a local, offline voice-controlled agent system. You speak into a Bluetooth headset, whisper.cpp transcribes your speech, a Rust agent parses your intent, and the resulting command is routed to the right target — a tmux pane, VSCode, or a shell. Everything runs on your machine. No cloud, no API keys, no latency.

## Architecture

```
                        ┌──────────────────────────────────────────────────┐
                        │                  MQTT Broker                     │
                        │              mosquitto :1884                     │
                        │                                                  │
                        │  voice/          agent/commands/   agent/        │
                        │  transcriptions  tmux|vscode|...   responses     │
                        └──┬───────────────────┬─────────────────┬─────────┘
                           │                   │                 │
              ┌────────────┴──┐        ┌───────┴───────┐        │
              │               │        │               │        │
         ┌────▼────┐    ┌─────▼─────┐  │  ┌────────────▼──┐     │
         │   stt   │    │   agent   │  │  │    plugin     │     │
         │publisher│    │   core    │  │  │    runner     │     │
         └────┬────┘    └─────┬─────┘  │  └────────┬──────┘     │
              │               │        │           │            │
              │               │        │           ├──► tmux send-keys
  ┌───────────┴──┐            │        │           ├──► code (vscode)
  │ whisper.cpp  │            │        │           └──► bash (termux)
  │   stream     │     parse_intent()  │
  └───────┬──────┘     (pure function) │
          │                            │
  ┌───────┴──────┐                     │
  │  Bluetooth   │              ┌──────┴──────────────────┐
  │  microphone  │              │      tmux sessions      │
  └──────────────┘              │  (created by `dev` cmd) │
                                │                         │
                                │  ┌─────────┬─────────┐  │
                                │  │ claude  │  shell  │  │
                                │  │ (pane 1)│ (pane 2)│  │
                                │  └─────────┴─────────┘  │
                                └─────────────────────────┘
```

## Data Flow

```
 1. Speak          "in chops run cargo test"
                          │
 2. Transcribe            ▼
                   ┌──────────────┐
                   │ stt-publisher │──► MQTT: voice/transcriptions
                   └──────────────┘    {"text": "in chops run cargo test", "is_final": true}
                                                │
 3. Parse                                       ▼
                                        ┌──────────────┐
                                        │  agent-core  │  parse_intent() → Tmux {
                                        └──────────────┘    session: "chops",
                                                │           pane: "shell",
                                                │           command: "cargo test"
                                                │         }
 4. Route                                       ▼
                                        MQTT: agent/commands/tmux
                                                │
 5. Execute                                     ▼
                                        ┌──────────────┐
                                        │ plugin-runner │──► tmux send-keys -t chops:1.2 "cargo test" Enter
                                        └──────────────┘
                                                │
 6. Respond                                     ▼
                                        MQTT: agent/responses
                                        {"status": "ok", "output": "Sent to chops:1.2: "}
```

## Components

### stt-publisher

Spawns `whisper.cpp stream`, reads its stdout line by line, buffers partial transcriptions, and publishes finalized text to MQTT when it detects silence (800ms threshold) or sentence-ending punctuation.

- Supervisor loop auto-restarts whisper.cpp on crash
- Publishes partials (`is_final: false`) for UI feedback
- Publishes finals (`is_final: true`) when speech segment is complete

### agent-core

Subscribes to `voice/transcriptions`, ignores partials, and runs finalized text through `parse_intent()` — a pure function with no side effects. The parsed intent is routed to the appropriate plugin topic.

**Voice command patterns:**

| You say | What happens |
|---------|-------------|
| "in chops run cargo test" | Sends `cargo test` to the shell pane of the `chops` tmux session |
| "in chops tell claude fix the tests" | Sends `fix the tests` to the claude pane of the `chops` session |
| "run ls" | Sends `ls` to the active tmux session's shell pane |
| "open vscode main.rs" | Opens `main.rs` in VSCode |
| "termux echo hello" | Runs `echo hello` in bash |

### plugin-runner

Subscribes to all `agent/commands/*` topics. Each incoming command is executed in a separate tokio task (non-blocking). Results are published to `agent/responses`.

**tmux pane targeting:**
- Counts panes in the target session window
- 2-pane layout (from `dev claude <project>`): claude = pane 1, shell = pane 2
- 1-pane layout (from `dev <project>`): all commands go to pane 1
- 10-second timeout on all external commands

## MQTT Topics

```
voice/transcriptions          stt-publisher  ──►  agent-core
agent/commands/tmux           agent-core     ──►  plugin-runner
agent/commands/vscode         agent-core     ──►  plugin-runner
agent/commands/termux         agent-core     ──►  plugin-runner
agent/responses               plugin-runner  ──►  (any subscriber)
plugins/status/<name>         plugin-runner  ──►  (heartbeat)
```

All messages are JSON. Default MQTT port is **1884** (override with `CHOPS_MQTT_PORT`).

## Deployment (systemd)

All components run as systemd user services on the host machine. They start on boot, restart on failure, and survive logout.

```
~/.config/systemd/user/
├── chops-mosquitto.service   ← MQTT broker on port 1884
├── chops-agent.service       ← agent-core (intent parser + router)
├── chops-plugin.service      ← plugin-runner (tmux/vscode/termux executor)
└── chops-stt.service         ← stt-publisher (enable when whisper.cpp is ready)
```

**Service dependencies:**

```
chops-mosquitto
    ├──► chops-agent    (Requires + After)
    ├──► chops-plugin   (Requires + After)
    └──► chops-stt      (Requires + After)
```

**Common commands:**

```bash
# Status of all services
systemctl --user status chops-mosquitto chops-agent chops-plugin

# Live logs
journalctl --user -u chops-agent -u chops-plugin -f

# Restart after rebuild
cargo build --release --workspace
systemctl --user restart chops-agent chops-plugin

# Stop everything
systemctl --user stop chops-agent chops-plugin chops-mosquitto

# Enable STT when whisper.cpp is ready
systemctl --user enable --now chops-stt
```

**Key environment variables in service files:**

| Variable | Purpose | Set in |
|----------|---------|--------|
| `CHOPS_MQTT_PORT` | MQTT broker port (default 1884) | all services |
| `RUST_LOG` | Log level (info, debug, trace) | all services |
| `TMUX_TMPDIR` | tmux socket directory (`/tmp`) | chops-plugin |
| `PATH` | includes linuxbrew for tmux binary | chops-plugin |

## Testing Without Audio

```bash
# Send a voice command through the full pipeline
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "in chops run echo hello", "is_final": true}'

# Watch all MQTT traffic
mosquitto_sub -p 1884 -t '#' -v

# Watch responses only
mosquitto_sub -p 1884 -t 'agent/responses'

# Run unit tests (no broker needed)
cargo test --workspace

# Run integration tests (needs mosquitto running)
cargo test --workspace  # auto-skips if broker unavailable
```

## Project Structure

```
chops/
├── crates/
│   ├── stt-publisher/src/main.rs    # whisper.cpp → MQTT (261 lines)
│   ├── agent-core/src/main.rs       # intent parsing + routing (329 lines)
│   └── plugin-runner/src/main.rs    # command execution (209 lines)
├── tests/
│   └── integration.rs               # MQTT integration tests
├── docs/
│   └── overview.md                  # this file
├── .github/workflows/test.yml       # CI (fmt, clippy, test)
├── CLAUDE.md                        # instructions for Claude Code
├── README.md                        # quick start
├── Cargo.toml                       # workspace definition
└── Cargo.lock
```

~800 lines of Rust total. No frameworks. No macros. No cloud.
