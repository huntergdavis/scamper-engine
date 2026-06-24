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

/// Install a panic hook that writes the panic — message, location, and a full
/// backtrace — to the debug log before delegating to whatever hook was already
/// set. Without this, a panic only reaches stderr, which is invisible behind the
/// alt-screen/raw-mode terminal and never lands in `scamp.log`; a crash during
/// play would leave no trace. Cheap no-op when logging is disabled.
///
/// Call this once, early (right after `init`), so it runs *before*
/// `TerminalGuard::enter` wraps the hook with terminal teardown — that ordering
/// makes the guard tear the terminal down first, then this logger records the
/// crash, then the original hook prints to the now-restored stderr.
pub fn install_panic_logger() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if enabled() {
            let loc = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "<unknown location>".into());
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".into());
            // force_capture always captures, regardless of RUST_BACKTRACE.
            let bt = std::backtrace::Backtrace::force_capture();
            log(format_args!("PANIC at {loc}: {msg}\n--- backtrace ---\n{bt}"));
        }
        prev(info);
    }));
}

#[macro_export]
macro_rules! dlog {
    ($($a:tt)*) => { $crate::dbg::log(format_args!($($a)*)) };
}
