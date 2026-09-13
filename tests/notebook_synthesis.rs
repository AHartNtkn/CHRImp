#[allow(dead_code)]
mod support;
use chr::{
    engine::Engine,
    program::prepare,
    syntax::{Atom, Body, Program},
};
use serde_json::Value;
use std::sync::Arc;
use support::{Answer, Reader};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Term {
    K,
    S,
    App(Box<Term>, Box<Term>),
    Symbol(u8),
    Hole(u64),
}
use Term::*;
fn app(f: Term, x: Term) -> Term {
    App(Box::new(f), Box::new(x))
}
fn apps(f: Term, args: impl IntoIterator<Item = Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn identity() -> Term {
    apps(S, [K, K])
}
fn compose() -> Term {
    apps(S, [app(K, S), K])
}
fn swap() -> Term {
    apps(S, [apps(compose(), [compose(), S]), app(K, K)])
}
fn witnesses() -> Vec<Term> {
    vec![
        identity(),
        K,
        app(K, identity()),
        S,
        compose(),
        swap(),
        apps(S, [S, app(S, K)]),
        app(swap(), identity()),
        apps(S, [identity(), identity()]),
    ]
}
// Independent normal-order SK reduction. A hole must never be forced to satisfy a target.
fn reduce(t: Term, fuel: &mut usize) -> Term {
    assert!(*fuel > 0, "oracle reduction did not terminate");
    *fuel -= 1;
    let mut args = vec![];
    let mut head = t;
    while let App(f, x) = head {
        args.push(*x);
        head = *f;
    }
    args.reverse();
    if head == K && args.len() >= 2 {
        let first = args.remove(0);
        args.remove(0);
        return reduce(apps(first, args), fuel);
    }
    if head == S && args.len() >= 3 {
        let f = args.remove(0);
        let g = args.remove(0);
        let x = args.remove(0);
        return reduce(apps(app(app(f, x.clone()), app(g, x)), args), fuel);
    }
    apps(head, args.into_iter().map(|x| reduce(x, fuel)))
}
fn expectations() -> Vec<(Vec<Term>, Term)> {
    let (x, y, z) = (Symbol(0), Symbol(1), Symbol(2));
    vec![
        (vec![x.clone()], x.clone()),
        (vec![x.clone(), y.clone()], x.clone()),
        (vec![x.clone(), y.clone()], y.clone()),
        (
            vec![x.clone(), y.clone(), z.clone()],
            apps(x.clone(), [z.clone(), app(y.clone(), z.clone())]),
        ),
        (
            vec![x.clone(), y.clone(), z.clone()],
            app(x.clone(), app(y.clone(), z.clone())),
        ),
        (
            vec![x.clone(), y.clone(), z.clone()],
            apps(x.clone(), [z.clone(), y.clone()]),
        ),
        (
            vec![x.clone(), y.clone()],
            apps(x.clone(), [y.clone(), y.clone()]),
        ),
        (vec![x.clone(), y.clone()], app(y.clone(), x.clone())),
        (vec![x.clone()], app(x.clone(), x)),
    ]
}
fn document(kind: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/examples/{kind}-synthesis.chrnb",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
fn atom(name: &str, args: Vec<String>) -> Body {
    Body::Atom {
        atom: Atom {
            relation: name.into(),
            args,
        },
    }
}
fn encode(t: &Term, root: String, n: &mut usize, rows: &mut Vec<Body>) {
    match t {
        K => rows.push(atom("k", vec![root])),
        S => rows.push(atom("s", vec![root])),
        App(f, x) => {
            *n += 1;
            let a = format!("WitnessF{n}");
            *n += 1;
            let b = format!("WitnessX{n}");
            rows.push(atom("app", vec![root, a.clone(), b.clone()]));
            encode(f, a, n, rows);
            encode(x, b, n, rows);
        }
        _ => panic!("ground SK witness expected"),
    }
}
fn start(doc: &Value, i: usize, witness: Option<&Term>) -> Engine {
    let p: Program = serde_json::from_value(doc["program"].clone()).unwrap();
    let mut q: Body = serde_json::from_value(doc["queries"][i]["body"].clone()).unwrap();
    if let Some(w) = witness {
        let mut rows = vec![q];
        encode(w, "Program".into(), &mut 0, &mut rows);
        q = Body::And { items: rows };
    }
    Engine::new(Arc::new(prepare(&p, &q).unwrap()))
}
fn root(e: &Engine, a: &Answer, name: &str) -> u64 {
    a.variables[e
        .program()
        .query_variables()
        .iter()
        .position(|n| n == name)
        .unwrap()]
}
fn term(e: &Engine, a: &Answer, node: u64, parents: &mut Vec<u64>) -> Term {
    assert!(!parents.contains(&node), "cyclic output");
    parents.push(node);
    let mut value = None;
    for r in a.rows.iter().filter(|r| r.ports.first() == Some(&node)) {
        let t = match e.program().signatures()[r.relation].name.as_str() {
            "k" => Some(K),
            "s" => Some(S),
            "app" => Some(app(
                term(e, a, r.ports[1], parents),
                term(e, a, r.ports[2], parents),
            )),
            "constant" => Some(term(e, a, r.ports[1], parents)),
            "symbol_x" => Some(Symbol(0)),
            "symbol_y" => Some(Symbol(1)),
            "symbol_z" => Some(Symbol(2)),
            _ => None,
        };
        if let Some(t) = t {
            if let Some(old) = &value {
                assert_eq!(old, &t, "conflicting graph definitions");
            }
            value = Some(t);
        }
    }
    parents.pop();
    value.unwrap_or(Hole(node))
}
fn first(e: &mut Engine, budget: usize) -> Option<Answer> {
    let mut r = Reader::default();
    for _ in 0..budget {
        e.advance(1);
        if let Some(a) = r.next(e) {
            return Some(a);
        }
        if e.delivery_done() {
            return None;
        }
    }
    panic!(
        "query did not produce an answer within {budget} work units ({} applications)",
        e.applications()
    )
}
fn settled(e: &Engine, a: &Answer) {
    for r in &a.rows {
        let n = e.program().signatures()[r.relation].name.as_str();
        assert!(
            !matches!(
                n,
                "infer" | "apply_type" | "eval" | "fold" | "apply_k" | "apply_s" | "apply_s_two"
            ),
            "unresolved {n}"
        );
    }
}
#[test]
fn behavior_program_accepts_cyclic_relational_structures() {
    let doc = document("behavior");
    let p: Program = serde_json::from_value(doc["program"].clone()).unwrap();
    let q = chr::syntax::parse_query("app(A,A,K),k(K),cons(L,K,L),constant(C,C)").unwrap();
    let mut e = Engine::new(Arc::new(prepare(&p, &q).unwrap()));
    let a = first(&mut e, 100_000).expect("cyclic structures are permitted");
    assert_eq!(a.rows.len(), 4);
    for (name, variable, port) in [("app", "A", 1), ("cons", "L", 2), ("constant", "C", 1)] {
        let node = root(&e, &a, variable);
        assert!(a.rows.iter().any(|r| {
            e.program().signatures()[r.relation].name == name
                && r.ports[0] == node
                && r.ports[port] == node
        }));
    }
}
#[test]
fn all_type_and_behavior_targets_accept_independently_checked_combinators() {
    let ws = witnesses();
    let expectations = expectations();
    for (i, w) in ws.iter().enumerate() {
        let (args, expected) = &expectations[i];
        assert_eq!(reduce(apps(w.clone(), args.clone()), &mut 10000), *expected);
        for kind in ["type", "behavior"] {
            if kind == "type" && i == 8 {
                continue;
            }
            let doc = document(kind);
            let mut e = start(&doc, i, Some(w));
            let a = first(&mut e, 10_000_000)
                .unwrap_or_else(|| panic!("{kind} target {i} rejected its witness"));
            settled(&e, &a);
            assert_eq!(term(&e, &a, root(&e, &a, "Program"), &mut vec![]), *w);
            if kind == "behavior" {
                assert_eq!(term(&e, &a, root(&e, &a, "Output"), &mut vec![]), *expected);
            }
        }
    }
}
#[test]
fn unrestricted_synthesis_returns_new_programs_and_can_continue() {
    for kind in ["type", "behavior"] {
        let doc = document(kind);
        let mut identity_search = start(&doc, 0, None);
        let answer =
            first(&mut identity_search, 60_000_000).expect("unrestricted identity synthesis");
        settled(&identity_search, &answer);
        let p = term(
            &identity_search,
            &answer,
            root(&identity_search, &answer, "Program"),
            &mut vec![],
        );
        assert_eq!(reduce(app(p, Symbol(0)), &mut 10000), Symbol(0));
        let mut e = start(&doc, 1, None);
        let mut r = Reader::default();
        let mut count = 0;
        for _ in 0..60_000_000 {
            e.advance(1);
            if let Some(a) = r.next(&mut e) {
                settled(&e, &a);
                let p = term(&e, &a, root(&e, &a, "Program"), &mut vec![]);
                assert_eq!(
                    reduce(apps(p, [Symbol(0), Symbol(1)]), &mut 10000),
                    Symbol(0)
                );
                count += 1;
                if count == 2 {
                    break;
                }
            }
        }
        assert_eq!(
            count, 2,
            "{kind}: enumeration must continue past its first constant program"
        );
        assert!(!e.delivery_done(), "recursive synthesis remains open");
    }
}
fn type_shape(
    e: &Engine,
    a: &Answer,
    node: u64,
    vars: &mut std::collections::BTreeMap<u64, usize>,
    path: &mut Vec<u64>,
) -> String {
    assert!(!path.contains(&node), "recursive type survived");
    let arrows: Vec<_> = a
        .rows
        .iter()
        .filter(|r| e.program().signatures()[r.relation].name == "arrow" && r.ports[0] == node)
        .collect();
    if let Some(first) = arrows.first() {
        assert!(arrows.iter().all(|r| r.ports == first.ports));
        path.push(node);
        let left = type_shape(e, a, first.ports[1], vars, path);
        let right = type_shape(e, a, first.ports[2], vars, path);
        path.pop();
        format!("({left}->{right})")
    } else {
        let next = vars.len();
        vars.entry(node).or_insert(next).to_string()
    }
}
#[test]
fn forward_queries_and_finite_type_failure_execute() {
    let doc = document("type");
    let expected_types = [
        "(0->(1->0))",
        "((0->(1->2))->((0->1)->(0->2)))",
        "(0->0)",
        "((0->1)->((2->0)->(2->1)))",
        "((0->(1->2))->(1->(0->2)))",
        "((0->(0->1))->(0->1))",
    ];
    for i in 8..14 {
        let mut e = start(&doc, i, None);
        let a = first(&mut e, 10_000_000).expect("infer a finite principal type");
        settled(&e, &a);
        assert_eq!(
            type_shape(
                &e,
                &a,
                root(&e, &a, "Type"),
                &mut Default::default(),
                &mut vec![]
            ),
            expected_types[i - 8]
        );
    }
    let mut e = start(&doc, 14, None);
    assert!(
        first(&mut e, 10_000_000).is_none(),
        "self-application has no finite simple type"
    );
    assert!(e.delivery_done());
    let doc = document("behavior");
    let specs = expectations();
    let outputs = vec![
        specs[0].1.clone(),
        specs[1].1.clone(),
        specs[3].1.clone(),
        specs[4].1.clone(),
        specs[5].1.clone(),
        specs[6].1.clone(),
        app(K, identity()),
        apps(S, [identity(), K]),
        S,
    ];
    for i in 9..18 {
        let mut e = start(&doc, i, None);
        let a = first(&mut e, 10_000_000).expect("forward evaluation");
        settled(&e, &a);
        assert_eq!(
            term(&e, &a, root(&e, &a, "Output"), &mut vec![]),
            outputs[i - 9]
        );
        if i == 17 {
            assert!(
                a.rows
                    .iter()
                    .any(|r| e.program().signatures()[r.relation].name == "no_c"
                        && r.ports == [root(&e, &a, "Hole")])
            );
        }
    }
    let mut e = start(&doc, 18, None);
    assert!(first(&mut e, 10_000_000).is_none());
    assert!(e.delivery_done());
}
