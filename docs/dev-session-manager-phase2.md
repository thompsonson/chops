# Phase 2 — Multi-host SSH + voice routing

> Prerequisite: Phase 1 deployed. Adds remote host SSH tunnels, host-grouped session UI,
> and voice command bypass of MQTT for session operations.

## Steps

### 6. `TunnelManager` with per-host lifecycle

**File:** `app/src-tauri/src/tunnel.rs`

```rust
pub struct TunnelManager {
    tunnels: HashMap<String, SshTunnel>,
}

impl TunnelManager {
    /// Returns the forwarded socket path, creating a tunnel if needed.
    pub fn ensure_tunnel(&mut self, host: &str) -> Result<&Path>;

    /// Tear down a specific tunnel.
    pub fn stop(&mut self, host: &str);

    /// Tear down all tunnels (called on app quit).
    pub fn stop_all(&mut self);
}

struct SshTunnel {
    host: String,
    child: Option<Child>,
    socket_path: PathBuf,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}
```

SSH command: `ssh -NL /tmp/dev-{host}.sock:/run/user/{uid}/dev.sock {host}`
Liveness check via `SshTunnel.child.try_wait()` every 30s (poll in `list_sessions`).

Android: detect `target_os = "android"`, use `/data/data/com.termux/files/usr/bin/ssh`.

**File:** `app/src-tauri/src/lib.rs` — add to AppState:

```rust
pub struct AppState {
    pub tunnel_mgr: Mutex<TunnelManager>,
    pub dev_client: fn(host: &str) -> Result<DevClient>,  // uses tunnel_mgr.ensure_tunnel
    // ... existing fields
}
```

`client_for_host(&self, host: &str) -> Result<DevClient>` pattern:

```rust
pub fn client_for_host(&self, host: &str) -> Result<DevClient> {
    let path = self.tunnel_mgr.lock().ensure_tunnel(host)?;
    Ok(DevClient::new(path.to_path_buf()))
}
```

**Acceptance:** `npx tauri dev` — calling `ensure_tunnel("pop-mini.monkey-ladon.ts.net")` spawns SSH and returns a socket path.

---

### 7. Host management Tauri commands

**File:** `app/src-tauri/src/lib.rs`

| Command | Signature | Details |
|---------|-----------|---------|
| `tunnel_status` | `() -> Result<Vec<TunnelStatus>>` | Per-host tunnel health |
| `add_host` | `(hostname: String) -> Result<()>` | Persists to `localStorage`-analogue |
| `remove_host` | `(hostname: String) -> Result<()>` | Removes + tears down tunnel |
| `list_hosts` | `() -> Result<Vec<String>>` | Configured hosts |

Host list stored in `localStorage` on the JS side:
```js
const hosts = JSON.parse(localStorage.getItem('chops-hosts') || '[]');
```
Tauri commands read/write via `app.get_webview_window().eval()` or add a dedicated state
field. localStorage survives app updates and avoids filesystem permission issues on Android.
(If `app_data_dir` persistence is needed later, add then — not now.)

Update Phase 1 commands to accept `host: String` parameter; use `client_for_host(host)` instead of `DevClient::from_env()`.

**Acceptance:** Hosts persist across app restarts. Adding a host creates a tunnel. Removing it tears down the tunnel.

---

### 8. `sessions.js` — host-grouped session UI

**File:** `app/src/js/session/sessions.js`

Host-grouped session list render:

```js
async function loadSessions() {
  const hosts = await dispatch({ type: "list_hosts" });
  const sessions = await Promise.all(
    hosts.map(async (host) => ({
      host,
      listing: await dispatch({ type: "list_sessions", host }),
    }))
  );
  renderGroupedSessions(sessions);
}

function renderGroupedSessions(data) {
  data.forEach(({ host, listing }) => {
    const group = createHostGroup(host, listing);
    sessionList.appendChild(group);
  });
}
```

Each host group renders as a collapsible card with sessions inside.

Tunnel status bar at bottom of tab:

```html
<div class="tunnel-bar">
  <span>Tunnels:</span>
  <span class="tunnel-status" data-host="pop-mini">pop-mini ●</span>
  <span class="tunnel-status" data-host="build-server">build-server ●</span>
</div>
```

**File:** `app/src/js/app.js` — add `initSessionPolling()` that calls `loadSessions()` every 3s and updates tunnel status.

**Acceptance:** Multi-host session list renders grouped by host. Tunnel status bar shows green dots.

---

### 9. IntentParser — route session voice commands via `SessionAction`

**File:** `app/src/js/voice.js` and `app/src/js/commands.js`

In the `sendCommand` flow (after transcription, before MQTT publish):

```js
function isSessionCommand(text) {
  const match = text.match(/^in\s+(\S+)\s+(.+)/i);
  if (!match) return null;
  return { project: match[1], command: match[2] };
}

async function sendCommand(text, conversationId) {
  const sessionCmd = isSessionCommand(text);

  if (sessionCmd) {
    // Try fast path: send keys directly to session
    const sessions = await dispatch({ type: "list_sessions" });
    // Flat-map across hosts
    const all = sessions.flatMap(h => h.listing.sessions);
    const target = all.find(s =>
      s.name.toLowerCase() === sessionCmd.project.toLowerCase()
    );
    if (target) {
      await dispatch({
        type: "send_keys",
        host: target.host,
        session: target.name,
        pane: "1.1",
        keys: sessionCmd.command + "\n",
      });
      return;
    }
  }

  // Fall through to MQTT as before
  await tauriInvoke("send_transcription", { text, conversationId });
}
```

Intent matching uses exact name match (simpler than Jaro-Winkler — the backend parser handles fuzzy if needed via AtomicGuard).

**Acceptance:** Saying "in dev run cargo test" sends keys directly to the session pane. Non-session commands still go through MQTT.

---

### 10. Android SSH path in `tunnel.rs`

**File:** `app/src-tauri/src/tunnel.rs`

```rust
#[cfg(target_os = "android")]
fn ssh_binary() -> &'static str {
    "/data/data/com.termux/files/usr/bin/ssh"
}

#[cfg(not(target_os = "android"))]
fn ssh_binary() -> &'static str {
    "ssh"
}
```

Android also needs `tailscale` or direct IP for reachability (Tailscale Funnel if on different network). The `host` config should accept `user@host` SSH-style strings.

**Acceptance:** Android build uses Termux SSH path. Tunnels work from an Android device on the same Tailscale network.
