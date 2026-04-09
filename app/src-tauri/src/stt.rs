use anyhow::{Context, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_MODEL: &str = "ggml-base.en.bin";
const MODEL_PATH_CONFIG: &str = "model-path.txt";
const SAMPLE_RATE: u32 = 16000;

/// Lazy-loaded whisper context, shared across transcription calls.
pub struct SttEngine {
    ctx: std::sync::Mutex<Option<WhisperContext>>,
}

impl SttEngine {
    pub fn new() -> Self {
        Self {
            ctx: std::sync::Mutex::new(None),
        }
    }

    /// Transcribe f32 PCM samples (16kHz mono) using whisper-rs.
    /// Loads the model on first call. Optionally saves audio to cache dir.
    pub fn transcribe(
        &self,
        app: &AppHandle,
        samples: &[f32],
    ) -> Result<String> {
        let duration_secs = samples.len() as f32 / SAMPLE_RATE as f32;
        if duration_secs < 0.3 {
            return Err(anyhow::anyhow!("Audio too short ({:.1}s)", duration_secs));
        }

        info!("Transcribing {:.1}s of audio ({} samples)", duration_secs, samples.len());

        // Save audio to cache for debugging/replay
        if let Err(e) = save_wav_cache(app, samples) {
            warn!("Failed to save audio cache: {e}");
        }

        // Load model if not already loaded
        let path = model_path(app)?;
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "Whisper model not found at {}",
                path.display()
            ));
        }

        let mut ctx_lock = self.ctx.lock().unwrap();
        if ctx_lock.is_none() {
            info!("Loading whisper model from {}", path.display());
            let ctx = WhisperContext::new_with_params(
                path.to_str().unwrap(),
                WhisperContextParameters::default(),
            )
            .context("Failed to load whisper model")?;
            *ctx_lock = Some(ctx);
            info!("Whisper model loaded");
        }

        let ctx = ctx_lock.as_ref().unwrap();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);

        let mut state = ctx.create_state().context("Failed to create whisper state")?;

        state
            .full(params, samples)
            .map_err(|e| anyhow::anyhow!("Whisper transcription failed: {e}"))?;

        let num_segments = state.full_n_segments().unwrap_or(0);
        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                text.push_str(&segment);
            }
        }

        let text = text.trim().to_string();
        if text.is_empty() || text == "[BLANK_AUDIO]" {
            return Ok(String::new());
        }

        info!("Transcribed: {text}");
        Ok(text)
    }
}

/// Save PCM samples as a WAV file in the cache directory for debugging.
fn save_wav_cache(app: &AppHandle, samples: &[f32]) -> Result<()> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .context("Could not determine cache directory")?
        .join("voice");
    std::fs::create_dir_all(&cache_dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = cache_dir.join(format!("{timestamp}.wav"));

    write_wav(&path, samples, SAMPLE_RATE)?;
    info!("Saved audio to {}", path.display());
    Ok(())
}

/// Write f32 PCM samples as a 16-bit WAV file.
fn write_wav(path: &PathBuf, samples: &[f32], sample_rate: u32) -> Result<()> {
    use std::io::Write;

    let num_samples = samples.len() as u32;
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let byte_rate = sample_rate * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_size = num_samples * u32::from(bits_per_sample) / 8;
    let file_size = 36 + data_size;

    let mut file = std::fs::File::create(path)?;

    // RIFF header
    file.write_all(b"RIFF")?;
    file.write_all(&file_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;

    // fmt chunk
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM format
    file.write_all(&num_channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;

    // data chunk
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Convert f32 to i16 and write
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_val = (clamped * 32767.0) as i16;
        file.write_all(&i16_val.to_le_bytes())?;
    }

    Ok(())
}

// --- Model path management (unchanged) ---

fn model_path(app: &AppHandle) -> Result<PathBuf> {
    let data_dir = app
        .path()
        .app_data_dir()
        .context("Could not determine app data directory")?;
    std::fs::create_dir_all(&data_dir)?;

    let config_file = data_dir.join(MODEL_PATH_CONFIG);
    if let Ok(custom_path) = std::fs::read_to_string(&config_file) {
        let custom_path = custom_path.trim();
        if !custom_path.is_empty() {
            return Ok(PathBuf::from(custom_path));
        }
    }

    Ok(data_dir.join(WHISPER_MODEL))
}

pub fn set_model_path_config(app: &AppHandle, path: &str) -> Result<()> {
    let data_dir = app
        .path()
        .app_data_dir()
        .context("Could not determine app data directory")?;
    std::fs::create_dir_all(&data_dir)?;
    let config_file = data_dir.join(MODEL_PATH_CONFIG);
    std::fs::write(&config_file, path)?;
    info!("Model path set to: {path}");
    Ok(())
}

pub fn get_model_path_config(app: &AppHandle) -> String {
    match model_path(app) {
        Ok(p) => p.display().to_string(),
        Err(e) => format!("Error: {e}"),
    }
}

pub fn model_status(app: &AppHandle) -> (bool, String) {
    match model_path(app) {
        Ok(p) => {
            let exists = p.exists();
            (exists, p.display().to_string())
        }
        Err(e) => (false, format!("Error: {e}")),
    }
}
