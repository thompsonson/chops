use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Max size of the *active* log file before it rotates. One rotated backup
/// (`.1`) is kept alongside it, so total retained history is up to 2x this.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// Global log path used by the panic hook (set once at startup).
pub static PANIC_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Suffix used for the single rotated backup of a log file.
fn backup_path(path: &PathBuf) -> PathBuf {
    let mut backup = path.clone();
    backup.set_extension(
        path.extension()
            .map(|ext| format!("{}.1", ext.to_string_lossy()))
            .unwrap_or_else(|| "1".to_string()),
    );
    backup
}

/// A `Write` implementation that rotates the underlying file once it grows
/// past `cap` bytes, keeping exactly one rotated backup. Rotation happens
/// on every write call (not just at startup), so a long-running session
/// can't grow the log file unbounded.
struct RotatingLog {
    path: PathBuf,
    backup: PathBuf,
    file: std::fs::File,
    written: u64,
    cap: u64,
}

impl RotatingLog {
    fn open(path: PathBuf, cap: u64) -> std::io::Result<Self> {
        let dir = path.parent().unwrap_or(&path);
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        let backup = backup_path(&path);
        Ok(Self {
            path,
            backup,
            file,
            written,
            cap,
        })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let _ = std::fs::remove_file(&self.backup);
        let _ = std::fs::rename(&self.path, &self.backup);
        self.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for RotatingLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written >= self.cap {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Open a rotating log file for appending. Bounded to ~2x `MAX_LOG_BYTES`
/// on disk (active file + one rotated backup) for the lifetime of the app,
/// not just at the next launch.
pub fn open_log(path: &PathBuf) -> std::io::Result<Mutex<impl Write>> {
    RotatingLog::open(path.clone(), MAX_LOG_BYTES).map(Mutex::new)
}

/// Read the full retained log history (rotated backup, if any, then the
/// active file) for display/export.
pub fn read_log(path: &PathBuf) -> std::io::Result<String> {
    let mut out = String::new();
    if let Ok(backup) = std::fs::read_to_string(backup_path(path)) {
        out.push_str(&backup);
    }
    out.push_str(&std::fs::read_to_string(path)?);
    Ok(out)
}

/// Clear both the active log file and its rotated backup.
pub fn clear_log(path: &PathBuf) -> std::io::Result<()> {
    let _ = std::fs::remove_file(backup_path(path));
    std::fs::write(path, b"[cleared]\n")
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
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "--- PANIC ---\n{info}\n--- END PANIC ---");
        }
        // Also print to stderr for logcat visibility on Android
        let _ = writeln!(
            std::io::stderr(),
            "--- PANIC ---\n{info}\n--- END PANIC ---"
        );
    }));
}
