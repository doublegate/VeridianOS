//! Shared-frame reference counts for copy-on-write (MEM-ARCH-02, PROC-ARCH-01).
//!
//! A frame normally has one owner (one mapping in one address space), and
//! that case costs nothing: only frames shared by a copy-on-write fork have
//! an entry here, holding the number of *extra* owners. Releasing a frame
//! drops one owner; the frame goes back to the allocator only when the last
//! one releases it.
//!
//! Lock order: the frame allocator's lock may be held while this one is
//! taken (release inside a free loop), never the other way round.

#[cfg(feature = "alloc")]
use alloc::collections::BTreeMap;

use spin::Mutex;

use super::FrameNumber;

/// Extra owners per shared frame (absent = exactly one owner).
#[cfg(feature = "alloc")]
static EXTRA_OWNERS: Mutex<BTreeMap<u64, u32>> = Mutex::new(BTreeMap::new());

/// Record one more owner of `frame` (a fork sharing it).
#[cfg(feature = "alloc")]
pub fn share(frame: FrameNumber) {
    *EXTRA_OWNERS.lock().entry(frame.as_u64()).or_insert(0) += 1;
}

/// Drop one owner of `frame`. Returns true when that was the last owner and
/// the caller must free the frame.
#[cfg(feature = "alloc")]
pub fn release(frame: FrameNumber) -> bool {
    let mut owners = EXTRA_OWNERS.lock();
    match owners.get_mut(&frame.as_u64()) {
        Some(extra) => {
            *extra -= 1;
            if *extra == 0 {
                owners.remove(&frame.as_u64());
            }
            false
        }
        None => true,
    }
}

/// Whether `frame` has more than one owner.
#[cfg(feature = "alloc")]
pub fn is_shared(frame: FrameNumber) -> bool {
    EXTRA_OWNERS.lock().contains_key(&frame.as_u64())
}

/// Number of frames with more than one owner (diagnostics, tests).
#[cfg(feature = "alloc")]
pub fn shared_frames() -> usize {
    EXTRA_OWNERS.lock().len()
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    #[test]
    fn last_owner_frees() {
        // Frame numbers well away from any other test's.
        let f = FrameNumber::new(0xF_0000_0001);
        assert!(!is_shared(f));
        share(f); // two owners
        share(f); // three owners
        assert!(is_shared(f));
        assert!(!release(f));
        assert!(!release(f));
        assert!(!is_shared(f));
        assert!(release(f), "the last owner frees the frame");
        // An unshared frame is freed by its only owner.
        assert!(release(FrameNumber::new(0xF_0000_0002)));
    }
}
