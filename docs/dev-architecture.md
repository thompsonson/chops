# dev — Architecture & Integration

## Crate Dependency Graph

```mermaid
graph TD
    subgraph "chops workspace"
        DEV_LIB["dev-lib<br/><i>library</i>"]
        DEV_CLI["dev-cli<br/><i>binary: dev</i>"]
        WEB_UI["web-ui<br/><i>binary: web-ui</i>"]
        AGENT["agent-core<br/><i>binary: agent-core</i>"]
        PLUGIN["plugin-runner<br/><i>binary: plugin-runner</i>"]
        COMMON["chops-common<br/><i>MQTT constants</i>"]
        STT["stt-publisher<br/><i>binary: stt-publisher</i>"]
    end

    DEV_CLI --> DEV_LIB
    WEB_UI --> DEV_LIB
    AGENT --> DEV_LIB
    AGENT --> COMMON
    PLUGIN --> COMMON
    STT --> COMMON
    WEB_UI -.->|"serves"| WEB_STATIC["web/<br/>index.html"]

    style DEV_LIB fill:#4ecca3,color:#000
    style DEV_CLI fill:#4ecca3,color:#000
    style WEB_UI fill:#e94560,color:#fff
    style AGENT fill:#e94560,color:#fff
    style PLUGIN fill:#e94560,color:#fff
    style COMMON fill:#16213e,color:#e0e0e0
    style STT fill:#16213e,color:#e0e0e0
```

## dev-lib Internal Structure

```mermaid
graph LR
    subgraph "dev-lib"
        CONFIG["config.rs<br/>parse ~/.config/dev/config<br/>Layout, ProjectEntry"]
        DISC["discovery.rs<br/>scan ~/Projects for .git<br/>collision handling"]
        RESOLVE["resolve.rs<br/>exact → basename → substring"]
        TMUX["tmux.rs<br/>TmuxBackend trait<br/>RealTmux / MockTmux"]
        API["api.rs<br/>DevManager<br/>list / start / stop / open"]
    end

    API --> CONFIG
    API --> DISC
    API --> RESOLVE
    API --> TMUX
    DISC --> CONFIG

    style API fill:#4ecca3,color:#000
    style TMUX fill:#0f3460,color:#e0e0e0
```

## System Integration

```mermaid
flowchart TB
    subgraph AUDIO["Audio Input"]
        MIC["Bluetooth Mic"]
        WEB_MIC["Web UI<br/>Browser STT /<br/>Dictaphone"]
        TAURI["Tauri App<br/>whisper-rs"]
    end

    subgraph STT_LAYER["Transcription"]
        STT_PUB["stt-publisher<br/>whisper.cpp stream"]
        TRANSCRIBE["/api/transcribe<br/>whisper-cpp on upload"]
    end

    subgraph MQTT_BUS["MQTT Broker :1884"]
        TOPIC_VOICE["voice/<br/>transcriptions"]
        TOPIC_CMD["agent/commands/<br/>tmux | vscode | termux"]
        TOPIC_RESP["agent/<br/>responses"]
    end

    subgraph CORE["Processing"]
        AGENT_CORE["agent-core<br/>intent parsing<br/>+ accumulator"]
        PLUGIN_RUN["plugin-runner<br/>command execution"]
    end

    subgraph DEV["dev (session manager)"]
        DEV_LIB_BOX["dev-lib"]
        DEV_CLI_BOX["dev-cli<br/><i>interactive terminal</i>"]
        WEB_UI_BOX["web-ui<br/><i>HTTPS :8443</i>"]
    end

    subgraph TMUX_SESSIONS["tmux sessions"]
        S1["chops<br/>┌────────┬────────┐<br/>│ claude │ shell  │<br/>└────────┴────────┘"]
        S2["lestash<br/>┌────────┬────────┐<br/>│ claude │ shell  │<br/>└────────┴────────┘"]
        S3["atomicguard<br/>┌─────────────────┐<br/>│     shell       │<br/>└─────────────────┘"]
    end

    MIC --> STT_PUB
    STT_PUB --> TOPIC_VOICE
    WEB_MIC -->|"Browser STT"| TOPIC_VOICE
    WEB_MIC -->|"Dictaphone"| TRANSCRIBE
    TRANSCRIBE --> TOPIC_VOICE
    TAURI --> TOPIC_VOICE

    TOPIC_VOICE --> AGENT_CORE
    AGENT_CORE -->|"parse_intent()"| TOPIC_CMD
    TOPIC_CMD --> PLUGIN_RUN
    PLUGIN_RUN -->|"tmux send-keys"| TMUX_SESSIONS
    PLUGIN_RUN --> TOPIC_RESP
    TOPIC_RESP --> WEB_UI_BOX

    DEV_CLI_BOX --> DEV_LIB_BOX
    WEB_UI_BOX -->|"list / start / stop"| DEV_LIB_BOX
    AGENT_CORE -->|"discover_projects()"| DEV_LIB_BOX
    DEV_LIB_BOX -->|"TmuxBackend"| TMUX_SESSIONS

    WEB_UI_BOX -->|"serves"| WEB_STATIC2["web/index.html<br/>PWA dashboard"]
    WEB_UI_BOX -->|"/api/sessions/*"| DEV_LIB_BOX
    WEB_UI_BOX -->|"/api/transcribe"| TRANSCRIBE

    style DEV_LIB_BOX fill:#4ecca3,color:#000
    style DEV_CLI_BOX fill:#4ecca3,color:#000
    style AGENT_CORE fill:#e94560,color:#fff
    style PLUGIN_RUN fill:#e94560,color:#fff
    style WEB_UI_BOX fill:#e94560,color:#fff
```

## dev-lib Consumer Patterns

```mermaid
flowchart LR
    subgraph CONSUMERS["Three consumers, one library"]
        CLI["dev-cli<br/><b>interactive</b><br/>picker, attach,<br/>SSH forward"]
        WEBUI["web-ui<br/><b>HTTP API</b><br/>list, start, stop<br/>via spawn_blocking"]
        AC["agent-core<br/><b>project discovery</b><br/>discover_projects()<br/>for fuzzy matching"]
    end

    DEVLIB["dev-lib<br/>DevManager"]

    CLI --> DEVLIB
    WEBUI --> DEVLIB
    AC --> DEVLIB

    subgraph DEVLIB_DOES["DevManager provides"]
        LIST["list()<br/>sessions + projects JSON"]
        START["start(project, layout)<br/>create detached session"]
        STOP["stop(session)<br/>kill session"]
        OPEN["open(query, layout)<br/>resolve + create + attach info"]
        DISCOVER["discover_projects()<br/>scan ~/Projects"]
    end

    DEVLIB --> LIST
    DEVLIB --> START
    DEVLIB --> STOP
    DEVLIB --> OPEN
    DEVLIB --> DISCOVER

    style DEVLIB fill:#4ecca3,color:#000
    style CLI fill:#1a1a2e,color:#e0e0e0
    style WEBUI fill:#e94560,color:#fff
    style AC fill:#e94560,color:#fff
```

## Voice Command Flow (end-to-end)

```mermaid
sequenceDiagram
    actor User
    participant Mic as Microphone
    participant STT as stt-publisher
    participant MQTT as MQTT Broker
    participant Agent as agent-core
    participant DevLib as dev-lib
    participant Plugin as plugin-runner
    participant Tmux as tmux session

    User->>Mic: "in lestash run the tests"
    Mic->>STT: audio stream
    STT->>MQTT: voice/transcriptions<br/>{"text": "in lestash run the tests", "is_final": true}
    MQTT->>Agent: message
    Agent->>DevLib: discover_projects() (at startup)
    Note over Agent: preprocess → regex match<br/>→ resolve "lestash" (exact match)<br/>→ Intent::Tmux { session: "lestash",<br/>pane: "shell", command: "cargo test" }
    Agent->>MQTT: agent/commands/tmux<br/>TmuxCommand JSON
    MQTT->>Plugin: message
    Plugin->>Tmux: tmux send-keys -t lestash:1.2<br/>"cargo test" Enter
    Plugin->>MQTT: agent/responses<br/>{"status": "ok"}
```

## Web UI Session Management Flow

```mermaid
sequenceDiagram
    actor User
    participant Browser as Web UI (browser)
    participant WebUI as web-ui (Axum)
    participant DevLib as dev-lib
    participant Tmux as tmux

    User->>Browser: Opens dashboard
    Browser->>WebUI: GET /api/sessions
    WebUI->>DevLib: DevManager::new()?.list()
    DevLib->>Tmux: tmux list-sessions
    Tmux-->>DevLib: session data
    DevLib-->>WebUI: ListOutput { sessions, projects }
    WebUI-->>Browser: JSON response
    Note over Browser: Populates session dropdown

    User->>Browser: Clicks "Start" on lestash
    Browser->>WebUI: POST /api/sessions/start?project=lestash
    WebUI->>DevLib: DevManager::new()?.start("lestash", None)
    DevLib->>Tmux: tmux new-session -d -s lestash ...
    DevLib->>Tmux: tmux split-window (if claude layout)
    Tmux-->>DevLib: ok
    DevLib-->>WebUI: "lestash"
    WebUI-->>Browser: {"project": "lestash", "output": "started"}
```

## Dictaphone Transcription Flow

```mermaid
sequenceDiagram
    actor User
    participant Browser as Web UI (browser)
    participant WebUI as web-ui (Axum)
    participant FFmpeg as ffmpeg
    participant Whisper as whisper-cpp
    participant MQTT as MQTT Broker

    User->>Browser: Click mic (Dictaphone mode)
    Note over Browser: MediaRecorder captures WebM/Opus
    User->>Browser: Click mic again (stop)
    Browser->>WebUI: POST /api/transcribe<br/>multipart: audio blob
    WebUI->>FFmpeg: Convert WebM → 16kHz mono WAV
    FFmpeg-->>WebUI: WAV file
    WebUI->>Whisper: whisper-cpp -m model -f audio.wav
    Whisper-->>WebUI: transcribed text
    WebUI-->>Browser: {"text": "run cargo test", "is_final": true}
    Browser->>MQTT: publish to voice/transcriptions
    Note over MQTT: Normal intent pipeline continues
```
