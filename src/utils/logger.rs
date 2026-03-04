use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

const MAX_LOG_SIZE: u64 = 512 * 1024; // 512KB

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

fn get_log_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude").join("ccline").join("ccline.log"))
}

fn rotate_if_needed(path: &PathBuf) {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() > MAX_LOG_SIZE {
            // Keep the last ~256KB
            if let Ok(content) = std::fs::read(path) {
                let keep_from = content.len().saturating_sub(256 * 1024);
                // Find the next newline after the cut point to avoid partial lines
                let start = content[keep_from..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|pos| keep_from + pos + 1)
                    .unwrap_or(keep_from);
                let _ = std::fs::write(path, &content[start..]);
            }
        }
    }
}

pub fn log_debug(tag: &str, msg: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }

    let Some(log_path) = get_log_path() else {
        return;
    };

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    rotate_if_needed(&log_path);

    let timestamp = {
        let now = std::time::SystemTime::now();
        let duration = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();
        // Format as simple UTC timestamp
        let hours = (secs % 86400) / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    };

    let line = format!("[{}] [{}] {}\n", timestamp, tag, msg);

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = file.write_all(line.as_bytes());
    }
}
