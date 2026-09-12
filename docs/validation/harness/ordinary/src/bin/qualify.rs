include!(concat!(env!("OUT_DIR"), "/rewrite.rs"));
use chr_compiled::{Access, Policy, PreparedRuleset};
use chr_syntax::{Query, Rule, Term, Var, c, v};
use std::{collections::BTreeSet, sync::Arc, time::Instant};
fn old(p: &PreparedRuleset, q: &Query, policy: Policy) -> chr_syntax::Answer {
    let mut e = p.start(q.clone(), policy, Access::Indexed).unwrap();
    for _ in 0..100000 {
        let s = e.advance(2048);
        assert!(!s.failed && !s.pending_split);
        if s.exhausted {
            return e.observe().unwrap();
        }
    }
    panic!("old budget");
}
fn new(p: &Arc<chr::program::Prepared>) -> (Vec<chr::observe::Output>, u64) {
    let mut e = chr::engine::Engine::new(p.clone());
    let mut output = vec![];
    for _ in 0..1000000 {
        e.advance(1);
        if let Some(o) = e.take_output() {
            output.push(o);
        }
        if e.delivery_done() {
            return (output, e.applications());
        }
    }
    panic!("new budget");
}
fn check_joins() {
    let join = Rule::propagate(
        "join",
        vec![c("p", vec![v(0)]), c("q", vec![v(0)])],
        c("hit", vec![v(0)]).into(),
    );
    for (label, alias, duplicates, expected) in [
        ("nonbinding", false, false, 0),
        ("alias-wake", true, false, 1),
        ("duplicate-occurrences", false, true, 2),
    ] {
        let mut rules = vec![join.clone()];
        let mut constraints = vec![
            c("p", vec![v(0)]),
            c("q", vec![v(if duplicates { 0 } else { 1 })]),
        ];
        let mut source = String::from("p(X),q(X) ==> hit(X).");
        let mut query = if duplicates {
            String::from("p(A),q(A),p(A)")
        } else {
            String::from("p(A),q(B)")
        };
        if duplicates {
            constraints.push(c("p", vec![v(0)]));
        }
        if alias {
            rules.push(Rule::simplify(
                "alias",
                vec![c("merge", vec![v(0), v(1)])],
                chr_syntax::eq(v(0), v(1)),
            ));
            constraints.push(c("merge", vec![v(0), v(1)]));
            source.push_str("merge(X,Y)<=>X=Y.");
            query.push_str(",merge(A,B)");
        }
        let outputs = if duplicates {
            vec![("A".into(), Var(0))]
        } else {
            vec![("A".into(), Var(0)), ("B".into(), Var(1))]
        };
        let q = Query {
            constraints,
            outputs,
        };
        let p = PreparedRuleset::new(rules, None).unwrap();
        for policy in [Policy::Global, Policy::Active] {
            let a = old(&p, &q, policy);
            assert_eq!(
                a.residual.iter().filter(|r| r.name == "hit").count(),
                expected
            );
            if alias {
                assert_eq!(a.outputs[0].1, a.outputs[1].1);
            } else if !duplicates {
                assert_ne!(a.outputs[0].1, a.outputs[1].1);
            }
        }
        let p = Arc::new(
            chr::program::prepare(
                &chr::syntax::parse_program(&source).unwrap(),
                &chr::syntax::parse_query(&query).unwrap(),
            )
            .unwrap(),
        );
        let (a, _) = new(&p);
        let mut vars = vec![];
        let mut hits = 0;
        for o in a {
            match o {
                chr::observe::Output::Fact { relation, .. }
                    if p.signatures[relation].name == "hit" =>
                {
                    hits += 1
                }
                chr::observe::Output::Variable { variable, .. } => vars.push(variable),
                _ => {}
            }
        }
        assert_eq!(hits, expected);
        if alias {
            assert_eq!(vars[0], vars[1]);
        } else if !duplicates {
            assert_ne!(vars[0], vars[1]);
        }
        println!("semantic_check={label}:pass");
    }
}
fn check_fresh() {
    let rules = vec![Rule::simplify(
        "fresh",
        vec![c("seed", vec![v(0)])],
        c("q", vec![v(1)]).into(),
    )];
    let q = Query {
        constraints: vec![c("seed", vec![v(0)]), c("seed", vec![v(1)])],
        outputs: vec![("A".into(), Var(0)), ("B".into(), Var(1))],
    };
    let p = PreparedRuleset::new(rules, None).unwrap();
    for policy in [Policy::Global, Policy::Active] {
        let a = old(&p, &q, policy);
        let inputs: BTreeSet<_> = a.outputs.iter().map(|(_, v)| v.clone()).collect();
        let locals: BTreeSet<_> = a
            .residual
            .iter()
            .map(|r| {
                assert_eq!(r.name, "q");
                r.args[0].clone()
            })
            .collect();
        assert_eq!(locals.len(), 2);
        assert!(inputs.is_disjoint(&locals));
    }
    let p = Arc::new(
        chr::program::prepare(
            &chr::syntax::parse_program("seed(X)<=>q(Y).").unwrap(),
            &chr::syntax::parse_query("seed(A),seed(B)").unwrap(),
        )
        .unwrap(),
    );
    let (a, apps) = new(&p);
    assert_eq!(apps, 2);
    let mut inputs = BTreeSet::new();
    let mut locals = BTreeSet::new();
    for o in a {
        match o {
            chr::observe::Output::Variable { variable, .. } => {
                inputs.insert(variable);
            }
            chr::observe::Output::Port { variable } => {
                locals.insert(variable);
            }
            _ => {}
        }
    }
    assert_eq!(locals.len(), 2);
    assert!(inputs.is_disjoint(&locals));
    println!("semantic_check=fresh-body-locals:pass");
}
fn main() {
    check_joins();
    check_fresh();
    for same in [false, true] {
        let rules = vec![Rule::propagate(
            "pair",
            vec![c("p", vec![v(0)]), c("p", vec![v(1)])],
            c("pair", vec![v(0), v(1)]).into(),
        )];
        let q = Query {
            constraints: vec![
                c("p", vec![v(0)]),
                c("p", vec![v(if same { 0 } else { 1 })]),
            ],
            outputs: if same {
                vec![("A".into(), Var(0))]
            } else {
                vec![("A".into(), Var(0)), ("B".into(), Var(1))]
            },
        };
        let p = PreparedRuleset::new(rules, None).unwrap();
        for policy in [Policy::Global, Policy::Active] {
            let mut e = p.start(q.clone(), policy, Access::Indexed).unwrap();
            e.enable_trace();
            for _ in 0..10000 {
                let s = e.advance(2048);
                assert!(!s.failed && !s.pending_split);
                if s.exhausted {
                    break;
                }
            }
            assert!(e.status().exhausted);
            assert_eq!(e.trace().len(), 2);
            let trace = e.trace();
            assert_eq!(
                trace[0].1,
                trace[1].1.iter().rev().copied().collect::<Vec<_>>()
            );
            assert_ne!(trace[0].1[0], trace[0].1[1]);
            let a = e.observe().unwrap();
            let actual = a
                .residual
                .iter()
                .filter(|r| r.name == "pair")
                .map(|r| r.args.clone())
                .collect::<Vec<_>>();
            let av = a.outputs[0].1.clone();
            let bv = if same {
                av.clone()
            } else {
                a.outputs[1].1.clone()
            };
            let mut expected = vec![vec![av.clone(), bv.clone()], vec![bv, av]];
            expected.sort();
            let mut actual = actual;
            actual.sort();
            assert_eq!(actual, expected);
        }
        let p = Arc::new(
            chr::program::prepare(
                &chr::syntax::parse_program("p(X),p(Y) ==> pair(X,Y).").unwrap(),
                &chr::syntax::parse_query(if same { "p(A),p(A)" } else { "p(A),p(B)" }).unwrap(),
            )
            .unwrap(),
        );
        let (out, apps) = new(&p);
        assert_eq!(apps, 2);
        let mut vars = vec![];
        let mut tuples = vec![];
        let mut current = None;
        for o in out {
            match o {
                chr::observe::Output::Variable { variable, .. } => vars.push(variable),
                chr::observe::Output::Fact { relation, .. } => {
                    current = Some((p.signatures[relation].name.clone(), vec![]))
                }
                chr::observe::Output::Port { variable } => {
                    current.as_mut().unwrap().1.push(variable)
                }
                chr::observe::Output::EndFact => {
                    let (r, t) = current.take().unwrap();
                    if r == "pair" {
                        tuples.push(t)
                    }
                }
                _ => {}
            }
        }
        let a = vars[0];
        let b = if same { a } else { vars[1] };
        let mut expected = vec![vec![a, b], vec![b, a]];
        expected.sort();
        tuples.sort();
        assert_eq!(tuples, expected);
        println!("semantic_check=ordered-self-join-same-{same}:pass");
    }
}
