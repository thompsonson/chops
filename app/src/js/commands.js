// commands.js — Conversation feed, send command, workflow display

import { IS_TAURI, tauriInvoke, escapeHtml, timeNow, showToast } from './app.js';
import { getSessionContext } from './terminal.js';

const conversationEl = document.getElementById('conversation');
const cmdEl = document.getElementById('cmd');
const sendBtn = document.getElementById('send');
let firstMessage = true;

// --- Conversation feed ---

function addMessage(type, content) {
  if (firstMessage) {
    conversationEl.innerHTML = '';
    firstMessage = false;
  }
  const div = document.createElement('div');
  div.className = `msg msg-${type}`;
  div.innerHTML = `<div class="msg-time">${timeNow()}</div>${content}`;
  conversationEl.appendChild(div);
  conversationEl.scrollTop = conversationEl.scrollHeight;
  return div;
}

// --- Send command ---

function sendCommand() {
  let text = cmdEl.value.trim();
  if (!text) return;

  // Auto-prepend session context
  const session = getSessionContext();
  const lower = text.toLowerCase();
  if (session && !session.startsWith('project:')
      && !lower.startsWith('in ')
      && !lower.includes('vscode') && !lower.includes('editor')
      && !lower.includes('termux') && !lower.includes('terminal')) {
    text = `in ${session} ${text}`;
  }

  // Auto-append terminator for "tell claude" so it flushes immediately
  if (/tell claude/i.test(text) && !/\b(over|done|send it|finish|end)\s*$/i.test(text)) {
    text += ' over';
  }

  if (IS_TAURI && tauriInvoke) {
    tauriInvoke('send_transcription', { text }).then(() => {
      addMessage('sent', escapeHtml(text));
    }).catch(e => {
      addMessage('error', `Send failed: ${escapeHtml(String(e))}`);
    });
  }

  cmdEl.value = '';
  cmdEl.focus();
}

// --- Workflow tracking ---

const activeWorkflows = {};

export function handleWorkflowEvent(event) {
  const id = event.workflow_id || event.workflow;

  if (event.type === 'step_start') {
    if (!activeWorkflows[id]) {
      activeWorkflows[id] = { name: event.workflow, steps: {}, done: false, el: null };
    }
    activeWorkflows[id].steps[event.step] = 'running';
  } else if (event.type === 'step_complete') {
    if (activeWorkflows[id]) {
      activeWorkflows[id].steps[event.step] = event.passed ? 'pass' : 'fail';
    }
  } else if (event.type === 'workflow_complete') {
    if (activeWorkflows[id]) {
      activeWorkflows[id].done = true;
      activeWorkflows[id].status = event.status;
      activeWorkflows[id].summary = event.summary;
    }
    setTimeout(() => { delete activeWorkflows[id]; }, 30000);
  }

  renderWorkflow(id);
}

function renderWorkflow(id) {
  const wf = activeWorkflows[id];
  if (!wf) return;

  // Create or update the workflow message in conversation
  if (!wf.el) {
    if (firstMessage) {
      conversationEl.innerHTML = '';
      firstMessage = false;
    }
    wf.el = document.createElement('div');
    wf.el.className = 'msg msg-workflow';
    conversationEl.appendChild(wf.el);
  }

  let html = `<div class="msg-time">${timeNow()}</div>`;
  html += `<div class="wf-title">${escapeHtml(wf.name)}</div>`;
  for (const [step, state] of Object.entries(wf.steps)) {
    const icon = state === 'running' ? '...' : state === 'pass' ? '\u2713' : '\u2717';
    html += `<div class="workflow-step ${state}"><span class="step-icon">${icon}</span>${escapeHtml(step)}</div>`;
  }
  if (wf.done && wf.summary) {
    html += `<div class="wf-summary ${wf.status || ''}">${escapeHtml(wf.summary)}</div>`;
  }
  wf.el.innerHTML = html;
  conversationEl.scrollTop = conversationEl.scrollHeight;
}

// --- Escalation ---

export function handleEscalation(esc) {
  const text = `ESCALATION: ${esc.workflow}/${esc.step} \u2014 ${esc.feedback || esc.reason || 'Unknown'}`;
  addMessage('escalation', escapeHtml(text));
  showToast(text, 'error');
}

// --- Response handling ---

export function handleResponse(msg) {
  if (msg.type === 'toast') {
    // Toast-only messages handled in mqtt.js
    return;
  }
  const content = msg.message || msg.output || msg.error || JSON.stringify(msg);
  const isError = msg.level === 'error' || msg.status === 'error';
  addMessage(isError ? 'error' : 'response', escapeHtml(content));
}

// --- Intent response ---

export function handleIntentResponse(resp) {
  if (resp.status === 'success') {
    showToast(`Running ${resp.intent?.workflow || 'workflow'}...`, 'info');
    addMessage('response', `Running: ${escapeHtml(resp.intent?.workflow || '?')}`);
  } else if (resp.status === 'failed') {
    showToast("Didn't understand that", 'warn');
    addMessage('error', escapeHtml(resp.error || 'Command not recognized'));
  } else if (resp.status === 'escalated') {
    showToast(`Rejected: ${resp.error}`, 'error');
    addMessage('escalation', `Rejected: ${escapeHtml(resp.error || 'Unknown')}`);
  }
}

// --- Public: add message from other modules ---

export function addSentMessage(text) {
  addMessage('sent', escapeHtml(text));
}

// --- Init ---

export function initCommands() {
  sendBtn.addEventListener('click', sendCommand);
  cmdEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendCommand();
  });
}
