// commands.js — Conversation feed, send command, workflow display

import { IS_TAURI, tauriInvoke, escapeHtml, timeNow, showToast } from './app.js';
import { getSessionContext } from './terminal.js';
import { appendLog, createLogEntry } from './messages.js';

const conversationEl = document.getElementById('conversation');
const cmdEl = document.getElementById('cmd');
const sendBtn = document.getElementById('send');
let firstMessage = true;

// --- Conversation grouping ---

const conversationGroups = new Map(); // conversation_id → DOM element

function ensureReady() {
  if (firstMessage) {
    conversationEl.innerHTML = '';
    firstMessage = false;
  }
}

function getGroupEl(conversationId) {
  if (!conversationId) return null;
  return conversationGroups.get(conversationId) || null;
}

function createGroup(conversationId) {
  ensureReady();
  const group = document.createElement('div');
  group.className = 'conversation-group';
  group.dataset.convId = conversationId;
  conversationEl.appendChild(group);
  conversationGroups.set(conversationId, group);
  return group;
}

// --- Conversation feed ---

export function addMessage(type, content, label, conversationId) {
  ensureReady();
  const div = document.createElement('div');
  div.className = `msg msg-${type}`;
  const labelHtml = label ? `<span class="msg-label">${escapeHtml(label)}</span> ` : '';
  div.innerHTML = `<div class="msg-time">${timeNow()} ${labelHtml}</div><div class="msg-body">${content}</div>`;

  const parent = getGroupEl(conversationId) || conversationEl;
  parent.appendChild(div);
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

  // Generate conversation ID for message correlation
  const conversationId = `conv-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  createGroup(conversationId);

  // Show in conversation immediately
  addMessage('sent', escapeHtml(text), 'you', conversationId);
  appendLog(createLogEntry('sent', 'sent', text));

  if (IS_TAURI && tauriInvoke) {
    tauriInvoke('send_transcription', { text, conversationId }).catch(e => {
      addMessage('error', `Send failed: ${escapeHtml(String(e))}`, 'error', conversationId);
    });
  }

  cmdEl.value = '';
  cmdEl.focus();
}

// --- Workflow tracking ---

const activeWorkflows = {};

export function handleWorkflowEvent(event) {
  const id = event.workflow_id || event.workflow;
  const convId = event.conversation_id;

  if (event.type === 'step_start') {
    if (!activeWorkflows[id]) {
      activeWorkflows[id] = { name: event.workflow, steps: {}, done: false, el: null, convId };
    }
    activeWorkflows[id].steps[event.step] = 'running';
  } else if (event.type === 'step_complete') {
    if (!activeWorkflows[id]) {
      activeWorkflows[id] = { name: event.workflow, steps: {}, done: false, el: null, convId };
    }
    activeWorkflows[id].steps[event.step] = event.passed ? 'pass' : 'fail';
  } else if (event.type === 'workflow_complete') {
    if (!activeWorkflows[id]) {
      activeWorkflows[id] = { name: event.workflow, steps: {}, done: false, el: null, convId };
    }
    activeWorkflows[id].done = true;
    activeWorkflows[id].status = event.status;
    activeWorkflows[id].summary = event.summary;
    activeWorkflows[id].humanSummary = event.human_summary;
    setTimeout(() => { delete activeWorkflows[id]; }, 30000);
  }

  renderWorkflow(id);
}

function renderWorkflow(id) {
  const wf = activeWorkflows[id];
  if (!wf) return;

  if (!wf.el) {
    ensureReady();
    wf.el = document.createElement('div');
    wf.el.className = 'msg msg-workflow';
    const parent = getGroupEl(wf.convId) || conversationEl;
    parent.appendChild(wf.el);
  }

  let html = `<div class="msg-time">${timeNow()} <span class="msg-label">workflow</span></div>`;
  html += `<div class="msg-body">`;
  html += `<div class="wf-title">${escapeHtml(wf.name)}</div>`;
  for (const [step, state] of Object.entries(wf.steps)) {
    const icon = state === 'running' ? '...' : state === 'pass' ? '\u2713' : '\u2717';
    html += `<div class="workflow-step ${state}"><span class="step-icon">${icon}</span>${escapeHtml(step)}</div>`;
  }
  if (wf.done) {
    const display = wf.humanSummary || wf.summary;
    if (display) {
      html += `<div class="wf-summary ${wf.status || ''}">${escapeHtml(display)}</div>`;
    }
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
    'escalation',
    esc.conversation_id
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
    addMessage('response', `Running workflow: <strong>${escapeHtml(workflow)}</strong>`, 'intent', resp.conversation_id);
  } else if (resp.status === 'failed') {
    addMessage('error', escapeHtml(resp.error || 'Command not recognized'), 'intent', resp.conversation_id);
    showToast("Didn't understand that", 'warn');
  } else if (resp.status === 'escalated') {
    addMessage('escalation',
      `<strong>Rejected</strong> &mdash; ${escapeHtml(resp.error || 'Unknown')}`,
      'safety',
      resp.conversation_id
    );
    showToast(`Rejected: ${resp.error}`, 'error');
  }
}

// --- Clear / Copy ---

export function clearConversation() {
  conversationEl.innerHTML = '<div class="empty">No messages yet. Type a command or hold the mic to speak.</div>';
  firstMessage = true;
  conversationGroups.clear();
}

export function copyAllMessages() {
  const msgs = conversationEl.querySelectorAll('.msg');
  const lines = Array.from(msgs).map(m => m.textContent.trim());
  navigator.clipboard.writeText(lines.join('\n')).then(() => {
    showToast('Copied to clipboard', 'ok');
  });
}

// --- Init ---

export function initCommands() {
  sendBtn.addEventListener('click', () => sendCommand());
  cmdEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendCommand();
  });
}
