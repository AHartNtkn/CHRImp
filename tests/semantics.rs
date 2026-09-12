use chr::condition::Condition;
use chr::engine::Engine;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

fn engine(program: &str, query: &str) -> Engine {
    Engine::new(Arc::new(
        prepare(
            &parse_program(program).unwrap(),
            &parse_query(query).unwrap(),
        )
        .unwrap(),
    ))
}
fn finish(e: &mut Engine) -> chr::engine::Completion {
    for _ in 0..200_000 {
        e.advance(1);
        if let Some(c) = e.take_completion() {
            return c;
        }
    }
    panic!("finite query must complete");
}
fn facts(e: &Engine, c: &chr::engine::Completion) -> Vec<String> {
    let mut result = vec![];
    for (relation, sig) in e.program().signatures.iter().enumerate() {
        let mut rows = e.graph().relation(c.state.graph, relation).unwrap();
        while let Some((_, support)) = rows.next(e.graph()) {
            if support == Condition::TRUE {
                result.push(sig.name.clone());
            }
        }
    }
    result.sort();
    result
}
#[test]
fn executes_all_head_modes_and_reaches_residual_normal_form() {
    let mut e = engine(
        "seed(X) <=> p(X). p(X) ==> q(X). p(X) \\ q(X) <=> done(X).",
        "seed(X)",
    );
    let c = finish(&mut e);
    assert_eq!(c.support, Condition::TRUE);
    assert_eq!(facts(&e, &c), ["done", "p"]);
    assert_eq!(e.applications(), 3);
    assert!(e.exhausted());
}
#[test]
fn only_explicit_choices_create_search_and_common_work_is_shared() {
    let mut e = engine(
        "work(X) <=> next(X). next(X) <=> done(X).",
        "(a(X);b(X)),work(X)",
    );
    let c = finish(&mut e);
    assert_eq!(c.support, Condition::TRUE);
    assert_eq!(e.choices().count(), 1);
    assert_eq!(e.applications(), 2);
    let mut competing = engine("p(X) <=> a(X). p(X) <=> b(X).", "p(X)");
    let c = finish(&mut competing);
    assert_eq!(competing.choices().count(), 0);
    assert_eq!(competing.applications(), 1);
    assert_eq!(facts(&competing, &c).len(), 1);
}
#[test]
fn explicit_merge_reactivates_nonbinding_cross_predicate_heads() {
    let mut e = engine(
        "p(X), q(X) <=> hit(X). trigger(X,Y) <=> X=Y.",
        "p(A),q(B),trigger(A,B)",
    );
    let c = finish(&mut e);
    assert_eq!(facts(&e, &c), ["hit"]);
    assert_eq!(e.applications(), 2);
    let mut different = engine("p(X),q(X) <=> hit(X).", "p(A),q(B)");
    let c = finish(&mut different);
    assert_eq!(facts(&different, &c), ["p", "q"]);
}
#[test]
fn finite_sibling_completes_during_divergent_execution() {
    let mut e = engine("loop(X) <=> loop(X).", "loop(X);answer(X)");
    let c = finish(&mut e);
    assert_ne!(c.support, Condition::FALSE);
    assert_ne!(c.support, Condition::TRUE);
    assert!(!e.exhausted());
    let (_, birth) = e.choices().next().unwrap();
    assert_eq!(c.support, birth.decision.not());
}
#[test]
fn failure_is_explicit_and_budget_exhaustion_is_unfinished() {
    let mut e = engine("p(X) <=> fail.", "p(X)");
    e.advance(0);
    assert!(!e.exhausted());
    assert!(e.take_completion().is_none());
    for _ in 0..10000 {
        e.advance(1);
        if e.exhausted() {
            break;
        }
    }
    assert!(e.exhausted());
    assert!(e.take_completion().is_none());
    assert_eq!(e.failed(), Condition::TRUE);
}

#[test]
fn independent_choices_do_not_multiply_common_rewrite_execution() {
    let rules = (0..24)
        .map(|i| format!("work{i}(X) <=> work{}(X).", i + 1))
        .collect::<String>();
    let mut query = (0..12)
        .map(|i| format!("(left{i}(X);right{i}(X))"))
        .collect::<Vec<_>>();
    query.push("work0(X)".into());
    let mut e = engine(&rules, &query.join(","));
    let c = finish(&mut e);
    assert_eq!(c.support, Condition::TRUE);
    assert_eq!(e.applications(), 24);
    assert_eq!(e.choices().count(), 12);
    let relation = e
        .program()
        .signatures
        .iter()
        .position(|s| s.name == "work24")
        .unwrap();
    let mut rows = e.graph().relation(c.state.graph, relation).unwrap();
    assert_eq!(rows.next(e.graph()).unwrap().1, Condition::TRUE);
    assert!(rows.next(e.graph()).is_none());
}

#[test]
fn fresh_locals_and_duplicate_occurrences_survive_real_execution() {
    let mut e = engine("p(X) ==> witness(X,Y).", "p(X),p(X)");
    let c = finish(&mut e);
    let relation = e
        .program()
        .signatures
        .iter()
        .position(|s| s.name == "witness")
        .unwrap();
    let mut rows = e.graph().relation(c.state.graph, relation).unwrap();
    let mut locals = vec![];
    while let Some((id, _)) = rows.next(e.graph()) {
        let row = e.graph().fact(c.state.graph, id).unwrap();
        assert_eq!(row.args[0], e.query_variables()[0]);
        locals.push(row.args[1]);
    }
    assert_eq!(locals.len(), 2);
    assert_ne!(locals[0], locals[1]);
    assert_eq!(e.applications(), 2);
}

#[test]
fn disconnected_failure_excludes_exactly_its_alternative() {
    let mut e = engine("bad(Z) <=> fail.", "answer(X),(bad(Y);good(Y))");
    let c = finish(&mut e);
    let decision = e.choices().next().unwrap().1.decision;
    assert_eq!(c.support, decision.not());
    for _ in 0..10000 {
        if e.exhausted() {
            break;
        }
        e.advance(1);
        assert!(e.take_completion().is_none());
    }
    assert_eq!(e.failed(), decision);
    assert!(e.exhausted());
}

#[test]
fn cyclic_multihead_join_checks_every_ordered_port() {
    let mut e = engine(
        "edge(X,Y),edge(Y,Z),edge(Z,X) ==> triangle(X,Y,Z).",
        "edge(A,B),edge(B,C),edge(C,A),edge(A,D)",
    );
    let c = finish(&mut e);
    let relation = e
        .program()
        .signatures
        .iter()
        .position(|s| s.name == "triangle")
        .unwrap();
    let mut rows = e.graph().relation(c.state.graph, relation).unwrap();
    let mut count = 0;
    while let Some((id, _)) = rows.next(e.graph()) {
        let row = e.graph().fact(c.state.graph, id).unwrap();
        assert!(!row.args.contains(&e.query_variables()[3]));
        count += 1;
    }
    assert_eq!(count, 3);
    assert_eq!(e.applications(), 3);
}
