// sessions.js — Multi-host session list UI
// Reads hosts from localStorage, groups sessions by host

import { dispatch } from './SessionAction.js';
import { showToast, tauriInvoke } from '../app.js';

const HOSTS_KEY = 'chops-hosts';

// --- Host list persistence ---

export function getHosts() {
  try {
    return JSON.parse(localStorage.getItem(HOSTS_KEY) || '[]');
  } catch {
    return [];
  }
}

export function setHosts(hosts) {
  localStorage.setItem(HOSTS_KEY, JSON.stringify(hosts));
}

export function addHost(host) {
  const hosts = getHosts();
  if (!hosts.includes(host)) {
    hosts.push(host);
    setHosts(hosts);
  }
}

export function removeHost(host) {
  setHosts(getHosts().filter(h => h !== host));
}

// --- Session fetching per host ---

async function fetchHostSessions(host) {
  try {
    const listing = await dispatch({ type: 'list_sessions', host });
    return { host, listing, error: null };
  } catch (e) {
    return { host, listing: null, error: String(e) };
  }
}

// --- Render grouped sessions ---

export async function renderGroupedSessions(container) {
  const hosts = getHosts();

  if (hosts.length === 0) {
    return false; // no hosts — caller falls back to single-host
  }

  container.innerHTML = '';

  const results = await Promise.all(hosts.map(fetchHostSessions));

  for (const { host, listing, error } of results) {
    const group = document.createElement('div');
    group.className = 'host-group';

    const header = document.createElement('div');
    header.className = 'host-group-header';
    header.innerHTML = `<span class="host-name">${esc(host)}</span>
      <span class="host-count">${listing ? listing.sessions.length + ' sessions' : 'error'}</span>
      <button class="tab-action-btn btn-remove-host" data-host="${esc(host)}">Remove</button>`;
    group.appendChild(header);

    if (error) {
      const errEl = document.createElement('div');
      errEl.className = 'host-error';
      errEl.textContent = `Connection failed: ${error}`;
      const retryBtn = document.createElement('button');
      retryBtn.className = 'tab-action-btn';
      retryBtn.textContent = 'Retry';
      retryBtn.addEventListener('click', () => renderGroupedSessions(container));
      errEl.appendChild(retryBtn);
      group.appendChild(errEl);
    } else if (listing) {
      const list = document.createElement('div');
      list.className = 'host-sessions';

      for (const s of listing.sessions) {
        const card = document.createElement('div');
        card.className = 'session-card';
        card.dataset.session = s.name;
        card.dataset.host = host;

        const info = document.createElement('div');
        info.className = 'session-info';
        info.innerHTML = `<div class="session-name-row">
          <span class="session-name">${esc(s.name)}</span>
          ${s.layout === 'claude' ? '<span class="session-badge badge-layout">claude</span>' : ''}
          ${s.attached ? '<span class="session-badge badge-attached">attached</span>' : ''}
          ${s.agent && !s.agent_running ? '<span class="session-badge badge-danger">agent down</span>' : ''}
        </div>
        <div class="session-meta">${s.pane_count} pane${s.pane_count !== 1 ? 's' : ''} · ${relativeTime(s.last_activity)}</div>`;
        card.appendChild(info);

        const actions = document.createElement('div');
        actions.className = 'session-actions';

        const inspectBtn = document.createElement('button');
        inspectBtn.className = 'session-action-btn';
        inspectBtn.textContent = 'Inspect';
        inspectBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          window.dispatchEvent(new CustomEvent('inspect-session', { detail: { host, name: s.name } }));
        });
        actions.appendChild(inspectBtn);

        const termBtn = document.createElement('button');
        termBtn.className = 'session-action-btn';
        termBtn.textContent = 'Terminal';
        termBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          window.dispatchEvent(new CustomEvent('open-terminal', { detail: { host, name: s.name } }));
        });
        actions.appendChild(termBtn);

        const killBtn = document.createElement('button');
        killBtn.className = 'session-action-btn btn-danger';
        killBtn.textContent = 'Kill';
        killBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          stopHostSession(host, s.name);
        });
        actions.appendChild(killBtn);

        card.appendChild(actions);
        list.appendChild(card);
      }

      if (listing.sessions.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'empty';
        empty.textContent = 'No active sessions';
        list.appendChild(empty);
      }

      group.appendChild(list);
    }

    container.appendChild(group);
  }

  // Tunnel status bar
  const bar = document.createElement('div');
  bar.className = 'tunnel-bar';
  const statuses = await dispatch({ type: 'tunnel_status' });
  bar.innerHTML = '<span>Tunnels:</span> ' + hosts.map(h => {
    const s = statuses ? statuses.find(st => st.host === h) : null;
    const alive = s ? s.alive : false;
    return `<span class="tunnel-status ${alive ? 'alive' : 'dead'}">${esc(h)} ${alive ? '●' : '○'}</span>`;
  }).join(' ');
  container.appendChild(bar);

  return true; // rendered multi-host
}

async function stopHostSession(host, name) {
  try {
    await dispatch({ type: 'stop', host, session: name });
    showToast(`Stopped: ${name}`, 'ok');
  } catch (e) {
    showToast(`Failed to stop: ${e}`, 'error');
  }
}

// --- Android SSH provisioning ---

export async function isAndroid() {
  if (!tauriInvoke) return false;
  try {
    await tauriInvoke('ssh_key_status', { host: '__test__' });
    return true;
  } catch {
    return false;
  }
}

export async function provisionAndroidHost(host) {
  const alias = host.replace('.', '_');
  const hasKey = await tauriInvoke('ssh_key_status', { host: alias });
  if (hasKey) {
    addHost(host);
    return true;
  }

  // Show password-auth dialog
  const overlay = document.getElementById('provision-overlay');
  const hostnameEl = document.getElementById('provision-hostname');
  const usernameEl = document.getElementById('provision-username');
  const passwordEl = document.getElementById('provision-password');
  const portEl = document.getElementById('provision-port');
  const errorEl = document.getElementById('provision-error');
  const spinnerEl = document.getElementById('provision-spinner');
  const cancelBtn = document.getElementById('provision-cancel');
  const doneBtn = document.getElementById('provision-done');

  hostnameEl.textContent = host;
  usernameEl.value = 'mt';
  passwordEl.value = '';
  portEl.value = '22';
  errorEl.style.display = 'none';
  spinnerEl.style.display = 'none';
  doneBtn.disabled = false;

  overlay.classList.add('visible');
  usernameEl.focus();

  function setLoading(loading) {
    doneBtn.disabled = loading;
    spinnerEl.style.display = loading ? '' : 'none';
    cancelBtn.disabled = loading;
  }

  return new Promise((resolve) => {
    const cleanup = () => {
      overlay.classList.remove('visible');
      passwordEl.value = '';
      cancelBtn.removeEventListener('click', onCancel);
      doneBtn.removeEventListener('click', onDone);
      passwordEl.removeEventListener('keydown', onKeydown);
    };

    function onCancel() {
      cleanup();
      resolve(false);
    }

    async function onDone() {
      const username = usernameEl.value.trim() || 'mt';
      const password = passwordEl.value;
      if (!password) {
        errorEl.textContent = 'Password is required';
        errorEl.style.display = '';
        return;
      }

      setLoading(true);
      errorEl.style.display = 'none';

      try {
        const port = parseInt(portEl.value) || 22;
        const result = await tauriInvoke('ssh_authorize_key', {
          hostname: host,
          port,
          username,
          password,
        });
        addHost(host);
        cleanup();
        showToast(result || 'Key authorized', 'ok');
        resolve(true);
      } catch (e) {
        errorEl.textContent = String(e);
        errorEl.style.display = '';
        setLoading(false);
      }
    }

    function onKeydown(e) {
      if (e.key === 'Enter') onDone();
      if (e.key === 'Escape') onCancel();
    }

    cancelBtn.addEventListener('click', onCancel);
    doneBtn.addEventListener('click', onDone);
    passwordEl.addEventListener('keydown', onKeydown);
  });
}

function esc(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

// --- Relative time ---

function relativeTime(epochSecs) {
  const diff = Math.floor(Date.now() / 1000) - epochSecs;
  if (diff < 5) return 'just now';
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
