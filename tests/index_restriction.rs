use chr::condition::{Arena, Condition, Operation, Progress};
use chr::store::{Root, Store, Substitution};
use chr::trace::{Cursor, Step, Trace};
use std::collections::BTreeMap;
use std::sync::Arc;

fn boolean(a: &mut Arena, op: Operation) -> Condition {
    let mut job = a.start(op);
    loop {
        if let Progress::Complete(c) = job.tick(a) {
            return c;
        }
    }
}

fn collect(store: &mut Store<Condition>, a: &mut Arena, job: &Substitution, extra: &[Root]) {
    let mut roots = vec![];
    let mut cursor = Cursor::default();
    loop {
        match job.trace(&mut cursor) {
            Step::Root(c) => roots.push(c),
            Step::Pending => {}
            Step::Done => break,
        }
    }
    let mut inventory: Vec<_> = job.condition_roots().collect();
    inventory.sort();
    roots.sort();
    assert_eq!(roots, inventory);
    let mut gc = store.collect(
        job.roots()
            .chain(extra.iter().copied())
            .collect::<Vec<_>>()
            .into_iter(),
    );
    while !gc.done() {
        if let Some((_, value)) = gc.tick(store) {
            roots.push(value);
        }
    }
    drop(gc);
    let mut gc = a.collect(roots.into_iter());
    while !gc.tick(a) {}
}

#[test]
fn graph_namespaces_cofactor_preserves_old_snapshot_and_sharing() {
    let mut a = Arena::default();
    let (xid, x) = a.fresh_choice();
    let (_, y) = a.fresh_choice();
    let mut store = Store::default();
    let mut root = store.empty();
    for i in 0..128 {
        root = store.insert(root, [i % 7, i, u64::MAX - i, i.rotate_left(53)], y);
    }
    let changed = [7, 0, 0, 0];
    let removed = [7, 0, 0, 1];
    root = store.insert(root, changed, boolean(&mut a, Operation::And(x, y)));
    root = store.insert(root, removed, x.not());
    // Remove construction garbage so the allocation comparison measures this pass.
    let mut gc = store.collect(vec![root].into_iter());
    while !gc.done() {
        gc.tick(&mut store);
    }
    drop(gc);
    let before = store.node_count();
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(xid, Condition::TRUE)])));
    let result = (0..10000)
        .find_map(|_| job.tick(&mut store, &mut a))
        .unwrap();
    assert_eq!(store.get(result, &changed), Some(y));
    assert_eq!(store.get(result, &removed), None);
    assert_eq!(store.get(root, &removed), Some(x.not()));
    assert_ne!(store.get(root, &changed), Some(y));
    assert!(
        store.node_count() - before < 20,
        "unchanged namespaces must share their subtrees"
    );
    assert_eq!(job.tick(&mut store, &mut a), Some(result));
}

#[test]
fn no_op_reuses_the_exact_root_without_allocating() {
    let mut a = Arena::default();
    let (xid, _) = a.fresh_choice();
    let (_, y) = a.fresh_choice();
    let mut store = Store::default();
    let mut root = store.empty();
    for i in 0..100 {
        root = store.insert(root, [0, i, 0, 0], y);
    }
    let before = (store.node_count(), a.node_count());
    for bindings in [BTreeMap::new(), BTreeMap::from([(xid, Condition::TRUE)])] {
        let mut job = store.substitute(root, Arc::new(bindings));
        assert_eq!(
            (0..10000).find_map(|_| job.tick(&mut store, &mut a)),
            Some(root)
        );
        assert_eq!((store.node_count(), a.node_count()), before);
    }
}

#[test]
fn broad_rewrite_allocates_at_most_one_node_per_original_node() {
    let mut a = Arena::default();
    let (xid, x) = a.fresh_choice();
    let (_, y) = a.fresh_choice();
    let xy = boolean(&mut a, Operation::And(x, y));
    let mut store = Store::default();
    let mut root = store.empty();
    let mut expected = BTreeMap::new();
    for i in 0..512_u64 {
        let key = [i.rotate_left(53), !i, i.wrapping_mul(0x9e3779b97f4a7c15), i];
        let (value, replacement) = match i % 4 {
            0 => (x, Some(Condition::TRUE)),
            1 => (x.not(), None),
            2 => (xy, Some(y)),
            _ => (y, Some(y)),
        };
        expected.insert(key, (value, replacement));
        root = store.insert(root, key, value);
    }
    let mut gc = store.collect(vec![root].into_iter());
    while !gc.done() {
        gc.tick(&mut store);
    }
    drop(gc);
    let old_nodes = store.node_count();
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(xid, Condition::TRUE)])));
    let result = (0..50000)
        .find_map(|_| job.tick(&mut store, &mut a))
        .expect("finite broad rewrite");
    assert!(store.node_count() - old_nodes <= old_nodes);
    collect(&mut store, &mut a, &job, &[root]);
    for (key, (original, replacement)) in expected {
        assert_eq!(store.get(root, &key), Some(original));
        assert_eq!(store.get(result, &key), replacement);
    }
}

#[test]
fn newly_built_conditions_and_staged_subtrees_survive_every_tick_gc() {
    let mut a = Arena::default();
    let (_, x) = a.fresh_choice();
    let (yid, y) = a.fresh_choice();
    let (_, z) = a.fresh_choice();
    let xy = boolean(&mut a, Operation::And(x, y));
    let nxz = boolean(&mut a, Operation::And(x.not(), z));
    let input = boolean(&mut a, Operation::Or(xy, nxz));
    let mut store = Store::default();
    let mut root = store.empty();
    for i in 0..16 {
        root = store.insert(root, [i, u64::MAX, 0, i], input);
    }
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(yid, Condition::TRUE)])));
    let mut result = None;
    for _ in 0..10000 {
        collect(&mut store, &mut a, &job, &[]);
        result = job.tick(&mut store, &mut a);
        if result.is_some() {
            break;
        }
    }
    collect(&mut store, &mut a, &job, &[]);
    let result = result.expect("finite index substitution");
    for i in 0..16 {
        let value = store.get(result, &[i, u64::MAX, 0, i]).unwrap();
        for bits in 0..8 {
            assert_eq!(a.evaluate(value, |v| bits & (1 << v) != 0), bits & 5 != 0);
        }
    }
}

#[test]
fn false_leaves_are_removed_and_last_assignment_owner_is_drained() {
    let mut a = Arena::default();
    let (id, x) = a.fresh_choice();
    let mut store = Store::default();
    let mut root = store.empty();
    for i in 0..64 {
        root = store.insert(root, [0, i, 0, 0], x);
    }
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(id, Condition::FALSE)])));
    assert_eq!(
        (0..10000).find_map(|_| job.tick(&mut store, &mut a)),
        Some(store.empty())
    );
    collect(&mut store, &mut a, &job, &[]);
    assert_eq!((store.node_count(), a.node_count()), (0, 0));

    let size = 4096;
    let bindings = Arc::new(
        (0..size)
            .map(|_| (a.fresh_choice().0, Condition::TRUE))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut sole = store.substitute(store.empty(), bindings.clone());
    let mut shared = store.substitute(store.empty(), bindings);
    assert!(!shared.discard_tick());
    assert!(shared.discard_tick());
    assert!(!sole.discard_tick());
    for remaining in (0..size).rev() {
        assert_eq!(sole.discard_tick(), remaining == 0);
    }
    assert!(sole.discard_tick());

    let bindings = Arc::new(
        (1..=size)
            .map(|i| (i, Condition::TRUE))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut completion = store.substitute(store.empty(), bindings);
    let mut ticks = 0;
    loop {
        ticks += 1;
        if let Some(root) = completion.tick(&mut store, &mut a) {
            assert_eq!(root, store.empty());
            break;
        }
        assert!(ticks < size + 16);
    }
    assert!(
        ticks >= size,
        "completion must drain sole-owned assignments incrementally"
    );
}

#[test]
fn discard_nested_substitutions_is_traceable_without_finishing_the_index() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for cancel_at in [0, 1, 3, 8, 20, 45, 80] {
        let mut a = Arena::default();
        let (_, x) = a.fresh_choice();
        let (yid, y) = a.fresh_choice();
        let (_, z) = a.fresh_choice();
        let xy = boolean(&mut a, Operation::And(x, y));
        let nz = boolean(&mut a, Operation::And(x.not(), z));
        let value = boolean(&mut a, Operation::Or(xy, nz));
        let mut store = Store::default();
        let mut root = store.empty();
        for i in 0..32 {
            root = store.insert(root, [0, i, 0, 0], value);
        }
        let mut job = store.substitute(root, Arc::new(BTreeMap::from([(yid, Condition::TRUE)])));
        for _ in 0..cancel_at {
            collect(&mut store, &mut a, &job, &[]);
            assert_eq!(job.tick(&mut store, &mut a), None);
        }
        let mut done = false;
        for _ in 0..100 {
            let before = (store.node_count(), a.node_count());
            done = job.discard_tick();
            assert_eq!(
                (store.node_count(), a.node_count()),
                before,
                "discard must not evaluate or allocate"
            );
            collect(&mut store, &mut a, &job, &[]);
            if done {
                break;
            }
        }
        assert!(done);
        assert!(job.discard_tick());
        assert_eq!((store.node_count(), a.node_count()), (0, 0));
        assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut store, &mut a))).is_err());
    }
}

#[test]
fn owner_and_freeze_checks_precede_any_store_or_arena_mutation() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut a = Arena::default();
    let (choice, x) = a.fresh_choice();
    let mut store = Store::default();
    let root = store.insert(store.empty(), [0; 4], x);
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(choice, Condition::TRUE)])));
    let mut other_store = Store::default();
    assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut other_store, &mut a))).is_err());
    assert_eq!(other_store.node_count(), 0);
    let lease = store.collect(vec![root].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut store, &mut a))).is_err());
    drop(lease);
    let lease = a.collect(vec![x].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut store, &mut a))).is_err());
    drop(lease);
    let mut other_arena = Arena::default();
    assert!(catch_unwind(AssertUnwindSafe(|| job.tick(&mut store, &mut other_arena))).is_err());
    assert_eq!(
        (store.node_count(), a.node_count(), other_arena.node_count()),
        (1, 1, 0)
    );
    let result = (0..100).find_map(|_| job.tick(&mut store, &mut a)).unwrap();
    assert_eq!(store.get(result, &[0; 4]), Some(Condition::TRUE));
    let mut invalid = store.substitute(
        result,
        Arc::new(BTreeMap::from([(choice + 1, Condition::TRUE)])),
    );
    let before = store.node_count();
    assert!(catch_unwind(AssertUnwindSafe(|| invalid.tick(&mut store, &mut a))).is_err());
    assert_eq!(store.node_count(), before);
}

#[test]
fn functional_images_survive_gc_before_first_leaf_and_while_draining() {
    let mut arena = Arena::default();
    let (_, y) = arena.fresh_choice();
    let (_, z) = arena.fresh_choice();
    let (xid, x) = arena.fresh_choice();
    let image = boolean(&mut arena, Operation::Or(y, z));
    let mut store = Store::default();
    let root = store.insert(store.empty(), [0; 4], x);
    let mut job = store.substitute(root, Arc::new(BTreeMap::from([(xid, image)])));
    let result = (0..10000)
        .find_map(|_| {
            collect(&mut store, &mut arena, &job, &[]);
            assert!(
                arena.contains(image),
                "image must survive before a leaf transform exists"
            );
            job.tick(&mut store, &mut arena)
        })
        .expect("finite functional substitution");
    assert_eq!(store.get(result, &[0; 4]), Some(image));
    collect(&mut store, &mut arena, &job, &[]);
    assert_eq!(store.get(result, &[0; 4]), Some(image));

    let mut discarded = store.substitute(store.empty(), Arc::new(BTreeMap::from([(xid, image)])));
    assert!(!discarded.discard_tick());
    collect(&mut store, &mut arena, &discarded, &[]);
    assert!(arena.contains(image), "sole-owned draining map is a root");
    assert!(discarded.discard_tick());
    collect(&mut store, &mut arena, &discarded, &[]);
    assert!(!arena.contains(image));
}
