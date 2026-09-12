mod support;
use chr::engine::Engine;
use support::{Answer, Reader, engine, facts, finish};
fn all(e: &mut Engine) -> Vec<Answer> {
    let mut answers = vec![];
    let mut reader = Reader::default();
    for _ in 0..500000 {
        e.advance(1);
        if let Some(a) = reader.next(e) {
            answers.push(a);
        }
        if e.delivery_done() {
            return answers;
        }
    }
    panic!("finite output must finish");
}
#[test]
fn duplicate_successes_and_inactive_nested_choices_have_exact_multiplicity() {
    for (query, count) in [
        ("true;true", 2),
        ("(true;true);true", 3),
        ("(true;true),(true;true)", 4),
        ("(fail;true);true", 2),
        ("(true;)", 1),
        ("true;true;true;true", 4),
    ] {
        let mut e = engine("", query);
        let answers = all(&mut e);
        assert_eq!(answers.len(), count, "{query}");
        assert!(answers.iter().all(|a| a.rows.is_empty()));
        let ids = answers
            .iter()
            .map(|a| a.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), count);
    }
}
#[test]
fn conditional_identity_is_projected_in_query_variables_and_ordered_ports() {
    let mut e = engine("", "edge(X,Y,X),(X=Y;true)");
    let answers = all(&mut e);
    assert_eq!(answers.len(), 2);
    assert_eq!(
        answers
            .iter()
            .filter(|a| a.variables[0] == a.variables[1])
            .count(),
        1
    );
    for a in answers {
        assert_eq!(a.rows.len(), 1);
        assert_eq!(
            a.rows[0].ports,
            [a.variables[0], a.variables[1], a.variables[0]]
        );
    }
}
#[test]
fn outputs_keep_duplicate_occurrences_and_nullary_relations() {
    let mut e = engine("", "p(X),p(X),tag(),true()");
    let answers = all(&mut e);
    assert_eq!(answers.len(), 1);
    let a = &answers[0];
    assert_eq!(facts(&e, a), ["p", "p", "tag", "true"]);
    let rows = a
        .rows
        .iter()
        .filter(|r| e.program().signatures[r.relation].name == "p")
        .collect::<Vec<_>>();
    assert_ne!(rows[0].occurrence, rows[1].occurrence);
    assert_eq!(rows[0].ports, rows[1].ports);
}
#[test]
fn choices_in_distinct_rule_applications_remain_independent() {
    let mut e = engine("p(X) <=> (a(X);b(X)).", "p(X),p(Y)");
    let answers = all(&mut e);
    assert_eq!(answers.len(), 4);
    assert_eq!(e.applications(), 2);
    let patterns = answers
        .iter()
        .map(|a| {
            let mut rows = a
                .rows
                .iter()
                .map(|r| (r.ports[0], e.program().signatures[r.relation].name.clone()))
                .collect::<Vec<_>>();
            rows.sort();
            rows
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(patterns.len(), 4);
}
#[test]
fn output_backpressure_keeps_only_one_scalar_event_and_resumes_wide_ports() {
    let ports = (0..1024)
        .map(|i| format!("X{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut e = engine("", &format!("wide({ports})"));
    e.advance(500000);
    let first = e.take_output().expect("one event available");
    assert!(matches!(first, chr::observe::Output::Begin { .. }));
    assert!(e.take_output().is_none());
    let mut reader = Reader::default();
    assert!(reader.push(first).is_none());
    let mut answer = None;
    for _ in 0..500000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            answer = Some(a);
            break;
        }
    }
    let a = answer.unwrap();
    assert_eq!(a.variables.len(), 1024);
    assert_eq!(a.rows[0].ports, a.variables);
    let mut finite = engine("", "true");
    let empty = finish(&mut finite);
    assert!(empty.rows.is_empty());
}
