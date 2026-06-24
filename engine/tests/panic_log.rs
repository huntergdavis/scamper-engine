//! The debug panic logger must capture a crash into the log file (it's the only
//! trace of a panic that happens behind the alt-screen/raw-mode terminal).
//! Each integration test file is its own process, so the `OnceLock`-backed logger
//! starts fresh here.

use std::panic;

#[test]
fn panic_is_written_to_the_debug_log() {
    let path = std::env::temp_dir().join("scamper-panic-log-test.log");
    let _ = std::fs::remove_file(&path);

    scamper::dbg::init(true, path.to_str().unwrap());
    scamper::dbg::install_panic_logger();

    // Swallow the default hook's stderr print so the test output stays clean,
    // but keep our logger (it was installed as this hook's `prev`).
    let logger_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| logger_hook(info)));

    let result = panic::catch_unwind(|| {
        assert_eq!(2 + 2, 5, "intentional test panic");
    });
    assert!(result.is_err(), "the closure should have panicked");

    let logged = std::fs::read_to_string(&path).expect("log file should exist");
    assert!(logged.contains("PANIC at"), "missing panic header:\n{logged}");
    assert!(logged.contains("intentional test panic"), "missing message:\n{logged}");
    assert!(logged.contains("backtrace"), "missing backtrace:\n{logged}");

    let _ = std::fs::remove_file(&path);
}
