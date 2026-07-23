// debug.js — Debug tab: config display, API testing, network log

import { IS_TAURI, tauriInvoke, getApiBase, getTtydUrl, getHost, settings } from './app.js';

const debugConfig = document.getElementById('debug-config');
const debugApiResult = document.getElementById('debug-api-result');
const btnDebugSessions = document.getElementById('btn-debug-sessions');
const btnDebugMqtt = document.getElementById('btn-debug-mqtt');
const btnExportLogs = document.getElementById('btn-export-logs');
const btnClearLogs = document.getElementById('btn-clear-logs');
const debugLogsOutput = document.getElementById('debug-logs-output');

let logLines = [];
const MAX_LOG_LINES = 200;

// --- Public: log from anywhere ---

export function debugAppend(tag, message) {
  const ts = new Date().toLocaleTimeString('en-GB', { hour12: false });
  const line = `[${ts}] [${tag}] ${message}`;
  logLines.push(line);
  if (logLines.length > MAX_LOG_LINES) logLines.shift();
  const el = document.getElementById('debug-log');
  if (el) el.textContent = logLines.join('\n');
}

// --- Config display ---

function renderConfig() {
  const info = {
    isTauri: IS_TAURI,
    host: getHost(),
    apiBase: getApiBase(),
    ttydUrl: getTtydUrl(),
    settings: settings,
    userAgent: navigator.userAgent,
    protocol: window.location.protocol,
    origin: window.location.origin,
  };
  debugConfig.textContent = JSON.stringify(info, null, 2);
}

// --- API test ---

async function testSessions() {
  const url = `${getApiBase()}/api/sessions`;
  debugApiResult.textContent = `Fetching ${url}...\n`;
  debugAppend('api', `GET ${url}`);

  try {
    const resp = await fetch(url);
    const status = `${resp.status} ${resp.statusText}`;
    const body = await resp.text();
    debugApiResult.textContent = `${url}\nStatus: ${status}\n\n${body}`;
    debugAppend('api', `${status} (${body.length} bytes)`);
  } catch (e) {
    const msg = `${url}\nERROR: ${e}\n\nThis usually means:\n- CORS blocked (check browser console)\n- TLS cert not trusted\n- Server unreachable\n- Wrong host/port in settings`;
    debugApiResult.textContent = msg;
    debugAppend('api', `FAILED: ${e}`);
  }
}

async function testMqtt() {
  const wsUrl = `wss://${getHost()}:${settings.wssPort}/mqtt`;
  debugApiResult.textContent = `Testing WebSocket: ${wsUrl}...\n`;
  debugAppend('mqtt', `Connecting ${wsUrl}`);

  try {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      ws.onopen = () => { resolve(); };
      ws.onerror = (e) => { reject(new Error('WebSocket connection failed')); };
      setTimeout(() => reject(new Error('Timeout after 5s')), 5000);
    });
    debugApiResult.textContent = `${wsUrl}\nStatus: CONNECTED\n\nWebSocket opened successfully.`;
    debugAppend('mqtt', 'Connected OK');
    ws.close();
  } catch (e) {
    debugApiResult.textContent = `${wsUrl}\nERROR: ${e}\n\nThis usually means:\n- WSS port wrong\n- TLS cert not trusted for this host\n- Mosquitto not running`;
    debugAppend('mqtt', `FAILED: ${e}`);
  }
}

// --- Init ---

export function initDebug() {
  renderConfig();
  btnDebugSessions.addEventListener('click', testSessions);
  btnDebugMqtt.addEventListener('click', testMqtt);
  debugAppend('init', `Debug tab ready. IS_TAURI=${IS_TAURI}`);

  // Log export/clear (Tauri only)
  if (IS_TAURI) {
    btnExportLogs.addEventListener('click', async () => {
      try {
        const logs = await tauriInvoke('get_logs');
        debugLogsOutput.textContent = logs || '(empty)';
        await navigator.clipboard.writeText(logs);
        debugAppend('logs', 'Copied to clipboard');
      } catch (e) {
        debugLogsOutput.textContent = `Error: ${e}`;
      }
    });

    btnClearLogs.addEventListener('click', async () => {
      try {
        await tauriInvoke('clear_logs');
        debugLogsOutput.textContent = '(cleared)';
        debugAppend('logs', 'Cleared');
      } catch (e) {
        debugLogsOutput.textContent = `Error: ${e}`;
      }
    });
  } else {
    btnExportLogs.disabled = true;
    btnClearLogs.disabled = true;
    btnExportLogs.title = 'Logs require Tauri';
    btnClearLogs.title = 'Logs require Tauri';
  }
}
