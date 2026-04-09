// voice.js — Push-to-talk: Web Audio capture -> Tauri -> whisper-rs -> review popup

import { IS_TAURI, tauriInvoke, showToast } from './app.js';
import { sendCommand } from './commands.js';

const micBtn = document.getElementById('mic');
const cmdEl = document.getElementById('cmd');
const reviewPopup = document.getElementById('review-popup');
const reviewText = document.getElementById('review-text');
const reviewSend = document.getElementById('review-send');
const reviewCancel = document.getElementById('review-cancel');
const whisperDot = document.getElementById('whisper-dot');
const whisperText = document.getElementById('whisper-text');

let isRecording = false;
let isTranscribing = false;
let audioContext = null;
let mediaStream = null;
let scriptNode = null;
let recordedSamples = [];
let recordingStartTime = 0;
let timerInterval = null;

// --- Recording ---

async function startRecording() {
  if (isRecording || isTranscribing) return;

  try {
    mediaStream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        sampleRate: 16000,
        echoCancellation: true,
        noiseSuppression: true,
      }
    });
  } catch (e) {
    showToast(`Mic access denied: ${e.message}`, 'error');
    return;
  }

  audioContext = new (window.AudioContext || window.webkitAudioContext)({
    sampleRate: 16000,
  });

  const source = audioContext.createMediaStreamSource(mediaStream);

  // ScriptProcessorNode for broad WebView compatibility
  scriptNode = audioContext.createScriptProcessor(4096, 1, 1);
  recordedSamples = [];
  recordingStartTime = Date.now();

  scriptNode.onaudioprocess = (e) => {
    if (!isRecording) return;
    const input = e.inputBuffer.getChannelData(0);
    recordedSamples.push(new Float32Array(input));
  };

  source.connect(scriptNode);
  scriptNode.connect(audioContext.destination);

  isRecording = true;
  micBtn.classList.add('recording');
  updateRecordingTimer();
  timerInterval = setInterval(updateRecordingTimer, 100);
}

function updateRecordingTimer() {
  if (!isRecording) return;
  const elapsed = (Date.now() - recordingStartTime) / 1000;
  const secs = Math.floor(elapsed);
  const tenths = Math.floor((elapsed - secs) * 10);
  cmdEl.placeholder = `Recording ${secs}.${tenths}s...`;
}

async function stopRecording() {
  if (!isRecording) return;
  isRecording = false;
  micBtn.classList.remove('recording');
  if (timerInterval) {
    clearInterval(timerInterval);
    timerInterval = null;
  }

  // Stop audio nodes
  if (scriptNode) {
    scriptNode.disconnect();
    scriptNode = null;
  }
  if (mediaStream) {
    mediaStream.getTracks().forEach(t => t.stop());
    mediaStream = null;
  }
  if (audioContext) {
    audioContext.close().catch(() => {});
    audioContext = null;
  }

  // Flatten recorded chunks
  const totalLength = recordedSamples.reduce((sum, chunk) => sum + chunk.length, 0);
  const duration = totalLength / 16000;

  if (totalLength === 0 || duration < 0.3) {
    cmdEl.placeholder = 'run cargo test';
    showToast('Recording too short', 'warn');
    recordedSamples = [];
    return;
  }

  const allSamples = new Float32Array(totalLength);
  let offset = 0;
  for (const chunk of recordedSamples) {
    allSamples.set(chunk, offset);
    offset += chunk.length;
  }
  recordedSamples = [];

  // Transcribe
  await transcribe(allSamples, duration);
}

async function transcribe(samples, duration) {
  if (!IS_TAURI || !tauriInvoke) {
    cmdEl.placeholder = 'run cargo test';
    showToast('Voice requires the Tauri app', 'warn');
    return;
  }

  isTranscribing = true;
  micBtn.classList.add('transcribing');
  cmdEl.placeholder = `Transcribing ${duration.toFixed(1)}s of audio...`;
  whisperDot.classList.add('ok');
  whisperText.textContent = 'whisper (transcribing)';

  try {
    const samplesArray = Array.from(samples);
    const text = await tauriInvoke('transcribe_audio', { samples: samplesArray });

    if (text && text.trim() && text.trim() !== '[BLANK_AUDIO]') {
      showReviewPopup(text.trim());
    } else {
      showToast('No speech detected', 'warn');
    }
  } catch (e) {
    showToast(`Transcription failed: ${e}`, 'error');
  } finally {
    isTranscribing = false;
    micBtn.classList.remove('transcribing');
    cmdEl.placeholder = 'run cargo test';
    whisperText.textContent = 'whisper (ready)';
  }
}

// --- Review popup ---

function showReviewPopup(text) {
  reviewText.value = text;
  reviewPopup.classList.add('visible');
  reviewText.focus();
  reviewText.select();
}

function hideReviewPopup() {
  reviewPopup.classList.remove('visible');
  reviewText.value = '';
  cmdEl.focus();
}

function sendReviewedText() {
  const text = reviewText.value.trim();
  hideReviewPopup();
  if (!text) return;
  sendCommand(text);
}

// --- Init ---

export function initVoice() {
  // Hold-to-record: press to start, release to stop
  micBtn.addEventListener('mousedown', (e) => {
    e.preventDefault();
    startRecording();
  });
  micBtn.addEventListener('mouseup', (e) => {
    e.preventDefault();
    stopRecording();
  });
  micBtn.addEventListener('mouseleave', () => {
    if (isRecording) stopRecording();
  });

  // Touch events for mobile
  micBtn.addEventListener('touchstart', (e) => {
    e.preventDefault();
    startRecording();
  });
  micBtn.addEventListener('touchend', (e) => {
    e.preventDefault();
    stopRecording();
  });
  micBtn.addEventListener('touchcancel', () => {
    if (isRecording) stopRecording();
  });

  // Review popup buttons
  reviewSend.addEventListener('click', sendReviewedText);
  reviewCancel.addEventListener('click', hideReviewPopup);
  reviewText.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendReviewedText();
    }
    if (e.key === 'Escape') {
      hideReviewPopup();
    }
  });
}
