//! IPC fast-path counters.
//!
//! The register fast path (a sender copying a small message straight into a
//! blocked receiver's saved registers) was built on the old scheduler's
//! task structure. It was never reachable from a system call: the receive
//! side only yielded once and then returned an empty message (N-84), and the
//! send side was used by nothing but an uncalled batching helper. It went
//! with the old scheduler (sprint D4); every message now goes through an
//! endpoint queue, whose receivers sleep on the endpoint's wait queue. A
//! direct hand-off returns with the IPC redesign (v0.28, N-79 to N-91).
//!
//! The counters stay for the shell's `ipcstat` and the in-kernel benchmarks;
//! they read zero until a fast path exists again.

use core::sync::atomic::{AtomicU64, Ordering};

/// Performance counter for fast path operations
static FAST_PATH_COUNT: AtomicU64 = AtomicU64::new(0);
static FAST_PATH_CYCLES: AtomicU64 = AtomicU64::new(0);
/// Counter for slow-path fallbacks
static SLOW_PATH_FALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Get performance statistics (fast_path_count, avg_cycles)
pub fn get_fast_path_stats() -> (u64, u64) {
    let count = FAST_PATH_COUNT.load(Ordering::Relaxed);
    let cycles = FAST_PATH_CYCLES.load(Ordering::Relaxed);
    let avg_cycles = if count > 0 { cycles / count } else { 0 };
    (count, avg_cycles)
}

/// Get the number of slow-path fallbacks
pub fn get_slow_path_count() -> u64 {
    SLOW_PATH_FALLBACK_COUNT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_path_stats() {
        let (count, avg) = get_fast_path_stats();
        assert_eq!(count, 0);
        assert_eq!(avg, 0);
    }

    #[test]
    fn test_slow_path_count() {
        assert_eq!(get_slow_path_count(), 0);
    }
}
