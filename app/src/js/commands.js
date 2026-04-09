// commands.js — Conversation feed, send command, workflow display

import { IS_TAURI, tauriInvoke, escapeHtml, timeNow, showToast } from './app.js';
import { getSessionContext } from './terminal.js';
import { appendLog, createLogEntry } from './messages.js';

const conversationEl = document.getElementById('conversation');
const cmdEl = document.getElementById('cmd');
const sendBtn = document.getElementById('send');
let firstMessage = true;

// --- Conversation feed ---

function addMessage(type, content, label) {
  if (firstMessage) {
    conversationEl.innerHTML = '';
    firstMessage = false;
  }
  const div = document.createElement('div');
  div.className = `msg msg-${type}`;
  const labelHtml = label ? `<span class="msg-label">${escapeHtml(label)}</span> ` : '';
  div.innerHTML = `<div class="msg-time">${timeNow()} ${labelHtml}</div><div class="msg-body">${content}</div>`;
  conversationEl.appendChild(div);
  conversationEl.scrollTop = conversationEl.scrollHeight;
  return div;
}

// --- Send command (exported for voice.js review popup) ---

export function sendCommand(overrideText) {
  let text = overrideText || cmdEl.value.trim();
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

  // Show in conversation immediately
  addMessage('sent', escapeHtml(text), 'you');
  appendLog(createLogEntry('sent', 'sent', text));

  if (IS_TAURI && tauriInvoke) {
    tauriInvoke('send_transcription', { text }).catch(e => {
      addMessage('error', `Send failed: ${escapeHtml(String(e))}`, 'error');
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

  if (!wf.el) {
    if (firstMessage) {
      conversationEl.innerHTML = '';
      firstMessage = false;
    }
    wf.el = document.createElement('div');
    wf.el.className = 'msg msg-workflow';
    conversationEl.appendChild(wf.el);
  }

  let html = `<div class="msg-time">${timeNow()} <span class="msg-label">workflow</span></div>`;
  html += `<div class="msg-body">`;
  html += `<div class="wf-title">${escapeHtml(wf.name)}</div>`;
  for (const [step, state] of Object.entries(wf.steps)) {
    const icon = state === 'running' ? '...' : state === 'pass' ? '\u2713' : '\u2717';
    html += `<div class="workflow-step ${state}"><span class="step-icon">${icon}</span>${escapeHtml(step)}</div>`;
  }
  if (wf.done && wf.summary) {
    html += `<div class="wf-summary ${wf.status || ''}">${escapeHtml(wf.summary)}</div>`;
  }
  html += `</div>`;
  wf.el.innerHTML = html;
  conversationEl.scrollTop = conversationEl.scrollHeight;
}

// --- Escalation ---

export function handleEscalation(esc) {
  const feedback = esc.feedback || esc.reason || 'Unknown';
  addMessage('escalation',
    `<strong>${escapeHtml(esc.workflow)}/${escapeHtml(esc.step)}</strong> &mdash; ${escapeHtml(feedback)}`,
    'escalation'
  );
  showToast(`ESCALATION: ${esc.workflow}/${esc.step} — ${feedback}`, 'error');
}

// --- Response handling ---

export function handleResponse(msg) {
  if (msg.type === 'toast') {
    return;
  }

  // Legacy plugin-runner responses
  if (msg.status === 'ok') {
    const output = msg.output || 'ok';
    const source = (msg.topic || 'agent').replace('agent/commands/', '');
    addMessage('response', escapeHtml(output), source);
    return;
  }
  if (msg.status === 'error' || msg.status === 'fail') {
    const output = msg.error || msg.output || 'error';
    const source = (msg.topic || 'agent').replace('agent/commands/', '');
    addMessage('error', escapeHtml(output), source);
    return;
  }

  // Generic responses
  const content = msg.message || msg.output || msg.error || JSON.stringify(msg);
  const isError = msg.level === 'error';
  const source = msg.source || 'agent';
  addMessage(isError ? 'error' : 'response', escapeHtml(content), source);
}

// --- Intent response ---

export function handleIntentResponse(resp) {
  if (resp.status === 'success') {
    const workflow = resp.intent?.workflow || '?';
    addMessage('response', `Running workflow: <strong>${escapeHtml(workflow)}</strong>`, 'intent');
  } else if (resp.status === 'failed') {
    addMessage('error', escapeHtml(resp.error || 'Command not recognized'), 'intent');
    showToast("Didn't understand that", 'warn');
  } else if (resp.status === 'escalated') {
    addMessage('escalation',
      `<strong>Rejected</strong> &mdash; ${escapeHtml(resp.error || 'Unknown')}`,
      'safety'
    );
    showToast(`Rejected: ${resp.error}`, 'error');
  }
}

// --- Init ---

export function initCommands() {
  sendBtn.addEventListener('click', () => sendCommand());
  cmdEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendCommand();
  });
}
