# Claude Code Instructions — chops

## Project Overview

chops (chat ops) is a local, offline voice-controlled agent system. Audio from a Bluetooth headset is transcribed by whisper.cpp, parsed for intent by a Rust agent, and routed to plugins via MQTT. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp] → [MQTT: voice/transcriptions]
                                              ↓
                                    [agent-core: intent parser]
                                      ↙       ↓         ↘
                              [tmux]    [vscode]    [termux]
```

All components communicate over MQTT (Mosquitto) on port 1884 (configurable via `CHOPS_MQTT_PORT`).

## Workspace Layout

```
chops/
├── crates/
│   ├── stt-publisher/    # whisper.cpp stdout → MQTT transcription publisher
│   ├── agent-core/       # intent parsing + command routing
│   └── plugin-runner/    # tmux/vscode/termux command execution
├── tests/
│   └── integration.rs    # MQTT integration tests (need running broker)
└── .github/workflows/
    └── test.yml          # CI with mosquitto service container
```

## Common Commands

```bash
# Build
cargo build --workspace

# Run unit tests (no broker needed)
cargo test --workspace

# Run with integration tests (needs mosquitto on CHOPS_MQTT_PORT)
mosquitto -p 1884 -d
cargo test --workspace

# Run a single crate
RUST_LOG=info cargo run -p agent-core

# Override MQTT port
CHOPS_MQTT_PORT=1885 cargo run -p agent-core
```

## MQTT Port

Default port is **1884** (not 1883, to avoid conflict with other MQTT services). Override with `CHOPS_MQTT_PORT` env var. CI uses 1883 with a mosquitto service container.

## MQTT Topics

| Topic | Direction | Purpose |
|-------|-----------|---------|
| `voice/transcriptions` | stt → agent | Transcription payloads |
| `agent/commands/tmux` | agent → plugin | Send keys to tmux session panes |
| `agent/commands/vscode` | agent → plugin | VSCode file open commands |
| `agent/commands/termux` | agent → plugin | Shell/termux commands |
| `agent/responses` | plugin → agent | Execution results |
| `plugins/status/<name>` | plugin → agent | Heartbeat |

## Voice Command Patterns

| Pattern | Effect |
|---------|--------|
| "in \<project\> run \<command\>" | Send command to project's shell pane |
| "in \<project\> tell claude \<msg\>" | Send message to project's claude pane |
| "run \<command\>" | Send to active tmux session's shell pane |
| "open vscode \<file\>" | Open file in VSCode |

## Integration with `dev` Sessions

The tmux plugin targets panes created by the `dev` command:
- `session:1.1` = claude pane (left, in claude layout)
- `session:1.2` = shell pane (right, in claude layout)
- Falls back to `session:1.1` for single-pane (default layout) sessions

## Testing

- **Unit tests**: Pure logic tests for `TranscriptionBuffer` and `parse_intent` — no external deps
- **Integration tests**: Require mosquitto running on `CHOPS_MQTT_PORT` — test MQTT pub/sub pipeline
- Integration tests auto-skip if broker is unavailable

## When Making Changes

1. Keep intent parsing in `parse_intent()` as a pure function — no MQTT calls
2. Add new plugin topics by: adding topic const + subscribe + handler in plugin-runner, adding routing rule in agent-core's `parse_intent()`
3. Run `cargo test --workspace` before committing
4. Run `cargo clippy --workspace` and `cargo fmt --all` for CI compliance
