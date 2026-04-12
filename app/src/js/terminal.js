// terminal.js — Session list, polling, ttyd terminal

import { getApiBase, getTtydUrl, showToast } from './app.js';
import { debugAppend } from './debug.js';

const sessionList = document.getElementById('session-list');
const terminalFrame = document.getElementById('terminal-frame');
const terminalIframe = document.getElementById('terminal-iframe');
const terminalSessionName = document.getElementById('terminal-session-name');
const btnCloseTerminal = document.getElementById('btn-close-terminal');
const sessionTarget = document.getElementById('session-target');
const lastUpdatedEl = document.getElementById('last-updated');
const daemonBanner = document.getElementById('daemon-banner');
const daemonBannerDetail = document.getElementById('daemon-banner-detail');
const daemonRefreshBtn = document.getElementById('daemon-refresh-btn');
const btnRefresh = document.getElementById('btn-refresh');

let selectedSession = '';
let lastData = null;
let lastFetchTime = null;
let pollTimer = null;
let pollInterval = 3000; // 3s default
const POLL_MIN = 3000;
const POLL_MAX = 30000;
let daemonDown = false;

export function getSessionContext() {
  return selectedSession;
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

// --- Render ---

function renderSessions(data) {
  sessionList.innerHTML = '';

  // Active sessions
  if (data.sessions && data.sessions.length > 0) {
    const heading = document.createElement('div');
    heading.className = 'session-heading';
    heading.textContent = 'Active Sessions';
    sessionList.appendChild(heading);

    data.sessions.forEach(s => {
      const card = document.createElement('div');
      card.className = 'session-card' + (s.name === selectedSession ? ' selected' : '');
      card.dataset.session = s.name;

      const info = document.createElement('div');
      info.className = 'session-info';
      info.addEventListener('click', () => selectSession(s.name));

      const nameRow = document.createElement('div');
      nameRow.className = 'session-name-row';
      nameRow.innerHTML = `<span class="session-name">${esc(s.name)}</span>`;
      if (s.layout === 'claude') {
        nameRow.innerHTML += '<span class="session-badge badge-layout">claude</span>';
      }
      if (s.attached) {
        nameRow.innerHTML += '<span class="session-badge badge-attached">attached</span>';
      }
      info.appendChild(nameRow);

      const meta = document.createElement('div');
      meta.className = 'session-meta';
      meta.textContent = `${s.pane_count} pane${s.pane_count !== 1 ? 's' : ''} \u00b7 ${relativeTime(s.last_activity)}`;
      info.appendChild(meta);

      card.appendChild(info);

      const actions = document.createElement('div');
      actions.className = 'session-actions';

      const termBtn = document.createElement('button');
      termBtn.className = 'session-action-btn';
      termBtn.textContent = 'Terminal';
      termBtn.title = 'Open terminal';
      termBtn.addEventListener('click', (e) => { e.stopPropagation(); openTerminal(s.name); });
      actions.appendChild(termBtn);

      const killBtn = document.createElement('button');
      killBtn.className = 'session-action-btn btn-danger';
      killBtn.textContent = 'Kill';
      killBtn.title = 'Stop session';
      killBtn.addEventListener('click', (e) => { e.stopPropagation(); stopSession(s.name); });
      actions.appendChild(killBtn);

      card.appendChild(actions);
      sessionList.appendChild(card);
    });
  }

  // Dormant projects
  if (data.projects && data.projects.length > 0) {
    const heading = document.createElement('div');
    heading.className = 'session-heading session-heading-dormant';
    heading.textContent = 'Available Projects';
    sessionList.appendChild(heading);

    data.projects.forEach(p => {
      const row = document.createElement('div');
      row.className = 'project-row';

      const info = document.createElement('div');
      info.className = 'project-info';
      info.innerHTML = `<span class="project-name">${esc(p.name)}</span>`;
      const pathShort = p.path.replace(/^\/home\/[^/]+/, '~');
      info.innerHTML += `<span class="project-path">${esc(pathShort)}</span>`;
      if (p.layout && p.layout !== 'default') {
        info.innerHTML += `<span class="session-badge badge-layout">${esc(p.layout)}</span>`;
      }
      if (p.host) {
        info.innerHTML += `<span class="session-badge badge-remote">@${esc(p.host)}</span>`;
      }
      row.appendChild(info);

      const startBtn = document.createElement('button');
      startBtn.className = 'session-action-btn btn-start';
      startBtn.textContent = p.host ? 'Start (SSH)' : 'Start';
      startBtn.addEventListener('click', () => startSession(p.name));
      row.appendChild(startBtn);

      sessionList.appendChild(row);
    });
  }

  if ((!data.sessions || data.sessions.length === 0) && (!data.projects || data.projects.length === 0)) {
    sessionList.innerHTML = '<div class="empty">No sessions or projects found.</div>';
  }
}

function esc(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

// --- Selection ---

function selectSession(name) {
  selectedSession = name;
  sessionTarget.textContent = name;
  sessionTarget.classList.add('has-session');
  // Re-render to update selected highlight
  if (lastData) renderSessions(lastData);
}

// --- Terminal ---

function openTerminal(name) {
  selectSession(name);
  // Switch ttyd to this session
  fetch(`${getApiBase()}/api/sessions/switch?session=${encodeURIComponent(name)}`, { method: 'POST' }).catch(() => {});
  terminalSessionName.textContent = name;
  terminalIframe.src = `${getTtydUrl()}?t=${Date.now()}`;
  terminalFrame.style.display = 'flex';
}

function closeTerminal() {
  terminalFrame.style.display = 'none';
  terminalIframe.src = 'about:blank';
}

// --- Actions ---

async function startSession(project) {
  try {
    const resp = await fetch(`${getApiBase()}/api/sessions/start?project=${encodeURIComponent(project)}`, { method: 'POST' });
    const data = await resp.json();
    if (data.error) { showToast(data.error, 'error'); return; }
    showToast(`Started: ${project}`, 'ok');
    selectSession(project);
    await loadSessions();
  } catch (e) {
    showToast(`Failed to start: ${e}`, 'error');
  }
}

async function stopSession(session) {
  try {
    const resp = await fetch(`${getApiBase()}/api/sessions/stop?session=${encodeURIComponent(session)}`, { method: 'POST' });
    const data = await resp.json();
    if (data.error) { showToast(data.error, 'error'); return; }
    showToast(`Stopped: ${session}`, 'ok');
    if (selectedSession === session) {
      selectedSession = '';
      sessionTarget.textContent = 'No session selected';
      sessionTarget.classList.remove('has-session');
      closeTerminal();
    }
    await loadSessions();
  } catch (e) {
    showToast(`Failed to stop: ${e}`, 'error');
  }
}

// --- Polling ---

async function loadSessions() {
  try {
    const url = `${getApiBase()}/api/sessions`;
    debugAppend('sessions', `GET ${url}`);
    const resp = await fetch(url);
    debugAppend('sessions', `${resp.status} ${resp.statusText}`);
    if (resp.status === 503) {
      const body = await resp.json().catch(() => ({}));
      showDaemonBanner(body.detail || body.error || 'dev daemon unreachable');
      return;
    }
    const data = await resp.json();

    // Daemon recovered
    if (daemonDown) hideDaemonBanner();
    pollInterval = POLL_MIN; // reset backoff on success

    lastData = data;
    lastFetchTime = Date.now();
    updateLastUpdated();
    renderSessions(data);

    // Auto-select first session if none selected
    if (!selectedSession && data.sessions && data.sessions.length > 0) {
      selectSession(data.sessions[0].name);
    }
    // If selected session disappeared, clear selection
    if (selectedSession && data.sessions && !data.sessions.some(s => s.name === selectedSession)) {
      selectedSession = '';
      sessionTarget.textContent = 'No session selected';
      sessionTarget.classList.remove('has-session');
    }
  } catch (e) {
    debugAppend('sessions', `ERROR: ${e}`);
    if (e && e.toString().includes('unreachable')) {
      showDaemonBanner(e.toString());
    }
    // Keep last-known state visible — don't clear the list
    pollInterval = Math.min(pollInterval * 2, POLL_MAX);
  }
}

function updateLastUpdated() {
  if (!lastFetchTime) return;
  const secs = Math.floor((Date.now() - lastFetchTime) / 1000);
  lastUpdatedEl.textContent = secs < 2 ? 'just now' : `${secs}s ago`;
}

function schedulePoll() {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = setTimeout(async () => {
    await loadSessions();
    schedulePoll();
  }, pollInterval);
}

// --- Daemon banner ---

function showDaemonBanner(detail) {
  daemonDown = true;
  daemonBannerDetail.textContent = detail || 'check systemctl --user status dev-daemon';
  daemonBanner.classList.add('visible');
  sessionList.classList.add('daemon-down');
  // Exponential backoff
  pollInterval = Math.min(pollInterval * 2, POLL_MAX);
}

function hideDaemonBanner() {
  daemonDown = false;
  daemonBanner.classList.remove('visible');
  sessionList.classList.remove('daemon-down');
}

// --- Visibility ---

function handleVisibility() {
  if (document.visibilityState === 'visible') {
    pollInterval = POLL_MIN;
    loadSessions();
    schedulePoll();
  } else {
    if (pollTimer) clearTimeout(pollTimer);
  }
}

// --- Init ---

export function initTerminal() {
  btnCloseTerminal.addEventListener('click', closeTerminal);
  daemonRefreshBtn.addEventListener('click', () => {
    pollInterval = POLL_MIN;
    loadSessions();
  });
  btnRefresh.addEventListener('click', () => {
    pollInterval = POLL_MIN;
    loadSessions();
  });

  document.addEventListener('visibilitychange', handleVisibility);

  // Update "last updated" display every second
  setInterval(updateLastUpdated, 1000);

  loadSessions();
  schedulePoll();
}
