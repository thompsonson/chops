// mqtt.js — Tauri MQTT event handling and message dispatch

import { IS_TAURI, tauriListen, showToast } from './app.js';
import { handleResponse, handleWorkflowEvent, handleEscalation, handleIntentResponse } from './commands.js';

const TOPIC_RESPONSES = 'agent/responses';
const TOPIC_WORKFLOW_EVENTS = 'agent/workflow/events';
const TOPIC_WORKFLOW_ESCALATION = 'agent/workflow/escalation';
const TOPIC_INTENT_RESPONSE = 'agent/intent/response';

function handleMqttMessage(topic, text) {
  if (topic === TOPIC_RESPONSES || topic === 'agent/responses') {
    try {
      const msg = JSON.parse(text);
      if (msg.type === 'toast') {
        showToast(msg.message, msg.level);
      }
      handleResponse(msg);
    } catch {
      handleResponse({ message: text, source: topic });
    }
  } else if (topic === TOPIC_WORKFLOW_EVENTS) {
    try {
      handleWorkflowEvent(JSON.parse(text));
    } catch {}
  } else if (topic === TOPIC_WORKFLOW_ESCALATION) {
    try {
      handleEscalation(JSON.parse(text));
    } catch {}
  } else if (topic === TOPIC_INTENT_RESPONSE) {
    try {
      handleIntentResponse(JSON.parse(text));
    } catch {}
  }
}

export async function initMqtt() {
  if (!IS_TAURI || !tauriListen) return;

  await tauriListen('mqtt-message', (event) => {
    const { topic, payload } = event.payload;
    handleMqttMessage(topic, payload);
  });

  await tauriListen('mqtt-status', (event) => {
    const status = event.payload;
    const dot = document.getElementById('mqtt-dot');
    const text = document.getElementById('mqtt-text');
    const sendBtn = document.getElementById('send');
    if (status === 'connected') {
      dot.classList.add('ok');
      text.textContent = 'mqtt';
      sendBtn.disabled = false;
    } else {
      dot.classList.remove('ok');
      text.textContent = `mqtt (${status})`;
      sendBtn.disabled = true;
    }
  });

  await tauriListen('stt-status', (event) => {
    const status = event.payload;
    const dot = document.getElementById('whisper-dot');
    const text = document.getElementById('whisper-text');
    if (status === 'listening') {
      dot.classList.add('ok');
      text.textContent = 'whisper (listening)';
    } else if (status === 'stopped') {
      dot.classList.remove('ok');
      text.textContent = 'whisper';
    } else if (status === 'model_loaded') {
      dot.classList.add('ok');
      text.textContent = 'whisper (ready)';
    }
  });
}
