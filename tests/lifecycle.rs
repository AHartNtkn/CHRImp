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
