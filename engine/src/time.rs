//! Monotonic clock + frame pacing (PROJECT_PLAN.md §4.2).
//! CLOCK_MONOTONIC, absolute-deadline sleep via clock_nanosleep, with a short spin
//! tail for sub-ms accuracy. All times in nanoseconds.

pub const NS_PER_SEC: u64 = 1_000_000_000;

/// Current CLOCK_MONOTONIC time in nanoseconds.
pub fn now_ns() -> u64 {
    unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        (ts.tv_sec as u64) * NS_PER_SEC + ts.tv_nsec as u64
    }
}

fn timespec_from_ns(ns: u64) -> libc::timespec {
    libc::timespec {
        tv_sec: (ns / NS_PER_SEC) as libc::time_t,
        tv_nsec: (ns % NS_PER_SEC) as _,
    }
}

/// Sleep until the absolute monotonic deadline `target_ns`, sleeping up to
/// `spin_margin_ns` short of it and busy-spinning the remainder. Returns early on
/// EINTR (so signals like SIGWINCH/SIGTERM are handled promptly by the loop).
pub fn sleep_until_ns(target_ns: u64, spin_margin_ns: u64) {
    let sleep_target = target_ns.saturating_sub(spin_margin_ns);
    let start = now_ns();
    if start < sleep_target {
        // Linux: absolute-deadline sleep against CLOCK_MONOTONIC. macOS lacks
        // clock_nanosleep/TIMER_ABSTIME, so sleep the relative remaining interval.
        #[cfg(target_os = "linux")]
        let r = {
            let ts = timespec_from_ns(sleep_target);
            // clock_nanosleep returns the errno value directly (0 on success).
            unsafe {
                libc::clock_nanosleep(
                    libc::CLOCK_MONOTONIC,
                    libc::TIMER_ABSTIME,
                    &ts,
                    std::ptr::null_mut(),
                )
            }
        };
        #[cfg(not(target_os = "linux"))]
        let r = {
            let ts = timespec_from_ns(sleep_target - start);
            // nanosleep returns -1 and sets errno on interruption.
            let rc = unsafe { libc::nanosleep(&ts, std::ptr::null_mut()) };
            if rc == 0 {
                0
            } else {
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(0)
            }
        };
        // On EINTR (a signal like SIGWINCH/SIGTERM, since we don't set SA_RESTART),
        // bail immediately — skip the spin tail so the loop reacts to the signal now.
        if r == libc::EINTR {
            return;
        }
    }
    // Spin the short tail for sub-ms deadline accuracy.
    while now_ns() < target_ns {
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_advances() {
        let a = now_ns();
        let b = now_ns();
        assert!(b >= a);
    }
}
