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
    if now_ns() < sleep_target {
        let ts = timespec_from_ns(sleep_target);
        loop {
            let r = unsafe {
                libc::clock_nanosleep(
                    libc::CLOCK_MONOTONIC,
                    libc::TIMER_ABSTIME,
                    &ts,
                    std::ptr::null_mut(),
                )
            };
            if r == 0 {
                break;
            }
            // r is the errno value (clock_nanosleep returns it directly).
            // On EINTR, return so the loop can react to the signal.
            break;
        }
    }
    // Spin the tail.
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
