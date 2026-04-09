// voice.js — Push-to-talk mic button (Phase 2: Web Audio capture + whisper-rs)
//
// Current: placeholder UI with hold-to-record button state.
// Phase 2 will add: AudioWorklet capture, Tauri invoke to send PCM,
// whisper-rs transcription, and review popup integration.

import { IS_TAURI, tauriInvoke, showToast } from './app.js';

const micBtn = document.getElementById('mic');
const cmdEl = document.getElementById('cmd');
const reviewPopup = document.getElementById('review-popup');
const reviewText = document.getElementById('review-text');
const reviewSend = document.getElementById('review-send');
const reviewCancel = document.getElementById('review-cancel');

let isRecording = false;

function startRecording() {
  if (isRecording) return;
  isRecording = true;
  micBtn.classList.add('recording');
  cmdEl.placeholder = 'Recording...';

  // Phase 2: Start Web Audio capture here
  // For now, show a toast indicating the feature is coming
  showToast('Voice recording coming soon (Phase 2)', 'info');
}

function stopRecording() {
  if (!isRecording) return;
  isRecording = false;
  micBtn.classList.remove('recording');
  cmdEl.placeholder = 'run cargo test';

  // Phase 2: Stop capture, send PCM to Rust backend, show review popup
  // showReviewPopup(transcribedText);
}

// --- Review popup ---

function showReviewPopup(text) {
  reviewText.value = text;
  reviewPopup.classList.add('visible');
  reviewText.focus();
}

function hideReviewPopup() {
  reviewPopup.classList.remove('visible');
  reviewText.value = '';
}

function sendReviewedText() {
  const text = reviewText.value.trim();
  if (!text) {
    hideReviewPopup();
    return;
  }

  if (IS_TAURI && tauriInvoke) {
    // Put the reviewed text into the command input and let commands.js handle sending
    cmdEl.value = text;
    hideReviewPopup();
    // Trigger send via the send button click
    document.getElementById('send').click();
  } else {
    hideReviewPopup();
  }
}

// --- Init ---

export function initVoice() {
  // Hold-to-record: mousedown/touchstart to start, mouseup/touchend to stop
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
