# Claude Code Instructions — chops

## Project Overview

chops (chat ops) is a local, offline voice-controlled agent system. Audio from a Bluetooth headset is transcribed by whisper.cpp, parsed for intent by a Rust agent, and routed to plugins via MQTT. A web dashboard (PWA) provides browser-based voice input, session management, and a live tmux viewer. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp] → [MQTT: voice/transcriptions]
                                              ↓
[Web UI (mic)] ──────────────────►  [agent-core: intent parser]
                                      ↙       ↓         ↘
                              [tmux]    [vscode]    [termux]
```

All components communicate over MQTT (Mosquitto) on port 1884 (configurable via `CHOPS_MQTT_PORT`). The web UI connects via WebSocket on port 9884 (plain) or 9885 (TLS).

## Workspace Layout

```
chops/
├── crates/
│   ├── stt-publisher/        # whisper.cpp stdout → MQTT transcription publisher
│   ├── agent-core/           # intent parsing + command routing
│   │   └── src/
│   │       ├── main.rs       # MQTT loop, accumulator, route_intent
│   │       ├── intent.rs     # parse_intent, preprocessing, regex, fuzzy matching
│   │       └── lib.rs        # re-exports intent module for integration tests
│   ├── plugin-runner/        # tmux/vscode/termux command execution
│   └── web-ui/               # HTTPS web dashboard (axum + rustls)
├── web/
│   ├── index.html            # Single-file dashboard (MQTT.js, mic, ttyd, session mgmt)
│   ├── manifest.json         # PWA manifest
│   └── sw.js                 # Service worker (install support)
├── scripts/
│   ├── chops-send.sh         # CLI helper for remote command injection
│   ├── ttyd-attach.sh        # ttyd wrapper (reads session from state file)
│   └── termux-setup.md       # Android setup guide
├── tests/
│   └── integration.rs        # MQTT pipeline integration tests
├── docs/
│   ├── overview.md           # System overview + architecture diagrams
│   └── commands.md           # Voice command reference
└── .github/workflows/
    └── test.yml              # CI with mosquitto service container
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

# Rebuild and restart services
cargo build --release --workspace
systemctl --user restart chops-agent chops-plugin chops-web
```

## Systemd Services

```
~/.config/systemd/user/
├── chops-mosquitto.service   # MQTT broker (TCP :1884 + WS :9884 + WSS :9885)
├── chops-agent.service       # agent-core (intent parser + router)
├── chops-plugin.service      # plugin-runner (tmux/vscode/termux executor)
├── chops-web.service         # web-ui (HTTPS :8443, HTTP fallback :8080)
├── chops-ttyd.service        # ttyd (read-only tmux viewer :7681)
└── chops-stt.service         # stt-publisher (enable when whisper.cpp is ready)
```

## MQTT Port

Default port is **1884** (not 1883, to avoid conflict with other MQTT services). Override with `CHOPS_MQTT_PORT` env var. CI uses 1883 with a mosquitto service container.

## MQTT Topics

| Topic | Direction | Purpose |
|-------|-----------|---------|
| `voice/transcriptions` | stt/web → agent | Transcription payloads |
| `agent/commands/tmux` | agent → plugin | Send keys to tmux session panes |
| `agent/commands/vscode` | agent → plugin | VSCode file open commands |
| `agent/commands/termux` | agent → plugin | Shell/termux commands |
| `agent/responses` | plugin → any | Execution results |
| `plugins/status/<name>` | plugin → any | Heartbeat |

## Voice Command Patterns

| Pattern | Effect |
|---------|--------|
| "in \<project\> run \<command\>" | Send command to project's shell pane |
| "in \<project\> tell claude \<msg\>... over" | Accumulate + send message to claude pane |
| "run \<command\>" | Send to active tmux session's shell pane |
| "open vscode \<file\>" | Open file in VSCode |

**Preprocessing:** filler words stripped, synonyms normalized, fuzzy project matching via Jaro-Winkler.

**Accumulation:** "tell claude" messages buffer until a terminator ("over", "done", "send it") is heard. 30s safety timeout.

## Integration with `dev` Sessions

The tmux plugin targets panes created by the `dev` command:
- `session:1.1` = claude pane (left, in claude layout)
- `session:1.2` = shell pane (right, in claude layout)
- Falls back to `session:1.1` for single-pane (default layout) sessions

The `dev` script supports headless management for the web UI:
- `dev list` — JSON output of active sessions + available projects
- `dev start <project>` — create session without attaching
- `dev stop <session>` — kill a session

## Web UI

The dashboard at `https://pop-mini.monkey-ladon.ts.net:8443` provides:
- Text input and mic button (Web Speech API) for sending commands
- Session dropdown with start/stop buttons
- Read-only tmux terminal viewer (ttyd iframe)
- Collapsible debug message log
- PWA support (installable on phone home screen)

## Web UI API Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/sessions` | GET | List sessions + projects (calls `dev list`) |
| `/api/sessions/switch?session=<name>` | POST | Switch ttyd to a different session |
| `/api/sessions/start?project=<name>` | POST | Start a new session (calls `dev start`) |
| `/api/sessions/stop?session=<name>` | POST | Stop a session (calls `dev stop`) |

## Testing

- **Unit tests**: Pure logic tests for `TranscriptionBuffer`, `parse_intent`, preprocessing, fuzzy matching, terminators
- **Integration tests**: Require mosquitto running on `CHOPS_MQTT_PORT` — test MQTT pub/sub pipeline
- Integration tests auto-skip if broker is unavailable

## When Making Changes

1. Keep intent parsing in `parse_intent()` as a pure function — no MQTT calls
2. Add new plugin topics by: adding topic const + subscribe + handler in plugin-runner, adding routing rule in agent-core's `parse_intent()`
3. Run `cargo test --workspace` before committing
4. Run `cargo clippy --workspace` and `cargo fmt --all` for CI compliance
5. After rebuilding: `systemctl --user restart chops-agent chops-plugin chops-web`
