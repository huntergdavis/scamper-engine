//! Lightweight file logging for development (PROJECT_PLAN.md §4.5/§8).
//!
//! stdout is the graphics channel, so we can't print diagnostics to the screen
//! while the game runs — they go to a log file instead. Disabled (zero-cost) by
//! default; `dbg::init(true, path)` turns it on. Use the `dlog!` macro.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::time::now_ns;

struct Logger {
    file: Mutex<std::fs::File>,
    start_ns: u64,
}

static LOG: OnceLock<Option<Logger>> = OnceLock::new();

/// Open the log file (truncating) if `enabled`. Safe to call once; later calls
/// are ignored. When disabled, `dlog!` compiles to a cheap no-op check.
pub fn init(enabled: bool, path: &str) {
    let logger = if enabled {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .ok()
            .map(|file| Logger { file: Mutex::new(file), start_ns: now_ns() })
    } else {
        None
    };
    let _ = LOG.set(logger);
}

pub fn enabled() -> bool {
    matches!(LOG.get(), Some(Some(_)))
}

/// Append a timestamped line (ms since init). Used via the `dlog!` macro.
pub fn log(args: std::fmt::Arguments) {
    if let Some(Some(l)) = LOG.get() {
        let ms = now_ns().saturating_sub(l.start_ns) as f64 / 1_000_000.0;
        if let Ok(mut f) = l.file.lock() {
            let _ = writeln!(f, "[{ms:9.1}ms] {args}");
            let _ = f.flush();
        }
    }
}

#[macro_export]
macro_rules! dlog {
    ($($a:tt)*) => { $crate::dbg::log(format_args!($($a)*)) };
}
