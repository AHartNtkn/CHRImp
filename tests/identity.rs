use chr::condition::{Arena, Condition};
use chr::graph::Graph;
use chr::identity::{Equal, Merge, Resolve, ResolveStatus};
use chr::store::Root;

fn merge(g: &mut Graph, a: &mut Arena, root: Root, x: u64, y: u64, c: Condition) -> Root {
    let mut job = Merge::new(g, root, x, y, c);
    for _ in 0..100_000 {
        if let Some(root) = job.tick(g, a) {
            return root;
        }
    }
    panic!("finite union did not complete");
}
fn equal(g: &Graph, a: &mut Arena, root: Root, x: u64, y: u64, c: Condition) -> Condition {
    let mut job = Equal::new(g, root, x, y, c);
    for _ in 0..100_000 {
        if let Some(c) = job.tick(g, a) {
            return c;
        }
    }
    panic!("finite identity test did not complete");
}

#[test]
fn identity_tests_are_nonbinding_and_merges_are_conditional() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let before = g.index_node_count();
    assert_eq!(
        equal(&g, &mut a, empty, 1, 2, Condition::TRUE),
        Condition::FALSE
    );
    assert_eq!(g.index_node_count(), before);
    let root = merge(&mut g, &mut a, empty, 1, 2, c);
    assert_eq!(equal(&g, &mut a, root, 1, 2, Condition::TRUE), c);
    assert_eq!(equal(&g, &mut a, root, 1, 2, c.not()), Condition::FALSE);
    assert_eq!(
        equal(&g, &mut a, empty, 1, 2, Condition::TRUE),
        Condition::FALSE
    );
    let before = g.index_node_count();
    let mut repeat = Merge::new(&g, root, 1, 2, c);
    let same = loop {
        if let Some(root) = repeat.tick(&mut g, &mut a) {
            break root;
        }
    };
    assert_eq!(same, root);
    assert_eq!(repeat.changed_support(), Condition::FALSE);
    assert_eq!(g.index_node_count(), before);
}

#[test]
fn opposite_parent_directions_in_siblings_do_not_create_a_logical_cycle() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 0, 2, c);
    let root = merge(&mut g, &mut a, root, 1, 3, c.not());
    let root = merge(&mut g, &mut a, root, 0, 1, Condition::TRUE);
    assert_eq!(
        equal(&g, &mut a, root, 0, 1, Condition::TRUE),
        Condition::TRUE
    );
    assert_eq!(
        equal(&g, &mut a, root, 2, 3, Condition::TRUE),
        Condition::FALSE
    );
    let mut resolve = Resolve::new(&g, root, 0, Condition::TRUE);
    let mut parts = Vec::new();
    let mut done = false;
    for _ in 0..1000 {
        match resolve.tick(&g, &mut a) {
            ResolveStatus::Found { variable, support } => parts.push((variable, support)),
            ResolveStatus::Done => {
                done = true;
                break;
            }
            ResolveStatus::Pending => {}
        }
    }
    assert!(done, "conditional cycle resolution must finish");
    parts.sort_by_key(|p| p.0);
    assert_eq!(parts, [(0, c), (1, c.not())]);
}

#[test]
fn rank_prevents_adversarial_descending_alias_chains() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let mut root = g.empty();
    for x in (0..511).rev() {
        root = merge(&mut g, &mut a, root, x, x + 1, Condition::TRUE);
    }
    for x in [0, 1, 127, 255, 510, 511] {
        let mut resolve = Resolve::new(&g, root, x, Condition::TRUE);
        let mut found = Vec::new();
        loop {
            match resolve.tick(&g, &mut a) {
                ResolveStatus::Found { variable, support } => found.push((variable, support)),
                ResolveStatus::Done => break,
                ResolveStatus::Pending => {}
            }
        }
        assert_eq!(found, [(510, Condition::TRUE)]);
        assert!(
            resolve.visits() <= 2,
            "rank must prevent linear parent chains"
        );
    }
    assert_eq!(
        a.node_count(),
        0,
        "uniform identity work creates no Boolean nodes"
    );
}

#[test]
fn projected_equivalence_matches_independent_union_find_oracles() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let choices: Vec<_> = (0..3).map(|_| a.fresh_choice().1).collect();
    let mut worlds = vec![(0..8).collect::<Vec<usize>>(); 8];
    fn find(p: &[usize], mut x: usize) -> usize {
        while p[x] != x {
            x = p[x];
        }
        x
    }
    let mut root = g.empty();
    let mut seed = 19_u64;
    for n in 0..40 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = (seed >> 16) as usize % 8;
        let y = (seed >> 32) as usize % 8;
        let c = if n % 7 == 0 {
            Condition::TRUE
        } else if n % 2 == 0 {
            choices[n % 3]
        } else {
            choices[n % 3].not()
        };
        root = merge(&mut g, &mut a, root, x as u64, y as u64, c);
        for (bits, p) in worlds.iter_mut().enumerate() {
            if a.evaluate(c, |i| bits & (1 << i) != 0) {
                let rx = find(p, x);
                let ry = find(p, y);
                p[rx] = ry;
            }
        }
        for x in 0..8 {
            for y in 0..8 {
                let support = equal(&g, &mut a, root, x as u64, y as u64, Condition::TRUE);
                for (bits, p) in worlds.iter().enumerate() {
                    assert_eq!(
                        a.evaluate(support, |i| bits & (1 << i) != 0),
                        find(p, x) == find(p, y),
                        "merge{n} pair{x},{y} world{bits}"
                    );
                }
            }
        }
    }
}

#[test]
fn staged_union_survives_graph_and_condition_collection_at_every_tick() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let (_, d) = a.fresh_choice();
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 0, 2, c);
    let root = merge(&mut g, &mut a, root, 1, 3, d);
    let mut job = Merge::new(&g, root, 0, 1, Condition::TRUE);
    let final_root = loop {
        let mut gc = g.collect(job.roots().into_iter());
        let mut supports = vec![c, d];
        while !gc.done() {
            if let Some(support) = gc.tick(&mut g) {
                supports.push(support);
            }
        }
        drop(gc);
        let mut gc = a.collect(job.condition_roots().chain(supports));
        while !gc.tick(&mut a) {}
        drop(gc);
        if let Some(root) = job.tick(&mut g, &mut a) {
            break root;
        }
    };
    assert_eq!(job.changed_support(), Condition::TRUE);
    assert_eq!(
        equal(&g, &mut a, root, 0, 1, Condition::TRUE),
        Condition::FALSE
    );
    let support = equal(&g, &mut a, final_root, 2, 3, Condition::TRUE);
    for bits in 0..4 {
        assert_eq!(a.evaluate(support, |i| bits & (1 << i) != 0), bits == 3);
    }
    assert_eq!(
        equal(&g, &mut a, final_root, 0, 1, Condition::TRUE),
        Condition::TRUE
    );
}

fn traced<T: chr::trace::Trace>(
    job: &T,
    inventory: impl Iterator<Item = Condition>,
) -> Vec<Condition> {
    use chr::trace::{Cursor, Step};
    let mut expected: Vec<_> = inventory.collect();
    let mut cursor = Cursor::default();
    let mut roots = Vec::new();
    for _ in 0..(8 * expected.len() + 200) {
        match job.trace(&mut cursor) {
            Step::Root(c) => roots.push(c),
            Step::Pending => {}
            Step::Done => {
                assert_eq!(job.trace(&mut cursor), Step::Done);
                expected.sort();
                roots.sort();
                assert_eq!(roots, expected, "nested trace inventory differs");
                return roots;
            }
        }
    }
    panic!("identity trace did not finish in scalar steps");
}
fn collect_traced(
    g: &mut Graph,
    a: &mut Arena,
    roots: impl Iterator<Item = Root>,
    mut supports: Vec<Condition>,
) {
    let mut gc = g.collect(roots);
    while !gc.done() {
        if let Some(c) = gc.tick(g) {
            supports.push(c);
        }
    }
    drop(gc);
    let mut gc = a.collect(supports.into_iter());
    while !gc.tick(a) {}
}

#[test]
fn nested_identity_traces_survive_gc_through_all_merge_equal_and_resolve_phases() {
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let c = a.fresh_choice().1;
    let d = a.fresh_choice().1;
    let empty = g.empty();
    let root = merge(&mut g, &mut a, empty, 0, 2, c);
    let root = merge(&mut g, &mut a, root, 1, 3, d);
    let mut merging = Merge::new(&g, root, 0, 1, Condition::TRUE);
    let root = loop {
        let mut supports = traced(&merging, merging.condition_roots());
        supports.extend([c, d]);
        collect_traced(&mut g, &mut a, merging.roots().into_iter(), supports);
        if let Some(root) = merging.tick(&mut g, &mut a) {
            traced(&merging, merging.condition_roots());
            break root;
        }
    };
    let mut comparing = Equal::new(&g, root, 2, 3, Condition::TRUE);
    let result = loop {
        let supports = traced(&comparing, comparing.condition_roots());
        collect_traced(&mut g, &mut a, [root].into_iter(), supports);
        if let Some(result) = comparing.tick(&g, &mut a) {
            traced(&comparing, comparing.condition_roots());
            break result;
        }
    };
    for bits in 0..4 {
        assert_eq!(a.evaluate(result, |i| bits & (1 << i) != 0), bits == 3);
    }
    let mut resolving = Resolve::new(&g, root, 3, Condition::TRUE);
    let mut observed = vec![Vec::new(); 4];
    for _ in 0..10_000 {
        let supports = traced(&resolving, resolving.condition_roots());
        collect_traced(&mut g, &mut a, [root].into_iter(), supports);
        match resolving.tick(&g, &mut a) {
            ResolveStatus::Found { variable, support } => {
                for (bits, found) in observed.iter_mut().enumerate() {
                    if a.evaluate(support, |i| bits & (1 << i) != 0) {
                        found.push(variable);
                    }
                }
            }
            ResolveStatus::Pending => {}
            ResolveStatus::Done => {
                traced(&resolving, resolving.condition_roots());
                assert_eq!(observed, [vec![3], vec![3], vec![1], vec![0]]);
                return;
            }
        }
    }
    panic!("traced resolution did not finish");
}

#[test]
fn resolve_large_visited_map_cleanup_yields_one_record_per_tick() {
    use chr::condition::{Operation, Progress};
    let mut g = Graph::new(&[]);
    let mut a = Arena::default();
    let choices: Vec<_> = (0..5).map(|_| a.fresh_choice().1).collect();
    let mut root = g.empty();
    for bits in 0..32 {
        let mut support = Condition::TRUE;
        for (i, &c) in choices.iter().enumerate() {
            let literal = if bits & (1 << i) != 0 { c } else { c.not() };
            let mut job = a.start(Operation::And(support, literal));
            support = loop {
                if let Progress::Complete(c) = job.tick(&mut a) {
                    break c;
                }
            };
        }
        root = merge(&mut g, &mut a, root, 100, bits, support);
    }
    let mut resolving = Resolve::new(&g, root, 100, Condition::TRUE);
    let mut found = 0;
    for _ in 0..100_000 {
        let before = resolving.condition_roots().count();
        traced(&resolving, resolving.condition_roots());
        let status = resolving.tick(&g, &mut a);
        let after = resolving.condition_roots().count();
        assert!(
            before.saturating_sub(after) <= 8,
            "one tick discarded many visited roots: {before} -> {after}"
        );
        match status {
            ResolveStatus::Found { .. } => found += 1,
            ResolveStatus::Pending => {}
            ResolveStatus::Done => {
                assert_eq!(found, 32);
                return;
            }
        }
    }
    panic!("wide resolution did not finish");
}
