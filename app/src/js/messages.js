// messages.js — Raw MQTT message log for debugging

import { escapeHtml, timeNow, showToast } from './app.js';

const logEl = document.getElementById('log');
let firstMessage = true;

export function createLogEntry(type, topic, content) {
  const div = document.createElement('div');
  div.className = `log-entry ${type}`;
  const shortTopic = topic
    .replace('agent/commands/', '')
    .replace('agent/', '')
    .replace('plugins/status/', 'status/');
  div.innerHTML = `<span class="time">${timeNow()}</span><span class="badge">[${type}]</span> <span class="topic">${shortTopic}</span> ${escapeHtml(content)}`;
  return div;
}

export function appendLog(entry) {
  if (firstMessage) {
    logEl.innerHTML = '';
    firstMessage = false;
  }
  logEl.appendChild(entry);
  logEl.scrollTop = logEl.scrollHeight;
}

export function clearLog() {
  logEl.innerHTML = '<div class="empty">No messages yet.</div>';
  firstMessage = true;
}

export function copyAllLog() {
  const entries = logEl.querySelectorAll('.log-entry');
  const lines = Array.from(entries).map(e => e.textContent.trim());
  navigator.clipboard.writeText(lines.join('\n')).then(() => {
    showToast('Copied to clipboard', 'ok');
  });
}
