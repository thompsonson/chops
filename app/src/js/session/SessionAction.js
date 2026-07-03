// SessionAction.js — Action dispatcher for dev session operations
// Routes to Tauri invoke (desktop) or HTTP fallback (browser)

export async function dispatch(action) {
  if (window.__TAURI_INTERNALS__) {
    const { invoke } = window.__TAURI_INTERNALS__;
    switch (action.type) {
      case 'send_keys':
        return invoke('send_keys', { name: action.session, pane: action.pane, keys: action.keys });
      case 'start':
        return invoke('start_session', { project: action.project, layout: action.layout ?? null });
      case 'stop':
        return invoke('stop_session', { name: action.session });
      case 'inspect':
        return invoke('inspect_session', { name: action.session, lines: action.lines ?? null });
      case 'pane_content':
        return invoke('pane_content', { name: action.session, pane: action.pane, lines: action.lines ?? 20 });
      case 'list_sessions':
        return invoke('list_sessions');
    }
  }
  // TODO: browser-only HTTP fallback using getApiBase()
  throw new Error('SessionAction dispatch requires Tauri');
}
