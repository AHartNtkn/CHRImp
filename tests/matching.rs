use chr::condition::{Arena, Condition};
use chr::graph::{Graph, UpdateStatus};
use chr::identity::Merge;
use chr::matching::{MatchStatus, Matches};
use chr::program::{Prepared, prepare};
use chr::store::Root;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

fn code(source: &str) -> Arc<Prepared> {
    Arc::new(
        prepare(
            &parse_program(source).unwrap(),
            &parse_query("true").unwrap(),
        )
        .unwrap(),
    )
}
fn post(g: &mut Graph, root: Root, relation: usize, args: Vec<u64>) -> (Root, u64) {
    let mut p = g.post(root, relation, args, Condition::TRUE).unwrap();
    let id = p.occurrence();
    loop {
        if let UpdateStatus::Complete(root) = p.tick(g) {
            return (root, id);
        }
    }
}

#[test]
fn selective_join_uses_bound_ports_instead_of_scanning_the_relation() {
    let code = code("p(X), q(X) ==> hit(X).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let empty = g.empty();
    let (mut root, p) = post(&mut g, empty, 0, vec![73]);
    let mut expected = 0;
    for x in 0..512 {
        let (r, id) = post(&mut g, root, 1, vec![x]);
        root = r;
        if x == 73 {
            expected = id;
        }
    }
    let mut matches = Matches::new(&g, root, code, 0, Condition::TRUE, Some((0, p))).unwrap();
    let mut found = Vec::new();
    let mut done = false;
    for _ in 0..20_000 {
        match tick_matches(&mut matches, &g, &mut a) {
            MatchStatus::Found(m) => found.push(m),
            MatchStatus::Done => {
                done = true;
                break;
            }
            MatchStatus::Pending => {}
        }
    }
    assert!(done, "finite selective cursor must exhaust");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].occurrences, [p, expected]);
    assert_eq!(found[0].bindings, [73]);
    assert!(
        matches.candidate_visits() <= 2,
        "bound lookup must not scan all q occurrences"
    );
}

#[test]
fn repeated_variables_only_match_identity_that_already_holds() {
    let code = code("p(X,X) <=> done(X).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![7, 8]);
    let mut no = Matches::new(&g, root.clone(), code.clone(), 0, Condition::TRUE, None).unwrap();
    let before = g.index_node_count();
    loop {
        match tick_matches(&mut no, &g, &mut a) {
            MatchStatus::Found(_) => panic!("matching must not merge distinct variables"),
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
    assert_eq!(g.index_node_count(), before);
    let mut union = Merge::new(&g, root, 7, 8, c);
    let root = loop {
        if let Some(r) = union.tick(&mut g, &mut a) {
            break r;
        }
    };
    let mut yes = Matches::new(&g, root, code, 0, Condition::TRUE, None).unwrap();
    let mut found = Vec::new();
    loop {
        match tick_matches(&mut yes, &g, &mut a) {
            MatchStatus::Found(m) => found.push(m),
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].occurrences, [p]);
    assert_eq!(found[0].support, c);
}

#[test]
fn equal_rows_still_require_distinct_occurrences_and_preserve_head_order() {
    let code = code("p(X), p(Y) ==> pair_witness(X,Y).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let empty = g.empty();
    let (root, one) = post(&mut g, empty, 0, vec![9]);
    let mut no = Matches::new(&g, root.clone(), code.clone(), 0, Condition::TRUE, None).unwrap();
    loop {
        match tick_matches(&mut no, &g, &mut a) {
            MatchStatus::Found(_) => panic!("one occurrence cannot occupy two heads"),
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
    let (root, two) = post(&mut g, root, 0, vec![9]);
    let mut yes = Matches::new(&g, root, code, 0, Condition::TRUE, None).unwrap();
    let mut tuples = Vec::new();
    loop {
        match tick_matches(&mut yes, &g, &mut a) {
            MatchStatus::Found(m) => {
                assert_eq!(m.bindings, [9, 9]);
                tuples.push(m.occurrences);
            }
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
    tuples.sort();
    assert_eq!(tuples, vec![vec![one, two], vec![two, one]]);
}

#[test]
fn alias_sensitive_cross_predicate_join_retains_its_condition_through_collection() {
    let code = code("p(X) \\ q(X) <=> r(X,Fresh).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![1]);
    let (root, q) = post(&mut g, root, 1, vec![2]);
    let mut union = Merge::new(&g, root, 1, 2, c);
    let root = loop {
        if let Some(r) = union.tick(&mut g, &mut a) {
            break r;
        }
    };
    let mut matches = Matches::new(&g, root, code, 0, Condition::TRUE, Some((0, p))).unwrap();
    let mut found = Vec::new();
    loop {
        let mut gc = g.collect([matches.root()].into_iter());
        let mut supports = vec![c];
        while !gc.done() {
            if let Some(s) = gc.tick(&mut g) {
                supports.push(s);
            }
        }
        drop(gc);
        let mut gc = a.collect(
            traced_roots(&matches, matches.condition_roots())
                .into_iter()
                .chain(supports),
        );
        while !gc.tick(&mut a) {}
        drop(gc);
        match tick_matches(&mut matches, &g, &mut a) {
            MatchStatus::Found(m) => found.push(m),
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].occurrences, [p, q]);
    assert_eq!(found[0].bindings, [1]);
    assert_eq!(found[0].support, c);
}

#[test]
fn indexed_port_does_not_replace_other_repeated_variable_guards() {
    let code = code("p(X), q(X,X) ==> hit(X).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let empty = g.empty();
    let (root, p) = post(&mut g, empty, 0, vec![1]);
    let (root, _) = post(&mut g, root, 1, vec![2, 3]);
    let mut first = Merge::new(&g, root, 1, 2, c);
    let root = loop {
        if let Some(r) = first.tick(&mut g, &mut a) {
            break r;
        }
    };
    let mut second = Merge::new(&g, root, 1, 3, c.not());
    let root = loop {
        if let Some(r) = second.tick(&mut g, &mut a) {
            break r;
        }
    };
    let mut matches = Matches::new(&g, root, code, 0, Condition::TRUE, Some((0, p))).unwrap();
    loop {
        match tick_matches(&mut matches, &g, &mut a) {
            MatchStatus::Found(_) => panic!("port guards hold in incompatible alternatives"),
            MatchStatus::Done => break,
            MatchStatus::Pending => {}
        }
    }
}

#[test]
fn three_head_joins_restore_bindings_after_failed_prefixes_and_allow_any_anchor() {
    let code = code("p(X,Y), q(Y,Z), r(Z,X) ==> triangle(X,Y,Z).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let mut root = g.empty();
    let mut ids = Vec::new();
    for (relation, args) in [
        (0, vec![1, 2]),
        (0, vec![3, 4]),
        (1, vec![2, 5]),
        (1, vec![2, 6]),
        (1, vec![4, 7]),
        (2, vec![5, 1]),
        (2, vec![6, 9]),
        (2, vec![7, 3]),
    ] {
        let (r, id) = post(&mut g, root, relation, args);
        root = r;
        ids.push(id);
    }
    for anchor in [
        None,
        Some((0, ids[0])),
        Some((1, ids[2])),
        Some((2, ids[5])),
        Some((1, ids[3])),
    ] {
        let mut matches =
            Matches::new(&g, root.clone(), code.clone(), 0, Condition::TRUE, anchor).unwrap();
        let mut result = Vec::new();
        let mut done = false;
        for _ in 0..100_000 {
            match tick_matches(&mut matches, &g, &mut a) {
                MatchStatus::Found(m) => result.push((m.occurrences, m.bindings)),
                MatchStatus::Done => {
                    done = true;
                    break;
                }
                MatchStatus::Pending => {}
            }
        }
        assert!(done, "finite anchored cursor must exhaust");
        result.sort();
        let first = (vec![ids[0], ids[2], ids[5]], vec![1, 2, 5]);
        let expected = if anchor.is_none() {
            vec![first, (vec![ids[1], ids[4], ids[7]], vec![3, 4, 7])]
        } else if anchor == Some((1, ids[3])) {
            vec![]
        } else {
            vec![first]
        };
        assert_eq!(result, expected, "anchor{anchor:?}");
    }
}

fn traced_roots(
    job: &impl chr::trace::Trace,
    expected: impl Iterator<Item = Condition>,
) -> Vec<Condition> {
    use chr::trace::{Cursor, Step};
    let mut expected: Vec<_> = expected.collect();
    let mut cursor = Cursor::default();
    let mut roots = Vec::new();
    for _ in 0..16 * expected.len() + 256 {
        match job.trace(&mut cursor) {
            Step::Root(root) => roots.push(root),
            Step::Pending => {}
            Step::Done => {
                assert_eq!(job.trace(&mut cursor), Step::Done);
                roots.sort_unstable();
                expected.sort_unstable();
                assert_eq!(roots, expected, "incremental trace changed root inventory");
                return roots;
            }
        }
    }
    panic!("root walk exceeded its linear step budget");
}

fn tick_matches(job: &mut Matches, graph: &Graph, arena: &mut Arena) -> MatchStatus {
    traced_roots(job, job.condition_roots());
    job.tick(graph, arena)
}

#[test]
fn skewed_multi_bound_join_selects_small_buckets_in_either_port_order() {
    for swapped in [false, true] {
        let code = code("r(X,Y), s(X,Y) ==> hit(X,Y).");
        let mut g = Graph::new(code.signatures());
        let mut a = Arena::default();
        let mut root = g.empty();
        let mut expected = Vec::new();
        for i in 1..=256 {
            let args = if swapped { vec![i, 0] } else { vec![0, i] };
            let (r, left) = post(&mut g, root, 0, args.clone());
            let (r, right) = post(&mut g, r, 1, args.clone());
            root = r;
            expected.push((vec![left, right], args, Condition::TRUE));
        }
        let mut matches = Matches::new(&g, root, code, 0, Condition::TRUE, None).unwrap();
        let mut found = Vec::new();
        let mut done = false;
        for _ in 0..200_000 {
            match tick_matches(&mut matches, &g, &mut a) {
                MatchStatus::Found(m) => found.push((m.occurrences, m.bindings, m.support)),
                MatchStatus::Done => {
                    done = true;
                    break;
                }
                MatchStatus::Pending => {}
            }
        }
        assert!(
            done,
            "skewed join exceeded finite step budget; swapped={swapped}"
        );
        found.sort();
        expected.sort();
        assert_eq!(found, expected);
        assert_eq!(matches.candidate_visits(), 512, "swapped={swapped}");
    }
}

#[test]
fn selective_alias_buckets_preserve_all_port_and_row_supports_during_collection() {
    use chr::condition::{Operation, Progress};
    for swapped in [false, true] {
        let code = code("r(X,Y), s(X,Y) ==> hit(X,Y).");
        let mut g = Graph::new(code.signatures());
        let mut a = Arena::default();
        let (_, common) = a.fresh_choice();
        let (_, selective) = a.fresh_choice();
        let (_, row) = a.fresh_choice();
        let args = |x, y| if swapped { vec![y, x] } else { vec![x, y] };
        let empty = g.empty();
        let (mut root, anchor) = post(&mut g, empty, 0, args(0, 1));
        for i in 3..=34 {
            root = post(&mut g, root, 1, args(100, i)).0;
        }
        let mut p = g.post(root, 1, args(100, 2), row).unwrap();
        let target = p.occurrence();
        root = loop {
            if let UpdateStatus::Complete(r) = p.tick(&mut g) {
                break r;
            }
        };
        for (x, y, support) in [(0, 100, common), (1, 2, selective)] {
            let mut merge = Merge::new(&g, root, x, y, support);
            root = loop {
                if let Some(r) = merge.tick(&mut g, &mut a) {
                    break r;
                }
            };
        }
        let mut expected = row;
        for c in [common, selective] {
            let mut job = a.start(Operation::And(expected, c));
            expected = loop {
                if let Progress::Complete(c) = job.tick(&mut a) {
                    break c;
                }
            };
        }
        let mut matches =
            Matches::new(&g, root, code, 0, Condition::TRUE, Some((0, anchor))).unwrap();
        let mut found = Vec::new();
        let mut done = false;
        for _ in 0..20_000 {
            let mut supports = vec![expected];
            let mut gc = g.collect([matches.root()].into_iter());
            while !gc.done() {
                if let Some(c) = gc.tick(&mut g) {
                    supports.push(c);
                }
            }
            drop(gc);
            supports.extend(traced_roots(&matches, matches.condition_roots()));
            let mut gc = a.collect(supports.into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            match tick_matches(&mut matches, &g, &mut a) {
                MatchStatus::Found(m) => found.push((m.occurrences, m.bindings, m.support)),
                MatchStatus::Done => {
                    done = true;
                    break;
                }
                MatchStatus::Pending => {}
            }
        }
        assert!(done, "conditional alias join exceeded step budget");
        assert_eq!(found, vec![(vec![anchor, target], args(0, 1), expected)]);
        assert_eq!(
            matches.candidate_visits(),
            2,
            "count supported alias buckets; swapped={swapped}"
        );
    }
}

#[test]
fn rejected_three_head_prefix_uses_empty_partner_before_broad_partner() {
    for source in [
        "p(X,Y),q(Y,Z),r(Z,X) ==> hit(X,Y,Z).",
        "q(Y,Z),r(Z,X),p(X,Y) ==> hit(X,Y,Z).",
    ] {
        let code = code(source);
        let relation = |name: &str| {
            code.signatures()
                .iter()
                .position(|s| s.name == name)
                .unwrap()
        };
        let anchor_head = code
            .heads(&code.rules()[0])
            .iter()
            .position(|h| h.relation == relation("p"))
            .unwrap();
        for n in [16, 64, 256] {
            let mut g = Graph::new(code.signatures());
            let mut a = Arena::default();
            let empty = g.empty();
            let (mut root, anchor) = post(&mut g, empty, relation("p"), vec![1, 2]);
            for i in 0..n {
                root = post(&mut g, root, relation("q"), vec![2, 10 + i]).0;
                root = post(&mut g, root, relation("r"), vec![10 + i, 1000 + i]).0;
            }
            let mut job = Matches::new(
                &g,
                root,
                code.clone(),
                0,
                Condition::TRUE,
                Some((anchor_head, anchor)),
            )
            .unwrap();
            let mut done = false;
            for _ in 0..100_000 {
                match tick_matches(&mut job, &g, &mut a) {
                    MatchStatus::Found(_) => panic!("distinct endpoints cannot close the cycle"),
                    MatchStatus::Done => {
                        done = true;
                        break;
                    }
                    MatchStatus::Pending => {}
                }
            }
            assert!(done);
            assert!(
                job.candidate_visits() <= 2,
                "n={n}: {} candidates for an empty partner",
                job.candidate_visits()
            );
        }
    }
}

#[test]
fn small_relation_lookup_does_not_enumerate_large_alias_class() {
    let code = code("p(X),q(X) ==> hit(X).");
    let mut first_ticks = 0;
    for n in [16, 64, 256] {
        let mut g = Graph::new(code.signatures());
        let mut a = Arena::default();
        let empty = g.empty();
        let (root, p) = post(&mut g, empty, 0, vec![0]);
        let (mut root, q) = post(&mut g, root, 1, vec![n]);
        for i in 1..=n {
            let mut merge = Merge::new(&g, root, 0, i, Condition::TRUE);
            root = loop {
                if let Some(root) = merge.tick(&mut g, &mut a) {
                    break root;
                }
            };
        }
        let mut job =
            Matches::new(&g, root, code.clone(), 0, Condition::TRUE, Some((0, p))).unwrap();
        let mut found = vec![];
        let ticks = (1..100_000)
            .find(|_| match tick_matches(&mut job, &g, &mut a) {
                MatchStatus::Found(m) => {
                    found.push(m);
                    false
                }
                MatchStatus::Done => true,
                MatchStatus::Pending => false,
            })
            .expect("finite alias lookup");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].occurrences, [p, q]);
        assert_eq!(found[0].bindings, [0]);
        assert_eq!(found[0].support, Condition::TRUE);
        if n == 16 {
            first_ticks = ticks;
        }
        assert!(
            ticks <= first_ticks * 3,
            "alias class {n}: {first_ticks} -> {ticks} ticks for one partner"
        );
    }
}

#[test]
fn fully_bound_correlation_lookup_does_not_scan_individual_port_buckets() {
    let code = code("probe(X,Y,I),row(X,Y) ==> hit(I).");
    for n in [16, 64, 256] {
        let mut g = Graph::new(code.signatures());
        let mut a = Arena::default();
        let empty = g.empty();
        let (mut root, p) = post(&mut g, empty, 0, vec![1, 2, 3]);
        for i in 0..n {
            root = post(&mut g, root, 1, vec![1, 100 + i]).0;
            root = post(&mut g, root, 1, vec![1000 + i, 2]).0;
        }
        for hit in [false, true] {
            let mut expected = vec![];
            if hit {
                let (r, q) = post(&mut g, root, 1, vec![1, 2]);
                root = r;
                expected.push(vec![p, q]);
            }
            let mut job = Matches::new(
                &g,
                root.clone(),
                code.clone(),
                0,
                Condition::TRUE,
                Some((0, p)),
            )
            .unwrap();
            let mut found = vec![];
            let done = (0..100_000).any(|_| match tick_matches(&mut job, &g, &mut a) {
                MatchStatus::Found(m) => {
                    assert_eq!(m.bindings, [1, 2, 3]);
                    assert_eq!(m.support, Condition::TRUE);
                    found.push(m.occurrences);
                    false
                }
                MatchStatus::Done => true,
                MatchStatus::Pending => false,
            });
            assert!(done);
            assert_eq!(found, expected);
            assert!(
                job.candidate_visits() <= 2,
                "n={n},hit={hit}: {} visits for correlated ports",
                job.candidate_visits()
            );
        }
    }
}

#[test]
fn tuple_buckets_recheck_collisions_swapped_ports_and_conditional_aliases() {
    let code = code("probe(X,Y),row(X,Y) ==> hit(X,Y).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let c = a.fresh_choice().1;
    let empty = g.empty();
    let (mut root, p) = post(&mut g, empty, 0, vec![1, 2]);
    // h(1,2) == h(0,P+2); a bucket collision is not an identity proof.
    root = post(&mut g, root, 1, vec![0, 0x9e3779b97f4a7c17]).0;
    root = post(&mut g, root, 1, vec![2, 1]).0;
    for i in 10..26 {
        root = post(&mut g, root, 1, vec![1, i]).0;
        root = post(&mut g, root, 1, vec![i, 2]).0;
    }
    let (r, exact) = post(&mut g, root, 1, vec![1, 2]);
    root = r;
    let (r, alias) = post(&mut g, root, 1, vec![100, 200]);
    root = r;
    let (r, parent_anchor) = post(&mut g, root, 0, vec![100, 200]);
    root = r;
    let old = root.clone();
    for (x, y) in [(1, 100), (2, 200)] {
        let mut merge = Merge::new(&g, root, x, y, c);
        root = loop {
            if let Some(r) = merge.tick(&mut g, &mut a) {
                break r;
            }
        };
    }
    let snapshots = [
        (
            old,
            Condition::TRUE,
            p,
            vec![1, 2],
            vec![(vec![p, exact], Condition::TRUE)],
        ),
        (
            root.clone(),
            Condition::TRUE,
            p,
            vec![1, 2],
            vec![(vec![p, exact], Condition::TRUE), (vec![p, alias], c)],
        ),
        (
            root.clone(),
            c.not(),
            p,
            vec![1, 2],
            vec![(vec![p, exact], c.not())],
        ),
        (
            root,
            Condition::TRUE,
            parent_anchor,
            vec![100, 200],
            vec![
                (vec![parent_anchor, exact], c),
                (vec![parent_anchor, alias], Condition::TRUE),
            ],
        ),
    ];
    for (snapshot, scope, anchor, bindings, expected) in &snapshots {
        let mut job = Matches::new(
            &g,
            snapshot.clone(),
            code.clone(),
            0,
            *scope,
            Some((0, *anchor)),
        )
        .unwrap();
        let mut found = vec![];
        for _ in 0..20_000 {
            let mut roots = traced_roots(&job, job.condition_roots());
            roots.push(c);
            let mut gc = g.collect(snapshots.iter().map(|(r, ..)| r.clone()));
            while !gc.done() {
                if let Some(c) = gc.tick(&mut g) {
                    roots.push(c);
                }
            }
            drop(gc);
            let mut gc = a.collect(roots.into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            match job.tick(&g, &mut a) {
                MatchStatus::Found(m) => {
                    assert_eq!(&m.bindings, bindings);
                    found.push((m.occurrences, m.support));
                }
                MatchStatus::Done => break,
                MatchStatus::Pending => {}
            }
        }
        // Discovery may split one tuple into disjoint support fragments.
        // Project independently onto each alternative and check exact multiplicity.
        for bit in [false, true] {
            let project = |rows: &Vec<(Vec<u64>, Condition)>| {
                let mut out: Vec<_> = rows
                    .iter()
                    .filter(|(_, c)| a.evaluate(*c, |_| bit))
                    .map(|(ids, _)| ids.clone())
                    .collect();
                out.sort();
                out
            };
            assert_eq!(project(&found), project(expected));
        }
    }
}

#[test]
fn interrupted_alias_planning_discards_incrementally_under_collection() {
    let code = code("p(X),q(X) ==> hit(X).");
    let mut g = Graph::new(code.signatures());
    let mut a = Arena::default();
    let c = a.fresh_choice().1;
    let empty = g.empty();
    let (mut root, p) = post(&mut g, empty, 0, vec![0]);
    for i in 1..=64 {
        let mut merge = Merge::new(&g, root, 0, i, c);
        root = loop {
            if let Some(r) = merge.tick(&mut g, &mut a) {
                break r;
            }
        };
    }
    for i in 1..=4 {
        root = post(&mut g, root, 1, vec![i]).0;
    }
    for prefix in [12, 24, 48, 96, 192] {
        let mut job = Matches::new(
            &g,
            root.clone(),
            code.clone(),
            0,
            Condition::TRUE,
            Some((0, p)),
        )
        .unwrap();
        for _ in 0..prefix {
            tick_matches(&mut job, &g, &mut a);
        }
        let visits = job.candidate_visits();
        let mut done = false;
        for step in 0..1000 {
            let mut conditions = traced_roots(&job, job.condition_roots());
            conditions.push(c);
            let mut gc = g.collect([root.clone()].into_iter());
            while !gc.done() {
                if let Some(c) = gc.tick(&mut g) {
                    conditions.push(c);
                }
            }
            drop(gc);
            let mut gc = a.collect(conditions.into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            let before = job.condition_roots().count();
            let nodes = (g.index_allocations(), a.node_count());
            done = job.discard_tick();
            assert_eq!(nodes, (g.index_allocations(), a.node_count()));
            if step > 0 {
                assert!(before.saturating_sub(job.condition_roots().count()) <= 12);
            }
            if done {
                break;
            }
        }
        assert!(done);
        assert_eq!(job.candidate_visits(), visits);
        assert!(job.condition_roots().all(Condition::is_terminal));
    }
}

#[test]
fn output_only_binary_rewrite_preserves_linear_work_without_unused_indexes() {
    let n = 128;
    let source = chr::syntax::parse_program("p(X,Y) <=> done(X,Y).").unwrap();
    let query = chr::syntax::parse_query(
        &(0..n)
            .map(|i| format!("p(V{i},W{i})"))
            .collect::<Vec<_>>()
            .join(","),
    )
    .unwrap();
    let code = Arc::new(prepare(&source, &query).unwrap());
    let mut e = chr::engine::Engine::new(code);
    let mut facts = 0;
    let mut ports = vec![];
    let mut variables = vec![];
    let mut answers = 0;
    let mut ticks = 0;
    while !e.delivery_done() && ticks < 100_000 {
        e.advance(1);
        ticks += 1;
        while let Some(o) = e.take_output() {
            use chr::observe::Output;
            match o {
                Output::Variable { variable, .. } => variables.push(variable),
                Output::Fact { relation, .. } => {
                    assert_eq!(e.program().signatures()[relation].name, "done");
                    facts += 1;
                }
                Output::Port { variable } => ports.push(variable),
                Output::End => answers += 1,
                _ => {}
            }
        }
    }
    assert!(e.delivery_done());
    assert_eq!(answers, 1);
    assert_eq!(facts, n);
    assert_eq!(e.applications(), n as u64);
    assert_eq!(
        variables
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2 * n
    );
    let mut actual: Vec<_> = ports.chunks_exact(2).map(|x| x.to_vec()).collect();
    actual.sort();
    let mut expected: Vec<_> = variables.chunks_exact(2).map(|x| x.to_vec()).collect();
    expected.sort();
    assert_eq!(actual, expected);
    assert!(
        ticks < 26_000,
        "binary rewrite paid unused index work: {ticks}"
    );
}
