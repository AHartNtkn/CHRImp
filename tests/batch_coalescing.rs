#![cfg(feature = "diagnostics")]

use chr::store::{BatchDiagnostics, Key, Root, Store};
use std::collections::BTreeMap;

#[allow(dead_code)]
#[path = "../examples/measure/allocation.rs"]
mod allocation;
use allocation::{Phase, during, snapshot};

fn key(i: usize, sparse: bool) -> Key {
    let n = i as u64;
    if sparse {
        // Differences in each word, including high bits: never a truncated key.
        let mut k = [u64::MAX; 4];
        k[i % 4] = n.rotate_left(33);
        k
    } else {
        [0, 0, 0, n]
    }
}

fn rows(store: &Store<u64>, root: Root<u64>) -> Vec<(Key, u64)> {
    let mut cursor = store.range(root, [0; 4], [u64::MAX; 4]);
    std::iter::from_fn(|| cursor.next(store)).collect()
}

// A wrong-direction pass, overwriting unread input during compaction, forgetting
// a final no-op in the seen set, or comparing partial keys must fail the forward
// ordered-map oracle. Old snapshots, cursor pins, and deferred release are checked.
fn probe(n: usize, distinct: usize, sparse: bool, mode: &str) -> BatchDiagnostics {
    let start = snapshot();
    let (mut store, root, original, expected, mut writes, retained) = during(Phase::Setup, || {
        let mut store = Store::default();
        let mut root = store.empty();
        let mut original = BTreeMap::new();
        for i in 0..distinct {
            if mode != "insert" {
                let k = key(i, sparse);
                original.insert(k, i as u64);
                root = store.insert(root, k, i as u64);
            }
        }
        while !store.release_tick() {}
        let mut writes = Vec::new();
        for i in 0..n {
            let id = (n - 1 - i) % distinct;
            let k = key(id, sparse);
            let value = match mode {
                "delete" => None,
                "noop" if i + distinct >= n => original.get(&k).copied(),
                "insert" => Some(i as u64 + 1000),
                _ if i % 3 == 0 => None,
                _ => Some(i as u64 + 1000),
            };
            writes.push((k, value));
        }
        let mut expected = original.clone();
        let mut last = BTreeMap::new();
        for &(k, v) in &writes {
            last.insert(k, v);
            match v {
                Some(v) => {
                    expected.insert(k, v);
                }
                None => {
                    expected.remove(&k);
                }
            }
        }
        let retained: Vec<_> = last
            .into_iter()
            .filter(|(k, v)| original.get(k).copied() != *v)
            .collect();
        (store, root, original, expected, writes, retained)
    });
    let pin = root.clone();
    let mut reader = store.range(pin.clone(), [0; 4], [u64::MAX; 4]);
    let prepared = snapshot();
    let pages_before = store.page_counts();
    let nodes_before = store.allocations();
    let root = during(Phase::Engine, || store.batch(root, &mut writes));
    let updated = snapshot();
    let d = store.batch_diagnostics();
    let pages_after = store.page_counts();
    let mutations = store.mutation_counts();
    let nodes_after = store.allocations();
    during(Phase::Validator, || {
        let want: Vec<_> = expected.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(rows(&store, root.clone()), want);
        assert_eq!(store.count(&root, [0; 4], [u64::MAX; 4]), want.len());
        assert_eq!(&writes[..retained.len()], &retained);
        assert_eq!(d.retained_writes, retained.len());
        if retained.is_empty() {
            assert_eq!(root, pin, "final no-ops must preserve the exact root");
            assert_eq!(nodes_before, nodes_after);
        }
        let mut gc = store.collect([root.clone(), reader.root()].into_iter());
        while !gc.done() {
            gc.tick(&mut store);
        }
        drop(gc);
        let got: Vec<_> = std::iter::from_fn(|| reader.next(&store)).collect();
        let old: Vec<_> = original.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(got, old);
        assert_eq!(rows(&store, pin.clone()), old);
    });
    let validated = snapshot();
    let cleanup_ticks = during(Phase::Cleanup, || {
        drop((root, pin, reader));
        let mut ticks = 0;
        while store.release_pending() {
            let before = store.node_count();
            store.release_tick();
            assert!(before - store.node_count() <= 2);
            ticks += 1;
        }
        assert_eq!(store.node_count(), 0);
        ticks
    });
    let released = snapshot();
    // Save exact semantic vectors, but serialize only after the measured drops.
    let semantic = during(Phase::Validator, || {
        serde_json::json!({
            "original": original.iter().collect::<Vec<_>>(),
            "final": expected.iter().collect::<Vec<_>>(), "ordered_writes": retained,
        })
        .to_string()
    });
    during(Phase::Cleanup, || {
        drop((store, original, expected, writes, retained))
    });
    let dropped = snapshot();
    println!(
        "batch_probe={}",
        serde_json::json!({
            "n": n, "distinct": distinct, "sparse": sparse, "mode": mode,
            "batch": d, "pages_before": pages_before, "pages_after": pages_after,
            "nodes_allocated": nodes_after - nodes_before, "mutations": mutations,
            "cleanup_ticks": cleanup_ticks, "semantic": semantic,
            "retained_report_bytes": semantic.capacity(),
            "checkpoints": {"start": start, "prepared": prepared, "updated": updated,
                "validated": validated, "released": released, "dropped": dropped},
        })
    );
    d
}

#[test]
fn batch_order_and_lifecycle_matrix() {
    if let Ok(case) = std::env::var("CHRIMP_BATCH_CASE") {
        let p: Vec<_> = case.split('/').collect();
        assert_eq!(p.len(), 4);
        probe(
            p[0].parse().unwrap(),
            p[1].parse().unwrap(),
            p[2] == "sparse",
            p[3],
        );
        return;
    }
    for n in [0, 1, 2, 8, 9, 16, 64, 256] {
        for distinct in [1, (n / 4).max(1), n.max(1)] {
            for sparse in [false, true] {
                for mode in ["insert", "mixed", "noop", "delete"] {
                    probe(n, distinct, sparse, mode);
                }
            }
        }
    }
}

#[test]
fn large_batch_coalescing_work_scales_linearly() {
    for n in [64, 256] {
        for distinct in [n / 4, n] {
            let d = probe(n, distinct, false, "mixed");
            assert!(
                d.comparisons + d.hash_requests <= 4 * n,
                "duplicate detection must avoid repeated suffix scans: {d:?}"
            );
        }
    }
}
