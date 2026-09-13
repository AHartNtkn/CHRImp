#[allow(dead_code)]
mod support;
use chr::{
    engine::{Engine, NormalizationMode},
    program::prepare,
    syntax::{Program, parse_program, parse_query},
};
use std::sync::Arc;
use support::{Answer, Reader};
fn behavior() -> Program {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../examples/behavior-synthesis.chrnb")).unwrap();
    serde_json::from_value(doc["program"].clone()).unwrap()
}
fn start(p: &Program, q: &str, mode: NormalizationMode) -> Engine {
    Engine::with_normalization(
        Arc::new(prepare(p, &parse_query(q).unwrap()).unwrap()),
        mode,
    )
    .unwrap()
}
fn answers(e: &mut Engine) -> Vec<Answer> {
    let mut reader = Reader::default();
    let mut out = vec![];
    for _ in 0..2_000_000 {
        e.advance(1);
        if let Some(a) = reader.next(e) {
            out.push(a);
        }
        if e.delivery_done() {
            return out;
        }
    }
    panic!("finite source execution did not finish");
}
#[test]
fn conditional_field_merges_enable_real_consumers() {
    let q = "apply_k(T,X,R,O),k(T),constant(X,C),symbol_x(C),cons(A,Y,E),cons(B,Z,F),nil(E),nil(F),no_c(Y),k(Z),(R=A,A=B;nil(R))";
    for mode in [
        NormalizationMode::Baseline,
        NormalizationMode::Priority,
        NormalizationMode::Direct,
    ] {
        let mut e = start(&behavior(), q, mode);
        let a = answers(&mut e);
        assert_eq!(a.len(), 2);
        let names = e.program().query_variables();
        let var = |a: &Answer, n: &str| a.variables[names.iter().position(|s| s == n).unwrap()];
        for a in a {
            let merged = var(&a, "A") == var(&a, "B");
            assert_eq!(var(&a, "Y") == var(&a, "Z"), merged);
            assert_eq!(var(&a, "E") == var(&a, "F"), merged);
            assert_eq!(var(&a, "O") == var(&a, "X"), merged);
            let no_c = a
                .rows
                .iter()
                .filter(|r| e.program().signatures()[r.relation].name == "no_c")
                .count();
            assert_eq!(no_c, usize::from(!merged));
            assert_eq!(rows(&e, &a, "cons").len(), if merged { 1 } else { 2 });
            let empty = rows(&e, &a, "nil");
            assert_eq!(empty.len(), if merged { 1 } else { 4 });
            assert_eq!(
                empty
                    .iter()
                    .map(|r| r.ports[0])
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                empty.len()
            );
            if !merged {
                assert!(
                    rows(&e, &a, "app")
                        .iter()
                        .any(|r| r.ports == [var(&a, "O"), var(&a, "T"), var(&a, "X")])
                );
            }
            assert_eq!(a.rows.len(), if merged { 6 } else { 12 });
            assert_eq!(
                a.variables
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                if merged { 6 } else { 11 }
            );
            let mut expected = if merged {
                vec![
                    ("k", vec![var(&a, "T")]),
                    ("k", vec![var(&a, "Y")]),
                    ("constant", vec![var(&a, "X"), var(&a, "C")]),
                    ("nil", vec![var(&a, "E")]),
                    ("cons", vec![var(&a, "R"), var(&a, "Y"), var(&a, "E")]),
                    ("symbol_x", vec![var(&a, "C")]),
                ]
            } else {
                let fresh: Vec<_> = empty
                    .iter()
                    .filter(|r| !a.variables.contains(&r.ports[0]))
                    .collect();
                assert_eq!(fresh.len(), 1);
                vec![
                    ("k", vec![var(&a, "T")]),
                    ("k", vec![var(&a, "Z")]),
                    ("app", vec![var(&a, "O"), var(&a, "T"), var(&a, "X")]),
                    ("constant", vec![var(&a, "X"), var(&a, "C")]),
                    ("nil", vec![var(&a, "E")]),
                    ("nil", vec![var(&a, "F")]),
                    ("nil", vec![var(&a, "R")]),
                    ("nil", fresh[0].ports.clone()),
                    ("cons", vec![var(&a, "A"), var(&a, "Y"), var(&a, "E")]),
                    ("cons", vec![var(&a, "B"), var(&a, "Z"), var(&a, "F")]),
                    ("symbol_x", vec![var(&a, "C")]),
                    ("no_c", vec![var(&a, "Y")]),
                ]
            };
            let mut actual: Vec<_> = a
                .rows
                .iter()
                .map(|r| {
                    (
                        e.program().signatures()[r.relation].name.as_str(),
                        r.ports.clone(),
                    )
                })
                .collect();
            expected.sort();
            actual.sort();
            assert_eq!(actual, expected);
        }
    }
}
#[test]
fn whole_program_admission_rejects_interference() {
    let p = parse_program(
        "a(N) \\ a(N) <=> true. b(N) \\ b(N) <=> true. a(N),b(N) <=> fail. a(N) <=> taken(N).",
    )
    .unwrap();
    let code = Arc::new(prepare(&p, &parse_query("a(X)").unwrap()).unwrap());
    assert!(Engine::with_normalization(code, NormalizationMode::Direct).is_err());
}
fn variable(e: &Engine, a: &Answer, name: &str) -> u64 {
    a.variables[e
        .program()
        .query_variables()
        .iter()
        .position(|n| n == name)
        .unwrap()]
}
fn rows<'a>(e: &Engine, a: &'a Answer, name: &str) -> Vec<&'a support::Row> {
    a.rows
        .iter()
        .filter(|r| e.program().signatures()[r.relation].name == name)
        .collect()
}
#[test]
fn pm_partition_oracles_preserve_support_and_identity() {
    for mode in [
        NormalizationMode::Baseline,
        NormalizationMode::Priority,
        NormalizationMode::Direct,
    ] {
        for (q, count, case) in [
            ("(app(R,A,B);app(R,C,D))", 2, 0),
            ("(app(R,A,B);k(R)),app(R,C,D)", 1, 1),
            ("app(R,A,B),app(S,C,D),((R=X,R=Y,R=S);(S=U,S=V,R=S))", 2, 2),
            ("app(R,R,B),app(R,C,D)", 1, 3),
        ] {
            let mut e = start(&behavior(), q, mode);
            let all = answers(&mut e);
            assert_eq!(all.len(), count, "{mode:?} {q}");
            for a in all {
                assert_eq!(a.rows.len(), 1);
                assert_eq!(rows(&e, &a, "app").len(), 1);
                let v = |n| variable(&e, &a, n);
                match case {
                    0 => assert_eq!(
                        a.variables
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len(),
                        5
                    ),
                    1 => {
                        assert_eq!(v("A"), v("C"));
                        assert_eq!(v("B"), v("D"));
                        assert_ne!(v("R"), v("A"));
                    }
                    2 => {
                        assert_eq!(v("R"), v("S"));
                        assert_eq!(v("A"), v("C"));
                        assert_eq!(v("B"), v("D"));
                        if v("R") == v("X") {
                            assert_eq!(v("R"), v("Y"));
                            assert_ne!(v("R"), v("U"));
                            assert_ne!(v("U"), v("V"));
                        } else {
                            assert_eq!(v("S"), v("U"));
                            assert_eq!(v("S"), v("V"));
                            assert_ne!(v("R"), v("Y"));
                            assert_ne!(v("X"), v("Y"));
                        }
                    }
                    _ => {
                        assert_eq!(v("R"), v("C"));
                        assert_eq!(v("B"), v("D"));
                        assert_ne!(v("R"), v("B"));
                    }
                }
            }
        }
    }
}
#[test]
fn recognition_is_invariant_under_source_renaming_and_order() {
    use chr::syntax::Body;
    fn atom(a: &mut chr::syntax::Atom) {
        a.relation = format!("renamed_{}", a.relation);
        for v in &mut a.args {
            *v = format!("V{v}");
        }
    }
    fn body(b: &mut Body) {
        match b {
            Body::Atom { atom: a } => atom(a),
            Body::Equal { left, right } => {
                *left = format!("V{left}");
                *right = format!("V{right}");
            }
            Body::And { items } | Body::Or { items } => {
                for b in items {
                    body(b)
                }
            }
            _ => {}
        }
    }
    let mut p = behavior();
    p.rules.reverse();
    for r in &mut p.rules {
        r.name = None;
        for a in r.kept.iter_mut().chain(&mut r.removed) {
            atom(a);
        }
        body(&mut r.body);
    }
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(&p, "renamed_app(R,A,B),renamed_app(R,C,D)", mode);
        let a = answers(&mut e);
        assert_eq!(a.len(), 1);
        assert_eq!(variable(&e, &a[0], "A"), variable(&e, &a[0], "C"));
        assert_eq!(variable(&e, &a[0], "B"), variable(&e, &a[0], "D"));
        assert_eq!(a[0].rows.len(), 1);
    }
}
#[test]
fn source_normalization_is_shared_across_independent_choices() {
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(
            &behavior(),
            "(true;true),(true;true),app(R,A,B),app(R,C,D)",
            mode,
        );
        let a = answers(&mut e);
        assert_eq!(a.len(), 4);
        assert_eq!(e.normalization_stats().applications, 1);
        assert_eq!(e.normalization_stats().field_equalities, 2);
        if mode == NormalizationMode::Direct {
            assert_eq!(e.normalization_stats().generic_matching_ticks, 0);
            assert_eq!(e.normalization_stats().generic_commit_ticks, 0);
            assert_eq!(e.normalization_stats().coalescences, 1);
        } else {
            assert!(e.normalization_stats().generic_matching_ticks > 0);
            assert!(e.normalization_stats().generic_commit_ticks > 0);
        }
    }
}
#[test]
fn terminal_consumer_failure_does_not_escape_its_arm() {
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(
            &behavior(),
            "(constant(R,C),no_c(R);app(R,A,B)),(R=S;true),app(S,D,E)",
            mode,
        );
        let a = answers(&mut e);
        assert_eq!(a.len(), 2);
        for a in a {
            assert!(rows(&e, &a, "constant").is_empty());
            assert!(rows(&e, &a, "no_c").is_empty());
            let equal = variable(&e, &a, "R") == variable(&e, &a, "S");
            assert_eq!(rows(&e, &a, "app").len(), if equal { 1 } else { 2 });
        }
    }
}
#[test]
fn finite_sibling_progresses_beside_actual_divergent_evaluator() {
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(&behavior(), "(app(T,T,X),eval(T,S,O),nil(S);k(K))", mode);
        let a = support::finish(&mut e);
        assert_eq!(support::facts(&e, &a), ["k"]);
        assert!(!e.exhausted());
        e.cancel();
        for _ in 0..200_000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert_eq!(e.memory().occurrences, 0);
    }
}
#[test]
fn suspended_normalization_survives_collection_and_cancellation() {
    let q = "app(R,A,B),app(S,C,D),((R=X,R=Y,R=S);(S=U,S=V,R=S)),(true;true)";
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(&behavior(), q, mode);
        let mut reader = Reader::default();
        let mut out = vec![];
        let mut next_gc = 29;
        for step in 0..500_000 {
            if step >= next_gc && !e.collecting() {
                e.request_collection();
                next_gc = step + 211;
            }
            e.advance(1);
            if let Some(a) = reader.next(&mut e) {
                out.push(a);
            }
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(out.len(), 4);
        for a in out {
            assert_eq!(a.rows.len(), 1);
            assert_eq!(variable(&e, &a, "A"), variable(&e, &a, "C"));
        }
        for prefix in (0..1800).step_by(31) {
            let mut e = start(&behavior(), q, mode);
            for _ in 0..prefix {
                e.advance(1);
                e.take_output();
            }
            e.request_collection();
            e.maintain(1 + prefix % 17);
            e.cancel();
            let apps = e.applications();
            for _ in 0..200_000 {
                e.advance(1);
                if e.cancel_done() {
                    break;
                }
            }
            assert!(e.cancel_done(), "{mode:?} prefix {prefix}");
            assert_eq!(e.applications(), apps);
            let m = e.memory();
            assert_eq!(
                (
                    m.occurrences,
                    m.graph_nodes,
                    m.conditions,
                    m.pending_nodes,
                    m.history_nodes
                ),
                (0, 0, 0, 0, 0)
            );
        }
    }
}
#[test]
fn malformed_consistency_cannot_silently_admit_a_smaller_family() {
    let mut p = behavior();
    p.rules
        .iter_mut()
        .find(|r| r.name.as_deref() == Some("app_consistent"))
        .unwrap()
        .body = chr::syntax::Body::True;
    let code = Arc::new(prepare(&p, &parse_query("app(R,A,B),app(R,C,D)").unwrap()).unwrap());
    assert!(Engine::with_normalization(code, NormalizationMode::Direct).is_err());
}
#[test]
fn constructor_attachments_follow_active_support_through_compaction() {
    // A continuing source stream exercises normalization after old failed scopes
    // and their coordinate records have been reclaimed.
    let mut p = behavior();
    p.rules.extend(
        parse_program("loop(X) <=> (constant(X,C),no_c(X);app(X,A,B),app(X,C,D),loop(A)).")
            .unwrap()
            .rules,
    );
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e = start(&p, "loop(R)", mode);
        let mut next_gc = 61;
        for step in 0..200_000 {
            if step >= next_gc && !e.collecting() {
                e.request_collection();
                next_gc = step + 501;
            }
            e.advance(1);
            assert!(e.take_output().is_none());
            if e.normalization_stats().field_equalities >= 8 && e.collections() >= 4 {
                break;
            }
        }
        assert!(e.normalization_stats().coalescences > 0 || mode == NormalizationMode::Priority);
        assert!(e.collections() >= 4);
        #[cfg(feature = "diagnostics")]
        assert!(e.diagnostics().shared.coordinates.assignments_published > 0);
        assert!(
            e.normalization_stats().field_equalities >= 8,
            "{mode:?} stats={:?}",
            e.normalization_stats()
        );
        e.cancel();
        for _ in 0..200_000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
        assert_eq!(e.memory().graph_nodes, 0);
    }
}
#[test]
fn repeated_conditional_and_cyclic_unions_agree_with_source_execution() {
    // Every row port is a named query identity here. Canonicalizing by first
    // query slot compares equality partitions and full residual multisets.
    fn canonical(e: &Engine, a: Answer) -> (Vec<usize>, Vec<(String, Vec<usize>)>) {
        let slot = |v| a.variables.iter().position(|&x| x == v).unwrap();
        let eq = a.variables.iter().map(|&v| slot(v)).collect();
        let mut rows: Vec<_> = a
            .rows
            .iter()
            .map(|r| {
                (
                    e.program().signatures()[r.relation].name.clone(),
                    r.ports.iter().map(|&v| slot(v)).collect(),
                )
            })
            .collect();
        rows.sort();
        (eq, rows)
    }
    for q in [
        "app(R,A,R),app(R,B,S),app(S,C,T),app(T,D,T)",
        "app(R,A,B),app(S,C,D),app(T,E,F),(R=S;S=T),R=T",
        "app(R,A,B),app(S,C,D),(R=S;true),(A=C;B=D)",
        "(app(R,A,B);app(R,C,D)),(app(S,E,F);k(S)),(R=S;A=E)",
        "k(R),k(S),(R=S;R=T),k(T)",
    ] {
        let mut reference = start(&behavior(), q, NormalizationMode::Baseline);
        let mut expected: Vec<_> = answers(&mut reference)
            .into_iter()
            .map(|a| canonical(&reference, a))
            .collect();
        expected.sort();
        for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
            let mut e = start(&behavior(), q, mode);
            let mut actual: Vec<_> = answers(&mut e)
                .into_iter()
                .map(|a| canonical(&e, a))
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{mode:?} {q}");
        }
    }
}
#[test]
fn unrestricted_identity_synthesis_runs_the_actual_evaluator() {
    #[derive(Clone)]
    enum Expr {
        Node(u64),
        App(Box<Expr>, Box<Expr>),
        Argument,
    }
    fn applies_as_identity(e: &Engine, a: &Answer) -> bool {
        let mut head = Expr::Node(variable(e, a, "Program"));
        let mut args = vec![Expr::Argument];
        for _ in 0..10_000 {
            match head {
                Expr::Argument => return args.is_empty(),
                Expr::App(f, x) => {
                    args.push(*x);
                    head = *f;
                }
                Expr::Node(v) => {
                    let Some(row) = a.rows.iter().find(|r| {
                        r.ports.first() == Some(&v)
                            && matches!(
                                e.program().signatures()[r.relation].name.as_str(),
                                "k" | "s" | "app"
                            )
                    }) else {
                        return false;
                    };
                    match e.program().signatures()[row.relation].name.as_str() {
                        "app" => {
                            args.push(Expr::Node(row.ports[2]));
                            head = Expr::Node(row.ports[1]);
                        }
                        "k" if args.len() >= 2 => {
                            head = args.pop().unwrap();
                            args.pop();
                        }
                        "s" if args.len() >= 3 => {
                            let x = args.pop().unwrap();
                            let y = args.pop().unwrap();
                            let z = args.pop().unwrap();
                            head = Expr::App(
                                Box::new(Expr::App(Box::new(x), Box::new(z.clone()))),
                                Box::new(Expr::App(Box::new(y), Box::new(z))),
                            );
                        }
                        _ => return false,
                    }
                }
            }
        }
        false
    }
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../examples/behavior-synthesis.chrnb")).unwrap();
    let q: chr::syntax::Body = serde_json::from_value(doc["queries"][0]["body"].clone()).unwrap();
    for mode in [NormalizationMode::Priority, NormalizationMode::Direct] {
        let mut e =
            Engine::with_normalization(Arc::new(prepare(&behavior(), &q).unwrap()), mode).unwrap();
        let mut reader = Reader::default();
        let mut found = None;
        for _ in 0..60_000_000 {
            e.advance(1);
            if let Some(a) = reader.next(&mut e) {
                found = Some(a);
                break;
            }
            if e.delivery_done() {
                break;
            }
        }
        let a = found.expect("unrestricted identity source must produce an answer");
        assert!(
            applies_as_identity(&e, &a),
            "independent SK reduction rejected the synthesized graph"
        );
        for name in ["eval", "fold", "apply_k", "apply_s", "apply_s_two"] {
            assert!(rows(&e, &a, name).is_empty());
        }
        e.cancel();
        for _ in 0..1_000_000 {
            e.advance(1);
            if e.cancel_done() {
                break;
            }
        }
        assert!(e.cancel_done());
    }
}
