use chr::store::{Key, Store};
use std::collections::BTreeMap;

fn key(n: u64) -> Key {
    [0, 0, 0, n]
}

#[test]
fn roots_share_unchanged_paths_and_preserve_old_values() {
    let mut store = Store::default();
    let mut root = store.empty();
    for n in 0..4096 {
        root = store.insert(root, key(n), n);
    }
    let snapshot = root.clone();
    let before = store.node_count();
    root = store.insert(root, key(2048), 77);
    assert_eq!(store.get(&snapshot, &key(2048)), Some(2048));
    assert_eq!(store.get(&root, &key(2048)), Some(77));
    assert!(
        store.allocations() - before <= 257,
        "an update copies only a bounded key path"
    );
    assert_eq!(store.get(&root, &key(0)), Some(0));
    assert_eq!(store.get(&root, &key(4095)), Some(4095));
    assert_eq!(
        store.insert(root.clone(), key(2048), 77),
        root,
        "unchanged write is free"
    );
    let removed = store.remove(root.clone(), &key(2048));
    assert_eq!(store.get(&removed, &key(2048)), None);
    assert_eq!(store.get(&root, &key(2048)), Some(77));
    assert_eq!(store.remove(removed.clone(), &key(2048)), removed);
}

#[test]
fn mixed_updates_and_prefix_ranges_match_an_ordered_map() {
    let mut store = Store::default();
    let mut root = store.empty();
    let mut oracle = BTreeMap::new();
    let mut random = 17_u64;
    for n in 0..4000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let k = [
            random % 3,
            (random >> 8) % 7,
            (random >> 16) % 5,
            (random >> 32) % 101,
        ];
        if n % 4 == 0 {
            root = store.remove(root, &k);
            oracle.remove(&k);
        } else {
            root = store.insert(root, k, n);
            oracle.insert(k, n);
        }
        assert_eq!(store.get(&root, &k), oracle.get(&k).copied());
    }
    for prefix in 0..3 {
        let low = [prefix, 0, 0, 0];
        let high = [prefix, u64::MAX, u64::MAX, u64::MAX];
        let mut cursor = store.range(root.clone(), low, high);
        let mut result = Vec::new();
        while let Some(pair) = cursor.next(&store) {
            result.push(pair);
        }
        assert_eq!(
            result,
            oracle
                .range(low..=high)
                .map(|(&k, &v)| (k, v))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn collection_preserves_snapshot_and_cursor_roots_and_reclaims_dead_versions() {
    let mut store = Store::default();
    let mut root = store.empty();
    for n in 0..1000 {
        root = store.insert(root, key(n), n);
    }
    let snapshot = root.clone();
    let mut cursor = store.range(snapshot.clone(), key(200), key(300));
    assert_eq!(cursor.next(&store), Some((key(200), 200)));
    for n in 0..1000 {
        root = store.insert(root, key(n), n + 1000);
    }
    let before = store.node_count();
    let mut gc = store.collect([root.clone(), cursor.root()].into_iter());
    let mut values = Vec::new();
    while !gc.done() {
        if let Some(pair) = gc.tick(&mut store) {
            values.push(pair);
        }
    }
    drop(gc);
    assert_eq!(store.node_count(), 3998); // two live 1000-leaf snapshots; unique updates leave no arena garbage
    assert!(store.node_count() <= before);
    assert_eq!(store.get(&snapshot, &key(42)), Some(42));
    assert_eq!(store.get(&root, &key(42)), Some(1042));
    assert_eq!(cursor.next(&store), Some((key(201), 201)));
    assert_eq!(
        values.len(),
        2000,
        "collector exposes each reachable leaf once"
    );
    let mut gc = store.collect(std::iter::empty());
    while !gc.done() {
        gc.tick(&mut store);
    }
    drop(gc);
    assert!(!store.contains(&snapshot));
    assert!(!store.contains(&root));
    let fresh = store.insert(store.empty(), key(0), 5);
    assert_ne!(fresh, root);
    drop(fresh);
    drop(snapshot);
    drop(root);
    drop(cursor);
    while !store.release_tick() {}
    assert_eq!(store.node_count(), 0);
}

#[test]
fn collection_rejects_foreign_and_reclaimed_roots_before_marking() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut store = Store::default();
    let live = store.insert(store.empty(), key(1), 10);
    let mut foreign = Store::default();
    let foreign_root = foreign.insert(foreign.empty(), key(1), 20);
    let mut gc = store.collect([foreign_root].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut store))).is_err());
    drop(gc);
    assert_eq!(store.get(&live, &key(1)), Some(10));
    let mut gc = store.collect(std::iter::empty());
    while !gc.done() {
        gc.tick(&mut store);
    }
    drop(gc);
    let mut gc = store.collect([live].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut store))).is_err());
    drop(gc);
    let fresh = store.insert(store.empty(), key(1), 30);
    assert_eq!(store.get(&fresh, &key(1)), Some(30));
}

#[test]
fn boundary_bits_and_sparse_range_seek_do_not_scan_unrelated_rows() {
    let mut store = Store::default();
    let mut root = store.empty();
    for n in 0..20_000 {
        root = store.insert(root, [0, 0, n, 0], n);
    }
    root = store.insert(root, [u64::MAX; 4], 99);
    root = store.insert(root, [0, 0, 0, 1], 100);
    let mut cursor = store.range(root.clone(), [u64::MAX; 4], [u64::MAX; 4]);
    assert_eq!(cursor.next(&store), Some(([u64::MAX; 4], 99)));
    assert_eq!(cursor.next(&store), None);
    assert!(
        cursor.visits() <= 514,
        "a keyed seek must not scan all rows"
    );
    assert_eq!(store.get(&root, &[0, 0, 0, 1]), Some(100));
    assert_eq!(store.get(&root, &[0, 0, 0, 0]), Some(0));
}

#[test]
fn a_maximal_key_path_remains_bounded_under_update_and_deletion() {
    let mut store = Store::default();
    let zero = [0; 4];
    let mut root = store.insert(store.empty(), zero, 0);
    for bit in 0..256 {
        let mut key = zero;
        key[bit / 64] = 1_u64 << (63 - bit % 64);
        root = store.insert(root, key, bit + 1);
    }
    let before = store.allocations();
    let changed = store.insert(root.clone(), zero, 500);
    assert!(store.allocations() - before <= 257);
    assert_eq!(store.get(&root, &zero), Some(0));
    assert_eq!(store.get(&changed, &zero), Some(500));
    let before = store.allocations();
    let removed = store.remove(changed, &zero);
    assert!(store.allocations() - before <= 256);
    assert_eq!(store.get(&removed, &zero), None);
    let mut cursor = store.range(removed, [0; 4], [u64::MAX; 4]);
    let mut previous = zero;
    let mut count = 0;
    while let Some((key, _)) = cursor.next(&store) {
        assert!(key > previous);
        previous = key;
        count += 1;
    }
    assert_eq!(count, 256);
}

#[test]
fn owned_collection_freezes_mutators_and_drop_releases_the_owner() {
    use chr::store::{Collector as GenericCollector, Root as GenericRoot};
    type Root = GenericRoot<u64>;
    type Collector<I> = GenericCollector<I, u64>;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn begin(store: &mut Store<u64>, root: Root) -> Collector<std::array::IntoIter<Root, 1>> {
        store.collect([root].into_iter())
    }
    for cutoff in 0..32 {
        let mut store = Store::default();
        let root = store.insert(store.empty(), key(1), 10);
        store.insert(root.clone(), key(2), 20);
        let mut gc = begin(&mut store, root.clone());
        let mut foreign = Store::<u64>::default();
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(
                || store.collect([root.clone()].into_iter())
            ))
            .is_err()
        );
        for _ in 0..cutoff {
            gc.tick(&mut store);
        }
        assert_eq!(store.get(&root, &key(1)), Some(10));
        let nodes = store.node_count();
        assert!(catch_unwind(AssertUnwindSafe(|| store.insert(root.clone(), key(3), 30))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| store.remove(root.clone(), &key(1)))).is_err());
        assert_eq!(store.node_count(), nodes);
        drop(gc);
        let updated = store.insert(root, key(3), 30);
        let mut gc = begin(&mut store, updated);
        let mut values = Vec::new();
        while !gc.done() {
            if let Some(pair) = gc.tick(&mut store) {
                values.push(pair);
            }
        }
        assert_eq!(gc.tick(&mut store), None);
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        drop(gc);
        values.sort_unstable();
        assert_eq!(values, [(key(1), 10), (key(3), 30)]);
    }
}

fn full_width_rows() -> BTreeMap<Key, u64> {
    let mut rows = BTreeMap::new();
    let mut random = 91_u64;
    for value in 0..512 {
        let mut key = [0; 4];
        for word in &mut key {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            *word = random;
        }
        rows.insert(key, value);
    }
    rows.insert([0; 4], 512);
    rows.insert([u64::MAX; 4], 513);
    rows.insert([0, 0, 0, 1], 514);
    rows.insert([1 << 63, 0, 0, 0], 515);
    rows
}

fn snapshot_rows(store: &Store<u64>, root: chr::store::Root<u64>) -> BTreeMap<Key, u64> {
    let mut cursor = store.range(root, [0; 4], [u64::MAX; 4]);
    std::iter::from_fn(|| cursor.next(store)).collect()
}

fn filter_rows(
    store: &mut Store<u64>,
    root: chr::store::Root<u64>,
    mut replace: impl FnMut(Key, u64) -> Option<u64>,
) -> (chr::store::Root<u64>, Vec<Key>, usize) {
    use chr::store::FilterStatus;
    let mut filter = store.filter(root);
    let mut visited = Vec::new();
    let before = store.node_count();
    let mut ticks = 0;
    loop {
        ticks += 1;
        assert!(ticks <= 8 * (visited.len() + 257), "bounded traversal work");
        let count = store.node_count();
        let status = filter.tick(store);
        assert!(store.node_count() - count <= 1, "one allocation per tick");
        match status {
            FilterStatus::Pending => {}
            FilterStatus::Leaf { key, value } => {
                visited.push(key);
                filter.replace(replace(key, value));
            }
            FilterStatus::Complete(result) => {
                assert_eq!(filter.tick(store), FilterStatus::Complete(result.clone()));
                assert_eq!(filter.values().count(), 0);
                assert!(filter.roots().all(|r| r == result || r == store.empty()));
                return (result, visited, store.node_count() - before);
            }
        }
    }
}

#[test]
fn filter_full_width_keys_reuses_noop_and_rebuilds_each_changed_node_once() {
    let original = full_width_rows();
    let mut store = Store::default();
    let mut root = store.empty();
    for (&key, &value) in &original {
        root = store.insert(root, key, value);
    }
    let (same, visited, allocations) =
        filter_rows(&mut store, root.clone(), |_, value| Some(value));
    assert_eq!(same, root);
    assert_eq!(allocations, 0);
    assert_eq!(visited, original.keys().copied().collect::<Vec<_>>());

    let old_nodes = 2 * original.len() - 1;
    let (changed, _, allocations) =
        filter_rows(&mut store, root.clone(), |_, value| Some(value + 1000));
    assert_eq!(
        allocations, old_nodes,
        "exactly one new copy of each changed leaf and branch"
    );
    assert_eq!(snapshot_rows(&store, root.clone()), original);
    assert_eq!(
        snapshot_rows(&store, changed),
        original.iter().map(|(&k, &v)| (k, v + 1000)).collect()
    );

    let (subset, _, allocations) =
        filter_rows(&mut store, root.clone(), |_, value| match value % 3 {
            0 => None,
            1 => Some(value),
            _ => Some(value + 2000),
        });
    assert!(allocations <= old_nodes);
    assert_eq!(
        snapshot_rows(&store, subset),
        original
            .iter()
            .filter_map(|(&k, &v)| match v % 3 {
                0 => None,
                1 => Some((k, v)),
                _ => Some((k, v + 2000)),
            })
            .collect()
    );
    assert_eq!(snapshot_rows(&store, root.clone()), original);
    let (empty, _, allocations) = filter_rows(&mut store, root, |_, _| None);
    assert_eq!(empty, store.empty());
    assert_eq!(allocations, 0);
    let (empty_again, visited, allocations) =
        filter_rows(&mut store, empty.clone(), |_, _| unreachable!());
    assert_eq!(empty_again, empty);
    assert!(visited.is_empty());
    assert_eq!(allocations, 0);
}

#[test]
fn filter_collapses_to_the_existing_child_without_allocating() {
    let mut store = Store::default();
    let child = store.insert(store.empty(), [0; 4], 1);
    let root = store.insert(child.clone(), [u64::MAX; 4], 2);
    let (result, _, allocations) = filter_rows(&mut store, root.clone(), |_, value| {
        (value == 1).then_some(value)
    });
    assert_eq!(result, child);
    assert_eq!(allocations, 0);
    assert_eq!(store.get(&root, &[u64::MAX; 4]), Some(2));
}

#[test]
fn filter_survives_gc_between_every_transition_and_traces_pending_values() {
    use chr::store::FilterStatus;
    let rows: BTreeMap<_, _> = full_width_rows().into_iter().take(48).collect();
    let mut store = Store::default();
    let mut root = store.empty();
    for (&key, &value) in &rows {
        root = store.insert(root, key, value);
    }
    let mut filter = store.filter(root);
    let collect = |store: &mut Store<u64>, filter: &chr::store::Filter<u64>| {
        let mut gc = store.collect(filter.roots());
        while !gc.done() {
            gc.tick(store);
        }
    };
    collect(&mut store, &filter);
    let result = loop {
        let status = filter.tick(&mut store);
        collect(&mut store, &filter);
        match status {
            FilterStatus::Pending => {}
            FilterStatus::Leaf { key: _, value } => {
                assert_eq!(filter.values().count(), 0);
                let replacement = (value % 3 != 0).then_some(value + 1000);
                filter.replace(replacement);
                assert_eq!(
                    filter.values().collect::<Vec<_>>(),
                    replacement.into_iter().collect::<Vec<_>>()
                );
                collect(&mut store, &filter);
            }
            FilterStatus::Complete(root) => break root,
        }
    };
    let expected: BTreeMap<_, _> = rows
        .into_iter()
        .filter_map(|(k, v)| (v % 3 != 0).then_some((k, v + 1000)))
        .collect();
    assert_eq!(snapshot_rows(&store, result), expected);
    assert_eq!(
        store.node_count(),
        2 * expected.len() - 1,
        "completed filter retains only its resulting tree"
    );
}

#[test]
fn filter_protocol_owner_freeze_and_stale_checks_precede_progress() {
    use chr::store::{Filter, FilterStatus, Root};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    fn begin(store: &Store<u64>, root: Root<u64>) -> Filter<u64> {
        store.filter(root)
    }
    let mut store = Store::default();
    let root = store.insert(store.empty(), key(1), 10);
    let mut filter = begin(&store, root.clone());
    let mut foreign = Store::<u64>::default();
    assert!(catch_unwind(AssertUnwindSafe(|| foreign.filter(root.clone()))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| filter.replace(None))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| filter.tick(&mut foreign))).is_err());
    assert_eq!(
        filter.tick(&mut store),
        FilterStatus::Leaf {
            key: key(1),
            value: 10
        }
    );
    assert!(catch_unwind(AssertUnwindSafe(|| filter.tick(&mut store))).is_err());
    filter.replace(Some(20));
    assert!(catch_unwind(AssertUnwindSafe(|| filter.replace(None))).is_err());
    assert_eq!(filter.values().collect::<Vec<_>>(), [20]);
    let mut gc = store.collect(filter.roots().collect::<Vec<_>>().into_iter());
    let before = store.node_count();
    assert!(catch_unwind(AssertUnwindSafe(|| filter.tick(&mut store))).is_err());
    assert_eq!(store.node_count(), before);
    assert_eq!(filter.values().collect::<Vec<_>>(), [20]);
    gc.tick(&mut store);
    drop(gc); // A cancelled collector releases the filter's owner too.
    let FilterStatus::Complete(result) = filter.tick(&mut store) else {
        panic!("leaf completes")
    };
    assert_eq!(store.get(&result, &key(1)), Some(20));
    assert_eq!(store.get(&root, &key(1)), Some(10));
    assert!(catch_unwind(AssertUnwindSafe(|| filter.replace(None))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| filter.tick(&mut foreign))).is_err());
    assert_eq!(filter.tick(&mut store), FilterStatus::Complete(result));
    let mut stale = store.filter(root.clone());
    let mut gc = store.collect(std::iter::empty());
    while !gc.done() {
        gc.tick(&mut store);
    }
    drop(gc);
    assert!(catch_unwind(AssertUnwindSafe(|| store.filter(root))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| stale.tick(&mut store))).is_err());
    drop(stale);
    drop(filter);
    while !store.release_tick() {}
    assert_eq!(store.node_count(), 0);
    let mut empty = store.filter(store.empty());
    assert!(catch_unwind(AssertUnwindSafe(|| empty.tick(&mut foreign))).is_err());
}
