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

/// Spend one disposal tick on an occupied slot. Finishing still consumes the tick.
pub(crate) fn discard_slot<T>(slot: &mut Option<T>, discard: impl FnOnce(&mut T) -> bool) -> bool {
    let Some(child) = slot.as_mut() else {
        return false;
    };
    if discard(child) {
        *slot = None;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::discard_slot;
    use std::cell::Cell;

    #[test]
    fn occupied_discard_spends_the_tick_even_when_it_finishes() {
        struct Child<'a> {
            remaining: usize,
            drops: &'a Cell<usize>,
        }
        impl Drop for Child<'_> {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }
        let drops = Cell::new(0);
        let mut first = Some(Child {
            remaining: 2,
            drops: &drops,
        });
        let mut second = Some(Child {
            remaining: 1,
            drops: &drops,
        });
        let mut tick = || {
            let discard = |child: &mut Child<'_>| {
                child.remaining -= 1;
                child.remaining == 0
            };
            if discard_slot(&mut first, discard) {
                return;
            }
            discard_slot(&mut second, discard);
        };
        tick();
        assert_eq!(drops.get(), 0);
        tick();
        assert_eq!(drops.get(), 1);
        tick();
        assert_eq!(drops.get(), 2);
        assert!(!discard_slot(&mut first, |_| panic!("empty slot called")));
        assert!(second.is_none());
    }
}
