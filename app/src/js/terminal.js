// terminal.js — Session management and ttyd terminal

import { IS_TAURI, tauriInvoke, getApiBase, getTtydUrl, showToast } from './app.js';

const sessionSelect = document.getElementById('session-select');
const btnStart = document.getElementById('btn-start');
const btnStop = document.getElementById('btn-stop');
const terminalIframe = document.getElementById('terminal-iframe');
let currentSession = '';

export function getSessionContext() {
  return sessionSelect.value;
}

function switchSession(name) {
  currentSession = name;
  terminalIframe.src = `${getTtydUrl()}?t=${Date.now()}`;
}

function updateButtons() {
  const val = sessionSelect.value;
  const isProject = val.startsWith('project:');
  const isSession = val && !isProject;
  btnStart.disabled = !isProject;
  btnStop.disabled = !isSession;
}

async function loadSessions() {
  try {
    let data;
    if (IS_TAURI && tauriInvoke) {
      const raw = await tauriInvoke('get_sessions');
      data = JSON.parse(raw);
    } else {
      const resp = await fetch(`${getApiBase()}/api/sessions`);
      data = await resp.json();
    }
    sessionSelect.innerHTML = '';

    if (data.sessions && data.sessions.length > 0) {
      const group = document.createElement('optgroup');
      group.label = 'Active Sessions';
      data.sessions.forEach(s => {
        const opt = document.createElement('option');
        opt.value = s.name;
        const status = s.attached ? ' (attached)' : '';
        const layout = s.layout === 'claude' ? ' [claude+shell]' : '';
        opt.textContent = `${s.name}${layout}${status}`;
        group.appendChild(opt);
      });
      sessionSelect.appendChild(group);
    }

    if (data.projects && data.projects.length > 0) {
      const group = document.createElement('optgroup');
      group.label = 'Available Projects';
      data.projects.forEach(p => {
        const opt = document.createElement('option');
        opt.value = `project:${p.name}`;
        opt.textContent = `${p.name} (not started)`;
        group.appendChild(opt);
      });
      sessionSelect.appendChild(group);
    }

    if (data.sessions && data.sessions.length > 0) {
      const prev = currentSession;
      const stillExists = data.sessions.some(s => s.name === prev);
      if (stillExists) {
        sessionSelect.value = prev;
      } else {
        sessionSelect.value = data.sessions[0].name;
        switchSession(data.sessions[0].name);
      }
    }
    updateButtons();
  } catch {
    sessionSelect.innerHTML = '<option value="">Failed to load sessions</option>';
  }
}

export function initTerminal() {
  // Load ttyd iframe on init
  terminalIframe.src = getTtydUrl();

  sessionSelect.addEventListener('change', async () => {
    updateButtons();
    const val = sessionSelect.value;
    if (!val.startsWith('project:') && val) {
      try {
        if (!IS_TAURI) {
          await fetch(`${getApiBase()}/api/sessions/switch?session=${encodeURIComponent(val)}`, { method: 'POST' });
        }
        switchSession(val);
      } catch (e) {
        showToast(`Failed to switch: ${e}`, 'error');
      }
    }
  });

  btnStart.addEventListener('click', async () => {
    const val = sessionSelect.value;
    if (!val.startsWith('project:')) return;
    const project = val.slice(8);
    btnStart.disabled = true;
    btnStart.textContent = '...';
    try {
      if (IS_TAURI && tauriInvoke) {
        await tauriInvoke('start_session', { project });
      } else {
        const resp = await fetch(`${getApiBase()}/api/sessions/start?project=${encodeURIComponent(project)}`, { method: 'POST' });
        const data = await resp.json();
        if (data.error) {
          showToast(data.error, 'error');
          btnStart.textContent = 'Start';
          return;
        }
        await fetch(`${getApiBase()}/api/sessions/switch?session=${encodeURIComponent(project)}`, { method: 'POST' });
      }
      showToast(`Started: ${project}`, 'ok');
      switchSession(project);
      await loadSessions();
      sessionSelect.value = project;
      updateButtons();
    } catch (e) {
      showToast(`Failed to start: ${e}`, 'error');
    }
    btnStart.textContent = 'Start';
  });

  btnStop.addEventListener('click', async () => {
    const session = sessionSelect.value;
    if (!session || session.startsWith('project:')) return;
    btnStop.disabled = true;
    btnStop.textContent = '...';
    try {
      if (IS_TAURI && tauriInvoke) {
        await tauriInvoke('stop_session', { session });
      } else {
        const resp = await fetch(`${getApiBase()}/api/sessions/stop?session=${encodeURIComponent(session)}`, { method: 'POST' });
        const data = await resp.json();
        if (data.error) {
          showToast(data.error, 'error');
          btnStop.textContent = 'Stop';
          return;
        }
      }
      showToast(`Stopped: ${session}`, 'ok');
      await loadSessions();
      updateButtons();
    } catch (e) {
      showToast(`Failed to stop: ${e}`, 'error');
    }
    btnStop.textContent = 'Stop';
  });

  loadSessions();
  setInterval(loadSessions, 30000);
}
