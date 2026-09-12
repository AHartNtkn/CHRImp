use chr::condition::{Arena, Condition, Operation, Progress};
use chr::graph::{Graph, Prune, UpdateStatus};
use chr::identity::{Equal, Merge, Resolve, ResolveStatus};
use chr::members::Members;
use chr::program::Signature;
use chr::store::Root;
use std::collections::BTreeMap;

fn graph() -> Graph {
    Graph::new(&[
        Signature {
            name: "p".into(),
            arity: 2,
        },
        Signature {
            name: "q".into(),
            arity: 0,
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
    panic!("condition did not finish")
}
fn merge(g: &mut Graph, a: &mut Arena, root: Root, x: u64, y: u64, c: Condition) -> Root {
    let mut job = Merge::new(g, root, x, y, c);
    for _ in 0..100_000 {
        if let Some(root) = job.tick(g, a) {
            return root;
        }
    }
    panic!("merge did not finish")
}
fn post(g: &mut Graph, root: Root, args: Vec<u64>, c: Condition) -> (Root, u64) {
    let relation = if args.is_empty() { 1 } else { 0 };
    let mut job = g.post(root, relation, args, c).unwrap();
    let id = job.occurrence();
    for _ in 0..100_000 {
        if let UpdateStatus::Complete(root) = job.tick(g) {
            return (root, id);
        }
    }
    panic!("post did not finish")
}
fn equal(g: &Graph, a: &mut Arena, root: Root, x: u64, y: u64) -> Condition {
    let mut job = Equal::new(g, root, x, y, Condition::TRUE);
    for _ in 0..100_000 {
        if let Some(c) = job.tick(g, a) {
            return c;
        }
    }
    panic!("equal did not finish")
}
fn members(g: &Graph, a: &mut Arena, root: Root, variable: u64) -> BTreeMap<u64, Condition> {
    let mut job = Members::new(g, root, variable, Condition::TRUE);
    let mut found = BTreeMap::new();
    for _ in 0..100_000 {
        match job.tick(g, a) {
            ResolveStatus::Pending => {}
            ResolveStatus::Done => return found,
            ResolveStatus::Found { variable, support } => {
                let old = found.get(&variable).copied().unwrap_or(Condition::FALSE);
                found.insert(variable, boolean(a, Operation::Or(old, support)));
            }
        }
    }
    panic!("members did not finish")
}
fn collect(g: &mut Graph, a: &mut Arena, roots: Vec<Root>, mut supports: Vec<Condition>) {
    let mut gc = g.collect(roots.into_iter());
    while !gc.done() {
        if let Some(c) = gc.tick(g) {
            supports.push(c);
        }
    }
    drop(gc);
    let mut gc = a.collect(supports.into_iter());
    while !gc.tick(a) {}
}
fn finish(
    g: &mut Graph,
    a: &mut Arena,
    job: &mut Prune,
    gc_each_tick: bool,
    retained: &[Condition],
) -> Root {
    for _ in 0..200_000 {
        if gc_each_tick {
            collect(
                g,
                a,
                job.graph_roots().collect(),
                job.condition_roots()
                    .chain(retained.iter().copied())
                    .collect(),
            );
        }
        let before = g.index_allocations();
        let result = job.tick(g, a);
        assert!(
            g.index_allocations() - before <= 1,
            "one index allocation per prune tick"
        );
        if let Some(root) = result {
            assert_eq!(job.tick(g, a), Some(root.clone()));
            assert!(job.graph_roots().all(|r| r == root || r == g.empty()));
            assert!(
                job.condition_roots().all(|c| c.is_terminal()),
                "completed scratch still retains supports"
            );
            return root;
        }
    }
    panic!("prune did not finish")
}

#[test]
fn live_raw_arguments_keep_bridge_ancestors_and_only_reachable_reverse_members() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 2, 3, Condition::TRUE);
    let root = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE);
    let root = merge(&mut g, &mut a, root, 0, 2, Condition::TRUE);
    let (root, id) = post(&mut g, root, vec![3, 3], c);
    let mut prune = g.prune(root, Condition::TRUE);
    let result = finish(&mut g, &mut a, &mut prune, true, &[c]);
    let fact = g.fact(result.clone(), id).unwrap();
    assert_eq!(fact.args, [3, 3]);
    assert_eq!(fact.support, c);
    assert_eq!(equal(&g, &mut a, result.clone(), 3, 0), c);
    assert_eq!(equal(&g, &mut a, result.clone(), 3, 2), c);
    assert_eq!(equal(&g, &mut a, result.clone(), 1, 0), Condition::FALSE);
    assert_eq!(
        members(&g, &mut a, result.clone(), 0),
        BTreeMap::from([(0, Condition::TRUE), (2, c), (3, c)])
    );
    for port in 0..2 {
        assert_eq!(
            g.port(result.clone(), 0, port, 3).unwrap().next(&g),
            Some((id, c))
        );
    }
    assert_eq!(g.incidence(result.clone(), 3).next(&g), Some((id, c)));
    // The old rank at representative 0 must survive in the marked region:
    // a new rank-1 peer must still attach to it rather than win by numeric ID.
    let peer = merge(&mut g, &mut a, result, 10, 11, c);
    let joined = merge(&mut g, &mut a, peer, 0, 10, c);
    assert_eq!(equal(&g, &mut a, joined.clone(), 3, 11), c);
    let mut resolve = Resolve::new(&g, joined, 3, c);
    let mut representative = None;
    for _ in 0..1000 {
        match resolve.tick(&g, &mut a) {
            ResolveStatus::Pending => {}
            ResolveStatus::Found { variable, support } => {
                assert_eq!(support, c);
                representative = Some(variable);
            }
            ResolveStatus::Done => break,
        }
    }
    assert_eq!(
        representative,
        Some(0),
        "marked rank-2 representative must beat a rank-1 peer"
    );
}

#[test]
fn conditional_opposite_parent_directions_reach_a_support_fixedpoint() {
    for gc in [false, true] {
        let mut g = graph();
        let mut a = Arena::default();
        let (_, c) = a.fresh_choice();
        let empty = g.empty();
        let root = merge(&mut g, &mut a, empty, 0, 2, c);
        let root = merge(&mut g, &mut a, root, 1, 3, c.not());
        let root = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE);
        let (root, id) = post(&mut g, root, vec![0, 1], Condition::TRUE);
        let mut prune = g.prune(root, Condition::TRUE);
        let result = finish(&mut g, &mut a, &mut prune, gc, &[c]);
        assert_eq!(g.fact(result.clone(), id).unwrap().support, Condition::TRUE);
        assert_eq!(equal(&g, &mut a, result.clone(), 0, 1), Condition::TRUE);
        let found = members(&g, &mut a, result.clone(), 0);
        assert_eq!(
            found,
            BTreeMap::from([(0, Condition::TRUE), (1, Condition::TRUE)])
        );
        assert_eq!(equal(&g, &mut a, result.clone(), 0, 2), Condition::FALSE);
        assert_eq!(equal(&g, &mut a, result.clone(), 1, 3), Condition::FALSE);
        let mut resolve = Resolve::new(&g, result, 0, Condition::TRUE);
        let mut representatives = BTreeMap::new();
        for _ in 0..1000 {
            match resolve.tick(&g, &mut a) {
                ResolveStatus::Pending => {}
                ResolveStatus::Found { variable, support } => {
                    representatives.insert(variable, support);
                }
                ResolveStatus::Done => break,
            }
        }
        assert_eq!(representatives, BTreeMap::from([(0, c), (1, c.not())]));
    }
}

#[test]
fn explicit_pending_variables_are_seeded_conditionally_and_duplicates_union() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 4, 5, Condition::TRUE);
    let root = merge(&mut g, &mut a, root, 8, 9, Condition::TRUE);
    let mut prune = g.prune(root, d);
    prune.seed(5, c);
    prune.seed(5, c.not());
    prune.seed(9, c);
    prune.seed(999, Condition::FALSE);
    let result = finish(&mut g, &mut a, &mut prune, true, &[c, d]);
    assert_eq!(equal(&g, &mut a, result.clone(), 4, 5), d);
    let cd = boolean(&mut a, Operation::And(c, d));
    assert_eq!(equal(&g, &mut a, result, 8, 9), cd);
    assert_eq!(g.occurrence_count(), 0);
}

#[test]
fn active_filter_updates_all_occurrence_indexes_and_old_snapshots_remain_valid() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, garbage) = a.fresh_choice();
    let dead = boolean(&mut a, Operation::And(c.not(), garbage));
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 90, 91, dead);
    let (root, live_id) = post(&mut g, root, vec![1, 2], Condition::TRUE);
    let (old, dead_id) = post(&mut g, root, vec![90, 91], dead);
    collect(&mut g, &mut a, vec![old.clone()], vec![c]);
    let mut prune = g.prune(old.clone(), c);
    let before = g.index_allocations();
    let result = finish(&mut g, &mut a, &mut prune, false, &[]);
    assert!(
        g.index_allocations() - before <= 2 * before,
        "two bottom-up passes have linear allocation growth"
    );
    assert_eq!(
        g.fact(old.clone(), live_id).unwrap().support,
        Condition::TRUE
    );
    assert_eq!(g.fact(old.clone(), dead_id).unwrap().support, dead);
    assert_eq!(equal(&g, &mut a, old.clone(), 90, 91), dead);
    assert!(g.fact(result.clone(), dead_id).is_none());
    assert_eq!(g.fact(result.clone(), live_id).unwrap().support, c);
    let mut relation = g.relation(result.clone(), 0).unwrap();
    assert_eq!(relation.next(&g), Some((live_id, c)));
    assert_eq!(relation.next(&g), None);
    for (port, variable) in [(0, 1), (1, 2)] {
        assert_eq!(
            g.port(result.clone(), 0, port, variable).unwrap().next(&g),
            Some((live_id, c))
        );
        assert_eq!(
            g.incidence(result.clone(), variable).next(&g),
            Some((live_id, c))
        );
    }
    assert_eq!(g.port(result.clone(), 0, 0, 90).unwrap().next(&g), None);
    assert_eq!(g.incidence(result.clone(), 91).next(&g), None);
    assert_eq!(equal(&g, &mut a, result.clone(), 90, 91), Condition::FALSE);
    collect(&mut g, &mut a, vec![old.clone(), result.clone()], vec![]);
    assert_eq!(g.occurrence_count(), 2);
    assert!(a.contains(garbage));
    assert_eq!(g.fact(old, dead_id).unwrap().support, dead);
    collect(
        &mut g,
        &mut a,
        prune.graph_roots().collect(),
        prune.condition_roots().collect(),
    );
    assert_eq!(g.occurrence_count(), 1);
    assert!(!a.contains(garbage));
    let mut empty_prune = g.prune(result, Condition::FALSE);
    let empty = finish(&mut g, &mut a, &mut empty_prune, true, &[]);
    assert_eq!(empty, g.empty());
    collect(
        &mut g,
        &mut a,
        empty_prune.graph_roots().collect(),
        empty_prune.condition_roots().collect(),
    );
    assert_eq!(g.occurrence_count(), 0);
    drop(prune);
    drop(relation);
    while !g.release_tick() {}
    assert_eq!(g.index_node_count(), 0);
    assert_eq!(a.node_count(), 0);
}

#[test]
fn noop_nullary_occurrences_and_owned_protocol_checks() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut g = graph();
    let mut a = Arena::default();
    let empty = g.empty();
    let (root, id) = post(&mut g, empty, vec![], Condition::TRUE);
    let mut prune = g.prune(root.clone(), Condition::TRUE);
    let before = g.index_allocations();
    let result = finish(&mut g, &mut a, &mut prune, false, &[]);
    assert_eq!(result, root);
    assert_eq!(g.index_allocations(), before);
    assert!(g.fact(result, id).unwrap().args.is_empty());
    assert!(catch_unwind(AssertUnwindSafe(|| prune.seed(1, Condition::TRUE))).is_err());
    let mut foreign_g = graph();
    let mut foreign_a = Arena::default();
    assert!(catch_unwind(AssertUnwindSafe(|| prune.tick(&mut foreign_g, &mut a))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| prune.tick(&mut g, &mut foreign_a))).is_err());
    let lease = g.collect([root.clone()].into_iter());
    assert!(catch_unwind(AssertUnwindSafe(|| prune.tick(&mut g, &mut a))).is_err());
    drop(lease);
    let lease = a.collect(std::iter::empty());
    assert!(catch_unwind(AssertUnwindSafe(|| prune.tick(&mut g, &mut a))).is_err());
    drop(lease);
    assert_eq!(prune.tick(&mut g, &mut a), Some(root));
    let mut empty = g.prune(g.empty(), Condition::TRUE);
    assert!(catch_unwind(AssertUnwindSafe(|| empty.tick(&mut foreign_g, &mut a))).is_err());
    assert_eq!(finish(&mut g, &mut a, &mut empty, true, &[]), g.empty());
}

#[test]
fn certified_no_identity_prune_work_is_independent_of_occurrence_count() {
    for count in [1, 128, 4096] {
        let mut g = graph();
        let mut a = Arena::default();
        let mut root = g.empty();
        for i in 0..count {
            root = post(&mut g, root, vec![i, i], Condition::TRUE).0;
        }
        let before = g.index_allocations();
        let mut job = g.prune(root.clone(), Condition::TRUE);
        let mut result = None;
        for _ in 0..4 {
            if let Some(r) = job.tick(&mut g, &mut a) {
                result = Some(r);
                break;
            }
        }
        assert_eq!(
            result,
            Some(root),
            "certified no-op must not scan {count} occurrences"
        );
        assert_eq!(g.index_allocations(), before);
    }
}
#[test]
fn certified_true_keeps_conditional_facts_and_gc_roots() {
    let mut g = graph();
    let mut a = Arena::default();
    let (_, x) = a.fresh_choice();
    let empty = g.empty();
    let (root, left) = post(&mut g, empty, vec![0, 1], x);
    let (root, right) = post(&mut g, root, vec![2, 3], x.not());
    let mut job = g.prune(root.clone(), Condition::TRUE);
    for i in 0..17 {
        job.seed(i, x);
    }
    let result = finish(&mut g, &mut a, &mut job, true, &[]);
    assert_eq!(result, root);
    assert_eq!(job.tick(&mut g, &mut a), Some(root));
    collect(
        &mut g,
        &mut a,
        job.graph_roots().collect(),
        job.condition_roots().collect(),
    );
    assert_eq!(g.fact(result.clone(), left).unwrap().support, x);
    assert_eq!(g.fact(result.clone(), right).unwrap().support, x.not());
    drop(job);
    let mut restricted = g.prune(result, x);
    let new = finish(&mut g, &mut a, &mut restricted, true, &[x]);
    assert_eq!(g.fact(new.clone(), left).unwrap().support, x);
    assert!(g.fact(new, right).is_none());
}
