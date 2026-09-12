use chr::condition::{Arena, Condition};
use chr::graph::{Graph, UpdateStatus};
use chr::history::History;
use chr::program::Signature;
use chr::store::Root;
use std::sync::Arc;

fn collect(history: &mut History, roots: &[Root]) -> Vec<Condition> {
    let mut gc = history.collect(roots.iter().copied());
    let mut supports = Vec::new();
    for _ in 0..100_000 {
        if gc.done() {
            assert_eq!(gc.tick(history), None);
            return supports;
        }
        if let Some(c) = gc.tick(history) {
            supports.push(c);
        }
    }
    panic!("finite history collection did not finish");
}

#[test]
fn ordered_tuples_rules_and_snapshot_supports_are_distinct() {
    let mut h = History::default();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let heads = Arc::new(vec![4, 9]);
    let reversed = Arc::new(vec![9, 4]);
    let empty = h.empty();
    let first = h.set_support(empty, 2, heads.clone(), c);
    let second = h.set_support(first, 2, reversed.clone(), c.not());
    let third = h.set_support(second, 3, heads.clone(), Condition::TRUE);
    assert_eq!(h.record_count(), 3);
    assert_eq!(h.support(third, 2, &Arc::new(vec![4, 9])), c);
    assert_eq!(h.support(third, 2, &reversed), c.not());
    assert_eq!(h.support(third, 3, &heads), Condition::TRUE);
    assert_eq!(h.support(first, 2, &reversed), Condition::FALSE);
    assert_eq!(h.support(empty, 2, &heads), Condition::FALSE);
    let replaced = h.set_support(third, 2, Arc::new(vec![4, 9]), c.not());
    assert_eq!(
        h.record_count(),
        3,
        "structurally equal tuples share metadata"
    );
    assert_eq!(
        h.support(replaced, 2, &heads),
        c.not(),
        "set replaces; the caller computes unions"
    );
    assert_eq!(h.support(third, 2, &heads), c);
    let nodes = h.node_count();
    assert_eq!(h.set_support(replaced, 2, heads.clone(), c.not()), replaced);
    assert_eq!(h.node_count(), nodes);
    let without = h.set_support(replaced, 2, heads.clone(), Condition::FALSE);
    assert_eq!(h.support(without, 2, &heads), Condition::FALSE);
    assert_eq!(h.support(replaced, 2, &heads), c.not());
}

#[test]
fn absent_false_does_not_allocate_or_intern() {
    let mut h = History::default();
    let empty = h.empty();
    let heads = Arc::new(vec![1, 2]);
    assert_eq!(
        h.set_support(empty, 10, heads.clone(), Condition::FALSE),
        empty
    );
    assert_eq!((h.record_count(), h.node_count()), (0, 0));
    assert_eq!(Arc::strong_count(&heads), 1);
    let root = h.set_support(empty, 0, Arc::new(vec![7]), Condition::TRUE);
    let counts = (h.record_count(), h.node_count());
    assert_eq!(
        h.set_support(root, 10, heads.clone(), Condition::FALSE),
        root
    );
    assert_eq!((h.record_count(), h.node_count()), counts);
    assert_eq!(Arc::strong_count(&heads), 1);
}

#[test]
fn entries_retain_ordered_shared_tuples_and_frozen_roots() {
    let mut h = History::default();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let tuples = [
        Arc::new(vec![8, 3, 8]),
        Arc::new(vec![]),
        Arc::new(vec![u64::MAX]),
    ];
    let mut root = h.empty();
    for (rule, heads) in tuples.iter().enumerate() {
        root = h.set_support(root, rule, heads.clone(), c);
    }
    let mut entries = h.entries(root);
    assert_eq!(entries.root(), root);
    let later = h.set_support(root, 77, Arc::new(vec![99]), Condition::TRUE);
    let supports = collect(&mut h, &[entries.root(), later]);
    let mut gc = a.collect(supports.into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    let mut seen = Vec::new();
    while let Some(entry) = entries.next(&h) {
        assert!(
            Arc::ptr_eq(&entry.heads, &tuples[entry.rule]),
            "tuple storage must be shared"
        );
        assert_eq!(entry.support, c);
        seen.push((entry.rule, entry.heads));
    }
    seen.sort_by_key(|p| p.0);
    assert_eq!(
        seen,
        tuples
            .iter()
            .enumerate()
            .map(|(r, h)| (r, h.clone()))
            .collect::<Vec<_>>()
    );
    assert!(entries.next(&h).is_none());
}

#[test]
fn collection_preserves_snapshots_then_releases_metadata_store_and_conditions() {
    let mut h = History::default();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let empty = h.empty();
    let heads = Arc::new(vec![1, 2]);
    let discarded = Arc::new(vec![3]);
    let old = h.set_support(empty, 0, heads.clone(), c);
    let updated = h.set_support(old, 0, heads.clone(), d);
    let latest = h.set_support(updated, 1, discarded.clone(), Condition::TRUE);
    assert!(h.contains(latest));
    let supports = collect(&mut h, &[old, updated]);
    assert_eq!(h.record_count(), 1);
    assert_eq!(h.node_count(), 2);
    assert_eq!(Arc::strong_count(&discarded), 1);
    assert_eq!(h.support(old, 0, &heads), c);
    assert_eq!(h.support(updated, 0, &heads), d);
    assert!(!h.contains(latest));
    let mut gc = a.collect(supports.into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert!(a.contains(c) && a.contains(d));
    let supports = collect(&mut h, &[empty]);
    assert!(supports.is_empty());
    assert_eq!((h.record_count(), h.node_count()), (0, 0));
    assert_eq!(Arc::strong_count(&heads), 1);
    assert!(!h.contains(old) && !h.contains(updated));
    let mut gc = a.collect(supports.into_iter());
    while !gc.tick(&mut a) {}
    drop(gc);
    assert_eq!(a.node_count(), 0);
    let fresh = h.set_support(empty, 0, heads.clone(), Condition::TRUE);
    assert_ne!(fresh, old, "root identities must not be reused");
    assert_eq!(h.support(fresh, 0, &heads), Condition::TRUE);
}

#[test]
fn unique_tuple_churn_keeps_only_live_metadata() {
    let mut h = History::default();
    let mut root = h.empty();
    let mut previous = Arc::new(vec![]);
    for i in 0..2000 {
        root = h.set_support(root, 0, previous, Condition::FALSE);
        let heads = Arc::new(vec![i, i + 1]);
        root = h.set_support(root, 0, heads.clone(), Condition::TRUE);
        assert_eq!(collect(&mut h, &[root]), [Condition::TRUE]);
        assert_eq!((h.record_count(), h.node_count()), (1, 1));
        assert_eq!(h.support(root, 0, &heads), Condition::TRUE);
        previous = heads;
    }
    assert_eq!(collect(&mut h, &[]), []);
    assert_eq!((h.record_count(), h.node_count()), (0, 0));
}

#[test]
fn foreign_nonempty_roots_are_rejected_even_for_missing_keys() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut h = History::default();
    let mut other = History::default();
    let foreign = other.set_support(other.empty(), 0, Arc::new(vec![1]), Condition::TRUE);
    let mut g = Graph::new(&[Signature {
        name: "p".into(),
        arity: 0,
    }]);
    let mut update = g.post(g.empty(), 0, vec![], Condition::TRUE).unwrap();
    let graph_root = loop {
        if let UpdateStatus::Complete(root) = update.tick(&mut g) {
            break root;
        }
    };
    let absent = Arc::new(vec![123]);
    for root in [foreign, graph_root] {
        assert!(!h.contains(root));
        assert!(catch_unwind(|| h.support(root, 100, &absent)).is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(|| h.set_support(
                root,
                100,
                absent.clone(),
                Condition::FALSE
            )))
            .is_err()
        );
        assert!(catch_unwind(|| h.entries(root)).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| collect(&mut h, &[root]))).is_err());
        assert_eq!((h.record_count(), h.node_count()), (0, 0));
    }
}

#[test]
fn collection_after_a_large_peak_retains_only_the_surviving_tuple() {
    let mut h = History::default();
    let survivor = Arc::new(vec![0, 1]);
    let retained = h.set_support(h.empty(), 0, survivor.clone(), Condition::TRUE);
    let mut root = retained;
    let mut payloads = Vec::new();
    for i in 1..4096 {
        let heads = Arc::new(vec![i, i + 1]);
        payloads.push(Arc::downgrade(&heads));
        root = h.set_support(root, 0, heads, Condition::TRUE);
    }
    assert_eq!(h.record_count(), 4096);
    assert!(h.contains(root));
    assert_eq!(collect(&mut h, &[retained]), [Condition::TRUE]);
    assert_eq!((h.record_count(), h.node_count()), (1, 1));
    assert!(payloads.into_iter().all(|p| p.upgrade().is_none()));
    assert_eq!(h.support(retained, 0, &survivor), Condition::TRUE);
    assert_eq!(collect(&mut h, &[retained]), [Condition::TRUE]);
}

#[test]
fn owned_collection_freezes_interning_through_metadata_sweep() {
    use chr::history::Collector;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn begin(history: &mut History, root: Root) -> Collector<std::array::IntoIter<Root, 1>> {
        history.collect([root].into_iter())
    }
    for cutoff in 0..48 {
        let mut h = History::default();
        let heads = Arc::new(vec![1, 2]);
        let root = h.set_support(h.empty(), 0, heads.clone(), Condition::TRUE);
        h.set_support(root, 0, Arc::new(vec![3, 4]), Condition::TRUE);
        let mut gc = begin(&mut h, root);
        let mut foreign = History::default();
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| h.collect([root].into_iter()))).is_err());
        for _ in 0..cutoff {
            gc.tick(&mut h);
        }
        assert_eq!(h.support(root, 0, &heads), Condition::TRUE);
        let counts = (h.record_count(), h.node_count());
        let fresh = Arc::new(vec![8, 9]);
        assert!(
            catch_unwind(AssertUnwindSafe(|| h.set_support(
                root,
                1,
                fresh.clone(),
                Condition::TRUE
            )))
            .is_err()
        );
        assert!(
            catch_unwind(AssertUnwindSafe(|| h.set_support(
                root,
                0,
                heads.clone(),
                Condition::FALSE
            )))
            .is_err()
        );
        assert_eq!((h.record_count(), h.node_count()), counts);
        assert_eq!(Arc::strong_count(&fresh), 1);
        drop(gc);
        let updated = h.set_support(root, 1, fresh.clone(), Condition::TRUE);
        let mut gc = begin(&mut h, updated);
        while !gc.done() {
            gc.tick(&mut h);
        }
        assert_eq!(gc.tick(&mut h), None);
        assert!(catch_unwind(AssertUnwindSafe(|| gc.tick(&mut foreign))).is_err());
        drop(gc);
        assert_eq!(h.support(updated, 1, &fresh), Condition::TRUE);
    }
}
