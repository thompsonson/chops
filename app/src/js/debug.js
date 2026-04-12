// debug.js — Debug tab: config display, API testing, network log

import { IS_TAURI, getApiBase, getTtydUrl, getHost, settings } from './app.js';

const debugConfig = document.getElementById('debug-config');
const debugApiResult = document.getElementById('debug-api-result');
const debugLog = document.getElementById('debug-log');
const btnDebugSessions = document.getElementById('btn-debug-sessions');
const btnDebugMqtt = document.getElementById('btn-debug-mqtt');

let logLines = [];
const MAX_LOG_LINES = 200;

// --- Public: log from anywhere ---

export function debugAppend(tag, message) {
  const ts = new Date().toLocaleTimeString('en-GB', { hour12: false });
  const line = `[${ts}] [${tag}] ${message}`;
  logLines.push(line);
  if (logLines.length > MAX_LOG_LINES) logLines.shift();
  if (debugLog) debugLog.textContent = logLines.join('\n');
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
}
