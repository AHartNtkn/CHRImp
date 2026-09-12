mod support;
use chr::condition::Condition;
use support::{Reader, engine, facts, finish};
#[test]
fn executes_all_head_modes_and_reaches_residual_normal_form() {
    let mut e = engine(
        "seed(X) <=> p(X). p(X) ==> q(X). p(X) \\ q(X) <=> done(X).",
        "seed(X)",
    );
    let a = finish(&mut e);
    assert_eq!(facts(&e, &a), ["done", "p"]);
    assert_eq!(e.applications(), 3);
    assert!(e.exhausted());
}
#[test]
fn only_explicit_choices_create_search_and_common_work_is_shared() {
    let mut e = engine(
        "work(X) <=> next(X). next(X) <=> done(X).",
        "(a(X);b(X)),work(X)",
    );
    let a = finish(&mut e);
    assert_eq!(e.choices().count(), 1);
    assert_eq!(e.applications(), 2);
    assert!(facts(&e, &a).contains(&"done".into()));
    let mut competing = engine("p(X) <=> a(X). p(X) <=> b(X).", "p(X)");
    let a = finish(&mut competing);
    assert_eq!(competing.choices().count(), 0);
    assert_eq!(competing.applications(), 1);
    assert_eq!(a.rows.len(), 1);
}
#[test]
fn explicit_merge_reactivates_nonbinding_cross_predicate_heads() {
    let mut e = engine(
        "p(X), q(X) <=> hit(X). trigger(X,Y) <=> X=Y.",
        "p(A),q(B),trigger(A,B)",
    );
    let a = finish(&mut e);
    assert_eq!(facts(&e, &a), ["hit"]);
    assert_eq!(e.applications(), 2);
    assert_eq!(a.variables[0], a.variables[1]);
    let mut different = engine("p(X),q(X) <=> hit(X).", "p(A),q(B)");
    let a = finish(&mut different);
    assert_eq!(facts(&different, &a), ["p", "q"]);
}
#[test]
fn finite_sibling_completes_during_divergent_execution() {
    let mut e = engine("loop(X) <=> loop(X).", "loop(X);answer(X)");
    let a = finish(&mut e);
    assert_eq!(facts(&e, &a), ["answer"]);
    assert!(!e.exhausted());
}
#[test]
fn failure_is_explicit_and_budget_exhaustion_is_unfinished() {
    let mut e = engine("p(X) <=> fail.", "p(X)");
    e.advance(0);
    assert!(!e.exhausted());
    assert!(e.take_output().is_none());
    for _ in 0..10000 {
        e.advance(1);
        if e.exhausted() {
            break;
        }
    }
    assert!(e.exhausted());
    assert!(e.take_output().is_none());
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
    let a = finish(&mut e);
    assert_eq!(e.applications(), 24);
    assert_eq!(e.choices().count(), 12);
    assert_eq!(a.rows.len(), 13);
    assert!(facts(&e, &a).contains(&"work24".into()));
}
#[test]
fn fresh_locals_and_duplicate_occurrences_survive_real_execution() {
    let mut e = engine("p(X) ==> witness(X,Y).", "p(X),p(X)");
    let a = finish(&mut e);
    let locals = a
        .rows
        .iter()
        .filter(|r| e.program().signatures[r.relation].name == "witness")
        .map(|r| {
            assert_eq!(r.ports[0], a.variables[0]);
            r.ports[1]
        })
        .collect::<Vec<_>>();
    assert_eq!(locals.len(), 2);
    assert_ne!(locals[0], locals[1]);
    assert_eq!(e.applications(), 2);
}
#[test]
fn disconnected_failure_excludes_exactly_its_alternative() {
    let mut e = engine("bad(Z) <=> fail.", "answer(X),(bad(Y);good(Y))");
    let a = finish(&mut e);
    assert_eq!(facts(&e, &a), ["answer", "good"]);
    let decision = e.choices().next().unwrap().1.decision;
    let mut reader = Reader::default();
    for _ in 0..10000 {
        e.advance(1);
        assert!(reader.next(&mut e).is_none());
        if e.delivery_done() {
            break;
        }
    }
    assert_eq!(e.failed(), decision);
    assert!(e.delivery_done());
}
#[test]
fn cyclic_multihead_join_checks_every_ordered_port() {
    let mut e = engine(
        "edge(X,Y),edge(Y,Z),edge(Z,X) ==> triangle(X,Y,Z).",
        "edge(A,B),edge(B,C),edge(C,A),edge(A,D)",
    );
    let a = finish(&mut e);
    let triangles = a
        .rows
        .iter()
        .filter(|r| e.program().signatures[r.relation].name == "triangle")
        .collect::<Vec<_>>();
    assert_eq!(triangles.len(), 3);
    assert!(triangles.iter().all(|r| !r.ports.contains(&a.variables[3])));
    assert_eq!(e.applications(), 3);
}
