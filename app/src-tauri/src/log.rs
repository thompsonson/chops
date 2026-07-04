use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_LOG_BYTES: u64 = 256 * 1024;

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
pub fn install_panic_hook(log_path: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "--- PANIC ---\n{info}\n--- END PANIC ---");
        }
    }));
}
