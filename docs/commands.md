# chops — Voice Command Reference

## How commands work

You speak a command into your microphone. whisper.cpp transcribes it to text. The agent parses your intent and routes it to the right target. Commands are natural language — you don't need to be precise.

### What gets cleaned up automatically

Before parsing, the agent strips noise that whisper.cpp commonly introduces:

- **Filler words** are removed: *uh, um, like, please, okay, ok, hey, so, well, just, actually, right*
- **Punctuation** at word boundaries is stripped: periods, commas, exclamation marks, question marks, semicolons, colons
- **Capitalization** is ignored for keywords (but preserved where it matters, like filenames)

So `"Uh, please run cargo test."` becomes `"run cargo test"` before matching.

### Synonym support

You don't need to remember exact keywords. These alternatives all work:

| You can say | Interpreted as |
|-------------|---------------|
| run, execute, start, launch, exec | **run** |
| tell, message, ask, send | **tell** |
| vscode, editor, code | **vscode** |

### Fuzzy project names

Project names are matched against your `~/Projects/` directory. Close misspellings are auto-corrected:

| You say | Matched to |
|---------|-----------|
| "chop" | chops |
| "shops" | chops |
| "manta-deplo" | manta-deploy |

Exact matches get full confidence (1.0). Fuzzy matches get 0.8. Unrecognized names are passed through as-is (0.6).

---

## Commands

### Run a command in a project

Send a shell command to a specific project's tmux session.

```
in <project> run <command>
```

**Examples:**
- "in chops run cargo test"
- "in manta-deploy run docker compose up"
- "in dotfiles run chezmoi apply"
- "okay, in chops execute cargo build --release" *(synonym + filler)*

**What happens:** Sends `<command>` via `tmux send-keys` to the shell pane of the `<project>` tmux session.

**Pane targeting:**
- In a 2-pane layout (from `dev claude <project>`): targets the **shell** pane (right, pane 2)
- In a 1-pane layout (from `dev <project>`): targets the only pane (pane 1)

---

### Send a message to Claude in a project

Type a message into the Claude Code pane of a project's tmux session. Supports multi-sentence messages using a terminator keyword.

```
in <project> tell claude <message>... over
```

**Examples:**

Short (single segment):
- "in chops tell claude fix the failing tests. Over."
- "in chops ask claude add error handling. Done."

Long (multi-segment — speak naturally, say "over" when finished):
- *"In chops tell claude review the parser."*
- *"Fix the edge cases around fuzzy matching."*
- *"And add tests for preprocessing. Over."*

All three segments are combined into one message: *"review the parser fix the edge cases around fuzzy matching and add tests for preprocessing"*

**Terminator keywords:** `over`, `out`, `done`, `end`, `finish`, `send it`

**How accumulation works:**
1. "tell claude" starts accumulation — the message is buffered, not sent
2. Subsequent segments without a command prefix are appended
3. When a terminator keyword is heard, the combined message is sent
4. Safety timeout: if you forget to say "over", the message auto-sends after 30 seconds
5. If a new command is detected while accumulating (e.g. "in chops run ..."), the accumulated message is flushed first, then the new command executes

**Note:** Only Claude messages accumulate. Shell commands (`run`), VSCode, and termux commands execute immediately — no terminator needed.

**What happens:** Sends `<message>` via `tmux send-keys` to the claude pane (left, pane 1) of the `<project>` tmux session. Requires a 2-pane layout created by `dev claude <project>`.

---

### Run a command in the active session

Send a command to whatever tmux session is currently attached.

```
run <command>
```

**Examples:**
- "run ls"
- "run git status"
- "run cargo test --workspace"
- "please launch git log --oneline" *(filler + synonym)*

**What happens:** Queries tmux for the currently active session and sends `<command>` to its shell pane.

---

### Open a file in VSCode

```
open vscode <file>
```

**Examples:**
- "open vscode README.md"
- "open editor src/main.rs" *(synonym)*
- "open code Cargo.toml" *(synonym)*

**What happens:** Runs `code <file>` to open the file in VSCode.

---

### Run a terminal command directly

```
termux <command>
terminal <command>
```

**Examples:**
- "termux echo hello"
- "terminal ls -la"

**What happens:** Runs `<command>` via `bash -c`. Originally designed for Termux on Android but works as a general shell executor.

---

## Testing commands without audio

You can inject commands directly via MQTT to test the pipeline:

```bash
# Send a command to a project
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "in chops run cargo test", "is_final": true}'

# Test with noisy whisper-like input
mosquitto_pub -p 1884 -t voice/transcriptions \
  -m '{"text": "Uh, please in chop execute cargo test.", "is_final": true}'

# Watch what gets routed
mosquitto_sub -p 1884 -t 'agent/commands/#' -v

# Watch responses from plugin execution
mosquitto_sub -p 1884 -t 'agent/responses'

# Monitor all MQTT traffic
mosquitto_sub -p 1884 -t '#' -v
```

### Message format

Transcription messages are JSON on the `voice/transcriptions` topic:

```json
{"text": "in chops run cargo test", "is_final": true}
```

- `is_final: true` — agent processes this as a complete utterance
- `is_final: false` — agent ignores this (partial transcription, used for UI feedback only)

### Routed command format

Tmux commands are JSON on `agent/commands/tmux`:

```json
{"session": "chops", "pane": "shell", "command": "cargo test"}
```

Responses come back on `agent/responses`:

```json
{"topic": "agent/commands/tmux", "status": "ok", "output": "Sent to chops:1.2: "}
```

---

## Command summary

| Pattern | Target | Pane | Accumulates? |
|---------|--------|------|-------------|
| `in <project> run <command>` | project's tmux session | shell | No — immediate |
| `in <project> tell claude <msg>... over` | project's tmux session | claude | Yes — until terminator |
| `run <command>` | active tmux session | shell | No — immediate |
| `open vscode <file>` | VSCode | — | No — immediate |
| `termux <command>` | bash shell | — | No — immediate |

All patterns support filler words, punctuation, synonyms, and fuzzy project names.

**Terminators:** `over`, `out`, `done`, `end`, `finish`, `send it`
