// SessionAction.js — Action dispatcher for dev session operations
// Routes to Tauri invoke (desktop) or HTTP fallback (browser)

export async function dispatch(action) {
  if (window.__TAURI_INTERNALS__) {
    const { invoke } = window.__TAURI_INTERNALS__;
    const host = action.host ?? null;
    switch (action.type) {
      case 'send_keys':
        return invoke('send_keys', { host, name: action.session, pane: action.pane, keys: action.keys });
      case 'start':
        return invoke('start_session', { host, project: action.project, layout: action.layout ?? null });
      case 'stop':
        return invoke('stop_session', { host, name: action.session });
      case 'inspect':
        return invoke('inspect_session', { host, name: action.session, lines: action.lines ?? null });
      case 'pane_content':
        return invoke('pane_content', { host, name: action.session, pane: action.pane, lines: action.lines ?? 20 });
      case 'list_sessions':
        return invoke('list_sessions', { host });
      case 'tunnel_status':
        return invoke('tunnel_status');
      case 'list_hosts':
        return invoke('list_hosts');
    }
  }
  // TODO: browser-only HTTP fallback using getApiBase()
  throw new Error('SessionAction dispatch requires Tauri');
}
