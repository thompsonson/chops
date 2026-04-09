// app.js — Main entry point, init, tab switching, settings, helpers

import { initMqtt } from './mqtt.js';
import { initCommands } from './commands.js';
import { initTerminal } from './terminal.js';
import { initVoice } from './voice.js';

// --- Tauri interop ---

export const IS_TAURI = window.__TAURI_INTERNALS__ !== undefined;
export let tauriInvoke = null;
export let tauriListen = null;

if (IS_TAURI) {
  tauriInvoke = window.__TAURI_INTERNALS__.invoke;
  tauriListen = async function(event, handler) {
    return window.__TAURI_INTERNALS__.invoke('plugin:event|listen', {
      event: event,
      target: { kind: 'Any' },
      handler: window.__TAURI_INTERNALS__.transformCallback(handler),
    });
  };
}

// --- Settings ---

const SETTINGS_KEY = 'chops-settings';
const DEFAULT_SETTINGS = {
  host: 'pop-mini.monkey-ladon.ts.net',
  wssPort: 9885,
  tcpPort: 1884,
  apiPort: 8443,
  ttydPort: 7681,
  updateChannel: 'stable',
};

export let settings = loadSettings();

function loadSettings() {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
  } catch {}
  return { ...DEFAULT_SETTINGS };
}

function saveSettings(s) {
  settings = s;
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
}

export function getHost() {
  return settings.host;
}
export function getApiBase() {
  return `https://${getHost()}:${settings.apiPort}`;
}
export function getTtydUrl() {
  return `https://${getHost()}:${settings.ttydPort}`;
}

// --- Helpers ---

export function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

export function timeNow() {
  return new Date().toLocaleTimeString('en-GB', { hour12: false });
}

// --- Toast ---

const toastsEl = document.getElementById('toasts');

export function showToast(message, level) {
  const el = document.createElement('div');
  el.className = `toast toast-${level || 'info'}`;
  el.textContent = message;
  while (toastsEl.children.length >= 5) {
    toastsEl.removeChild(toastsEl.firstChild);
  }
  toastsEl.appendChild(el);
  el.addEventListener('animationend', () => el.remove());
}

// --- Tab switching ---

function initTabs() {
  const tabs = document.querySelectorAll('.tab');
  const contents = document.querySelectorAll('.tab-content');

  tabs.forEach(tab => {
    tab.addEventListener('click', () => {
      const target = tab.dataset.tab;
      tabs.forEach(t => t.classList.remove('active'));
      contents.forEach(c => c.classList.remove('active'));
      tab.classList.add('active');
      document.getElementById(`tab-${target}`).classList.add('active');
    });
  });
}

// --- Settings modal ---

function initSettings() {
  const overlay = document.getElementById('settings-overlay');
  const btnOpen = document.getElementById('btn-settings');
  const btnSave = document.getElementById('settings-save');
  const btnCancel = document.getElementById('settings-cancel');
  const btnBrowse = document.getElementById('btn-browse-model');
  const btnCheckUpdate = document.getElementById('btn-check-update');
  const updateStatusEl = document.getElementById('update-status');

  const setHost = document.getElementById('set-host');
  const setWssPort = document.getElementById('set-wss-port');
  const setTcpPort = document.getElementById('set-tcp-port');
  const setApiPort = document.getElementById('set-api-port');
  const setTtydPort = document.getElementById('set-ttyd-port');
  const setModelPath = document.getElementById('set-model-path');
  const setUpdateChannel = document.getElementById('set-update-channel');

  async function open() {
    setHost.value = settings.host;
    setWssPort.value = settings.wssPort;
    setTcpPort.value = settings.tcpPort;
    setApiPort.value = settings.apiPort;
    setTtydPort.value = settings.ttydPort;
    setUpdateChannel.value = settings.updateChannel || 'stable';
    updateStatusEl.textContent = '';
    if (IS_TAURI && tauriInvoke) {
      try { setModelPath.value = await tauriInvoke('get_model_path'); } catch {}
    }
    overlay.classList.add('visible');
  }

  function close() {
    overlay.classList.remove('visible');
  }

  function apply() {
    const newSettings = {
      host: setHost.value.trim() || DEFAULT_SETTINGS.host,
      wssPort: parseInt(setWssPort.value) || DEFAULT_SETTINGS.wssPort,
      tcpPort: parseInt(setTcpPort.value) || DEFAULT_SETTINGS.tcpPort,
      apiPort: parseInt(setApiPort.value) || DEFAULT_SETTINGS.apiPort,
      ttydPort: parseInt(setTtydPort.value) || DEFAULT_SETTINGS.ttydPort,
      updateChannel: setUpdateChannel.value || 'stable',
    };
    saveSettings(newSettings);
    close();

    if (IS_TAURI && tauriInvoke) {
      tauriInvoke('connect_mqtt', { host: newSettings.host, port: newSettings.tcpPort })
        .then(() => showToast('MQTT reconnected', 'ok'))
        .catch(e => showToast(`MQTT reconnect failed: ${e}`, 'error'));

      const modelPath = setModelPath.value.trim();
      if (modelPath && !modelPath.startsWith('content://')) {
        tauriInvoke('set_model_path', { path: modelPath }).catch(() => {});
      }
    }
  }

  btnOpen.addEventListener('click', open);
  btnCancel.addEventListener('click', close);
  btnSave.addEventListener('click', apply);
  overlay.addEventListener('click', (e) => { if (e.target === overlay) close(); });

  btnBrowse.addEventListener('click', async () => {
    if (!IS_TAURI || !tauriInvoke) return;
    try {
      const selected = await tauriInvoke('plugin:dialog|open', {
        options: { title: 'Select Whisper Model', multiple: false },
      });
      if (!selected) return;
      const uri = typeof selected === 'string' ? selected : selected.path;
      if (uri.startsWith('content://')) {
        showToast('Importing model file...', 'info');
        const dest = await tauriInvoke('import_model', { uri });
        setModelPath.value = dest;
        showToast('Model imported', 'ok');
        document.getElementById('model-banner').classList.remove('visible');
      } else {
        setModelPath.value = uri;
      }
    } catch (e) {
      showToast(`Browse failed: ${e}`, 'error');
    }
  });

  btnCheckUpdate.addEventListener('click', async () => {
    if (!IS_TAURI || !tauriInvoke) {
      updateStatusEl.textContent = 'Updates require the desktop app.';
      return;
    }
    btnCheckUpdate.disabled = true;
    updateStatusEl.textContent = 'Checking...';
    try {
      const channel = setUpdateChannel.value || settings.updateChannel || 'stable';
      const result = await tauriInvoke('check_for_update', { channel });
      if (result.available) {
        updateStatusEl.textContent = `v${result.version} available!`;
        if (confirm(`Update to v${result.version}?\n\n${result.body || 'No release notes.'}`)) {
          updateStatusEl.textContent = 'Downloading...';
          await tauriInvoke('install_update', { channel });
        }
      } else {
        updateStatusEl.textContent = 'You are up to date.';
      }
    } catch (e) {
      updateStatusEl.textContent = `Error: ${e}`;
    }
    btnCheckUpdate.disabled = false;
  });
}

// --- Status check ---

async function checkStatus() {
  if (!IS_TAURI || !tauriInvoke) return;
  try {
    const status = await tauriInvoke('get_status');
    if (status.model_exists) {
      document.getElementById('whisper-dot').classList.add('ok');
      document.getElementById('whisper-text').textContent = 'whisper (ready)';
    } else {
      document.getElementById('model-banner').classList.add('visible');
      document.getElementById('model-path').textContent = ` Path: ${status.model_path}`;
    }
    if (status.mqtt_connected) {
      document.getElementById('mqtt-dot').classList.add('ok');
    }
  } catch {}
}

// --- Update progress listener ---

async function initUpdateListener() {
  if (!IS_TAURI || !tauriListen) return;
  await tauriListen('update-progress', (event) => {
    const { downloaded, total } = event.payload;
    const el = document.getElementById('update-status');
    if (total) {
      el.textContent = `Downloading... ${Math.round((downloaded / total) * 100)}%`;
    } else {
      el.textContent = `Downloading... ${(downloaded / 1024 / 1024).toFixed(1)} MB`;
    }
  });
}

async function silentUpdateCheck() {
  if (!IS_TAURI || !tauriInvoke) return;
  try {
    const channel = settings.updateChannel || 'stable';
    const result = await tauriInvoke('check_for_update', { channel });
    if (result.available) {
      showToast(`Update available: v${result.version} (${channel})`, 'info');
    }
  } catch {}
}

// --- Init ---

initTabs();
initSettings();
initMqtt();
initCommands();
initTerminal();
initVoice();

if (IS_TAURI) {
  checkStatus();
  initUpdateListener();
  tauriInvoke('connect_mqtt', { host: settings.host, port: settings.tcpPort }).catch(() => {});
  setTimeout(silentUpdateCheck, 5000);
}
