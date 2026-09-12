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
    let snapshot = root;
    let before = store.node_count();
    root = store.insert(root, key(2048), 77);
    assert_eq!(store.get(snapshot, &key(2048)), Some(2048));
    assert_eq!(store.get(root, &key(2048)), Some(77));
    assert!(
        store.node_count() - before <= 257,
        "an update copies only a bounded key path"
    );
    assert_eq!(store.get(root, &key(0)), Some(0));
    assert_eq!(store.get(root, &key(4095)), Some(4095));
    assert_eq!(
        store.insert(root, key(2048), 77),
        root,
        "unchanged write is free"
    );
    let removed = store.remove(root, &key(2048));
    assert_eq!(store.get(removed, &key(2048)), None);
    assert_eq!(store.get(root, &key(2048)), Some(77));
    assert_eq!(store.remove(removed, &key(2048)), removed);
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
        assert_eq!(store.get(root, &k), oracle.get(&k).copied());
    }
    for prefix in 0..3 {
        let low = [prefix, 0, 0, 0];
        let high = [prefix, u64::MAX, u64::MAX, u64::MAX];
        let mut cursor = store.range(root, low, high);
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
    let snapshot = root;
    let mut cursor = store.range(snapshot, key(200), key(300));
    assert_eq!(cursor.next(&store), Some((key(200), 200)));
    for n in 0..1000 {
        root = store.insert(root, key(n), n + 1000);
    }
    let before = store.node_count();
    let mut gc = store.collect([root, cursor.root()].into_iter());
    let mut values = Vec::new();
    while !gc.done() {
        if let Some(pair) = gc.tick() {
            values.push(pair);
        }
    }
    drop(gc);
    assert!(store.node_count() < before / 2);
    assert_eq!(store.get(snapshot, &key(42)), Some(42));
    assert_eq!(store.get(root, &key(42)), Some(1042));
    assert_eq!(cursor.next(&store), Some((key(201), 201)));
    assert_eq!(
        values.len(),
        2000,
        "collector exposes each reachable leaf once"
    );
    let mut gc = store.collect(std::iter::empty());
    while !gc.done() {
        gc.tick();
    }
    drop(gc);
    assert_eq!(store.node_count(), 0);
    assert!(!store.contains(snapshot));
    assert!(!store.contains(root));
    let fresh = store.insert(store.empty(), key(0), 5);
    assert_ne!(fresh, root);
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
    let mut cursor = store.range(root, [u64::MAX; 4], [u64::MAX; 4]);
    assert_eq!(cursor.next(&store), Some(([u64::MAX; 4], 99)));
    assert_eq!(cursor.next(&store), None);
    assert!(
        cursor.visits() <= 514,
        "a keyed seek must not scan all rows"
    );
    assert_eq!(store.get(root, &[0, 0, 0, 1]), Some(100));
    assert_eq!(store.get(root, &[0, 0, 0, 0]), Some(0));
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
    let before = store.node_count();
    let changed = store.insert(root, zero, 500);
    assert!(store.node_count() - before <= 257);
    assert_eq!(store.get(root, &zero), Some(0));
    assert_eq!(store.get(changed, &zero), Some(500));
    let before = store.node_count();
    let removed = store.remove(changed, &zero);
    assert!(store.node_count() - before <= 256);
    assert_eq!(store.get(removed, &zero), None);
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
