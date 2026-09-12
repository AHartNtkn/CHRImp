mod support;
use chr::engine::Engine;
use support::{Answer, Reader, engine, facts, finish};
#[test]
fn whole_heap_collection_reclaims_stable_rewriting_across_budget_returns() {
    let mut e = engine("loop(X) <=> loop(X).", "loop(X)");
    let mut peak = e.memory();
    for _ in 0..200000 {
        e.advance(1);
        let m = e.memory();
        peak.graph_nodes = peak.graph_nodes.max(m.graph_nodes);
        peak.occurrences = peak.occurrences.max(m.occurrences);
        peak.pending_nodes = peak.pending_nodes.max(m.pending_nodes);
    }
    assert!(!e.exhausted());
    assert!(e.applications() > 100);
    assert!(e.collections() > 3);
    assert!(peak.graph_nodes < 8192, "{peak:?}");
    assert!(peak.occurrences < 512, "{peak:?}");
    assert!(peak.pending_nodes < 8192, "{peak:?}");
}
fn collect(e: &mut Engine) {
    let before = e.collections();
    e.request_collection();
    for _ in 0..100000 {
        e.advance(1);
        if !e.collecting() {
            assert!(e.collections() > before);
            return;
        }
    }
    panic!("finite collection must finish");
}
fn answers(e: &mut Engine, reader: &mut Reader) -> Vec<Answer> {
    let mut result = vec![];
    for _ in 0..100000 {
        e.advance(1);
        if let Some(a) = reader.next(e) {
            result.push(a);
        }
        if e.delivery_done() {
            return result;
        }
    }
    panic!("finite execution must deliver");
}
type Projected = (Vec<u64>, Vec<(usize, Vec<u64>)>);
fn normalized(answers: Vec<Answer>) -> Vec<Projected> {
    let mut result = answers
        .into_iter()
        .map(|a| {
            let mut rows = a
                .rows
                .into_iter()
                .map(|r| (r.relation, r.ports))
                .collect::<Vec<_>>();
            rows.sort();
            (a.variables, rows)
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}
#[test]
fn collecting_suspended_jobs_preserves_conditional_execution_and_projection() {
    let program = "p(X),q(X) ==> witness(X,Y). trigger(X,Y) <=> X=Y. witness(X,Y) <=> result(X,Y).";
    let query = "p(A),q(B),(trigger(A,B);q(A)),(left(A);right(A))";
    let mut base = engine(program, query);
    let expected = normalized(answers(&mut base, &mut Reader::default()));
    assert_eq!(expected.len(), 4);
    for prefix in (0..1200).step_by(7) {
        let mut e = engine(program, query);
        let mut reader = Reader::default();
        let mut seen = vec![];
        for _ in 0..prefix {
            e.advance(1);
            if let Some(a) = reader.next(&mut e) {
                seen.push(a);
            }
        }
        collect(&mut e);
        seen.extend(answers(&mut e, &mut reader));
        assert_eq!(normalized(seen), expected, "prefix {prefix}");
    }
}
#[test]
fn an_owned_output_event_survives_collection_and_later_projection() {
    let mut e = engine("", "edge(X,Y),(X=Y;true)");
    e.advance(10000);
    let begin = e.take_output().expect("queued scalar");
    let mut reader = Reader::default();
    assert!(reader.push(begin).is_none());
    collect(&mut e);
    let result = answers(&mut e, &mut reader);
    assert_eq!(result.len(), 2);
    let mut check = engine("", "tag()");
    let a = finish(&mut check);
    assert_eq!(facts(&check, &a), ["tag"]);
}

#[test]
fn propagation_churn_releases_obsolete_tuples_during_execution() {
    let program = "p(X) ==> seen(X). p(X) \\ seen(X) <=> next(X). p(X),next(X) <=> p(Y).";
    let mut e = engine(program, "p(X)");
    let mut maximum = 0;
    for _ in 0..600000 {
        e.advance(1);
        maximum = maximum.max(e.memory().history_records);
    }
    assert!(!e.exhausted());
    assert!(e.applications() > 1000, "{}", e.applications());
    assert!(e.collections() > 5);
    assert!(
        maximum < 128,
        "obsolete propagation tuples accumulated: {maximum}"
    );
}

#[test]
fn history_pruning_does_not_replay_a_live_tuple() {
    let mut e = engine(
        "keep(X) ==> result(X,Y). loop(X) <=> X=Y,loop(Y).",
        "keep(A),(true;loop(A))",
    );
    let answer = finish(&mut e);
    assert_eq!(facts(&e, &answer), ["keep", "result"]);
    for _ in 0..100000 {
        e.advance(1);
        assert!(e.take_output().is_none());
    }
    let memory = e.memory();
    assert!(e.collections() > 3);
    assert!(e.applications() > 10);
    assert_eq!(memory.history_records, 1);
    assert!(!e.exhausted());
    // Each merge wakes keep again; its retained tuple must still remain once-only.
    let relation = e
        .program()
        .signatures
        .iter()
        .position(|s| s.name == "result")
        .unwrap();
    let mut rows = e.graph().relation(e.state().graph, relation).unwrap();
    assert!(rows.next(e.graph()).is_some());
    assert!(rows.next(e.graph()).is_none());
}

#[test]
fn continuing_alias_rewrites_reclaim_unreferenced_identity_members() {
    let mut e = engine("loop(X) <=> X=Y,loop(Y).", "loop(A)");
    let mut peak = 0;
    for _ in 0..1_000_000 {
        e.advance(1);
        peak = peak.max(e.memory().graph_nodes);
    }
    assert!(!e.exhausted());
    assert!(e.applications() > 1000, "{} applications", e.applications());
    assert!(e.collections() > 10);
    assert!(peak < 16384, "identity storage grew to {peak} nodes");
    eprintln!(
        "alias stream: {} applications, {} collections, {peak} peak graph nodes in 1000000 service steps",
        e.applications(),
        e.collections()
    );
}

#[test]
fn graph_pruning_preserves_body_locals_stale_candidates_wakes_and_observers() {
    let cases = [
        (
            "start(X) <=> X=Y,(saved(Y);saved(X)),later(Y). saved(X),later(X) ==> found(X).",
            "start(A)",
        ),
        (
            "p(X),q(X),r(X) ==> witness(X,Z). drop(X) \\ r(X) <=> true. merge(X,Y) <=> X=Y.",
            "p(A),q(B),r(B),(merge(A,B);drop(B))",
        ),
        (
            "connect(X,Y) <=> X=Y. left(X),right(X) ==> found(X).",
            "left(A),right(B),(connect(A,B);true),(tag(A);tag(B))",
        ),
    ];
    for (program, query) in cases {
        let mut base = engine(program, query);
        let expected = normalized(answers(&mut base, &mut Reader::default()));
        assert!(!expected.is_empty());
        for prefix in (0..2000).step_by(11) {
            let mut e = engine(program, query);
            let mut reader = Reader::default();
            let mut seen = vec![];
            for _ in 0..prefix {
                e.advance(1);
                if let Some(answer) = reader.next(&mut e) {
                    seen.push(answer);
                }
            }
            collect(&mut e);
            seen.extend(answers(&mut e, &mut reader));
            assert_eq!(normalized(seen), expected, "{program}, prefix {prefix}");
        }
    }
}

#[test]
fn delivered_results_do_not_pin_execution_occurrences() {
    let mut e = engine("edge(X,Y) ==> reverse(Y,X).", "edge(A,B)");
    let delivered = answers(&mut e, &mut Reader::default());
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].rows.len(), 2);
    collect(&mut e);
    assert_eq!(e.memory().occurrences, 0);
    assert_eq!(e.memory().graph_nodes, 0);
    assert_eq!(e.memory().history_records, 0);
    assert_eq!(delivered[0].rows.len(), 2);
}
