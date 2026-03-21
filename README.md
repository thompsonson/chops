# chops — voice-controlled agent system

Local, offline, composable voice-controlled agent. Speak into a Bluetooth headset or the web dashboard; a Rust agent parses intent and routes commands to tmux sessions, VSCode, or shell. All components communicate over MQTT. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp stream] → [MQTT: voice/transcriptions]
                                                        ↓
[Web UI (PWA/mic)] ──────────────────────►  [Rust Agent Core]
                                           (intent parser + router)
[Tauri App (whisper-rs)] ────────────────►       ↙    ↓    ↘
                                          [tmux]  [VSCode]  [termux]
                                               ↘    ↓    ↙
                                          [MQTT: agent/responses]
```

## Voice Commands

| You say | What happens |
|---------|-------------|
| "in chops run cargo test" | Sends `cargo test` to the shell pane of the chops tmux session |
| "in chops tell claude fix the tests... over" | Accumulates message and sends to Claude pane |
| "run git status" | Sends `git status` to the active tmux session |
| "open vscode README.md" | Opens file in VSCode |

Commands are natural language — filler words ("uh", "please"), punctuation, synonyms ("execute" for "run"), and project name typos ("chop" → "chops") are handled automatically.

See **[docs/commands.md](docs/commands.md)** for the full command reference.

## Components

| Component | Purpose |
|-----------|---------|
| `crates/stt-publisher` | Spawns whisper.cpp, buffers transcriptions, publishes to MQTT |
| `crates/agent-core` | Subscribes to transcriptions, parses intent, routes to plugins |
| `crates/plugin-runner` | Executes plugin commands (tmux, VSCode, termux), reports results |
| `crates/web-ui` | HTTPS web dashboard with mic input, session management, tmux viewer |
| `app/` | Tauri v2 desktop/mobile app with offline whisper-rs STT |

## Quick Start

```bash
# Start Mosquitto on port 1884
mosquitto -p 1884 -d

# Build
cargo build --workspace

# Run each in a separate terminal
RUST_LOG=info cargo run -p agent-core
RUST_LOG=info cargo run -p plugin-runner
RUST_LOG=info cargo run -p web-ui        # serves dashboard on https://localhost:8443
RUST_LOG=info cargo run -p stt-publisher  # needs whisper.cpp
```

Or use **systemd user services** for persistent operation — see [docs/overview.md](docs/overview.md).

## Web Dashboard

The web UI is a PWA accessible at `https://pop-mini.monkey-ladon.ts.net:8443`:

- **Voice input** — mic button using Web Speech API, with multi-segment accumulation
- **Text input** — type commands directly
- **Session management** — start, stop, and switch between tmux sessions
- **Terminal viewer** — read-only tmux session view via ttyd
- **Installable** — add to home screen on mobile via PWA

## Remote Access

All services are accessible over Tailscale:
- **Web UI**: `https://pop-mini.monkey-ladon.ts.net:8443`
- **MQTT**: `pop-mini:1884` (TCP) or `pop-mini:9885` (WSS)
- **CLI**: `scripts/chops-send.sh in chops run cargo test`
- **Android**: See `scripts/termux-setup.md`

## Testing Without Audio

```bash
# Send a command through the full pipeline
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "in chops run cargo test", "is_final": true}'

# Monitor all traffic
mosquitto_sub -p 1884 -t '#' -v

# Run tests
cargo test --workspace
```

## MQTT Topics

| Topic | Publisher | Subscriber | Purpose |
|-------|-----------|------------|---------|
| `voice/transcriptions` | stt-publisher, web UI, Tauri app | agent-core | Transcription payloads |
| `agent/commands/tmux` | agent-core | plugin-runner | Tmux send-keys commands |
| `agent/commands/vscode` | agent-core | plugin-runner | VSCode file open |
| `agent/commands/termux` | agent-core | plugin-runner | Shell commands |
| `agent/responses` | plugin-runner | web UI | Execution results |
| `plugins/status/<name>` | plugin-runner | — | Heartbeat |

## Documentation

- **[docs/overview.md](docs/overview.md)** — System overview, architecture diagrams, deployment
- **[docs/commands.md](docs/commands.md)** — Voice command reference (patterns, synonyms, accumulation, examples)
- **[docs/tauri-app.md](docs/tauri-app.md)** — Tauri app architecture, design decisions, setup
