use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

const MAX_LOG_BYTES: u64 = 256 * 1024;

/// Global log path used by the panic hook (set once at startup).
pub static PANIC_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Open a log file for appending, truncating if oversized.
pub fn open_log(path: &PathBuf) -> std::io::Result<Mutex<std::fs::File>> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::write(path, b"[truncated]\n");
        }
    }
    let dir = path.parent().unwrap_or(path);
    std::fs::create_dir_all(dir)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(Mutex::new(file))
}

/// Install a panic hook that appends to the log file.
/// Uses a globally-set path so it can be called before app_data is available.
pub fn set_panic_log_path(path: PathBuf) {
    let _ = PANIC_LOG_PATH.set(path);
}

pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(move |info| {
        // Try the global path first, fall back to temp dir
        let path = PANIC_LOG_PATH
            .get()
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().join("chops-panic.log"));
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "--- PANIC ---\n{info}\n--- END PANIC ---");
        }
        // Also print to stderr for logcat visibility on Android
        let _ = writeln!(std::io::stderr(), "--- PANIC ---\n{info}\n--- END PANIC ---");
    }));
}
