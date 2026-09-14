use chr::store::{FilterStatus, Store};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get().map(|n| n + 1)));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        COUNT.with(|c| c.set(c.get().map(|n| n + 1)));
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations<T>(f: impl FnOnce() -> T) -> (T, usize) {
    COUNT.with(|c| c.set(Some(0)));
    let result = f();
    let count = COUNT.with(|c| c.replace(None).unwrap());
    (result, count)
}

// Only the leaf-to-page promotion may allocate a payload; the following
// bounded insertions/removals must reuse it.
#[test]
fn unique_dense_updates_allocate_only_the_page_promotion() {
    let mut store = Store::default();
    let mut root = store.insert(store.empty(), [1, 2, 3, 7], 7_u64);
    let ((), count) = allocations(|| {
        for n in (0..7).rev() {
            root = store.insert(std::mem::take(&mut root), [1, 2, 3, n], n);
        }
        for n in 0..7 {
            root = store.remove(std::mem::take(&mut root), &[1, 2, 3, n]);
        }
    });
    assert_eq!(count, 1, "only leaf-to-page promotion allocates a payload");
    assert_eq!(store.get(&root, &[1, 2, 3, 7]), Some(7));
    drop(root);
    while !store.release_tick() {}
    assert_eq!(store.node_count(), 0);
}

#[test]
fn identical_page_values_keep_distinct_keys_and_active_runs_reject_invalid_roots() {
    for cutoff in 0..8 {
        let mut store = Store::default();
        let mut root = store.empty();
        for n in 0..16 {
            root = store.insert(root, [0, 0, 0, n], 7_u64);
        }
        let mut cursor = store.range(root.clone(), [0; 4], [u64::MAX; 4]);
        for n in 0..cutoff {
            assert_eq!(cursor.next(&store), Some(([0, 0, 0, n], 7)));
        }
        let foreign = Store::<u64>::default();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cursor.next(&foreign)))
                .is_err()
        );
        let mut gc = store.collect([cursor.root()].into_iter());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        for n in cutoff..16 {
            assert_eq!(cursor.next(&store), Some(([0, 0, 0, n], 7)));
        }
        assert_eq!(cursor.next(&store), None);
        let mut stale = store.range(root.clone(), [0; 4], [u64::MAX; 4]);
        assert_eq!(stale.next(&store), Some(([0, 0, 0, 0], 7)));
        let mut gc = store.collect(std::iter::empty());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stale.next(&store))).is_err()
        );
        drop((cursor, stale, root));
        while !store.release_tick() {}
        assert_eq!(store.node_count(), 0);
    }
}

// Vec COW clones only occupied entries and then reallocates to append. A fixed
// page must copy its record and payload just once, preserving the pinned root.
#[test]
fn shared_page_append_does_not_reallocate_the_copied_payload() {
    let mut store = Store::default();
    let mut root = store.empty();
    for n in 0..2 {
        root = store.insert(root, [0, 0, 0, n], n);
    }
    let old = root.clone();
    let (root, count) = allocations(|| store.insert(root, [0, 0, 0, 2], 2));
    assert!(count <= 2, "record and fixed payload suffice, got {count}");
    assert_eq!(store.get(&old, &[0, 0, 0, 2]), None);
    assert_eq!(store.count(&old, [0; 4], [u64::MAX; 4]), 2);
    assert_eq!(store.count(&root, [0; 4], [u64::MAX; 4]), 3);
    drop((root, old));
    while !store.release_tick() {}
    assert_eq!(store.node_count(), 0);
}

// Cursor transport must neither allocate a traversal Vec for a single page,
// nor lose boundary entries, frozen snapshots, or the no-op root witness.
#[test]
fn page_range_transport_is_allocation_free_and_keeps_snapshot_identity() {
    let mut store = Store::default();
    let mut root = store.empty();
    for n in 0..8 {
        root = store.insert(root, [0, 0, 0, n], n);
    }
    let old = root.clone();
    root = store.insert(root, [0, 0, 0, 3], 33);
    let ((rows, visits), count) = allocations(|| {
        let mut cursor = store.range(old.clone(), [0, 0, 0, 2], [0, 0, 0, 5]);
        let rows = std::array::from_fn::<_, 5, _>(|_| cursor.next(&store));
        (rows, cursor.visits())
    });
    assert_eq!(
        rows,
        [
            Some(([0, 0, 0, 2], 2)),
            Some(([0, 0, 0, 3], 3)),
            Some(([0, 0, 0, 4], 4)),
            Some(([0, 0, 0, 5], 5)),
            None
        ]
    );
    assert_eq!(count, 0, "a page run must not allocate a traversal buffer");
    assert_eq!(visits, 1);
    assert_eq!(store.count(&old, [0, 0, 0, 2], [0, 0, 0, 5]), 4);
    assert_eq!(store.insert(root.clone(), [0, 0, 0, 3], 33), root);
    let mut filter = store.filter(root.clone());
    loop {
        match filter.tick(&mut store) {
            FilterStatus::Leaf { value, .. } => filter.replace(Some(value)),
            FilterStatus::Complete(result) => {
                assert_eq!(result, root);
                break;
            }
            FilterStatus::Pending => {}
        }
    }
    drop((filter, old, root));
    while !store.release_tick() {}
    assert_eq!(store.node_count(), 0);
}

#[test]
fn interrupted_page_ranges_and_filters_release_every_owner() {
    for cutoff in 0..80 {
        let mut store = Store::default();
        let mut root = store.empty();
        for n in 0..32 {
            root = store.insert(root, [0, 0, 0, n], n);
        }
        let mut cursor = store.range(root.clone(), [0; 4], [u64::MAX; 4]);
        let mut filter = store.filter(root.clone());
        for _ in 0..cutoff {
            cursor.next(&store);
            match filter.tick(&mut store) {
                FilterStatus::Leaf { value, .. } => {
                    filter.replace((value % 2 == 0).then_some(value + 100))
                }
                FilterStatus::Complete(_) => break,
                FilterStatus::Pending => {}
            }
            let mut gc = store.collect(filter.roots().chain([cursor.root()]));
            while !gc.done() {
                gc.tick(&mut store);
            }
        }
        drop((cursor, filter, root));
        while !store.release_tick() {}
        assert_eq!(store.node_count(), 0, "cutoff {cutoff}");
    }
}
