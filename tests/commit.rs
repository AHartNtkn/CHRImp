use chr::commit::{Commit, CommitStatus, Committed, FreshIds, StateRoot};
use chr::condition::{Arena, Condition};
use chr::graph::{Graph, UpdateStatus};
use chr::history::History;
use chr::identity::Merge;
use chr::matching::{Match, MatchStatus, Matches};
use chr::program::{Prepared, prepare};
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

fn code(text: &str) -> Arc<Prepared> {
    Arc::new(prepare(&parse_program(text).unwrap(), &parse_query("true").unwrap()).unwrap())
}
fn post(g: &mut Graph, s: &mut StateRoot, args: Vec<u64>) -> u64 {
    let mut p = g.post(s.graph.clone(), 0, args, Condition::TRUE).unwrap();
    let id = p.occurrence();
    loop {
        if let UpdateStatus::Complete(root) = p.tick(g) {
            s.graph = root;
            return id;
        }
    }
}
fn find(g: &Graph, a: &mut Arena, s: StateRoot, p: Arc<Prepared>) -> Match {
    let mut m = Matches::new(g, s.graph, p, 0, Condition::TRUE, None).unwrap();
    loop {
        match m.tick(g, a) {
            MatchStatus::Found(m) => return m,
            MatchStatus::Done => panic!("expected a match"),
            MatchStatus::Pending => {}
        }
    }
}
fn run(
    j: &mut Commit,
    g: &mut Graph,
    a: &mut Arena,
    h: &mut History,
    ids: &mut FreshIds,
) -> Option<Committed> {
    for _ in 0..100_000 {
        traced_roots(j, j.condition_roots());
        match j.tick(g, a, h, ids) {
            CommitStatus::Applied(c) => return Some(c),
            CommitStatus::Rejected => return None,
            CommitStatus::Pending => {}
            CommitStatus::Done => panic!("result already consumed"),
        }
    }
    panic!("finite commitment failed to complete");
}

#[test]
fn propagation_records_tuple_once_and_allocates_fresh_locals_only_when_applying() {
    let p = code("p(X) ==> witness(Y,X).");
    let mut g = Graph::new(&p.signatures);
    let mut h = History::default();
    let mut a = Arena::default();
    let mut ids = FreshIds::default();
    let x = ids.variable();
    let mut s = StateRoot {
        graph: g.empty(),
        history: h.empty(),
    };
    let head = post(&mut g, &mut s, vec![x]);
    let m = find(&g, &mut a, s.clone(), p.clone());
    let mut j = Commit::new(&g, &h, s, p.clone(), 0, m, Condition::TRUE).unwrap();
    let committed = run(&mut j, &mut g, &mut a, &mut h, &mut ids).unwrap();
    s = committed.state;
    assert_eq!(committed.application.variables.as_slice(), &[x, x + 1]);
    assert_eq!(committed.application.heads.as_slice(), &[head]);
    assert_eq!(
        h.support(s.history.clone(), 0, &committed.application.heads),
        Condition::TRUE
    );
    assert!(g.fact(s.graph.clone(), head).is_some());
    let before = (ids.variable_count(), ids.event_count());
    let m = find(&g, &mut a, s.clone(), p.clone());
    let mut repeat = Commit::new(&g, &h, s, p, 0, m, Condition::TRUE).unwrap();
    assert!(run(&mut repeat, &mut g, &mut a, &mut h, &mut ids).is_none());
    assert_eq!((ids.variable_count(), ids.event_count()), before);
}

#[test]
fn additional_support_gets_a_distinct_fresh_event_without_replaying_overlap() {
    let p = code("p(X) ==> witness(Y,X).");
    let mut g = Graph::new(&p.signatures);
    let mut h = History::default();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let mut ids = FreshIds::default();
    let x = ids.variable();
    let mut s = StateRoot {
        graph: g.empty(),
        history: h.empty(),
    };
    let head = post(&mut g, &mut s, vec![x]);
    let m = find(&g, &mut a, s.clone(), p.clone());
    let mut first = Commit::new(&g, &h, s, p.clone(), 0, m, c).unwrap();
    let one = run(&mut first, &mut g, &mut a, &mut h, &mut ids).unwrap();
    s = one.state;
    let m = find(&g, &mut a, s.clone(), p.clone());
    let mut second = Commit::new(&g, &h, s, p.clone(), 0, m, Condition::TRUE).unwrap();
    let two = run(&mut second, &mut g, &mut a, &mut h, &mut ids).unwrap();
    s = two.state;
    assert_eq!(one.application.support, c);
    assert_eq!(two.application.support, c.not());
    assert_ne!(one.application.id, two.application.id);
    assert_ne!(one.application.variables[1], two.application.variables[1]);
    let y = ids.variable();
    let mut merge = Merge::new(&g, s.graph, x, y, Condition::TRUE);
    s.graph = loop {
        if let Some(root) = merge.tick(&mut g, &mut a) {
            break root;
        }
    };
    let m = Match {
        occurrences: vec![head],
        bindings: vec![x],
        support: Condition::TRUE,
    };
    let mut repeat = Commit::new(&g, &h, s, p, 0, m, Condition::TRUE).unwrap();
    assert!(run(&mut repeat, &mut g, &mut a, &mut h, &mut ids).is_none());
}

#[test]
fn stale_kept_heads_shrink_commitment_and_only_removed_heads_are_consumed() {
    let p = code("p(X) \\ q(X) <=> result(X,Fresh).");
    let mut g = Graph::new(&p.signatures);
    let mut h = History::default();
    let mut a = Arena::default();
    let (_, c) = a.fresh_choice();
    let mut ids = FreshIds::default();
    let x = ids.variable();
    let mut s = StateRoot {
        graph: g.empty(),
        history: h.empty(),
    };
    let kept = post(&mut g, &mut s, vec![x]);
    let mut q = g.post(s.graph, 1, vec![x], Condition::TRUE).unwrap();
    let removed = q.occurrence();
    s.graph = loop {
        if let UpdateStatus::Complete(r) = q.tick(&mut g) {
            break r;
        }
    };
    let m = find(&g, &mut a, s.clone(), p.clone());
    let original = s.clone();
    let mut consume = g.set_liveness(s.graph, kept, c.not()).unwrap();
    s.graph = loop {
        if let UpdateStatus::Complete(r) = consume.tick(&mut g) {
            break r;
        }
    };
    let mut commit = Commit::new(&g, &h, s, p, 0, m, Condition::TRUE).unwrap();
    let result = run(&mut commit, &mut g, &mut a, &mut h, &mut ids).unwrap();
    assert_eq!(result.application.support, c.not());
    assert_eq!(
        g.fact(result.state.graph.clone(), kept).unwrap().support,
        c.not()
    );
    assert_eq!(g.fact(result.state.graph, removed).unwrap().support, c);
    assert_eq!(
        g.fact(original.graph, removed).unwrap().support,
        Condition::TRUE
    );
    assert_eq!(result.state.history, original.history);
}

#[test]
fn malformed_identity_claim_cannot_bind_variables_or_consume_facts() {
    let p = code("p(X) \\ q(X) <=> result(X).");
    let mut g = Graph::new(&p.signatures);
    let mut h = History::default();
    let mut a = Arena::default();
    let mut ids = FreshIds::default();
    let x = ids.variable();
    let y = ids.variable();
    let mut s = StateRoot {
        graph: g.empty(),
        history: h.empty(),
    };
    let one = post(&mut g, &mut s, vec![x]);
    let mut q = g.post(s.graph, 1, vec![y], Condition::TRUE).unwrap();
    let two = q.occurrence();
    s.graph = loop {
        if let UpdateStatus::Complete(r) = q.tick(&mut g) {
            break r;
        }
    };
    let m = Match {
        occurrences: vec![one, two],
        bindings: vec![x],
        support: Condition::TRUE,
    };
    let mut j = Commit::new(&g, &h, s.clone(), p, 0, m, Condition::TRUE).unwrap();
    let before = g.index_node_count();
    assert!(run(&mut j, &mut g, &mut a, &mut h, &mut ids).is_none());
    assert_eq!(g.index_node_count(), before);
    assert!(g.fact(s.graph.clone(), one).is_some());
    assert!(g.fact(s.graph, two).is_some());
    assert_eq!(ids.event_count(), 0);
}

#[test]
fn duplicate_head_ids_and_failed_regions_cannot_apply() {
    let p = code("p(X), p(Y) <=> result(X,Y).");
    let mut g = Graph::new(&p.signatures);
    let mut h = History::default();
    let mut a = Arena::default();
    let mut ids = FreshIds::default();
    let x = ids.variable();
    let mut s = StateRoot {
        graph: g.empty(),
        history: h.empty(),
    };
    let id = post(&mut g, &mut s, vec![x]);
    for active in [Condition::TRUE, Condition::FALSE] {
        let m = Match {
            occurrences: vec![id, id],
            bindings: vec![x, x],
            support: Condition::TRUE,
        };
        let mut j = Commit::new(&g, &h, s.clone(), p.clone(), 0, m, active).unwrap();
        assert!(run(&mut j, &mut g, &mut a, &mut h, &mut ids).is_none());
    }
    assert!(g.fact(s.graph, id).is_some());
    assert_eq!(ids.event_count(), 0);
}

#[test]
fn collection_at_every_commit_boundary_preserves_staged_updates_and_guards() {
    for source in [
        "p(X,X), p(X,X) <=> result(X,Fresh).",
        "p(X,X), p(X,X) ==> result(X,Fresh).",
    ] {
        let p = code(source);
        let mut g = Graph::new(&p.signatures);
        let mut a = Arena::default();
        let mut h = History::default();
        let mut ids = FreshIds::default();
        let x = ids.variable();
        let y = ids.variable();
        let (_, c) = a.fresh_choice();
        let mut s = StateRoot {
            graph: g.empty(),
            history: h.empty(),
        };
        let first = post(&mut g, &mut s, vec![x, y]);
        let second = post(&mut g, &mut s, vec![y, x]);
        let mut merge = Merge::new(&g, s.graph, x, y, c);
        s.graph = loop {
            if let Some(root) = merge.tick(&mut g, &mut a) {
                break root;
            }
        };
        let candidate = Match {
            occurrences: vec![first, second],
            bindings: vec![x],
            support: Condition::TRUE,
        };
        let mut commit =
            Commit::new(&g, &h, s.clone(), p.clone(), 0, candidate, Condition::TRUE).unwrap();
        let mut result = None;
        for _ in 0..10000 {
            let mut conditions = traced_roots(&commit, commit.condition_roots());
            let mut gc = g.collect(commit.graph_roots());
            while !gc.done() {
                if let Some(c) = gc.tick(&mut g) {
                    conditions.push(c);
                }
            }
            drop(gc);
            let mut gc = h.collect(commit.history_roots().into_iter());
            while !gc.done() {
                if let Some(c) = gc.tick(&mut h) {
                    conditions.push(c);
                }
            }
            drop(gc);
            let mut gc = a.collect(conditions.into_iter());
            while !gc.tick(&mut a) {}
            drop(gc);
            assert_eq!(
                g.fact(s.graph.clone(), first).unwrap().support,
                Condition::TRUE
            );
            assert_eq!(
                h.support(s.history.clone(), 0, &Arc::new(vec![first, second])),
                Condition::FALSE
            );
            match commit.tick(&mut g, &mut a, &mut h, &mut ids) {
                CommitStatus::Applied(applied) => {
                    result = Some(applied);
                    break;
                }
                CommitStatus::Pending => {}
                _ => panic!("eligible transaction rejected"),
            }
        }
        let applied = result.expect("finite transaction completes");
        assert_eq!(applied.application.support, c);
        assert_eq!(applied.application.variables.as_slice(), [x, 2]);
        if p.rules[0].kept == 0 {
            for id in [first, second] {
                assert_eq!(
                    g.fact(applied.state.graph.clone(), id).unwrap().support,
                    c.not()
                );
            }
        } else {
            assert_eq!(
                h.support(applied.state.history, 0, &applied.application.heads),
                c
            );
        }
        assert!(matches!(
            commit.tick(&mut g, &mut a, &mut h, &mut ids),
            CommitStatus::Done
        ));
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
