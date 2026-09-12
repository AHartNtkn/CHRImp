use chr::condition::{Arena, Condition, Operation, Progress};
use chr::graph::{Graph, UpdateStatus};
use chr::identity::Merge;
use chr::program::Signature;
use chr::store::Root;
use chr::wake::{Wake, WakeStatus};
use std::collections::BTreeMap;

fn graph() -> Graph {
    Graph::new(&[
        Signature {
            name: "p".into(),
            arity: 3,
        },
        Signature {
            name: "q".into(),
            arity: 1,
        },
    ])
}
fn boolean(a: &mut Arena, op: Operation) -> Condition {
    let mut job = a.start(op);
    for _ in 0..100_000 {
        if let Progress::Complete(c) = job.tick(a) {
            return c;
        }
    }
    panic!("condition operation did not finish");
}
fn post(
    g: &mut Graph,
    root: Root,
    relation: usize,
    args: Vec<u64>,
    support: Condition,
) -> (Root, u64) {
    let mut job = g.post(root, relation, args, support).unwrap();
    let id = job.occurrence();
    loop {
        if let UpdateStatus::Complete(root) = job.tick(g) {
            return (root, id);
        }
    }
}
fn merge(
    g: &mut Graph,
    a: &mut Arena,
    root: Root,
    x: u64,
    y: u64,
    scope: Condition,
) -> (Root, Condition) {
    let mut job = Merge::new(g, root, x, y, scope);
    for _ in 0..100_000 {
        if let Some(root) = job.tick(g, a) {
            return (root, job.changed_support());
        }
    }
    panic!("merge did not finish");
}
fn run(
    g: &mut Graph,
    a: &mut Arena,
    wake: &mut Wake,
    gc: bool,
) -> (BTreeMap<u64, Condition>, usize) {
    let mut found = BTreeMap::new();
    for ticks in 1..100_000 {
        if gc {
            let mut roots = Vec::new();
            let mut collector = g.collect(std::iter::once(wake.root()));
            while !collector.done() {
                if let Some(c) = collector.tick() {
                    roots.push(c);
                }
            }
            drop(collector);
            let mut collector = a.collect(
                wake.condition_roots()
                    .chain(found.values().copied())
                    .chain(roots),
            );
            while !collector.tick() {}
        }
        match wake.tick(g, a) {
            WakeStatus::Found {
                occurrence,
                support,
            } => {
                assert_ne!(support, Condition::FALSE);
                let old = found.get(&occurrence).copied().unwrap_or(Condition::FALSE);
                assert_eq!(
                    boolean(a, Operation::And(old, support)),
                    Condition::FALSE,
                    "overlapping wake for occurrence {occurrence}"
                );
                found.insert(occurrence, boolean(a, Operation::Or(old, support)));
            }
            WakeStatus::Pending => {}
            WakeStatus::Done => {
                assert_eq!(wake.tick(g, a), WakeStatus::Done);
                assert!(wake.condition_roots().all(|c| c.is_terminal()));
                return (found, ticks);
            }
        }
    }
    panic!("finite wake did not finish");
}

#[test]
fn conditional_merge_wakes_cross_predicate_incidence_on_live_overlap() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![0, 0, 0], Condition::TRUE);
    let (root, q) = post(&mut g, root, 1, vec![1], d);
    let (root, _) = post(&mut g, root, 1, vec![2], Condition::TRUE);
    let (root, changed) = merge(&mut g, &mut a, root, 0, 1, c);
    assert_eq!(changed, c);
    let mut wake = Wake::new(&g, root, 0, changed);
    let found = run(&mut g, &mut a, &mut wake, true).0;
    let cd = boolean(&mut a, Operation::And(c, d));
    assert_eq!(found, BTreeMap::from([(p, c), (q, cd)]));
    assert_eq!(a.fresh_choice().0, 2, "wake must not create choices");
}

#[test]
fn repeated_ports_and_overlapping_members_emit_only_novel_contexts() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![1, 2, 2], Condition::TRUE);
    let (root, q) = post(&mut g, root, 1, vec![0], Condition::TRUE);
    let (root, _) = merge(&mut g, &mut a, root, 0, 1, c);
    let (root, _) = merge(&mut g, &mut a, root, 0, 2, d);
    let mut wake = Wake::new(&g, root, 0, Condition::TRUE);
    let nodes = g.index_node_count();
    let found = run(&mut g, &mut a, &mut wake, false).0;
    assert_eq!(
        found,
        BTreeMap::from([
            (p, boolean(&mut a, Operation::Or(c, d))),
            (q, Condition::TRUE)
        ])
    );
    assert_eq!(
        g.index_node_count(),
        nodes,
        "wake is a read-only graph operation"
    );
    let mut wake = Wake::new(&g, root, 0, Condition::FALSE);
    assert!(run(&mut g, &mut a, &mut wake, true).0.is_empty());
}

#[test]
fn suspended_wake_keeps_its_frozen_root_and_collects_between_every_tick() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![1, 2, 2], Condition::TRUE);
    let (root, _) = merge(&mut g, &mut a, root, 0, 1, c);
    let (root, _) = merge(&mut g, &mut a, root, 0, 2, c.not());
    let mut wake = Wake::new(&g, root, 0, Condition::TRUE);
    assert_eq!(wake.root(), root);
    assert_eq!(wake.tick(&g, &mut a), WakeStatus::Pending);
    let (_new_root, _) = post(&mut g, root, 1, vec![0], Condition::TRUE);
    assert_eq!(
        run(&mut g, &mut a, &mut wake, true).0,
        BTreeMap::from([(p, Condition::TRUE)])
    );
    assert_eq!(g.occurrence_count(), 1, "only the frozen root is retained");
}

#[test]
fn unrelated_occurrences_do_not_add_wake_work() {
    let mut g = graph();
    let mut a = Arena::default();
    let empty = g.empty();
    let (mut root, p) = post(&mut g, empty, 1, vec![0], Condition::TRUE);
    let mut wake = Wake::new(&g, root, 0, Condition::TRUE);
    let (found, small_ticks) = run(&mut g, &mut a, &mut wake, false);
    assert_eq!(found, BTreeMap::from([(p, Condition::TRUE)]));
    for variable in 1000..6000 {
        root = post(&mut g, root, 1, vec![variable], Condition::TRUE).0;
    }
    let mut wake = Wake::new(&g, root, 0, Condition::TRUE);
    let (large, large_ticks) = run(&mut g, &mut a, &mut wake, false);
    assert_eq!(large, found);
    assert_eq!(
        large_ticks, small_ticks,
        "unrelated incidence must not be enumerated"
    );
    let mut empty_wake = Wake::new(&g, root, u64::MAX, Condition::TRUE);
    assert!(run(&mut g, &mut a, &mut empty_wake, false).0.is_empty());
}
