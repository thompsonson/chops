// messages.js — Raw MQTT message log for debugging

import { escapeHtml, timeNow } from './app.js';

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
