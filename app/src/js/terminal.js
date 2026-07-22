// terminal.js — Session list, polling, ttyd terminal

import { IS_TAURI, tauriInvoke, getApiBase, getTtydUrl, showToast, settings } from './app.js';
import { debugAppend } from './debug.js';
import { dispatch } from './session/SessionAction.js';
import { renderGroupedSessions, getHosts, addHost, removeHost, isAndroid, provisionAndroidHost } from './session/sessions.js';

const sessionList = document.getElementById('session-list');
const terminalFrame = document.getElementById('terminal-frame');
const terminalIframe = document.getElementById('terminal-iframe');
const terminalSessionName = document.getElementById('terminal-session-name');
const btnCloseTerminal = document.getElementById('btn-close-terminal');

// Inspect panel
const inspectPanel = document.getElementById('inspect-panel');
const inspectSessionName = document.getElementById('inspect-session-name');
const inspectGitValue = document.getElementById('inspect-git-value');
const inspectLastValue = document.getElementById('inspect-last-value');
const inspectRepoValue = document.getElementById('inspect-repo-value');
const inspectTail = document.getElementById('inspect-tail');
const btnRefreshInspect = document.getElementById('btn-refresh-inspect');
const btnCloseInspect = document.getElementById('btn-close-inspect');
const sessionTarget = document.getElementById('session-target');
const lastUpdatedEl = document.getElementById('last-updated');
const daemonBanner = document.getElementById('daemon-banner');
const daemonBannerDetail = document.getElementById('daemon-banner-detail');
const daemonRefreshBtn = document.getElementById('daemon-refresh-btn');
const btnRefresh = document.getElementById('btn-refresh');

// Host management
const hostInput = document.getElementById('host-input');
const btnAddHost = document.getElementById('btn-add-host');

let selectedSession = '';
let lastData = null;
let lastFetchTime = null;
let refreshTimer = null;
let daemonDown = false;
const DAEMON_RETRY_MS = 10000; // 10s retry while daemon is down

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

      const inspectBtn = document.createElement('button');
      inspectBtn.className = 'session-action-btn';
      inspectBtn.textContent = 'Inspect';
      inspectBtn.title = 'Show session details';
      inspectBtn.addEventListener('click', (e) => { e.stopPropagation(); inspectSession(s.name); });
      actions.appendChild(inspectBtn);

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
  closeInspect();
  selectSession(name);
  // Switch ttyd to this session
  fetch(`${getApiBase()}/api/sessions/switch?session=${encodeURIComponent(name)}`, { method: 'POST' }).catch(() => {});
  terminalSessionName.textContent = name;
  terminalIframe.src = `${getTtydUrl()}?t=${Date.now()}`;
  terminalFrame.style.display = 'flex';
  sessionList.classList.add('hidden');
  // Focus iframe so keyboard input goes to ttyd
  terminalIframe.addEventListener('load', () => {
    terminalIframe.focus();
    // Try to focus xterm inside the iframe
    try { terminalIframe.contentWindow.focus(); } catch {}
  }, { once: true });
}

function closeTerminal() {
  terminalFrame.style.display = 'none';
  sessionList.classList.remove('hidden');
  terminalIframe.src = 'about:blank';
}

// --- Inspect ---

let inspectSessionName_ = '';

async function inspectSession(name) {
  closeTerminal();
  inspectSessionName_ = name;
  inspectSessionName.textContent = name;
  inspectGitValue.textContent = 'Loading...';
  inspectLastValue.textContent = '';
  inspectRepoValue.textContent = '';
  inspectTail.textContent = '';
  inspectPanel.style.display = 'flex';
  sessionList.classList.add('hidden');
  await refreshInspect();
}

async function inspectSessionFromHost(host, name) {
  closeTerminal();
  inspectSessionName_ = name;
  inspectSessionName.textContent = `${host} / ${name}`;
  inspectGitValue.textContent = 'Loading...';
  inspectLastValue.textContent = '';
  inspectRepoValue.textContent = '';
  inspectTail.textContent = '';
  inspectPanel.style.display = 'flex';
  sessionList.classList.add('hidden');
  try {
    const data = await dispatch({ type: 'inspect', host, session: name, lines: 0 });
    const git = data.git || {};
    const session = data.session || {};
    const ts = session.last_activity
      ? new Date(session.last_activity * 1000).toLocaleString()
      : 'unknown';
    inspectGitValue.textContent = `${git.branch || '?'} (${(git.head || '?').slice(0, 7)})${git.dirty ? ' *dirty' : ''}`;
    inspectLastValue.textContent = ts;
    inspectRepoValue.textContent = session.repository || '—';
    const pane = data.content?.pane || '1.1';
    try {
      const pc = await dispatch({ type: 'pane_content', host, session: name, pane, lines: 20 });
      inspectTail.textContent = pc.content || '(empty)';
    } catch {
      inspectTail.textContent = '(unable to fetch pane content)';
    }
  } catch (e) {
    inspectGitValue.textContent = 'Error';
    inspectTail.textContent = `Failed: ${e}`;
  }
}

async function refreshInspect() {
  if (!inspectSessionName_) return;
  try {
    const data = await dispatch({ type: 'inspect', session: inspectSessionName_, lines: 0 });
    const git = data.git || {};
    const session = data.session || {};
    const ts = session.last_activity
      ? new Date(session.last_activity * 1000).toLocaleString()
      : 'unknown';
    inspectGitValue.textContent = `${git.branch || '?'} (${(git.head || '?').slice(0, 7)})${git.dirty ? ' *dirty' : ''}`;
    inspectLastValue.textContent = ts;
    inspectRepoValue.textContent = session.repository || '—';
    // Fetch pane tail on demand
    const pane = data.content?.pane || '1.1';
    try {
      const pc = await dispatch({ type: 'pane_content', session: inspectSessionName_, pane, lines: 20 });
      inspectTail.textContent = pc.content || '(empty)';
    } catch {
      inspectTail.textContent = '(unable to fetch pane content)';
    }
  } catch (e) {
    inspectGitValue.textContent = 'Error';
    inspectLastValue.textContent = '';
    inspectRepoValue.textContent = '';
    inspectTail.textContent = `Failed: ${e}`;
  }
}

function closeInspect() {
  inspectPanel.style.display = 'none';
  sessionList.classList.remove('hidden');
  inspectSessionName_ = '';
}

// --- Actions ---

async function startSession(project) {
  try {
    if (IS_TAURI) {
      await dispatch({ type: 'start', project, layout: null });
    } else {
      const resp = await fetch(`${getApiBase()}/api/sessions/start?project=${encodeURIComponent(project)}`, { method: 'POST' });
      const data = await resp.json();
      if (data.error) { showToast(data.error, 'error'); return; }
    }
    showToast(`Started: ${project}`, 'ok');
    selectSession(project);
    await loadSessions();
  } catch (e) {
    showToast(`Failed to start: ${e}`, 'error');
  }
}

async function stopSession(session) {
  try {
    if (IS_TAURI) {
      await dispatch({ type: 'stop', session });
    } else {
      const resp = await fetch(`${getApiBase()}/api/sessions/stop?session=${encodeURIComponent(session)}`, { method: 'POST' });
      const data = await resp.json();
      if (data.error) { showToast(data.error, 'error'); return; }
    }
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
    let data;
    if (IS_TAURI) {
      // Try multi-host first; fall back to single-host
      const hosts = getHosts();
      if (hosts.length > 0) {
        const rendered = await renderGroupedSessions(sessionList);
        if (rendered) {
          lastFetchTime = Date.now();
          updateLastUpdated();
          if (daemonDown) hideDaemonBanner();
          return;
        }
      }
      data = await dispatch({ type: 'list_sessions' });
    } else {
      const url = `${getApiBase()}/api/sessions`;
      const start = performance.now();
      debugAppend('sessions', `GET ${url}`);
      const resp = await fetch(url);
      const ms = Math.round(performance.now() - start);
      debugAppend('sessions', `${resp.status} ${resp.statusText} (${ms}ms)`);
      if (resp.status === 503) {
        const body = await resp.json().catch(() => ({}));
        showDaemonBanner(body.detail || body.error || 'dev daemon unreachable');
        return;
      }
      if (!resp.ok) {
        const body = await resp.text().catch(() => '');
        debugAppend('sessions', `ERROR: ${resp.status} ${body}`);
        return;
      }
      data = await resp.json();
    }

    // Daemon recovered
    if (daemonDown) hideDaemonBanner();

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
    let detail = `${e.name}: ${e.message}`;
    if (e.message === 'Failed to fetch' || e.message === 'NetworkError when attempting to fetch resource.') {
      detail += ' [network failure — DNS, TLS cert not trusted, CORS, or not connected to Tailscale]';
    }
    debugAppend('sessions', `ERROR: ${detail}`);
  }
}

function updateLastUpdated() {
  if (!lastFetchTime) return;
  const secs = Math.floor((Date.now() - lastFetchTime) / 1000);
  lastUpdatedEl.textContent = secs < 2 ? 'just now' : `${secs}s ago`;
}

function scheduleRefresh() {
  if (refreshTimer) clearTimeout(refreshTimer);
  const interval = daemonDown ? DAEMON_RETRY_MS : settings.refreshInterval;
  refreshTimer = setTimeout(async () => {
    await loadSessions();
    scheduleRefresh();
  }, interval);
}

// --- Daemon banner ---

function showDaemonBanner(detail) {
  daemonDown = true;
  daemonBannerDetail.textContent = detail || 'check systemctl --user status dev-daemon';
  daemonBanner.classList.add('visible');
  sessionList.classList.add('daemon-down');
}

function hideDaemonBanner() {
  daemonDown = false;
  daemonBanner.classList.remove('visible');
  sessionList.classList.remove('daemon-down');
}

// --- Visibility ---

function handleVisibility() {
  if (document.visibilityState === 'visible') {
    loadSessions();
    scheduleRefresh();
  } else {
    if (refreshTimer) clearTimeout(refreshTimer);
  }
}

// --- Send keys to terminal ---

async function sendKeysToTerminal(text) {
  if (!selectedSession) {
    showToast('No session selected', 'error');
    return;
  }
  const pane = '1.1'; // default pane
  if (IS_TAURI) {
    try {
      const keys = text + (terminalSendEnter.checked ? '\n' : '');
      await dispatch({ type: 'send_keys', session: selectedSession, pane, keys });
      debugAppend('keys', `sent ${keys.length} chars via DevClient`);
    } catch (e) {
      debugAppend('keys', `ERROR: ${e}`);
      showToast(`Send keys failed: ${e}`, 'error');
    }
    return;
  }
  const url = `${getApiBase()}/api/sessions/${encodeURIComponent(selectedSession)}/panes/${pane}/keys`;
  debugAppend('keys', `POST ${url}`);
  try {
    const resp = await fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ keys: text }),
    });
    if (!resp.ok) {
      const body = await resp.text().catch(() => '');
      debugAppend('keys', `ERROR: ${resp.status} ${body}`);
      showToast(`Send keys failed: ${resp.status}`, 'error');
      return;
    }
    debugAppend('keys', `sent ${text.length} chars`);
  } catch (e) {
    debugAppend('keys', `ERROR: ${e}`);
    showToast(`Send keys failed: ${e.message}`, 'error');
  }
}

// --- Terminal mic (record → transcribe → review → paste into pane) ---

const terminalMic = document.getElementById('terminal-mic');
const terminalReviewPopup = document.getElementById('terminal-review-popup');
const terminalReviewText = document.getElementById('terminal-review-text');
const terminalSendEnter = document.getElementById('terminal-send-enter');
const terminalReviewSend = document.getElementById('terminal-review-send');
const terminalReviewCancel = document.getElementById('terminal-review-cancel');

let tRecording = false;
let tTranscribing = false;
let tAudioCtx = null;
let tMediaStream = null;
let tScriptNode = null;
let tSamples = [];

async function tStartRecording() {
  if (tRecording || tTranscribing) return;
  try {
    tMediaStream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, sampleRate: 16000, echoCancellation: true, noiseSuppression: true },
    });
  } catch (e) {
    showToast(`Mic access denied: ${e.message}`, 'error');
    return;
  }
  tAudioCtx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 16000 });
  const source = tAudioCtx.createMediaStreamSource(tMediaStream);
  tScriptNode = tAudioCtx.createScriptProcessor(4096, 1, 1);
  tSamples = [];
  tScriptNode.onaudioprocess = (e) => {
    if (tRecording) tSamples.push(new Float32Array(e.inputBuffer.getChannelData(0)));
  };
  source.connect(tScriptNode);
  tScriptNode.connect(tAudioCtx.destination);
  tRecording = true;
  terminalMic.classList.add('recording');
}

async function tStopRecording() {
  if (!tRecording) return;
  tRecording = false;
  terminalMic.classList.remove('recording');
  if (tScriptNode) { tScriptNode.disconnect(); tScriptNode = null; }
  if (tMediaStream) { tMediaStream.getTracks().forEach(t => t.stop()); tMediaStream = null; }
  if (tAudioCtx) { tAudioCtx.close().catch(() => {}); tAudioCtx = null; }

  const totalLength = tSamples.reduce((sum, c) => sum + c.length, 0);
  if (totalLength === 0 || totalLength / 16000 < 0.3) {
    showToast('Recording too short', 'warn');
    tSamples = [];
    return;
  }
  const allSamples = new Float32Array(totalLength);
  let offset = 0;
  for (const chunk of tSamples) { allSamples.set(chunk, offset); offset += chunk.length; }
  tSamples = [];

  if (!IS_TAURI || !tauriInvoke) {
    showToast('Voice requires the Tauri app', 'warn');
    return;
  }
  tTranscribing = true;
  terminalMic.classList.add('transcribing');
  try {
    const text = await tauriInvoke('transcribe_audio', { samples: Array.from(allSamples) });
    if (text && text.trim() && text.trim() !== '[BLANK_AUDIO]') {
      showTerminalReview(text.trim());
    } else {
      showToast('No speech detected', 'warn');
    }
  } catch (e) {
    showToast(`Transcription failed: ${e}`, 'error');
  } finally {
    tTranscribing = false;
    terminalMic.classList.remove('transcribing');
  }
}

function showTerminalReview(text) {
  terminalReviewText.value = text;
  terminalReviewPopup.classList.add('visible');
  terminalReviewText.focus();
  terminalReviewText.select();
}

function hideTerminalReview() {
  terminalReviewPopup.classList.remove('visible');
  terminalReviewText.value = '';
}

function sendTerminalReview() {
  let text = terminalReviewText.value.trim();
  hideTerminalReview();
  if (!text) return;
  if (terminalSendEnter.checked) text += '\n';
  sendKeysToTerminal(text);
}

// --- Init ---

export function initTerminal() {
  btnCloseTerminal.addEventListener('click', closeTerminal);
  btnCloseInspect.addEventListener('click', closeInspect);
  btnRefreshInspect.addEventListener('click', refreshInspect);
  daemonRefreshBtn.addEventListener('click', () => {
    loadSessions();
  });
  btnRefresh.addEventListener('click', () => {
    loadSessions();
  });

  // Host management
  btnAddHost.addEventListener('click', async () => {
    const host = hostInput.value.trim();
    if (!host) return;
    hostInput.value = '';

    // On Android, run SSH provisioning flow first
    const android = await isAndroid();
    if (android) {
      const ok = await provisionAndroidHost(host);
      if (!ok) return;
    } else {
      addHost(host);
    }

    loadSessions();
  });
  hostInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') btnAddHost.click();
  });

  // Custom events from sessions.js (multi-host)
  window.addEventListener('inspect-session', (e) => {
    const { host, name } = e.detail;
    inspectSessionFromHost(host, name);
  });
  window.addEventListener('open-terminal', (e) => {
    const { host, name } = e.detail;
    selectSession(name);
    openTerminal(name);
  });

  // Terminal mic
  terminalMic.addEventListener('mousedown', (e) => { e.preventDefault(); tStartRecording(); });
  terminalMic.addEventListener('mouseup', (e) => { e.preventDefault(); tStopRecording(); });
  terminalMic.addEventListener('mouseleave', () => { if (tRecording) tStopRecording(); });
  terminalMic.addEventListener('touchstart', (e) => { e.preventDefault(); tStartRecording(); });
  terminalMic.addEventListener('touchend', (e) => { e.preventDefault(); tStopRecording(); });
  terminalMic.addEventListener('touchcancel', () => { if (tRecording) tStopRecording(); });

  // Terminal review popup
  terminalReviewSend.addEventListener('click', sendTerminalReview);
  terminalReviewCancel.addEventListener('click', hideTerminalReview);
  terminalReviewText.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendTerminalReview(); }
    if (e.key === 'Escape') hideTerminalReview();
  });

  document.addEventListener('visibilitychange', handleVisibility);
  window.addEventListener('settings-changed', () => {
    if (refreshTimer) clearTimeout(refreshTimer);
    scheduleRefresh();
  });

  // Update "last updated" display every second
  setInterval(updateLastUpdated, 1000);

  loadSessions();
  scheduleRefresh();
}
