// src-tauri/crates/pvpn-platform/src/log.rs
// Pluggable log sink: this crate must not depend on the GUI crate's logger,
// but its messages have to end up in the same ring buffer the support UI reads.
// The GUI installs `crate::logger::log` as the sink during startup.

use std::sync::OnceLock;

/// `(level, source, message)` — same shape as the GUI logger's entry point.
pub type Sink = fn(&str, &str, &str);

static SINK: OnceLock<Sink> = OnceLock::new();

/// Installs the process-wide sink. Only the first call wins; later calls are
/// ignored so a stray call from a test cannot hijack logging.
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

pub fn log(level: &str, source: &str, message: &str) {
    match SINK.get() {
        Some(sink) => sink(level, source, message),
        // Same stdout shape the GUI logger uses, so output is identical when
        // the sink has not been installed yet (early startup, unit tests).
        None => println!("[{}][{}] {}", source, level, message),
    }
}

pub fn info(source: &str, message: &str) {
    log("info", source, message);
}

pub fn warn(source: &str, message: &str) {
    log("warn", source, message);
}
