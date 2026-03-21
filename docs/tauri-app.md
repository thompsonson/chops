# chops — Tauri Desktop/Mobile App

## Overview

The Tauri v2 app provides offline speech-to-text via whisper-rs, replacing the PWA's dependency on Google's Web Speech API. It runs natively on Linux desktop (and eventually Android), capturing audio locally and transcribing with a bundled whisper model — no network required for speech recognition.

The existing PWA remains as-is for quick browser access.

## Architecture

```
┌────────────────────────────────────────────────┐
│              Tauri v2 App                      │
│                                                │
│  ┌──────────────┐      ┌───────────────────┐   │
│  │   WebView    │      │   Rust Backend    │   │
│  │              │      │                   │   │
│  │  index.html  │◄────►│  Tauri Commands   │   │
│  │  (MQTT.js)   │      │                   │   │
│  │              │      │  ┌─────────────┐  │   │
│  │  Sessions,   │      │  │   stt.rs    │  │   │
│  │  terminal,   │      │  │  cpal (mic) │  │   │
│  │  debug log   │      │  │  whisper-rs │  │   │
│  │              │      │  └─────────────┘  │   │
│  │              │      │                   │   │
│  │              │      │  ┌─────────────┐  │   │
│  │              │      │  │  mqtt.rs    │  │   │
│  │              │      │  │  rumqttc    │  │   │
│  │              │      │  └─────────────┘  │   │
│  └──────────────┘      └───────────────────┘   │
│         │                        │              │
└─────────┼────────────────────────┼──────────────┘
          │ WSS :9885              │ TCP :1884
          │ (responses, status)    │ (transcriptions)
          ▼                        ▼
   ┌──────────────────────────────────────┐
   │      MQTT Broker (pop-mini:1884)     │
   └──────────────────────────────────────┘
          │
          ▼
   ┌──────────────────────────────────────┐
   │  agent-core → plugin-runner → tmux  │
   └──────────────────────────────────────┘
```

### Dual communication paths

The app uses two separate MQTT connections for different purposes:

- **Rust backend (rumqttc, TCP :1884)** — publishes transcriptions to `voice/transcriptions`. This is the STT output path. Using TCP avoids the overhead of WebSocket framing for the high-frequency publish path.
- **WebView (MQTT.js, WSS :9885)** — subscribes to `agent/responses`, `agent/commands/#`, and `plugins/status/#` for UI feedback. Reuses the same WebSocket connection the PWA already uses.

The `send_transcription` Tauri command also publishes via TCP, so typed commands sent from the text input bypass the WebSocket path entirely.

## Design Decisions

### Why Tauri v2 (not Electron, not pure CLI)

- **Native performance** — Rust backend with no JS runtime overhead for audio processing
- **Small binary** — uses the system WebView (WebKitGTK on Linux), no bundled Chromium
- **Android support** — Tauri v2 has first-class Android support via the same codebase
- **Shared UI** — the WebView can reuse the existing web dashboard with minimal changes

### Why whisper-rs (not spawning whisper.cpp)

The existing `stt-publisher` crate spawns `whisper.cpp stream` as a subprocess and parses its stdout. The Tauri app uses whisper-rs (Rust bindings to whisper.cpp) instead:

- **In-process** — no subprocess management, no stdout parsing, no path dependencies
- **Direct audio control** — cpal captures mic audio as f32 PCM, fed directly to whisper
- **Portable** — whisper-rs compiles via cmake/bindgen, works on Linux and Android NDK
- **Same model format** — uses the same `ggml-base.en.bin` model as stt-publisher

### Why a separate app/ directory (not in crates/)

The Tauri app has a fundamentally different build lifecycle:

- It's built with `cargo tauri build`, not `cargo build --workspace`
- It has npm dependencies (`@tauri-apps/cli`) and a `package.json`
- It has frontend assets (`src/index.html`) alongside the Rust code
- It produces a desktop application, not a server binary

The workspace `Cargo.toml` uses `exclude = ["app/src-tauri"]` to keep it independent while allowing the server-side crates to build as before.

### Frontend: one HTML file, two modes

The `app/src/index.html` is adapted from `web/index.html`. It detects the Tauri runtime at load time:

```javascript
const IS_TAURI = window.__TAURI_INTERNALS__ !== undefined;
```

**In Tauri mode:**
- Mic button calls `invoke('start_listening')` → cpal + whisper-rs (local, offline)
- Text send calls `invoke('send_transcription')` → rumqttc (TCP)
- MQTT host is hardcoded to `pop-mini.monkey-ladon.ts.net`
- Session API calls go to the web-ui server at `https://pop-mini:8443`

**In browser mode (fallback):**
- Mic button uses Web Speech API (same as the PWA)
- Text send uses MQTT.js (WebSocket)
- MQTT host is inferred from `window.location.hostname`
- Session API calls use relative URLs

This means the same `index.html` can be opened in a browser for testing without the Tauri backend.

### Audio capture and silence detection

The STT pipeline in `stt.rs`:

```
cpal (mic) → f32 PCM ring buffer → silence detection → whisper-rs → MQTT
```

1. **cpal** captures mono 16kHz audio into a shared buffer
2. **Silence detection** checks RMS of the most recent 100ms of audio against a threshold (0.01)
3. When speech is detected followed by 800ms of silence, the buffer is fed to whisper
4. **whisper-rs** transcribes and the result is both emitted to the WebView (Tauri event) and published to MQTT
5. Buffer is capped at 30 seconds to prevent unbounded growth during long speech

The 800ms silence threshold matches the existing `stt-publisher` behavior.

### MQTT message format

The app publishes the same JSON format as stt-publisher and the PWA:

```json
{
  "text": "in chops run cargo test",
  "is_final": true,
  "timestamp": "2026-03-21T10:30:00Z",
  "source": "tauri-app"
}
```

The `source` field is added to distinguish Tauri app transcriptions in logs. The agent-core ignores this field — it only reads `text` and `is_final`.

## Workspace Layout

```
app/
├── package.json                  # npm config (@tauri-apps/cli)
├── .gitignore                    # node_modules/, src-tauri/target/
├── src/
│   └── index.html                # frontend (Tauri + browser dual-mode)
└── src-tauri/
    ├── Cargo.toml                # whisper-rs, cpal, rumqttc, tauri v2
    ├── Cargo.lock
    ├── build.rs                  # tauri_build
    ├── tauri.conf.json           # app config, CSP, window settings
    ├── capabilities/
    │   └── default.json          # Tauri v2 permissions
    ├── icons/
    │   └── icon.png              # app icon (placeholder)
    └── src/
        ├── lib.rs                # shared entry: run() + mobile_entry_point
        ├── main.rs               # desktop entry point (calls lib::run)
        ├── stt.rs                # cpal audio capture + whisper-rs transcription
        └── mqtt.rs               # rumqttc MQTT client wrapper
```

## Tauri Commands

Commands exposed to the WebView via `tauri::generate_handler!`:

| Command | Parameters | Purpose |
|---------|------------|---------|
| `start_listening` | — | Begin mic capture + whisper STT |
| `stop_listening` | — | Stop mic capture |
| `connect_mqtt` | `host?`, `port?` | Connect to MQTT broker |
| `send_transcription` | `text` | Publish text to `voice/transcriptions` |
| `get_status` | — | Return listening/mqtt/model state |

## Tauri Events

Events emitted from Rust to the WebView:

| Event | Payload | Purpose |
|-------|---------|---------|
| `stt-transcription` | `"transcribed text"` | Whisper produced a transcription |
| `stt-status` | `"listening"` / `"stopped"` / `"model_loaded"` | STT engine state changes |

## Prerequisites

### System dependencies (Linux)

```bash
# Tauri v2 WebView
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
  libsoup-3.0-dev libjavascriptcoregtk-4.1-dev

# Audio capture (cpal/ALSA)
sudo apt-get install -y libasound2-dev

# whisper-rs build (compiles whisper.cpp from source)
sudo apt-get install -y cmake
```

### Whisper model

Download `ggml-base.en.bin` (~142MB) to the app data directory:

```bash
mkdir -p ~/.local/share/chops
cd ~/.local/share/chops
curl -LO https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

The app shows a banner if the model is missing, with the expected path.

## Usage

### Development

```bash
cd app
npm run tauri dev
```

This starts the app in dev mode with hot-reload for frontend changes. The Rust backend recompiles on save.

### Build release

```bash
cd app
npm run tauri build
```

Produces a `.deb` package and AppImage in `app/src-tauri/target/release/bundle/`.

### Android

See [`docs/android-setup.md`](android-setup.md) for full SDK/NDK setup instructions.

```bash
cd app
npx tauri android init   # one-time setup
npx tauri android dev    # dev on connected device
npx tauri android build  # release APK
```

## Relationship to Other Components

```
Component          │ STT Source      │ MQTT Transport │ Runs On
───────────────────┼─────────────────┼────────────────┼──────────────
stt-publisher      │ whisper.cpp     │ TCP (rumqttc)  │ server
                   │ (subprocess)    │                │
───────────────────┼─────────────────┼────────────────┼──────────────
web UI (PWA)       │ Web Speech API  │ WSS (MQTT.js)  │ browser
                   │ (Google cloud)  │                │
───────────────────┼─────────────────┼────────────────┼──────────────
Tauri app          │ whisper-rs      │ TCP (rumqttc)  │ desktop/mobile
                   │ (local, offline)│ + WSS (MQTT.js)│
```

All three publish to the same MQTT topic (`voice/transcriptions`) in the same JSON format. The downstream pipeline (agent-core → plugin-runner → tmux) is completely agnostic to the source.

## Open Questions

- **Model download UX** — first launch requires ~142MB model download; needs a progress bar
- **Wake word** — optional "hey chops" trigger before listening (reduces battery on mobile)
- **Always-on Android** — requires a foreground service for background listening
- **Model selection** — currently hardcoded to `base.en`; larger models (small, medium) trade speed for accuracy
