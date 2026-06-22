//! Terminal lifecycle & hygiene (PROJECT_PLAN.md §4.5).
//!
//! A `TerminalGuard` puts the terminal into raw mode + alt-screen, enables the
//! Kitty keyboard protocol, and — critically — restores everything on EVERY exit
//! path: normal Drop, panic (via a hook), and SIGTERM/SIGHUP (via handlers that
//! flag the loop to quit). Teardown is idempotent and uses async-signal-safe
//! `write`/`tcsetattr`. In raw mode ISIG is off, so Ctrl-C / Ctrl-Z arrive as bytes
//! (handled by input), not signals — so we only trap external term/hup + winch.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static QUIT: AtomicBool = AtomicBool::new(false);
static RESIZE: AtomicBool = AtomicBool::new(false);
static TORN_DOWN: AtomicBool = AtomicBool::new(false);

struct Termios(libc::termios);
// The saved termios is plain POD we only touch during setup/teardown.
unsafe impl Send for Termios {}
unsafe impl Sync for Termios {}
static ORIG: OnceLock<Termios> = OnceLock::new();

/// True once an external terminating signal (or input) has asked us to quit.
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}
/// Ask the loop to exit cleanly (called by the input layer on Ctrl-C / q / Esc).
pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
}
/// Returns true once (and clears) if a resize happened since the last check.
pub fn take_resize() -> bool {
    RESIZE.swap(false, Ordering::Relaxed)
}

extern "C" fn on_quit_signal(_sig: libc::c_int) {
    QUIT.store(true, Ordering::Relaxed);
}
extern "C" fn on_winch(_sig: libc::c_int) {
    RESIZE.store(true, Ordering::Relaxed);
}

/// Fatal/crash signals: restore the terminal (async-signal-safe write+tcsetattr),
/// then re-raise with the default disposition so the crash proceeds normally.
extern "C" fn on_fatal(sig: libc::c_int) {
    teardown_global();
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = 0;
        libc::sigaction(sig, &sa, std::ptr::null_mut());
        libc::raise(sig);
    }
}

unsafe fn set_handler(sig: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    let mut sa: libc::sigaction = std::mem::zeroed();
    sa.sa_sigaction = handler as usize;
    libc::sigemptyset(&mut sa.sa_mask);
    sa.sa_flags = 0; // no SA_RESTART → blocking syscalls return EINTR so the loop reacts
    libc::sigaction(sig, &sa, std::ptr::null_mut());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
    pub xpix: u16,
    pub ypix: u16,
}

/// Query terminal size in cells and pixels via TIOCGWINSZ.
pub fn query_winsize() -> WinSize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws);
        WinSize {
            cols: ws.ws_col,
            rows: ws.ws_row,
            xpix: ws.ws_xpixel,
            ypix: ws.ws_ypixel,
        }
    }
}

// Setup/teardown escape sequences.
// Alt screen, hide cursor, push kitty kbd flags, focus events, disable autowrap
// (so a full-width text row can't scroll the screen), clear.
const SETUP: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[>11u\x1b[?1004h\x1b[?7l\x1b[2J";
// Teardown: delete all images, focus off, pop kbd flags, re-enable autowrap,
// show cursor, leave alt screen.
const TEARDOWN: &[u8] = b"\x1b_Ga=d,d=A\x1b\\\x1b[?1004l\x1b[<u\x1b[?7h\x1b[?25h\x1b[?1049l";

fn teardown_global() {
    if TORN_DOWN.swap(true, Ordering::SeqCst) {
        return; // already done
    }
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            TEARDOWN.as_ptr() as *const libc::c_void,
            TEARDOWN.len(),
        );
        if let Some(orig) = ORIG.get() {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &orig.0);
        }
    }
}

pub struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    /// Enter raw mode + alt-screen, enable the Kitty keyboard protocol, install
    /// signal + panic teardown. Restores automatically on Drop.
    pub fn enter() -> std::io::Result<TerminalGuard> {
        use std::io::Write;
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut orig) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let _ = ORIG.set(Termios(orig));

            let mut raw = orig;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            raw.c_iflag &=
                !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
            raw.c_oflag &= !(libc::OPOST);
            raw.c_cflag |= libc::CS8;
            raw.c_cc[libc::VMIN] = 0; // non-blocking reads: return immediately
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            set_handler(libc::SIGTERM, on_quit_signal);
            set_handler(libc::SIGHUP, on_quit_signal);
            set_handler(libc::SIGWINCH, on_winch);
            // Crash signals: restore the terminal before dying.
            for &s in &[
                libc::SIGSEGV,
                libc::SIGBUS,
                libc::SIGABRT,
                libc::SIGQUIT,
                libc::SIGILL,
                libc::SIGFPE,
            ] {
                set_handler(s, on_fatal);
            }
        }

        TORN_DOWN.store(false, Ordering::SeqCst);

        {
            let mut out = std::io::stdout().lock();
            out.write_all(SETUP)?;
            out.flush()?;
        }

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            teardown_global();
            prev(info);
        }));

        Ok(TerminalGuard { _private: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        teardown_global();
    }
}

/// Probe Kitty keyboard protocol support: send the query and wait briefly for a
/// `CSI ? <flags> u` reply. Must be called right after `enter()`, before the input
/// loop starts consuming stdin. Returns false if unsupported (→ legacy fallback).
pub fn probe_kitty_keyboard() -> bool {
    use std::io::Write;
    {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(b"\x1b[?u\x1b[c"); // query kbd flags, then Primary DA as a fence
        let _ = out.flush();
    }
    // Read for up to ~150ms looking for an "...u" response before the DA "...c".
    let mut buf = [0u8; 256];
    let mut acc: Vec<u8> = Vec::new();
    let deadline_polls = 15;
    for _ in 0..deadline_polls {
        let mut pfd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, 10) };
        if r > 0 {
            let n = unsafe {
                libc::read(
                    libc::STDIN_FILENO,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n > 0 {
                acc.extend_from_slice(&buf[..n as usize]);
                // A kitty reply looks like ESC [ ? <digits> u
                if let Some(p) = find_subseq(&acc, b"\x1b[?") {
                    if acc[p..].iter().any(|&b| b == b'u') {
                        return true;
                    }
                }
                // If we got the DA reply (ends in 'c') without a 'u' reply → unsupported.
                if acc.iter().any(|&b| b == b'c') && !acc.contains(&b'u') {
                    return false;
                }
            }
        }
    }
    false
}

fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
