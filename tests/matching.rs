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
    let mut g = Graph::new(&code.signatures);
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
    let mut g = Graph::new(&code.signatures);
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
    let mut g = Graph::new(&code.signatures);
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
    let mut g = Graph::new(&code.signatures);
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
    let mut g = Graph::new(&code.signatures);
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
    let mut g = Graph::new(&code.signatures);
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
