//! Optional requested-allocation accounting. Categories identify the code doing
//! an allocation/free, not the owner of the resulting object. Live/peak bytes
//! cover Rust global-allocator requests across the process (including the harness);
//! direct foreign allocations are outside this counter. These are not RSS or the
//! allocator's internal arena size. Reallocation records logical old/new sizes.
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Phase {
    Setup = 1,
    Engine,
    Validator,
    Delivery,
    Cleanup,
    Inspection,
    #[cfg(all(test, feature = "diagnostics"))]
    Calibration,
}

#[inline]
pub fn during<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    #[cfg(feature = "diagnostics")]
    let _scope = enabled::Scope::new(phase);
    #[cfg(not(feature = "diagnostics"))]
    let _ = phase;
    f()
}

#[cfg(feature = "diagnostics")]
mod enabled {
    use super::Phase;
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
        sync::atomic::{AtomicU64, Ordering::Relaxed},
    };
    const NAMES: [&str; 8] = [
        "other",
        "setup",
        "engine",
        "validator",
        "delivery",
        "cleanup",
        "inspection",
        "calibration",
    ];
    thread_local! { static PHASE: Cell<usize> = const { Cell::new(0) }; }
    struct Counters {
        allocations: AtomicU64,
        deallocations: AtomicU64,
        allocated: AtomicU64,
        freed: AtomicU64,
    }
    impl Counters {
        const fn new() -> Self {
            Self {
                allocations: AtomicU64::new(0),
                deallocations: AtomicU64::new(0),
                allocated: AtomicU64::new(0),
                freed: AtomicU64::new(0),
            }
        }
    }
    static COUNTERS: [Counters; 8] = [const { Counters::new() }; 8];
    static LIVE: AtomicU64 = AtomicU64::new(0);
    static PEAK: AtomicU64 = AtomicU64::new(0);
    pub(super) struct Scope(usize);
    impl Scope {
        pub(super) fn new(phase: Phase) -> Self {
            Self(PHASE.with(|p| p.replace(phase as usize)))
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            let _ = PHASE.try_with(|p| p.set(self.0));
        }
    }
    fn counters() -> &'static Counters {
        &COUNTERS[PHASE.try_with(Cell::get).unwrap_or(0)]
    }
    fn allocated(size: usize) {
        let c = counters();
        c.allocations.fetch_add(1, Relaxed);
        c.allocated.fetch_add(size as u64, Relaxed);
        let live = LIVE.fetch_add(size as u64, Relaxed) + size as u64;
        PEAK.fetch_max(live, Relaxed);
    }
    fn freed(size: usize) {
        let c = counters();
        c.deallocations.fetch_add(1, Relaxed);
        c.freed.fetch_add(size as u64, Relaxed);
        LIVE.fetch_sub(size as u64, Relaxed);
    }
    pub(super) struct Allocator;
    // Each call delegates the same pointer/layout contract to System. Recording
    // uses fixed atomics and constant TLS; it does not recursively allocate.
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { System.alloc(layout) };
            if !p.is_null() {
                allocated(layout.size());
            }
            p
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { System.alloc_zeroed(layout) };
            if !p.is_null() {
                allocated(layout.size());
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            unsafe { System.dealloc(p, layout) };
            freed(layout.size());
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(p, layout, size) };
            if !result.is_null() {
                freed(layout.size());
                allocated(size);
            }
            result
        }
    }
    #[derive(Clone, Copy, serde::Serialize)]
    pub struct Counts {
        pub phase: &'static str,
        pub allocations: u64,
        pub deallocations: u64,
        pub allocated_bytes: u64,
        pub freed_bytes: u64,
    }
    #[derive(Clone, Copy, serde::Serialize)]
    pub struct Snapshot {
        pub phases: [Counts; 8],
        pub process_live_requested_bytes: u64,
        pub process_peak_requested_bytes: u64,
    }
    pub fn snapshot() -> Snapshot {
        Snapshot {
            phases: std::array::from_fn(|i| {
                let c = &COUNTERS[i];
                Counts {
                    phase: NAMES[i],
                    allocations: c.allocations.load(Relaxed),
                    deallocations: c.deallocations.load(Relaxed),
                    allocated_bytes: c.allocated.load(Relaxed),
                    freed_bytes: c.freed.load(Relaxed),
                }
            }),
            process_live_requested_bytes: LIVE.load(Relaxed),
            process_peak_requested_bytes: PEAK.load(Relaxed),
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn counts_real_allocations_reallocation_and_nested_scopes() {
            // Isolate process-wide live/peak assertions from concurrent tests.
            const CHILD: &str = "CHR_ALLOCATION_CALIBRATION_CHILD";
            if std::env::var_os(CHILD).is_none() {
                let result = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "allocation::enabled::tests::counts_real_allocations_reallocation_and_nested_scopes", "--nocapture"])
                    .env(CHILD, "1").output().unwrap();
                assert!(
                    result.status.success(),
                    "{}{}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                return;
            }
            // Exercise the installed global allocator, including a failed realloc.
            let before_global = snapshot();
            let layout = Layout::from_size_align(4096, 8).unwrap();
            // Force an observable allocation across checkpoints: allocator calls for
            // unused storage may legally disappear in optimized Rust builds.
            let p = std::hint::black_box(super::super::during(Phase::Setup, || unsafe {
                std::alloc::alloc(std::hint::black_box(layout))
            }));
            assert!(!p.is_null());
            let allocated_global = snapshot();
            assert_eq!(
                allocated_global.process_live_requested_bytes,
                before_global.process_live_requested_bytes + 4096
            );
            assert_eq!(
                allocated_global.process_peak_requested_bytes,
                before_global
                    .process_peak_requested_bytes
                    .max(before_global.process_live_requested_bytes + 4096)
            );
            assert_eq!(
                allocated_global.phases[Phase::Setup as usize].allocated_bytes,
                before_global.phases[Phase::Setup as usize].allocated_bytes + 4096
            );
            let failed = unsafe { std::alloc::realloc(p, layout, (isize::MAX as usize) - 4095) };
            assert!(failed.is_null());
            let after_failed = snapshot();
            assert_eq!(
                after_failed.process_live_requested_bytes,
                allocated_global.process_live_requested_bytes
            );
            assert_eq!(
                after_failed
                    .phases
                    .iter()
                    .map(|c| c.allocated_bytes)
                    .sum::<u64>(),
                allocated_global
                    .phases
                    .iter()
                    .map(|c| c.allocated_bytes)
                    .sum::<u64>()
            );
            super::super::during(Phase::Cleanup, || unsafe { std::alloc::dealloc(p, layout) });
            let released_global = snapshot();
            assert_eq!(
                released_global.process_live_requested_bytes,
                before_global.process_live_requested_bytes
            );
            assert_eq!(
                released_global.phases[Phase::Cleanup as usize].freed_bytes,
                before_global.phases[Phase::Cleanup as usize].freed_bytes + 4096
            );
            assert_eq!(
                released_global.process_peak_requested_bytes,
                allocated_global.process_peak_requested_bytes
            );
            let original = PHASE.with(Cell::get);
            let unwound = std::panic::catch_unwind(|| {
                super::super::during(Phase::Validator, || panic!("scope calibration"))
            });
            assert!(unwound.is_err());
            assert_eq!(PHASE.with(Cell::get), original);

            let before = snapshot();
            let before_other = PHASE.with(Cell::get);
            {
                let _scope = Scope::new(Phase::Calibration);
                unsafe {
                    let layout = Layout::from_size_align(64, 8).unwrap();
                    let p = Allocator.alloc(layout);
                    assert!(!p.is_null());
                    p.write_volatile(42);
                    let p = Allocator.realloc(p, layout, 128);
                    assert!(!p.is_null());
                    assert_eq!(p.read_volatile(), 42);
                    Allocator.dealloc(p, Layout::from_size_align(128, 8).unwrap());
                    let layout = Layout::from_size_align(32, 8).unwrap();
                    let p = Allocator.alloc_zeroed(layout);
                    assert!(!p.is_null());
                    assert_eq!(p.read_volatile(), 0);
                    Allocator.dealloc(p, layout);
                }
                {
                    let _nested = Scope::new(Phase::Engine);
                    assert_eq!(PHASE.with(Cell::get), Phase::Engine as usize);
                }
                assert_eq!(PHASE.with(Cell::get), Phase::Calibration as usize);
            }
            assert_eq!(PHASE.with(Cell::get), before_other);
            let after = snapshot();
            let i = Phase::Calibration as usize;
            assert_eq!(
                after.phases[i].allocations - before.phases[i].allocations,
                3
            );
            assert_eq!(
                after.phases[i].deallocations - before.phases[i].deallocations,
                3
            );
            assert_eq!(
                after.phases[i].allocated_bytes - before.phases[i].allocated_bytes,
                224
            );
            assert_eq!(
                after.phases[i].freed_bytes - before.phases[i].freed_bytes,
                224
            );
        }
    }
}
#[cfg(feature = "diagnostics")]
#[global_allocator]
static ALLOCATOR: enabled::Allocator = enabled::Allocator;
#[cfg(feature = "diagnostics")]
pub use enabled::snapshot;
