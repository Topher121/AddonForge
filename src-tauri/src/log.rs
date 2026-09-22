//! Tiny local log: `%APPDATA%\AddonForge\addonforge.log`.
//!
//! Local only, never uploaded. Rotates once past ~1 MB (one `.1` backup).
//! The "Report a problem" button attaches the tail of this file so bug
//! reports arrive with something useful in them.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: Mutex<Option<File>> = Mutex::new(None);
const MAX_BYTES: u64 = 1024 * 1024;

pub fn path() -> PathBuf {
    crate::state::AppState::dir().join("addonforge.log")
}

fn open() -> Option<File> {
    let p = path();
    fs::create_dir_all(p.parent()?).ok()?;
    if fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = fs::remove_file(p.with_extension("log.1"));
        let _ = fs::rename(&p, p.with_extension("log.1"));
    }
    OpenOptions::new().create(true).append(true).open(p).ok()
}

/// Wall-clock stamp as `YYYY-MM-DD HH:MM:SS` UTC (no chrono dependency).
fn stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    // Civil-from-days (Howard Hinnant), good for the range we care about.
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

pub fn write(level: &str, msg: &str) {
    let line = format!("{} [{level}] {msg}\n", stamp());
    if let Ok(mut guard) = FILE.lock() {
        if guard.is_none() {
            *guard = open();
        }
        if let Some(f) = guard.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }
    #[cfg(debug_assertions)]
    eprint!("{line}");
}

#[macro_export]
macro_rules! logi {
    ($($arg:tt)*) => { $crate::log::write("info", &format!($($arg)*)) };
}
#[macro_export]
macro_rules! loge {
    ($($arg:tt)*) => { $crate::log::write("error", &format!($($arg)*)) };
}

/// Last `n` lines of the log for diagnostics.
pub fn tail(n: usize) -> String {
    let text = fs::read_to_string(path()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}
