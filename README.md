# chops — voice-controlled agent system

Local, offline, composable voice-controlled agent. Speak into a Bluetooth headset; whisper.cpp transcribes; a Rust agent parses intent and routes commands to plugins. All components communicate over MQTT. No cloud dependency.

## Architecture

```
[Bluetooth Audio] → [whisper.cpp stream] → [MQTT: voice/transcriptions]
                                                        ↓
                                              [Rust Agent Core]
                                           (intent parser + router)
                                         ↙          ↓          ↘
                              [VSCode Plugin] [Termux Plugin] [...]
                                         ↘          ↓          ↙
                                          [MQTT: agent/responses]
```

## Crates

| Crate | Purpose |
|-------|---------|
| `stt-publisher` | Spawns whisper.cpp, buffers transcriptions, publishes to MQTT |
| `agent-core` | Subscribes to transcriptions, parses intent, routes to plugins |
| `plugin-runner` | Executes plugin commands (VSCode, Termux), reports results |

## Prerequisites

- Rust (stable)
- Mosquitto MQTT broker
- whisper.cpp (with `stream` binary compiled and a model downloaded)

## Quick Start

```bash
# Start Mosquitto
mosquitto

# Build
cargo build --workspace

# Run each in a separate terminal
RUST_LOG=info cargo run -p stt-publisher
RUST_LOG=info cargo run -p agent-core
RUST_LOG=info cargo run -p plugin-runner
```

## Testing Without Audio

```bash
# Finalized transcription
mosquitto_pub -t "voice/transcriptions" -m '{"text":"open vscode README.md","is_final":true,"timestamp":"2026-01-01T00:00:00Z"}'

# Partial (agent ignores)
mosquitto_pub -t "voice/transcriptions" -m '{"text":"open vs","is_final":false,"timestamp":"2026-01-01T00:00:00Z"}'

# Monitor all traffic
mosquitto_sub -t "#" -v
```

## MQTT Topics

| Topic | Publisher | Subscriber | Purpose |
|-------|-----------|------------|---------|
| `voice/transcriptions` | stt-publisher | agent-core | Transcription payloads |
| `agent/commands/vscode` | agent-core | plugin-runner | VSCode commands |
| `agent/commands/termux` | agent-core | plugin-runner | Termux commands |
| `agent/responses` | plugin-runner | agent / logger | Execution results |
| `plugins/status/<name>` | plugin-runner | agent-core | Heartbeat / availability |
