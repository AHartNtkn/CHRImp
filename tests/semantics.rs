mod support;
use support::{Reader, engine, facts, finish};

#[test]
fn proof_example_preserves_direct_and_composed_derivations() {
    let mut e = engine(
        include_str!("../examples/proofs.chr"),
        "edge(A,B,AB),edge(B,C,BC),edge(A,C,AC)",
    );
    let answer = finish(&mut e);
    let [a, b, ab, c, bc, ac] = answer.variables[..] else {
        panic!("query variables");
    };
    let rows = |name: &str| {
        answer
            .rows
            .iter()
            .filter(|row| e.program().signatures[row.relation].name == name)
            .map(|row| row.ports.clone())
            .collect::<Vec<_>>()
    };
    let composition = rows("compose");
    assert_eq!(composition.len(), 1);
    assert_eq!(&composition[0][..2], &[ab, bc]);
    let derived = composition[0][2];
    assert!(!answer.variables.contains(&derived));
    let mut paths = rows("path");
    paths.sort();
    let mut expected = vec![
        vec![a, b, ab],
        vec![b, c, bc],
        vec![a, c, ac],
        vec![a, c, derived],
    ];
    expected.sort();
    assert_eq!(paths, expected);
    assert_eq!(rows("edge").len(), 3);
    assert_eq!(answer.rows.len(), 8);
}

#[test]
fn synthesis_example_filters_explicit_program_choices_by_examples() {
    for (extra, expected) in [
        ("", vec!["constant", "negate"]),
        (",evaluate(P,B,A)", vec!["negate"]),
    ] {
        let mut e = engine(
            include_str!("../examples/synthesis.chr"),
            &format!("synthesize(P),zero(A),one(B),evaluate(P,A,B){extra}"),
        );
        let mut reader = Reader::default();
        let mut programs = Vec::new();
        for _ in 0..500_000 {
            e.advance(1);
            if let Some(answer) = reader.next(&mut e) {
                let mut selected = 0;
                for row in &answer.rows {
                    let name = e.program().signatures[row.relation].name.as_str();
                    assert!(!["evaluate", "synthesize", "identity"].contains(&name));
                    if ["constant", "negate"].contains(&name) {
                        assert_eq!(row.ports[0], answer.variables[0]);
                        if name == "constant" {
                            assert_eq!(row.ports[1], answer.variables[2]);
                        }
                        programs.push(name.to_owned());
                        selected += 1;
                    }
                }
                assert_eq!(selected, 1);
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        programs.sort();
        assert_eq!(programs, expected);
    }
}

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
        if e.delivery_done() && e.pending_tasks() == 0 {
            break;
        }
    }
    assert!(e.exhausted());
    assert!(e.take_output().is_none());
    assert!(e.delivery_done());
    assert_eq!(e.pending_tasks(), 0);
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
    let mut reader = Reader::default();
    for _ in 0..10000 {
        e.advance(1);
        assert!(reader.next(&mut e).is_none());
        if e.delivery_done() {
            break;
        }
    }
    assert_eq!(e.pending_tasks(), 0);
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

#[test]
fn high_degree_merge_activates_every_cross_predicate_partner() {
    for degree in [16, 32, 64, 128] {
        let mut query = vec!["hub(A)".to_string()];
        for i in 0..degree {
            query.push(format!("spoke(B,V{i})"));
        }
        query.push("merge(A,B)".into());
        let mut e = engine(
            "hub(X),spoke(X,Y) ==> reach(Y). merge(X,Y) <=> X=Y.",
            &query.join(","),
        );
        let mut reader = Reader::default();
        let mut completed = None;
        for steps in 1..=500000 {
            e.advance(1);
            if let Some(a) = reader.next(&mut e) {
                completed = Some((a, steps));
                break;
            }
        }
        let (a, steps) =
            completed.expect("high-degree activation must complete within its work bound");
        assert!(
            steps < degree as usize * 1600,
            "waiting writers must not be polled repeatedly: {steps}"
        );
        let mut reached = a
            .rows
            .iter()
            .filter(|r| e.program().signatures[r.relation].name == "reach")
            .map(|r| r.ports[0])
            .collect::<Vec<_>>();
        reached.sort();
        let mut expected = a.variables[2..].to_vec();
        expected.sort();
        assert_eq!(reached, expected);
        assert_eq!(e.applications(), degree + 1);
    }
}
