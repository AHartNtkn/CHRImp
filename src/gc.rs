//! Stop-the-mutator leases held by owned collection continuations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct GcLease(Arc<AtomicBool>);

impl GcLease {
    pub(crate) fn acquire(frozen: &Arc<AtomicBool>) -> Self {
        assert!(
            frozen
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
            "collection already active"
        );
        Self(Arc::clone(frozen))
    }

    pub(crate) fn assert_mutable(frozen: &Arc<AtomicBool>) {
        assert!(
            !frozen.load(Ordering::Acquire),
            "owner is frozen for collection"
        );
    }
}

impl Drop for GcLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
