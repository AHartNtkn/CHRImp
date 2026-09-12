use chr::condition::{Arena, Condition, Operation, Progress};
use chr::graph::Graph;
use chr::identity::{Equal, Merge, ResolveStatus};
use chr::members::Members;
use chr::store::Root;
use std::collections::BTreeMap;

fn boolean(a: &mut Arena, operation: Operation) -> Condition {
    let mut job = a.start(operation);
    for _ in 0..100_000 {
        if let Progress::Complete(c) = job.tick(a) {
            return c;
        }
    }
    panic!("finite condition operation did not complete");
}
fn merge(g: &mut Graph, a: &mut Arena, root: Root, x: u64, y: u64, c: Condition) -> Root {
    let mut job = Merge::new(g, root, x, y, c);
    for _ in 0..100_000 {
        if let Some(root) = job.tick(g, a) {
            return root;
        }
    }
    panic!("finite merge did not complete");
}
fn equal(g: &Graph, a: &mut Arena, root: Root, x: u64, y: u64, c: Condition) -> Condition {
    let mut job = Equal::new(g, root, x, y, c);
    for _ in 0..100_000 {
        if let Some(c) = job.tick(g, a) {
            return c;
        }
    }
    panic!("finite equality test did not complete");
}
fn enumerate(
    g: &mut Graph,
    a: &mut Arena,
    job: &mut Members,
    collect: bool,
    retained: &[Condition],
) -> BTreeMap<u64, Condition> {
    let mut found = BTreeMap::new();
    for _ in 0..100_000 {
        if collect {
            let mut supports = retained.to_vec();
            let mut gc = g.collect(std::iter::once(job.root()));
            while !gc.done() {
                if let Some(c) = gc.tick(g) {
                    supports.push(c);
                }
            }
            drop(gc);
            let mut gc = a.collect(
                job.condition_roots()
                    .chain(found.values().copied())
                    .chain(supports),
            );
            while !gc.tick(a) {}
        }
        match job.tick(g, a) {
            ResolveStatus::Found { variable, support } => {
                assert_ne!(support, Condition::FALSE);
                let old = found.get(&variable).copied().unwrap_or(Condition::FALSE);
                assert_eq!(
                    boolean(a, Operation::And(old, support)),
                    Condition::FALSE,
                    "overlapping membership emitted for {variable}"
                );
                found.insert(variable, boolean(a, Operation::Or(old, support)));
            }
            ResolveStatus::Pending => {}
            ResolveStatus::Done => {
                assert_eq!(job.tick(g, a), ResolveStatus::Done);
                assert!(
                    job.condition_roots().all(|c| c.is_terminal()),
                    "completed enumeration retains working supports"
                );
                return found;
            }
        }
    }
    panic!("finite member enumeration did not complete");
}

#[test]
fn uninvolved_variable_is_its_own_member_only_on_scope() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 0, 1, c);
    for scope in [Condition::TRUE, c, Condition::FALSE] {
        let mut job = Members::new(&g, root, u64::MAX, scope);
        assert_eq!(job.root(), root);
        let found = enumerate(&mut g, &mut a, &mut job, false, &[]);
        let expected = if scope == Condition::FALSE {
            BTreeMap::new()
        } else {
            BTreeMap::from([(u64::MAX, scope)])
        };
        assert_eq!(found, expected);
    }
}

#[test]
fn transitive_class_keeps_overlapping_child_supports() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let mut root = g.empty();
    for (x, y, support) in [
        (0, 1, Condition::TRUE),
        (2, 3, Condition::TRUE),
        (0, 2, c),
        (0, 4, d),
        (4, 5, Condition::TRUE),
    ] {
        root = merge(&mut g, &mut a, root, x, y, support);
    }
    let nodes = g.index_node_count();
    let mut job = Members::new(&g, root, 1, Condition::TRUE);
    assert_eq!(
        enumerate(&mut g, &mut a, &mut job, false, &[]),
        BTreeMap::from([
            (0, Condition::TRUE),
            (1, Condition::TRUE),
            (2, c),
            (3, c),
            (4, d),
            (5, d)
        ])
    );
    assert_eq!(
        g.index_node_count(),
        nodes,
        "enumeration must not write the graph"
    );
    assert_eq!(
        g.occurrence_count(),
        0,
        "members do not require occurrences"
    );
    assert_eq!(a.fresh_choice().0, 2, "enumeration must not create choices");
}

#[test]
fn opposite_parent_directions_in_siblings_terminate_without_duplicate_overlap() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 0, 2, c);
    let root = merge(&mut g, &mut a, root, 1, 3, c.not());
    let root = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE);
    for query in [0, 1] {
        let mut job = Members::new(&g, root, query, Condition::TRUE);
        assert_eq!(
            enumerate(&mut g, &mut a, &mut job, true, &[c]),
            BTreeMap::from([
                (0, Condition::TRUE),
                (1, Condition::TRUE),
                (2, c),
                (3, c.not())
            ])
        );
    }
}

#[test]
fn membership_matches_equal_on_multiple_scopes_with_collection_every_tick() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let choices: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let c = choices[0];
    let d = choices[1];
    let e = choices[2];
    let cd = boolean(&mut a, Operation::And(c, d));
    let ce = boolean(&mut a, Operation::Or(c.not(), e));
    let mut root = g.empty();
    for (x, y, support) in [
        (0, 2, c),
        (1, 3, c.not()),
        (0, 1, d),
        (2, 4, e),
        (3, 5, ce),
        (4, 6, cd),
        (5, 7, e.not()),
    ] {
        root = merge(&mut g, &mut a, root, x, y, support);
    }
    let scopes = [Condition::TRUE, Condition::FALSE, c, c.not(), cd, ce];
    for query in 0..9 {
        for scope in scopes {
            let expected: Vec<_> = (0..9)
                .map(|v| {
                    let support = equal(&g, &mut a, root, query, v, scope);
                    (0..8)
                        .map(|bits| a.evaluate(support, |i| bits & (1 << i) != 0))
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut job = Members::new(&g, root, query, scope);
            let retained: Vec<_> = choices.iter().copied().chain(scopes).collect();
            let found = enumerate(&mut g, &mut a, &mut job, true, &retained);
            for v in 0..9 {
                let support = found.get(&v).copied().unwrap_or(Condition::FALSE);
                for (bits, &expected) in expected[v as usize].iter().enumerate() {
                    assert_eq!(
                        a.evaluate(support, |i| bits & (1 << i) != 0),
                        expected,
                        "query {query}, member {v}, scope {scope:?}, world {bits}"
                    );
                }
            }
            assert!(found.keys().all(|v| *v < 9));
        }
    }
}

#[test]
fn frozen_root_survives_later_merges_and_collection() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 10, 20, c);
    let mut job = Members::new(&g, root, 20, Condition::TRUE);
    // Suspend after some resolver work, then create a newer graph version.
    for _ in 0..3 {
        assert_eq!(job.tick(&g, &mut a), ResolveStatus::Pending);
    }
    let _later = merge(&mut g, &mut a, root, 20, 30, Condition::TRUE);
    assert_eq!(
        enumerate(&mut g, &mut a, &mut job, true, &[c]),
        BTreeMap::from([(10, c), (20, Condition::TRUE)])
    );
    assert_eq!(job.root(), root);
}

#[test]
fn sparse_class_visits_do_not_scale_with_unrelated_variables() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let empty = g.empty();
    let mut root = merge(&mut g, &mut a, empty, 0, 1, Condition::TRUE);
    let mut small = Members::new(&g, root, 1, Condition::TRUE);
    let expected = enumerate(&mut g, &mut a, &mut small, false, &[]);
    assert_eq!(
        expected,
        BTreeMap::from([(0, Condition::TRUE), (1, Condition::TRUE)])
    );
    for x in (1000..9000).step_by(2) {
        root = merge(&mut g, &mut a, root, x, x + 1, Condition::TRUE);
    }
    let mut large = Members::new(&g, root, 1, Condition::TRUE);
    assert_eq!(enumerate(&mut g, &mut a, &mut large, false, &[]), expected);
    assert!(
        large.visits() <= small.visits() + 256,
        "small {}, large {}",
        small.visits(),
        large.visits()
    );
    assert_eq!(
        a.node_count(),
        0,
        "uniform enumeration needs no Boolean nodes"
    );
}
