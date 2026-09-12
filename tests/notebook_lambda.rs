#[allow(dead_code)]
mod support;
use chr::{
    engine::Engine,
    program::prepare,
    syntax::{Body, Program},
};
use serde_json::Value;
use std::sync::Arc;
use support::{Answer, Reader};

fn notebook() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/lambda.chrnb"
        ))
        .expect("lambda notebook must exist"),
    )
    .unwrap()
}

fn start(name: &str) -> Engine {
    let doc = notebook();
    let program: Program = serde_json::from_value(doc["program"].clone()).unwrap();
    let query = doc["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["name"] == name)
        .expect(name);
    let body: Body = serde_json::from_value(query["body"].clone()).unwrap();
    Engine::new(Arc::new(prepare(&program, &body).unwrap()))
}

fn finite(name: &str) -> (Engine, Vec<Answer>) {
    let mut e = start(name);
    let mut reader = Reader::default();
    let mut answers = vec![];
    for _ in 0..20_000_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            answers.push(a);
        }
        if e.delivery_done() {
            return (e, answers);
        }
    }
    panic!(
        "{name}: finite query did not exhaust ({} applications)",
        e.applications()
    );
}

fn wire(e: &Engine, a: &Answer, name: &str) -> u64 {
    a.variables[e
        .program()
        .query_variables()
        .iter()
        .position(|v| v == name)
        .expect(name)]
}
fn rows(e: &Engine, a: &Answer, name: &str) -> Vec<Vec<u64>> {
    a.rows
        .iter()
        .filter(|r| e.program().signatures()[r.relation].name == name)
        .map(|r| r.ports.clone())
        .collect()
}

#[test]
fn invalid_constraints_and_cyclic_structures_fail_without_search() {
    for name in [
        "Normal redex fails",
        "Equal disequality fails",
        "Application variable fails",
        "Lambda variable fails",
        "Structured left disequality fails",
        "Structured right disequality fails",
        "Structured left application disequality fails",
        "Structured right lambda disequality fails",
        "Merged disequality fails",
        "Merged variable structure fails",
        "Merged structure cycle fails",
        "Conflicting root ports fail",
        "Structure type clash fails",
        "Structure cycle fails",
        "Binder cycle fails",
    ] {
        let (e, answers) = finite(name);
        assert!(answers.is_empty(), "{name}: {answers:?}");
        assert_eq!(e.choices().count(), 0);
    }
}

// Render the answer graph only for assertions. Leaves are still engine wire IDs;
// query labels here make failures readable and never participate in reduction.
fn term(e: &Engine, a: &Answer, root: u64) -> String {
    fn visit(e: &Engine, a: &Answer, root: u64, depth: usize) -> String {
        assert!(
            depth <= a.rows.len(),
            "cyclic structure survived validation"
        );
        for relation in ["lam", "app"] {
            if let Some(row) = rows(e, a, relation).iter().find(|r| r[0] == root) {
                return format!(
                    "{relation}({},{})",
                    visit(e, a, row[1], depth + 1),
                    visit(e, a, row[2], depth + 1)
                );
            }
        }
        a.variables
            .iter()
            .position(|&v| v == root)
            .map(|i| e.program().query_variables()[i].clone())
            .unwrap_or_else(|| format!("_{root}"))
    }
    visit(e, a, root, 0)
}
fn residual(e: &Engine, a: &Answer) -> Vec<String> {
    let mut result = vec![];
    for row in &a.rows {
        let name = &e.program().signatures()[row.relation].name;
        match name.as_str() {
            // Structures and their finite-descendant certificates stay in the graph.
            "lam" | "app" | "below" => {}
            "var" | "neq" | "norm" => result.push(format!(
                "{name}({})",
                row.ports
                    .iter()
                    .map(|&v| term(e, a, v))
                    .collect::<Vec<_>>()
                    .join(",")
            )),
            _ => panic!("unevaluated constraint: {name}"),
        }
    }
    result.sort();
    result
}

#[test]
fn forward_steps_preserve_structure_sharing_and_residual_restrictions() {
    for (name, expected, constraints) in [
        ("Identity reduction", "A", "var(A);var(X)"),
        (
            "Constant reduction",
            "Y",
            "neq(X,Y);neq(X,Y);var(A);var(X);var(Y)",
        ),
        (
            "Application distribution",
            "app(app(lam(X,X),A),app(lam(X,Y),A))",
            "neq(X,Y);var(A);var(X);var(Y)",
        ),
        (
            "Repeated binder wire",
            "lam(X,Y)",
            "neq(X,Y);var(A);var(X);var(Y)",
        ),
        (
            "Under lambda with intentional sharing",
            "lam(Y,app(lam(X,X),Y))",
            "neq(X,Y);neq(X,Y);var(X);var(Y)",
        ),
        ("Lambda body congruence", "lam(Y,A)", "var(A);var(X);var(Y)"),
        ("Function congruence", "app(F,A)", "var(A);var(F);var(X)"),
        ("Argument congruence", "app(F,A)", "var(A);var(F);var(X)"),
        ("Normalize identity", "A", "var(A);var(X)"),
        (
            "Backwards constant body",
            "Body",
            "neq(A,Body);neq(X,Body);neq(X,Body);var(A);var(Body);var(X)",
        ),
    ] {
        let (e, answers) = finite(name);
        assert_eq!(answers.len(), 1, "{name}");
        let a = &answers[0];
        assert_eq!(term(&e, a, wire(&e, a, "Out")), expected, "{name}");
        assert_eq!(residual(&e, a).join(";"), constraints, "{name}");
    }
}

#[test]
fn unscoped_wires_can_merge_or_retain_disequality() {
    let (e, answers) = finite("Unconstrained wires may join");
    assert_eq!(answers.len(), 2);
    let mut actual = vec![];
    for a in &answers {
        actual.push((
            wire(&e, a, "X") == wire(&e, a, "Y"),
            term(&e, a, wire(&e, a, "Out")),
            residual(&e, a).join(";"),
        ));
    }
    actual.sort();
    assert_eq!(
        actual,
        vec![
            (false, "Y".into(), "neq(X,Y);var(A);var(X);var(Y)".into()),
            (true, "A".into(), "var(A);var(X);var(X)".into()),
        ]
    );
}

#[test]
fn norm_requires_companions_and_traverses_lambdas_and_application_spines() {
    for (name, expected) in [
        ("Normal wire without companion", "norm(X)"),
        ("Normal wire with companion", "var(X)"),
        ("Normal application without companion", "norm(app(F,A))"),
        ("Normal application with companions", "var(A);var(F)"),
        ("Normal lambda", "var(Y)"),
        ("Normal application spine", "var(A);var(B);var(F)"),
    ] {
        let (e, answers) = finite(name);
        assert_eq!(answers.len(), 1, "{name}");
        assert_eq!(residual(&e, &answers[0]).join(";"), expected, "{name}");
        assert_eq!(
            e.choices().count(),
            0,
            "ordinary CHR rules must not introduce search"
        );
    }
}

#[test]
fn same_roots_unify_ports_and_shared_substructure_is_acyclic() {
    for (name, relation, pairs, expected) in [
        (
            "Consistent lambda root",
            "lam",
            [("X", "A"), ("Y", "B")],
            "lam(X,Y)",
        ),
        (
            "Consistent application root",
            "app",
            [("F", "G"), ("X", "Y")],
            "app(F,X)",
        ),
    ] {
        let (e, answers) = finite(name);
        assert_eq!(answers.len(), 1);
        let a = &answers[0];
        for (x, y) in pairs {
            assert_eq!(wire(&e, a, x), wire(&e, a, y));
        }
        assert_eq!(rows(&e, a, relation).len(), 1);
        assert_eq!(term(&e, a, wire(&e, a, "In")), expected);
        assert!(residual(&e, a).is_empty());
        assert_eq!(e.choices().count(), 0);
    }
    let (e, answers) = finite("Shared structure is finite");
    assert_eq!(answers.len(), 1);
    let a = &answers[0];
    assert_eq!(term(&e, a, wire(&e, a, "In")), "app(lam(X,X),lam(X,X))");
    assert_eq!(
        rows(&e, a, "app"),
        vec![vec![wire(&e, a, "In"), wire(&e, a, "L"), wire(&e, a, "L")]]
    );
}

#[test]
fn backwards_search_produces_a_residual_constant_family() {
    let mut e = start("Open backwards synthesis");
    let mut reader = Reader::default();
    for _ in 0..20_000_000 {
        e.advance(1);
        if let Some(a) = reader.next(&mut e) {
            let input = wire(&e, &a, "In");
            let out = wire(&e, &a, "Out");
            for app in rows(&e, &a, "app").iter().filter(|r| r[0] == input) {
                for lam in rows(&e, &a, "lam")
                    .iter()
                    .filter(|r| r[0] == app[1] && r[2] == out)
                {
                    if rows(&e, &a, "neq") == [vec![lam[1], out]] {
                        assert_eq!(rows(&e, &a, "lam").len(), 1);
                        assert_eq!(rows(&e, &a, "app").len(), 1);
                        assert!(rows(&e, &a, "var").is_empty());
                        assert!(rows(&e, &a, "norm").is_empty());
                        assert!(rows(&e, &a, "lamEq").is_empty());
                        assert_ne!(lam[1], out);
                        assert_ne!(app[2], out);
                        assert_ne!(app[2], lam[1]);
                        assert!(rows(&e, &a, "step").is_empty());
                        e.cancel();
                        for _ in 0..2_000_000 {
                            e.advance(1);
                            if e.cancel_done() {
                                return;
                            }
                        }
                        panic!("backwards execution did not finish cancellation");
                    }
                }
            }
        }
        if e.delivery_done() {
            break;
        }
    }
    panic!(
        "backwards search must deliver app(lam(Binder,Out),Argument) with residual neq(Binder,Out)"
    );
}

#[test]
fn normal_form_equality_unifies_shared_binder_and_body_ports() {
    let (e, answers) = finite("Normal form equality merges wires");
    assert_eq!(answers.len(), 1);
    let a = &answers[0];
    assert_eq!(wire(&e, a, "L"), wire(&e, a, "R"));
    assert_eq!(wire(&e, a, "X"), wire(&e, a, "Y"));
    assert_eq!(
        rows(&e, a, "lam"),
        vec![vec![wire(&e, a, "L"), wire(&e, a, "X"), wire(&e, a, "X")]]
    );
    assert_eq!(residual(&e, a), ["var(X)", "var(X)"]);
}

#[test]
fn norm_reintroduces_the_companion_var_occurrence() {
    let doc = notebook();
    let mut program: Program = serde_json::from_value(doc["program"].clone()).unwrap();
    // Observe occurrence identity without changing any lambda rule.
    program.rules.insert(
        0,
        chr::syntax::parse_program("var(X) ==> seen(X).")
            .unwrap()
            .rules
            .remove(0),
    );
    let query = doc["queries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["name"] == "Normal wire with companion")
        .unwrap();
    let body: Body = serde_json::from_value(query["body"].clone()).unwrap();
    let mut e = Engine::new(Arc::new(prepare(&program, &body).unwrap()));
    let a = support::finish(&mut e);
    let x = wire(&e, &a, "X");
    assert_eq!(rows(&e, &a, "var"), [vec![x]]);
    assert_eq!(rows(&e, &a, "seen"), [vec![x], vec![x]]);
    assert_eq!(a.rows.len(), 3);
}

#[test]
fn binder_and_argument_ports_preserve_distinct_and_shared_wire_identities() {
    for (name, argument, shared) in [
        ("Identity reduction", "A", false),
        ("Identity with shared binder and argument", "X", true),
    ] {
        let (e, answers) = finite(name);
        assert_eq!(answers.len(), 1, "{name}");
        let a = &answers[0];
        let binder = wire(&e, a, "X");
        let argument = wire(&e, a, argument);
        assert_eq!(binder == argument, shared, "{name}");
        assert_eq!(wire(&e, a, "Out"), argument, "{name}");
        assert_eq!(rows(&e, a, "lam"), [vec![wire(&e, a, "L"), binder, binder]]);
        assert_eq!(
            rows(&e, a, "app"),
            [vec![wire(&e, a, "In"), wire(&e, a, "L"), argument]]
        );
    }
    for (name, argument, shared) in [
        ("Under lambda with distinct argument", "A", false),
        ("Under lambda with intentional sharing", "Y", true),
    ] {
        let (e, answers) = finite(name);
        assert_eq!(answers.len(), 1, "{name}");
        let a = &answers[0];
        let x = wire(&e, a, "X");
        let binder = wire(&e, a, "Y");
        let argument = wire(&e, a, argument);
        assert_ne!(x, binder);
        assert_ne!(x, argument);
        assert_eq!(binder == argument, shared, "{name}");
        let lams = rows(&e, a, "lam");
        let result = lams.iter().find(|r| r[0] == wire(&e, a, "Out")).unwrap();
        assert_eq!(
            result[1], binder,
            "the inner binder keeps its original wire"
        );
        let apps = rows(&e, a, "app");
        let body = apps.iter().find(|r| r[0] == result[2]).unwrap();
        assert_eq!(body[2], argument, "the argument keeps its original wire");
        assert!(lams.contains(&vec![body[1], x, x]));
        assert_eq!(
            rows(&e, a, "neq")
                .iter()
                .filter(|r| **r == [x, binder])
                .count(),
            2
        );
        assert!(rows(&e, a, "step").is_empty());
    }
}
