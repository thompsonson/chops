// mqtt.js — Tauri MQTT event handling and message dispatch

import { IS_TAURI, tauriInvoke, tauriListen, showToast } from './app.js';
import { addMessage, handleResponse, handleWorkflowEvent, handleEscalation, handleIntentResponse } from './commands.js';
import { appendLog, createLogEntry } from './messages.js';
import { escapeHtml } from './app.js';

const TOPIC_RESPONSES = 'agent/responses';
const TOPIC_WORKFLOW_EVENTS = 'agent/workflow/events';
const TOPIC_WORKFLOW_ESCALATION = 'agent/workflow/escalation';
const TOPIC_INTENT_RESPONSE = 'agent/intent/response';
const TOPIC_PING = 'agent/ping';

let pendingPingTs = null;

function handleMqttMessage(topic, text) {
  let logType = 'ok';

  if (topic === TOPIC_PING) {
    try {
      const msg = JSON.parse(text);
      const sentTs = msg.ping;
      if (sentTs && sentTs === pendingPingTs) {
        const rtt = Date.now() - sentTs;
        addMessage('response', `Pong received (${rtt}ms round-trip)`, 'mqtt');
        pendingPingTs = null;
      }
    } catch {}
  } else if (topic === TOPIC_RESPONSES || topic === 'agent/responses') {
    try {
      const msg = JSON.parse(text);
      if (msg.type === 'toast') {
        showToast(msg.message, msg.level);
      }
      if (msg.level === 'error') logType = 'error';
      handleResponse(msg);
    } catch {
      handleResponse({ message: text, source: topic });
    }
  } else if (topic === TOPIC_WORKFLOW_EVENTS) {
    try {
      handleWorkflowEvent(JSON.parse(text));
    } catch {}
  } else if (topic === TOPIC_WORKFLOW_ESCALATION) {
    logType = 'error';
    try {
      handleEscalation(JSON.parse(text));
    } catch {}
  } else if (topic === TOPIC_INTENT_RESPONSE) {
    try {
      const resp = JSON.parse(text);
      if (resp.status === 'failed' || resp.status === 'escalated') logType = 'error';
      handleIntentResponse(resp);
    } catch {}
  }

  appendLog(createLogEntry(logType, topic, text.substring(0, 500)));
}

export async function pingMqtt() {
  if (!IS_TAURI || !tauriInvoke) return;
  try {
    const ts = await tauriInvoke('mqtt_ping');
    pendingPingTs = ts;
    addMessage('sent', 'Ping sent...', 'mqtt');
    // Timeout after 5s
    setTimeout(() => {
      if (pendingPingTs === ts) {
        addMessage('error', 'Ping timeout (no response after 5s)', 'mqtt');
        pendingPingTs = null;
      }
    }, 5000);
  } catch (e) {
    addMessage('error', `Ping failed: ${escapeHtml(String(e))}`, 'mqtt');
  }
}

export async function initMqtt() {
  if (!IS_TAURI || !tauriListen) return;

  // MQTT messages from Rust subscription
  await tauriListen('mqtt-message', (event) => {
    const { topic, payload } = event.payload;
    handleMqttMessage(topic, payload);
  });

  // MQTT connection status
  await tauriListen('mqtt-status', (event) => {
    const status = event.payload;
    const dot = document.getElementById('mqtt-dot');
    const text = document.getElementById('mqtt-text');
    const sendBtn = document.getElementById('send');
    const pingBtn = document.getElementById('btn-ping');
    if (status === 'connected') {
      dot.classList.add('ok');
      text.textContent = 'mqtt';
      sendBtn.disabled = false;
      if (pingBtn) pingBtn.disabled = false;
    } else {
      dot.classList.remove('ok');
      text.textContent = `mqtt (${status})`;
      sendBtn.disabled = true;
      if (pingBtn) pingBtn.disabled = true;
    }
  });
}
