# chops — System Overview

## What is chops?

chops (chat ops) is a local, offline voice-controlled agent system. You speak into a Bluetooth headset or the web dashboard, a Rust agent parses your intent, and the resulting command is routed to the right target — a tmux pane, VSCode, or a shell. Everything runs on your machine. No cloud, no API keys, no latency.

## Architecture

```
                        ┌──────────────────────────────────────────────────┐
                        │                  MQTT Broker                     │
                        │           mosquitto :1884 / :9885 (WSS)         │
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
  │ whisper.cpp  │     parse_intent()  │           └──► bash (termux)
  │   stream     │     + accumulator   │
  └───────┬──────┘                     │
          │                            │
  ┌───────┴──────┐              ┌──────┴──────────────────┐
  │  Bluetooth   │              │      tmux sessions      │
  │  microphone  │              │  (created by `dev` cmd) │
  └──────────────┘              │                         │
                                │  ┌─────────┬─────────┐  │
  ┌──────────────┐              │  │ claude  │  shell  │  │
  │   Web UI     │              │  │ (pane 1)│ (pane 2)│  │
  │  (PWA/mic)   │──► MQTT WSS  │  └─────────┴─────────┘  │
  └──────────────┘              └─────────────────────────┘
```

## Data Flow

```
 1. Speak          "in chops run cargo test"
                          │
 2. Transcribe            ▼
                   ┌──────────────┐
                   │ stt-publisher │──► MQTT: voice/transcriptions
                   │  or Web UI   │    {"text": "in chops run cargo test", "is_final": true}
                   └──────────────┘                │
                                                   ▼
 3. Parse                                  ┌──────────────┐
                                           │  agent-core  │  preprocess → regex → fuzzy match
                                           └──────────────┘    → Intent::Tmux { session, pane, command }
                                                   │
 4. Route                                          ▼
                                           MQTT: agent/commands/tmux
                                                   │
 5. Execute                                        ▼
                                           ┌──────────────┐
                                           │ plugin-runner │──► tmux send-keys -t chops:1.2 "cargo test" Enter
                                           └──────────────┘
                                                   │
 6. Respond                                        ▼
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

Subscribes to `voice/transcriptions`, ignores partials, and runs finalized text through the intent pipeline:

1. **Preprocessing** (`intent.rs`): strips filler words, punctuation; normalizes synonyms
2. **Regex matching** (`intent.rs`): flexible pattern matching with capture groups
3. **Fuzzy project matching** (`intent.rs`): Jaro-Winkler similarity against known projects
4. **Accumulation** (`main.rs`): "tell claude" messages buffer across segments until terminator keyword

**Voice command patterns:**

| You say | What happens |
|---------|-------------|
| "in chops run cargo test" | Sends `cargo test` to the shell pane of the `chops` tmux session |
| "in chops tell claude fix the tests... over" | Accumulates and sends to the claude pane |
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

### web-ui

HTTPS web dashboard built with axum + rustls. Serves `web/index.html` and exposes API endpoints for session management.

**Features:**
- Voice input via Web Speech API (mic button)
- Text input → publishes to `voice/transcriptions` via MQTT.js over WebSocket
- Session dropdown with start/stop/switch (calls `dev start/stop/list`)
- Read-only tmux terminal viewer (ttyd embedded via iframe)
- Collapsible debug message log
- PWA support (installable on mobile)

**API endpoints:**

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/sessions` | GET | List sessions + projects |
| `/api/sessions/switch?session=<name>` | POST | Switch ttyd terminal view |
| `/api/sessions/start?project=<name>` | POST | Start a new tmux session |
| `/api/sessions/stop?session=<name>` | POST | Stop a tmux session |

## MQTT Topics

```
voice/transcriptions          stt-publisher / web UI  ──►  agent-core
agent/commands/tmux           agent-core              ──►  plugin-runner
agent/commands/vscode         agent-core              ──►  plugin-runner
agent/commands/termux         agent-core              ──►  plugin-runner
agent/responses               plugin-runner           ──►  web UI / any subscriber
plugins/status/<name>         plugin-runner           ──►  (heartbeat)
```

All messages are JSON. Default MQTT port is **1884** (override with `CHOPS_MQTT_PORT`).

## Deployment (systemd)

All components run as systemd user services. They start on boot, restart on failure, and survive logout (linger enabled).

```
~/.config/systemd/user/
├── chops-mosquitto.service   ← MQTT broker (TCP :1884 + WS :9884 + WSS :9885)
├── chops-agent.service       ← agent-core (intent parser + accumulator)
├── chops-plugin.service      ← plugin-runner (tmux/vscode/termux executor)
├── chops-web.service         ← web-ui (HTTPS :8443)
├── chops-ttyd.service        ← ttyd (read-only tmux viewer :7681)
└── chops-stt.service         ← stt-publisher (enable when whisper.cpp is ready)
```

**Service dependencies:**

```
chops-mosquitto
    ├──► chops-agent    (Requires + After)
    ├──► chops-plugin   (Requires + After)
    ├──► chops-web      (After)
    ├──► chops-ttyd     (After)
    └──► chops-stt      (Requires + After)
```

**Common commands:**

```bash
# Status of all services
systemctl --user status chops-mosquitto chops-agent chops-plugin chops-web chops-ttyd

# Live logs
journalctl --user -u chops-agent -u chops-plugin -f

# Restart after rebuild
cargo build --release --workspace
systemctl --user restart chops-agent chops-plugin chops-web

# Stop everything
systemctl --user stop chops-agent chops-plugin chops-web chops-ttyd chops-mosquitto
```

**Key environment variables in service files:**

| Variable | Purpose | Set in |
|----------|---------|--------|
| `CHOPS_MQTT_PORT` | MQTT broker port (default 1884) | all services |
| `RUST_LOG` | Log level (info, debug, trace) | all services |
| `TMUX_TMPDIR` | tmux socket directory (`/tmp`) | chops-plugin, chops-ttyd |
| `PATH` | includes linuxbrew for tmux/ttyd binaries | chops-plugin, chops-ttyd |
| `CHOPS_TLS_CERT` | Tailscale TLS certificate path | chops-web |
| `CHOPS_TLS_KEY` | Tailscale TLS key path | chops-web |
| `CHOPS_WEB_DIR` | Static files directory for web UI | chops-web |

## Remote Access (Tailscale)

All services are bound to `0.0.0.0` and accessible over Tailscale. HTTPS uses Tailscale-issued certificates (Let's Encrypt).

| Service | URL |
|---------|-----|
| Web UI | `https://pop-mini.monkey-ladon.ts.net:8443` |
| MQTT (TCP) | `pop-mini:1884` |
| MQTT (WSS) | `wss://pop-mini.monkey-ladon.ts.net:9885` |
| Terminal | `https://pop-mini.monkey-ladon.ts.net:7681` |

**CLI helper:** `scripts/chops-send.sh` — send commands from any machine with mosquitto-clients.

**Android:** See `scripts/termux-setup.md` for Termux/Tasker setup.

## Testing Without Audio

```bash
# Send a voice command through the full pipeline
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "in chops run echo hello", "is_final": true}'

# Test with noisy whisper-like input
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "Uh, please in chop execute cargo test.", "is_final": true}'

# Watch all MQTT traffic
mosquitto_sub -p 1884 -t '#' -v

# Run unit tests (no broker needed)
cargo test --workspace

# Run integration tests (needs mosquitto running)
cargo test --workspace  # auto-skips if broker unavailable
```

## Project Structure

```
chops/
├── crates/
│   ├── stt-publisher/src/main.rs    # whisper.cpp → MQTT
│   ├── agent-core/src/
│   │   ├── main.rs                  # MQTT loop, accumulator, routing
│   │   ├── intent.rs                # preprocessing, regex, fuzzy matching
│   │   └── lib.rs                   # re-exports for integration tests
│   ├── plugin-runner/src/main.rs    # command execution (tmux/vscode/termux)
│   └── web-ui/src/main.rs           # HTTPS server + session API
├── web/
│   ├── index.html                   # dashboard (MQTT.js, mic, ttyd, sessions)
│   ├── manifest.json                # PWA manifest
│   └── sw.js                        # service worker
├── scripts/
│   ├── chops-send.sh                # CLI command helper
│   ├── ttyd-attach.sh               # ttyd session switching wrapper
│   └── termux-setup.md              # Android setup guide
├── tests/
│   └── integration.rs               # MQTT pipeline tests
├── docs/
│   ├── overview.md                  # this file
│   └── commands.md                  # voice command reference
├── .github/workflows/test.yml       # CI (fmt, clippy, test)
├── CLAUDE.md                        # instructions for Claude Code
├── README.md                        # quick start
├── Cargo.toml                       # workspace definition
└── Cargo.lock
```
