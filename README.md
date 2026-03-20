# chops — voice-controlled agent system

Local, offline, composable voice-controlled agent. Speak into a Bluetooth headset; whisper.cpp transcribes; a Rust agent parses intent and routes commands to plugins. All components communicate over MQTT. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp stream] → [MQTT: voice/transcriptions]
                                                        ↓
                                              [Rust Agent Core]
                                           (intent parser + router)
                                         ↙          ↓          ↘
                                  [tmux]      [VSCode]      [termux]
                                         ↘          ↓          ↙
                                          [MQTT: agent/responses]
```

## Voice Commands

| You say | What happens |
|---------|-------------|
| "in chops run cargo test" | Sends `cargo test` to the shell pane of the chops tmux session |
| "in chops tell claude fix the tests" | Sends a message to the Claude pane of the chops session |
| "run git status" | Sends `git status` to the active tmux session |
| "open vscode README.md" | Opens file in VSCode |

Commands are natural language — filler words ("uh", "please", "okay"), punctuation, synonyms ("execute" for "run"), and project name typos ("chop" → "chops") are handled automatically.

See **[docs/commands.md](docs/commands.md)** for the full command reference.

## Crates

| Crate | Purpose |
|-------|---------|
| `stt-publisher` | Spawns whisper.cpp, buffers transcriptions, publishes to MQTT |
| `agent-core` | Subscribes to transcriptions, parses intent, routes to plugins |
| `plugin-runner` | Executes plugin commands (tmux, VSCode, termux), reports results |

## Quick Start

```bash
# Start Mosquitto on port 1884
mosquitto -p 1884 -d

# Build
cargo build --workspace

# Run each in a separate terminal
RUST_LOG=info cargo run -p agent-core
RUST_LOG=info cargo run -p plugin-runner
RUST_LOG=info cargo run -p stt-publisher  # needs whisper.cpp
```

Or use **systemd user services** for persistent operation — see [docs/overview.md](docs/overview.md).

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
| `voice/transcriptions` | stt-publisher | agent-core | Transcription payloads |
| `agent/commands/tmux` | agent-core | plugin-runner | Tmux send-keys commands |
| `agent/commands/vscode` | agent-core | plugin-runner | VSCode file open |
| `agent/commands/termux` | agent-core | plugin-runner | Shell commands |
| `agent/responses` | plugin-runner | — | Execution results |
| `plugins/status/<name>` | plugin-runner | — | Heartbeat |

## Documentation

- **[docs/commands.md](docs/commands.md)** — Voice command reference (patterns, synonyms, examples)
- **[docs/overview.md](docs/overview.md)** — System overview, architecture diagrams, deployment
