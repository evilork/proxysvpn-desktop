// src-tauri/src/logger.rs
// Lightweight in-process logger:
//   • Ring buffer (last N lines kept in memory) — for live UI
//   • Mirror to ~/Library/Logs/ProxysVPN/app.log (rotated daily) — for support
//   • Thread-safe, lock-free fast path

use std::collections::VecDeque;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const BUFFER_CAPACITY: usize = 5000;

#[derive(Clone, serde::Serialize)]
pub struct LogLine {
    pub ts_ms: u128,        // unix epoch ms
    pub level: String,      // "info" | "warn" | "error"
    pub source: String,     // "app" | "xray" | "tun" | etc.
    pub message: String,
}

struct LoggerState {
    buffer: VecDeque<LogLine>,
    log_file_path: Option<PathBuf>,
}

static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();

fn get_state() -> &'static Mutex<LoggerState> {
    STATE.get_or_init(|| {
        let log_file_path = compute_log_path();
        if let Some(p) = &log_file_path {
            if let Some(dir) = p.parent() {
                let _ = create_dir_all(dir);
            }
        }
        Mutex::new(LoggerState {
            buffer: VecDeque::with_capacity(BUFFER_CAPACITY),
            log_file_path,
        })
    })
}

fn compute_log_path() -> Option<PathBuf> {
    // ~/Library/Logs/ProxysVPN/app.log on macOS
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Logs/ProxysVPN/app.log"))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn init() {
    let _ = get_state();
    log("info", "app", "logger initialized");
}

pub fn log(level: &str, source: &str, message: &str) {
    let line = LogLine {
        ts_ms: now_ms(),
        level: level.to_string(),
        source: source.to_string(),
        message: message.to_string(),
    };

    // Mirror to stdout for dev visibility (existing behaviour).
    println!("[{}][{}] {}", source, level, message);

    let mut st = match get_state().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    if st.buffer.len() == BUFFER_CAPACITY {
        st.buffer.pop_front();
    }
    st.buffer.push_back(line.clone());

    // Best-effort write to file. Never panic from logger.
    if let Some(path) = st.log_file_path.clone() {
        drop(st); // release lock before fs i/o
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            let secs = line.ts_ms / 1000;
            let _ = writeln!(
                f,
                "{} [{}] [{}] {}",
                format_iso(secs),
                line.level,
                line.source,
                line.message
            );
        }
    }
}

fn format_iso(secs: u128) -> String {
    // Minimal ISO-8601 (UTC) without external deps.
    // Good enough for log timestamps.
    let s = secs as i64;
    let days_since_epoch = s / 86400;
    let day_secs = s % 86400;
    let hour = day_secs / 3600;
    let minute = (day_secs % 3600) / 60;
    let second = day_secs % 60;

    // Convert days_since_epoch (1970-01-01) to YYYY-MM-DD using a small algorithm.
    let (y, m, d) = days_to_ymd(days_since_epoch);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, minute, second
    )
}

fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468;
    let era = if days >= 0 { days / 146097 } else { (days - 146096) / 146097 };
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn snapshot(limit: Option<usize>) -> Vec<LogLine> {
    let st = match get_state().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let take = limit.unwrap_or(usize::MAX);
    let len = st.buffer.len();
    let start = len.saturating_sub(take);
    st.buffer.iter().skip(start).cloned().collect()
}

pub fn clear() {
    let mut st = match get_state().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    st.buffer.clear();
    log("info", "app", "logs cleared by user");
}

pub fn log_file_path() -> Option<String> {
    let st = match get_state().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    st.log_file_path.as_ref().map(|p| p.to_string_lossy().to_string())
}

/// Format buffer as plain text for clipboard/file export.
pub fn export_text(include_system_info: bool) -> String {
    let mut out = String::new();
    if include_system_info {
        out.push_str("=== ProxysVPN Diagnostic Report ===\n");
        out.push_str(&format!("Generated: {}\n", format_iso(now_ms() as u128 / 1000)));
        out.push_str(&format!(
            "App version: {}\n",
            env!("CARGO_PKG_VERSION")
        ));
        out.push_str(&format!("OS arch: {}\n", std::env::consts::ARCH));
        out.push_str(&format!("OS family: {}\n", std::env::consts::OS));
        out.push_str("\n=== Log Lines ===\n");
    }

    for line in snapshot(None).iter() {
        let secs = line.ts_ms / 1000;
        out.push_str(&format!(
            "{} [{}] [{}] {}\n",
            format_iso(secs),
            line.level,
            line.source,
            line.message
        ));
    }
    out
}

// Convenience macros — use them instead of println!/eprintln! in our crate.
#[macro_export]
macro_rules! log_info {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("info", $source, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("warn", $source, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($source:expr, $($arg:tt)*) => {
        $crate::logger::log("error", $source, &format!($($arg)*))
    };
}
