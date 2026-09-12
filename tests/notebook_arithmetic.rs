#[allow(dead_code)]
mod support;

use chr::{
    engine::Engine,
    program::prepare,
    syntax::{parse_program, parse_program_json, parse_query, parse_query_json},
};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};
use support::{Answer, Reader};

fn root(e: &Engine, a: &Answer, name: &str) -> u64 {
    a.variables[e
        .program()
        .query_variables()
        .iter()
        .position(|n| n == name)
        .unwrap()]
}

// Follow the answer's actual constructor graph, checking every duplicate definition.
fn unary(e: &Engine, a: &Answer, mut node: u64) -> (usize, Option<u64>) {
    let mut seen = BTreeSet::new();
    let mut depth = 0;
    loop {
        assert!(seen.insert(node), "cyclic unary answer: {a:?}");
        let mut zero = false;
        let mut children = BTreeSet::new();
        for row in &a.rows {
            if row.ports.first() != Some(&node) {
                continue;
            }
            match e.program().signatures()[row.relation].name.as_str() {
                "zero" => zero = true,
                "succ" => {
                    children.insert(row.ports[1]);
                }
                _ => {}
            }
        }
        assert!(children.len() <= 1, "successor roots must unify children");
        assert!(
            !zero || children.is_empty(),
            "zero/successor clash survived"
        );
        if zero {
            return (depth, None);
        }
        match children.first() {
            Some(child) => {
                node = *child;
                depth += 1;
            }
            None => return (depth, Some(node)),
        }
    }
}

#[test]
fn arithmetic_notebook_queries_execute_to_exhaustion_with_retained_graphs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/arithmetic.chrnb");
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("arithmetic notebook exists"))
            .unwrap();
    let program = parse_program_json(&document["program"].to_string()).unwrap();
    let queries = document["queries"].as_array().unwrap();
    let cases: &[(&str, &[[usize; 3]])] = &[
        ("Forward: 2 + 3 = ?", &[[2, 3, 5]]),
        ("Missing left: ? + 3 = 5", &[[2, 3, 5]]),
        ("Missing right: 2 + ? = 5", &[[2, 3, 5]]),
        (
            "Decompose 3: X + Y = 3",
            &[[0, 3, 3], [1, 2, 3], [2, 1, 3], [3, 0, 3]],
        ),
        ("Subtraction: 5 - 2 = ?", &[[5, 2, 3]]),
        ("Impossible: 2 + 3 = 4", &[]),
        ("Underflow: 2 - 3 = ?", &[]),
        ("Partial: 1 + Y = Z", &[]),
        ("Inconsistent: zero and successor", &[]),
        ("Duplicate successor roots unify children", &[[2, 2, 1]]),
        ("Reject a successor cycle", &[]),
        ("Impossible: 1 + Y = Y", &[]),
    ];
    assert_eq!(queries.len(), cases.len());
    for (q, (name, expected)) in queries.iter().zip(cases) {
        assert_eq!(q["name"], *name);
        let query = parse_query_json(&q["body"].to_string()).unwrap();
        let mut e = Engine::new(Arc::new(prepare(&program, &query).unwrap()));
        let mut reader = Reader::default();
        let mut answers = Vec::new();
        let mut steps = 0;
        while !e.delivery_done() && steps < 2_000_000 {
            e.advance(1);
            if let Some(a) = reader.next(&mut e) {
                answers.push(a);
            }
            steps += 1;
        }
        assert!(
            e.exhausted() && e.delivery_done(),
            "{name}: did not exhaust after {steps} work units"
        );
        eprintln!(
            "{name}: {} answers, {steps} work units, {} applications",
            answers.len(),
            e.applications()
        );
        for a in &answers {
            assert!(a.rows.iter().all(|r| matches!(
                e.program().signatures()[r.relation].name.as_str(),
                "zero" | "succ" | "predecessor"
            )));
            // Every defining fact supplied by this query must still be observable.
            let chr::syntax::Body::And { items } = &query else {
                panic!("expected conjunction")
            };
            for item in items {
                if let chr::syntax::Body::Atom { atom } = item
                    && matches!(atom.relation.as_str(), "zero" | "succ")
                {
                    let ports: Vec<_> = atom.args.iter().map(|n| root(&e, a, n)).collect();
                    assert!(
                        a.rows
                            .iter()
                            .any(
                                |r| e.program().signatures()[r.relation].name == atom.relation
                                    && r.ports == ports
                            ),
                        "{name}: missing defining fact {atom:?}"
                    );
                }
            }
        }
        if *name == "Partial: 1 + Y = Z" {
            assert_eq!(answers.len(), 1);
            let a = &answers[0];
            assert_eq!(unary(&e, a, root(&e, a, "X")), (1, None));
            assert_eq!(unary(&e, a, root(&e, a, "Y")), (0, Some(root(&e, a, "Y"))));
            assert_eq!(unary(&e, a, root(&e, a, "Z")), (1, Some(root(&e, a, "Y"))));
        } else {
            let mut actual: Vec<_> = answers
                .iter()
                .map(|a| {
                    ["X", "Y", "Z"].map(|n| {
                        let (value, hole) = unary(&e, a, root(&e, a, n));
                        assert_eq!(hole, None, "{name}: expected a finite natural");
                        value
                    })
                })
                .collect();
            actual.sort();
            assert_eq!(actual, *expected, "{name}");
        }
    }
}

#[test]
fn successor_cycles_fail_while_unknown_tails_remain_open() {
    let document: Value =
        serde_json::from_str(include_str!("../examples/arithmetic.chrnb")).unwrap();
    let mut program = parse_program_json(&document["program"].to_string()).unwrap();
    // Delay equality until a derived path exists, exercising merge reactivation.
    program.rules.extend(
        parse_program("close_path @ predecessor(X,Z), close(X,Z) ==> X=Z.")
            .unwrap()
            .rules,
    );
    // Both traversal-before-failure and failure-before-traversal must reject cycles.
    for _ in 0..2 {
        program.rules.reverse();
        for (query, expected) in [
            ("succ(X,X)", 0),
            ("succ(X,Y),succ(Y,X)", 0),
            ("succ(X,Y),succ(Y,Z),succ(Z,X)", 0),
            ("succ(X,Y),succ(Y,Z),X=Z", 0),
            ("X=Z,succ(X,Y),succ(Y,Z)", 0),
            ("succ(X,Y),succ(Y,Z),close(X,Z)", 0),
            ("succ(X,Y),succ(Y,Z),succ(X,Z)", 0),
            ("succ(X,Y),succ(Y,Z)", 1),
            ("succ(X,Y),succ(X,Z)", 1),
        ] {
            let mut e = Engine::new(Arc::new(
                prepare(&program, &parse_query(query).unwrap()).unwrap(),
            ));
            let mut reader = Reader::default();
            let mut answers = Vec::new();
            let mut steps = 0;
            while !e.delivery_done() && steps < 100_000 {
                e.advance(1);
                if let Some(a) = reader.next(&mut e) {
                    answers.push(a);
                }
                steps += 1;
            }
            assert!(
                e.delivery_done() && e.exhausted(),
                "{query}: exceeded {steps} work units"
            );
            eprintln!("{query}: {} answers, {steps} work units", answers.len());
            assert_eq!(answers.len(), expected, "{query}");
            for a in &answers {
                let (depth, tail) = unary(&e, a, root(&e, a, "X"));
                assert_eq!(depth, if query == "succ(X,Y),succ(Y,Z)" { 2 } else { 1 });
                assert_eq!(tail, Some(root(&e, a, "Z")));
            }
        }
    }
}
