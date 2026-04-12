# Claude Code Instructions — chops

## Project Overview

chops (chat ops) is a local, offline voice-controlled agent system. Audio from a Bluetooth headset is transcribed by whisper.cpp, parsed for intent by a Rust agent, and routed to plugins via MQTT. A web dashboard (PWA) provides browser-based voice input, session management, and a live tmux viewer. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp] → [MQTT: voice/transcriptions]
                                              ↓
[Web UI (mic)] ──────────────────►  [agent-core: intent parser]
[Tauri App (whisper-rs)] ─────────►       ↙       ↓         ↘
                              [tmux]    [vscode]    [termux]
```

All components communicate over MQTT (Mosquitto) on port 1884 (configurable via `CHOPS_MQTT_PORT`). The web UI connects via WebSocket on port 9884 (plain) or 9885 (TLS). The Tauri app connects via TCP for publishing and WSS for receiving.

## Workspace Layout

```
chops/
├── app/                          # Tauri v2 desktop/mobile app (excluded from cargo workspace)
│   ├── src/index.html            # Frontend (Tauri + browser dual-mode)
│   └── src-tauri/src/
│       ├── lib.rs                # shared entry: run() + mobile_entry_point
│       ├── main.rs               # desktop entry point (calls lib::run)
│       ├── stt.rs                # cpal mic capture + whisper-rs STT
│       └── mqtt.rs               # MQTT client wrapper (rumqttc)
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
│   ├── commands.md           # Voice command reference
│   ├── tauri-app.md          # Tauri app architecture + design decisions
│   └── android-setup.md      # Android SDK/NDK setup guide
├── .github/
│   ├── dependabot.yml        # Automated dependency updates
│   └── workflows/
│       ├── android.yml       # Android APK build on tags + manual dispatch
│       ├── claude.yml        # Claude code review on @claude mentions
│       ├── security.yml      # cargo-audit on Cargo.lock changes + weekly
│       └── test.yml          # CI with mosquitto service container
```

## Common Commands

```bash
# Build server-side crates
cargo build --workspace

# Build Tauri app
cd app && npm run tauri build

# Run unit tests (no broker needed)
cargo test --workspace

# Run with integration tests (needs mosquitto on CHOPS_MQTT_PORT)
mosquitto -p 1884 -d
cargo test --workspace

# Run a single crate
RUST_LOG=info cargo run -p agent-core

# Run Tauri app in dev mode
cd app && npm run tauri dev

# Build Tauri app for Android (requires SDK setup — see docs/android-setup.md)
cd app && npx tauri android dev

# Rebuild and restart services
cargo build --release --workspace
systemctl --user restart chops-agent chops-plugin chops-web
```

## Systemd Services

```
~/.config/systemd/user/
├── chops-mosquitto.service   # MQTT broker (TCP :1884 + WS :9884 + WSS :9885)
├── chops-agent.service       # agent-core (MQTT relay to AtomicGuard)
├── chops-plugin.service      # plugin-runner (tmux/vscode/termux executor)
├── chops-web.service         # web-ui (HTTPS :8443, HTTP fallback :8080)
├── chops-ttyd.service        # ttyd (read-only tmux viewer :7681)
└── chops-stt.service         # stt-publisher (enable when whisper.cpp is ready)
```

## MQTT Port

Default port is **1884** (not 1883, to avoid conflict with other MQTT services). Override with `CHOPS_MQTT_PORT` env var. CI uses 1883 with a mosquitto service container.

## MQTT Topics

| Topic | Direction | QoS | Purpose |
|-------|-----------|-----|---------|
| `voice/transcriptions` | app → agent-core | 0 | `{text, is_final, conversation_id}` |
| `agent/intent/request` | agent-core → AtomicGuard | 1 | `{text, conversation_id}` — all transcriptions forwarded |
| `agent/intent/response` | AtomicGuard → app | 1 | Intent classification result (success/failed/escalated) |
| `agent/workflow/events` | AtomicGuard → app | 0 | Step-level workflow progress |
| `agent/workflow/escalation` | AtomicGuard → app | 1 | Must-deliver human-attention alerts |
| `agent/escalation/response` | app → AtomicGuard | 1 | Human approve/reject/feedback for escalations |
| `agent/commands/tmux` | agent → plugin | 0 | Send keys to tmux session panes |
| `agent/commands/vscode` | agent → plugin | 0 | VSCode file open commands |
| `agent/commands/termux` | agent → plugin | 0 | Shell/termux commands |
| `agent/responses` | plugin → any | 0 | Execution results + toasts |
| `agent/ping` | app → app | 0 | MQTT connectivity check (self-echo) |
| `plugins/status/<name>` | plugin → any | 0 | Heartbeat |

**Full MQTT schemas with field tables:** see AtomicGuard repo `docs/design/notes/conversation_id_design.md`.

### Conversation ID

Every voice command gets a `conversation_id` generated by the Tauri app. AtomicGuard echoes it on all responses, enabling the UI to group messages into conversation threads. Agent-core passes it through; if missing (e.g. raw `mosquitto_pub`), agent-core generates a fallback.

## Agent-Core (Relay Mode)

Agent-core is a pure MQTT relay. All transcriptions are forwarded to `agent/intent/request` for AtomicGuard's embedding classifier (22 workflows, 93% accuracy, 29ms). No local regex parsing — AtomicGuard handles all intent classification and workflow dispatch.

The intent module (`intent.rs`) with regex parsing, fuzzy matching, and accumulator logic is preserved in `lib.rs` for integration tests but unused in production.

## Integration with `dev` Sessions

The tmux plugin targets panes created by the `dev` command:
- `session:1.1` = claude pane (left, in claude layout)
- `session:1.2` = shell pane (right, in claude layout)
- Falls back to `session:1.1` for single-pane (default layout) sessions

The `dev` script supports headless management for the web UI:
- `dev list` — JSON output of active sessions + available projects
- `dev start <project>` — create session without attaching
- `dev stop <session>` — kill a session

## Tauri App (Desktop + Mobile)

The primary UI at `app/src/`. Modular JS (`app/src/js/`) with three tabs:
- **Commands** — conversation feed with conversation_id grouping, workflow progress cards, interactive escalation cards (specification + feedback textareas, retry button), send command input + push-to-talk mic (whisper-rs)
- **Terminal** — session selector with start/stop, read-only tmux viewer (ttyd iframe)
- **Messages** — raw MQTT message log for debugging

Features: MQTT ping button, Clear/Copy All buttons, copy-message context menu (right-click/long-press), toast notifications, native notifications for escalations.

## Web Dashboard (Browser PWA)

The browser-only dashboard at `https://pop-mini.monkey-ladon.ts.net:8443` (`web/index.html`). **Note:** this is behind the Tauri app — it lacks conversation grouping, escalation cards, and the Messages tab. See issue #31 for parity work.

Provides:
- Text input and mic button (Web Speech API) for sending commands
- Session dropdown with start/stop buttons
- Read-only tmux terminal viewer (ttyd iframe)
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

1. Agent-core is relay mode — intent classification is AtomicGuard's responsibility
2. MQTT schemas are defined in AtomicGuard's `docs/design/notes/conversation_id_design.md` — keep in sync
3. Run `cargo test --workspace` before committing
4. Run `cargo clippy --workspace` and `cargo fmt --all` for CI compliance
5. After rebuilding: `systemctl --user restart chops-agent chops-plugin chops-web`
6. The Tauri app frontend is modular JS in `app/src/js/` — changes there don't need a Rust rebuild
