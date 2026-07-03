# Phase 1 — Direct DevClient integration (single-host)

> Replace the web-ui HTTP proxy with direct `DevClient` calls from the Tauri backend.
> Adds inspect/pane detail views. No SSH tunnels, no multi-host.

## Steps

### 1. Add `inspect()` + `pane_content()` to DevClient

**File:** `crates/chops-dev-client/src/lib.rs`

Two new methods following the `send_keys()` pattern:

```rust
pub async fn inspect(&self, name: &str, lines: Option<u32>, full: Option<bool>) -> Result<Value> {
    let mut path = format!("/sessions/{name}/inspect");
    let mut qs = Vec::new();
    if let Some(n) = lines { qs.push(format!("lines={n}")); }
    if let Some(true) = full { qs.push("full=true".into()); }
    if !qs.is_empty() { path.push_str(&format!("?{}", qs.join("&"))); }
    let body = self.request("GET", &path, None).await?;
    Ok(serde_json::from_str(&body)?)
}

pub async fn pane_content(&self, name: &str, pane: &str, lines: Option<u32>) -> Result<String> {
    let mut path = format!("/sessions/{name}/panes/{pane}/content");
    if let Some(n) = lines { path.push_str(&format!("?lines={n}")); }
    self.request("GET", &path, None).await
}
```

**Daemon endpoints (already exist, no server changes):**
- `GET /sessions/{name}/inspect?lines=N&full=B` → `{session, git, sandbox, content: {pane, tail}}`
- `GET /sessions/{name}/panes/{pane}/content?lines=N` → `{content: "..."}`

**Acceptance:** `cargo test --workspace` passes, new methods callable from tests.

---

### 2. Add `chops-dev-client` dep + wire into AppState

**File:** `app/src-tauri/Cargo.toml`

```toml
chops-dev-client = { path = "../../crates/chops-dev-client" }
```

**File:** `app/src-tauri/src/lib.rs`

```rust
use chops_dev_client::DevClient;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct AppState {
    pub mqtt: Arc<MqttClient>,
    pub stt: Arc<SttEngine>,
    pub dev_client: Mutex<DevClient>,
}
```

Initialize in `run()`:

```rust
let dev_client = Mutex::new(DevClient::from_env());
```

Set via `app.manage(state)` alongside the existing fields.

**Acceptance:** app compiles, `AppState` carries a `DevClient`.

---

### 3. Tauri commands (6 commands, single-host)

**File:** `app/src-tauri/src/lib.rs`

| Command | Signature | Details |
|---------|-----------|---------|
| `list_sessions` | `() -> Result<Listing>` | Calls `DevClient.list()` |
| `inspect_session` | `(name: String, lines: Option<u32>) -> Result<Value>` | Calls `DevClient.inspect()` — defaults lines=0 (no pane tail fetch on poll; fetch on demand when user clicks Inspect) |
| `pane_content` | `(name: String, pane: String, lines: Option<u32>) -> Result<String>` | Calls `DevClient.pane_content()` |
| `send_keys` | `(name: String, pane: String, keys: String)` | Calls `DevClient.send_keys()` |
| `start_session` | `(project: String, layout: Option<String>)` | Calls `DevClient.start()` |
| `stop_session` | `(name: String)` | Calls `DevClient.stop()` |

All commands extract `state.dev_client.lock()` and call the corresponding method.
Register in `invoke_handler` with the existing commands.

No `host` parameter yet — single-host via `DevClient::from_env()`.
> Phase 2 adds a `host` parameter to all commands and switches to `client_for_host()`.
> This means `SessionAction.js` and `terminal.js` dispatch calls need a mechanical
> refactor in Phase 2 to pass `host`. Plan for it.

**Acceptance:** `npx tauri dev` starts, each command can be invoked from the JS console.

---

### 4. `SessionAction.js` dispatcher

**File:** `app/src/js/session/SessionAction.js`

```js
export async function dispatch(action) {
  const { invoke } = window.__TAURI_INTERNALS__;
  switch (action.type) {
    case "send_keys":
      return invoke("send_keys", { name: action.session, pane: action.pane, keys: action.keys });
    case "start":
      return invoke("start_session", { project: action.project, layout: action.layout ?? null });
    case "stop":
      return invoke("stop_session", { name: action.session });
    case "inspect":
      return invoke("inspect_session", { name: action.session });
    case "pane_content":
      return invoke("pane_content", { name: action.session, pane: action.pane, lines: action.lines ?? 20 });
    case "list_sessions":
      return invoke("list_sessions");
  }
}
```

**Acceptance:** importable, can be tested by calling `dispatch({ type: "list_sessions" })`.

---

### 5. Inspect panel in HTML + wire session polling

**File:** `app/src/index.html`

Add to `#tab-sessions` after `#terminal-frame`:

```html
<div class="inspect-panel" id="inspect-panel" style="display:none">
  <div class="inspect-header">
    <span id="inspect-session-name"></span>
    <button class="tab-action-btn" id="btn-close-inspect">Close</button>
  </div>
  <div class="inspect-body" id="inspect-body">
    <div class="inspect-section" id="inspect-git"></div>
    <div class="inspect-section" id="inspect-meta"></div>
    <pre class="inspect-tail" id="inspect-tail"></pre>
  </div>
  <button class="session-action-btn" id="btn-refresh-inspect">Refresh</button>
</div>
```

**File:** `app/src/styles.css`

Add styles for `.inspect-panel`, `.inspect-header`, `.inspect-body`, `.inspect-section`, `.inspect-tail`.

**File:** `app/src/js/terminal.js`

Replace `loadSessions()` HTTP fetch with:

```js
async function loadSessions() {
  if (!IS_TAURI) {
    // fallback to HTTP fetch for browser mode
    return loadSessionsHttp();
  }
  try {
    const data = await tauriInvoke("list_sessions");
    lastData = data;
    renderSessions(data);
  } catch (e) {
    showDaemonBanner(e);
  }
}
```

Add `inspectSession(name)` that calls `dispatch({ type: "inspect", session: name })` and renders result into `#inspect-panel`. Add "Inspect" button next to existing "Terminal" and "Kill" buttons on session cards.

**Acceptance:** Sessions tab polls via Tauri invoke. Clicking a session shows inspect panel with git state and pane tail.

---

### 6. Refactor `terminal.js` to use `SessionAction` instead of HTTP

**File:** `app/src/js/terminal.js`

Replace `sendKeysToTerminal()` HTTP fetch with `SessionAction` dispatch:

```js
async function sendKeysToTerminal(text) {
  if (IS_TAURI) {
    await dispatch({
      type: "send_keys",
      session: selectedSession,
      pane: "1.1",
      keys: text + (terminalSendEnter.checked ? "\n" : ""),
    });
  } else {
    await sendKeysHttp(text);
  }
}
```

Same for `startSession()` and `stopSession()` — use `dispatch()` instead of `fetch()`.
`openTerminal(name)` still uses ttyd iframe — unchanged.

> Phase 2 adds the `host` field to these dispatch calls. The interface stays the same;
> only the payload grows.

**Acceptance:** Tauri mode sends keys via DevClient. Browser-only mode falls back to HTTP.
