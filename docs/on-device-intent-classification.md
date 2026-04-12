# On-Device Intent Classification Plan

Voice-to-intent classification running entirely on the Android device, using embedding similarity with AtomicGuard guards for confidence gating.

## Goal

Move Tier 1 intent classification (14/22 workflows that need no parameters) onto the phone. The transcript goes through whisper-rs → embedding → cosine similarity → guard → MQTT dispatch, all on-device. Tier 2/3 fall back to the server via MQTT as today.

## Architecture

```
[Hold mic] → [Web Audio f32 PCM] → [whisper-rs transcription]
                                           ↓
                                    [Embed utterance]        ← ONNX (nomic-embed-text, ~50-100ms)
                                           ↓
                                    [Cosine similarity]      ← against 22 pre-computed centroids
                                           ↓
                                    [ThresholdGuard]          ← ag-guards (score ≥ 0.85?)
                                      ↙         ↘
                              PASS              FAIL
                            ↙      ↘              ↓
                     no params   needs params   agent/intent/request
                        ↓            ↓           (server fallback)
              agent/intent/    agent/intent/
              classified       request
              (Tier 1)         (Tier 2)
```

## Components

### 1. Embedding model (ONNX)

**Crate:** `ort` (ONNX Runtime for Rust, supports `aarch64-linux-android`)

**Model:** `nomic-embed-text` exported to ONNX format (~274MB). Bundled in app data directory (downloaded on first launch, like whisper model).

**Module:** `app/src-tauri/src/embed.rs`

```rust
pub struct EmbeddingModel {
    session: ort::Session,
}

impl EmbeddingModel {
    pub fn load(model_path: &Path) -> Result<Self>;
    pub fn embed(&self, text: &str) -> Result<Vec<f32>>;
}
```

Tokenization: nomic-embed-text uses a standard BERT tokenizer. Bundle `tokenizer.json` alongside the ONNX model. Use the `tokenizers` crate (HuggingFace, Rust-native, works on Android).

### 2. Centroid store

**File:** `centroids.json` — 22 workflow centroids, pre-computed from 79 example utterances on the server.

```json
{
  "health_check": [0.012, -0.034, ...],
  "disk_check": [0.008, -0.021, ...],
  "restart_service": [0.045, 0.012, ...],
  ...
}
```

Generated once by AtomicGuard's `scripts/intent-embed-test.sh`, shipped with the app or downloaded alongside the embedding model. ~22 × 768 floats = ~67KB.

**Module:** `app/src-tauri/src/intent.rs`

```rust
pub struct IntentClassifier {
    model: EmbeddingModel,
    centroids: HashMap<String, Vec<f32>>,
    guard: ThresholdGuard,        // from ag-guards
}

pub struct ClassificationResult {
    pub workflow: String,
    pub score: f64,
    pub guard_result: GuardResult, // from ag-domain
    pub tier: Tier,
}

pub enum Tier {
    Local,          // Tier 1: dispatch directly
    NeedsParams,    // Tier 2: send to server for param extraction
    Fallback,       // Tier 3: low confidence, full LLM needed
}

// Workflows that need no parameters (Tier 1 — dispatch directly)
const PARAMLESS_WORKFLOWS: &[&str] = &[
    "health_check", "disk_check", "resource_monitor", "triage",
    // ... 14 total from the system-view doc
];

impl IntentClassifier {
    pub fn classify(&self, text: &str) -> Result<ClassificationResult>;
}
```

### 3. Guard integration (from atomicguard-rs)

**Dependencies:** Add `ag-domain` and `ag-guards` from the atomicguard-rs workspace as path or git dependencies.

```toml
# app/src-tauri/Cargo.toml
ag-domain = { git = "https://github.com/thompsonson/atomicguard", branch = "claude/rust-port-phase-1-35duu", path = "atomicguard-rs/crates/ag-domain" }
ag-guards = { git = "https://github.com/thompsonson/atomicguard", branch = "claude/rust-port-phase-1-35duu", path = "atomicguard-rs/crates/ag-guards" }
```

**Usage:** The `ThresholdGuard` validates the cosine similarity score. It operates on an `Artifact` — we wrap the classification result as an artifact so the guard interface is satisfied:

```rust
// Create artifact from classification
let artifact = Artifact::from_generator_output(
    format!("score={:.4}", best_score),
    metadata,
    "intent-classification",
    "on-device",
    1,
    ArtifactId::new(uuid()),
);

// Guard checks confidence
let guard = ThresholdGuard::new(r"score=(\d+\.\d+)", 0.85, "cosine_similarity")?;
let result = guard.validate(&artifact, &GuardContext::default())?;
```

Note: `ThresholdGuard` checks `value <= max_value` (it was designed for "is this metric below a limit"). For cosine similarity we want `value >= threshold`. Options:
- (a) Add a `MinThresholdGuard` to ag-guards (check `value >= min_value`)
- (b) Invert: use `1.0 - score` as the value and threshold at `0.15`
- (c) Write a simple `ConfidenceGuard` in chops that implements the `Guard` trait directly

Option (c) is simplest and avoids modifying atomicguard-rs:

```rust
pub struct ConfidenceGuard {
    min_score: f64,
}

impl Guard for ConfidenceGuard {
    fn validate(&self, artifact: &Artifact, _ctx: &GuardContext) -> Result<GuardResult, DomainError> {
        let score: f64 = artifact.metadata.get("score")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if score >= self.min_score {
            Ok(GuardResult::pass())
        } else {
            Ok(GuardResult::fail(format!(
                "confidence {score:.3} below threshold {:.3}", self.min_score
            )))
        }
    }
}
```

### 4. Integration point in stt.rs

**File:** `app/src-tauri/src/stt.rs` lines 88-94

Currently:
```rust
let text = text.trim().to_string();
if text.is_empty() || text == "[BLANK_AUDIO]" {
    return Ok(String::new());
}
info!("Transcribed: {text}");
Ok(text)
```

After integration, the classification happens here. But the actual dispatch decision is in `lib.rs` where the Tauri command handler calls `publish_transcription()`. Better to:

1. Keep `stt.rs` returning the transcript text (no change)
2. Add a new Tauri command `classify_and_dispatch` that:
   - Runs the classifier
   - If Tier 1: publishes to `agent/intent/classified` with workflow name
   - If Tier 2/3: publishes to `voice/transcriptions` as today (server handles it)
3. The frontend calls `classify_and_dispatch` instead of `send_transcription` when the classifier is loaded

### 5. MQTT topic

New topic: **`agent/intent/classified`** (QoS 1)

```json
{
  "workflow": "health_check",
  "score": 0.95,
  "source": "on-device",
  "transcript": "check the system"
}
```

AtomicGuard's MQTT listener subscribes to this topic and dispatches the workflow directly (skipping embedding + intent classification since the phone already did it).

### 6. Model management

Same pattern as whisper model:
- Settings modal gets an "Embedding Model" path field
- Browse button for file picker (Android content:// URI import)
- Model status shown in header (dot indicator)
- Download script or first-launch download from a release asset

## Files to create/modify

| File | Action | Purpose |
|------|--------|---------|
| `app/src-tauri/src/embed.rs` | Create | ONNX embedding model wrapper |
| `app/src-tauri/src/intent.rs` | Create | IntentClassifier, ConfidenceGuard, centroid loading |
| `app/src-tauri/src/lib.rs` | Modify | Add `classify_and_dispatch` command, init classifier |
| `app/src-tauri/Cargo.toml` | Modify | Add `ort`, `tokenizers`, `ag-domain`, `ag-guards` deps |
| `app/src/js/commands.js` | Modify | Call `classify_and_dispatch` when classifier available |
| `CLAUDE.md` | Modify | Add `agent/intent/classified` to MQTT topics |

## Dependencies to add

```toml
ort = { version = "2", features = ["load-dynamic"] }  # ONNX Runtime
tokenizers = { version = "0.21", default-features = false }  # HuggingFace tokenizer
ag-domain = { git = "...", ... }
ag-guards = { git = "...", ... }
```

`load-dynamic` feature for `ort` lets the ONNX Runtime shared library be loaded at runtime, which is important for Android where the `.so` is bundled in the APK.

## Risks and open questions

1. **ONNX Runtime on Android ARM64** — `ort` crate supports it but cross-compilation setup may need NDK-specific CMake flags. Need to validate with a test build.

2. **Model size** — 274MB for nomic-embed-text is significant for a mobile app. Consider:
   - Downloading on first use (not bundled in APK)
   - Using a smaller model (e.g., `all-MiniLM-L6-v2` at 90MB, may still have good accuracy)
   - Quantized ONNX model (INT8 could halve the size)

3. **Tokenizer on Android** — The `tokenizers` crate is pure Rust and should work, but hasn't been tested in the chops Android build.

4. **Latency on phone** — 29ms on Mac i9. Phone (Snapdragon 8 Gen 2) should be ~50-150ms. Need to benchmark.

5. **Centroid drift** — When AtomicGuard adds new workflows, centroids need updating on the phone. Could version-check against the server on connect.

6. **ag-domain/ag-guards dependency** — Currently on an unmerged branch. Once merged to main and published as a crate, switch to a proper version dependency.

## Implementation order

1. **Validate ONNX on Android** — Add `ort` dep, load a tiny test model, verify it runs on the phone. This is the biggest risk.
2. **Export nomic-embed-text to ONNX** — Use `optimum` Python library to export and test equivalence.
3. **Implement embed.rs** — ONNX model loading + inference wrapper.
4. **Implement intent.rs** — Classifier with cosine similarity + ConfidenceGuard.
5. **Wire into lib.rs** — New Tauri command, model management.
6. **Update AtomicGuard listener** — Subscribe to `agent/intent/classified`.
7. **Frontend changes** — Use classifier when available, show classification results in conversation feed.
