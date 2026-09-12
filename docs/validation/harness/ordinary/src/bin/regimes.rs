use chr_compiled::{Access, Policy, PreparedRuleset};
use chr_syntax::{Query, Rule, Term, Var, c, v};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
    time::Instant,
};
type Counts = BTreeMap<String, usize>;
const CASES: &str = "rewrite sparse dense alias degree common failure distinct cyclic";
fn counts(items: &[(&str, usize)]) -> Counts {
    items
        .iter()
        .map(|(name, count)| ((*name).into(), *count))
        .collect()
}
fn inputs(name: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("{name}(V{i})"))
        .collect::<Vec<_>>()
        .join(",")
}
fn workload(case: &str, n: usize) -> Result<(String, String, usize, Vec<Counts>), String> {
    let rewrite = "p(X) <=> q(X). q(X) <=> done(X).";
    let p = inputs("p", n);
    let one = |program: &str, query: String, apps, facts: Counts| {
        (program.into(), query, apps, vec![facts])
    };
    Ok(match case {
        "rewrite" => one(rewrite, p, 2 * n, counts(&[("done", n)])),
        "sparse" => one(
            "p(X),q(X) ==> hit(X).",
            format!("p(V0),{}", inputs("q", n)),
            1,
            counts(&[("p", 1), ("q", n), ("hit", 1)]),
        ),
        "dense" => one(
            "p(X),q(Y) ==> hit(X,Y).",
            format!(
                "{p},{}",
                (0..n)
                    .map(|i| format!("q(W{i})"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            n * n,
            counts(&[("p", n), ("q", n), ("hit", n * n)]),
        ),
        "alias" => {
            let mut query = vec!["p(V0)".into(), format!("q(V{n})")];
            query.extend((0..n).map(|i| format!("merge(V{i},V{})", i + 1)));
            one(
                "p(X),q(X) ==> hit(X). merge(X,Y) <=> X=Y.",
                query.join(","),
                n + 1,
                counts(&[("p", 1), ("q", 1), ("hit", 1)]),
            )
        }
        "degree" => {
            let mut query = vec!["hub(A)".into()];
            query.extend((0..n).map(|i| format!("spoke(B,V{i})")));
            query.push("merge(A,B)".into());
            one(
                "hub(X),spoke(X,Y) ==> reach(Y). merge(X,Y) <=> X=Y.",
                query.join(","),
                n + 1,
                counts(&[("hub", 1), ("spoke", n), ("reach", n)]),
            )
        }
        "common" => (
            rewrite.into(),
            format!("(true;true),{p}"),
            2 * n,
            vec![counts(&[("done", n)]); 2],
        ),
        "failure" => one(
            rewrite,
            format!("(fail;({p}))"),
            2 * n,
            counts(&[("done", n)]),
        ),
        "distinct" => (
            "p(X) <=> first(X). q(X) <=> second(X).".into(),
            format!("({p};{})", inputs("q", n)),
            2 * n,
            vec![counts(&[("first", n)]), counts(&[("second", n)])],
        ),
        "cyclic" => {
            let query = (0..n)
                .map(|i| format!("edge(A{i},B{i}),edge(B{i},C{i}),edge(C{i},A{i})"))
                .collect::<Vec<_>>()
                .join(",");
            one(
                "edge(X,Y),edge(Y,Z),edge(Z,X) ==> triangle(X,Y,Z).",
                query,
                3 * n,
                counts(&[("edge", 3 * n), ("triangle", 3 * n)]),
            )
        }
        _ => return Err(format!("unknown case {case}; cases: {CASES}")),
    })
}

// Exact ordered tuples are derived from the input variables, not output facts.
type Tuples = BTreeMap<String, HashSet<Vec<u64>>>;
fn expected_tuples(
    case: &str,
    n: usize,
    bindings: &BTreeMap<String, u64>,
) -> Result<Tuples, String> {
    let var = |name: &str| {
        bindings
            .get(name)
            .copied()
            .ok_or_else(|| format!("missing binding {name}"))
    };
    let distinct = bindings
        .iter()
        .filter(|(name, _)| case != "degree" || name.as_str() != "B")
        .map(|(_, id)| *id)
        .collect::<HashSet<_>>()
        .len();
    let identities_valid = match case {
        "alias" => distinct == 1,
        "degree" => var("A")? == var("B")? && distinct + 1 == bindings.len(),
        _ => distinct == bindings.len(),
    };
    if !identities_valid {
        return Err(format!("incorrect query-variable identities: {bindings:?}"));
    }
    let mut tuples = Tuples::new();
    let mut add = |relation: &str, ports: Vec<u64>| {
        tuples.entry(relation.into()).or_default().insert(ports);
    };
    match case {
        "rewrite" | "common" | "failure" | "distinct" => {
            for i in 0..n {
                let v = var(&format!("V{i}"))?;
                if case == "distinct" {
                    add("first", vec![v]);
                    add("second", vec![v]);
                } else {
                    add("done", vec![v]);
                }
            }
        }
        "sparse" => {
            add("p", vec![var("V0")?]);
            add("hit", vec![var("V0")?]);
            for i in 0..n {
                add("q", vec![var(&format!("V{i}"))?]);
            }
        }
        "dense" => {
            for i in 0..n {
                let v = var(&format!("V{i}"))?;
                add("p", vec![v]);
                add("q", vec![var(&format!("W{i}"))?]);
                for j in 0..n {
                    add("hit", vec![v, var(&format!("W{j}"))?]);
                }
            }
        }
        "alias" => {
            for name in ["p", "q", "hit"] {
                add(name, vec![var("V0")?]);
            }
        }
        "degree" => {
            let a = var("A")?;
            add("hub", vec![a]);
            for i in 0..n {
                let v = var(&format!("V{i}"))?;
                add("spoke", vec![a, v]);
                add("reach", vec![v]);
            }
        }
        "cyclic" => {
            for i in 0..n {
                let a = var(&format!("A{i}"))?;
                let b = var(&format!("B{i}"))?;
                let c = var(&format!("C{i}"))?;
                for ports in [[a, b], [b, c], [c, a]] {
                    add("edge", ports.to_vec());
                }
                for ports in [[a, b, c], [b, c, a], [c, a, b]] {
                    add("triangle", ports.to_vec());
                }
            }
        }
        _ => return Err(format!("unknown tuple oracle {case}")),
    }
    Ok(tuples)
}

fn vid(name: &str, m: &mut BTreeMap<String, u64>) -> u64 {
    let n = m.len() as u64;
    *m.entry(name.into()).or_insert(n)
}
fn atom(a: &chr::syntax::Atom, m: &mut BTreeMap<String, u64>) -> chr_syntax::Constraint {
    c(
        &a.relation,
        a.args.iter().map(|s| v(vid(s, m))).collect::<Vec<_>>(),
    )
}
fn goal(b: &chr::syntax::Body, m: &mut BTreeMap<String, u64>) -> chr_syntax::Goal {
    use chr::syntax::Body::*;
    match b {
        Atom { atom: a } => atom(a, m).into(),
        Equal { left, right } => chr_syntax::eq(v(vid(left, m)), v(vid(right, m))),
        And { items } => chr_syntax::and(items.iter().map(|b| goal(b, m)).collect::<Vec<_>>()),
        True => chr_syntax::Goal::True,
        Fail => chr_syntax::Goal::Fail,
        Or { .. } => panic!("choice is not qualified"),
    }
}
fn convert(p: &chr::syntax::Program, q: &chr::syntax::Body) -> (Vec<Rule>, Query) {
    let rules = p
        .rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut m = BTreeMap::new();
            Rule {
                name: format!("r{i}"),
                kept: r.kept.iter().map(|a| atom(a, &mut m)).collect(),
                removed: r.removed.iter().map(|a| atom(a, &mut m)).collect(),
                guards: vec![],
                body: goal(&r.body, &mut m),
            }
        })
        .collect();
    let mut m = BTreeMap::new();
    let g = goal(q, &mut m);
    let constraints = match g {
        chr_syntax::Goal::And(items) => items
            .into_iter()
            .map(|g| match g {
                chr_syntax::Goal::Constraint(c) => c,
                _ => panic!("non-flat input"),
            })
            .collect(),
        chr_syntax::Goal::Constraint(c) => vec![c],
        _ => panic!("non-flat input"),
    };
    (
        rules,
        Query {
            constraints,
            outputs: m.into_iter().map(|(s, i)| (s, Var(i))).collect(),
        },
    )
}
fn verify(case: &str, n: usize, bindings: &BTreeMap<String, u64>, facts: &[(String, Vec<u64>)]) {
    let mut expected = expected_tuples(case, n, bindings).unwrap();
    for (r, t) in facts {
        assert!(
            expected.get_mut(r).is_some_and(|s| s.remove(t)),
            "unexpected/duplicate {case} {r}{t:?}"
        );
    }
    assert!(
        expected.values().all(|s| s.is_empty()),
        "missing {case} {expected:?}"
    );
    let names = match case {
        "degree" => n + 2,
        "alias" => n + 1,
        "dense" => 2 * n,
        "cyclic" => 3 * n,
        _ => n,
    };
    assert_eq!(bindings.len(), names);
}
fn old_verify(case: &str, n: usize, a: &chr_syntax::Answer) {
    let var = |t: &Term| match t {
        Term::Var(Var(i)) => *i,
        _ => panic!("constructor escaped"),
    };
    let bindings = a.outputs.iter().map(|(s, t)| (s.clone(), var(t))).collect();
    let facts = a
        .residual
        .iter()
        .map(|c| (c.name.clone(), c.args.iter().map(var).collect()))
        .collect::<Vec<_>>();
    verify(case, n, &bindings, &facts);
}
fn new_verify(case: &str, n: usize, p: &chr::program::Prepared, out: &[chr::observe::Output]) {
    use chr::observe::Output::*;
    let mut b = BTreeMap::new();
    let mut facts = vec![];
    let mut current = None;
    let mut ends = 0;
    for o in out {
        match o {
            Variable { slot, variable } => {
                assert!(
                    b.insert(p.query_variables[*slot].clone(), *variable)
                        .is_none()
                );
            }
            Fact { relation, .. } => {
                assert!(current.is_none());
                current = Some((p.signatures[*relation].name.clone(), vec![]));
            }
            Port { variable } => current.as_mut().unwrap().1.push(*variable),
            EndFact => facts.push(current.take().unwrap()),
            End => ends += 1,
            chr::observe::Output::Begin { .. } => {}
            _ => panic!("unexpected output {o:?}"),
        }
    }
    assert_eq!(ends, 1);
    assert!(current.is_none());
    verify(case, n, &b, &facts);
}
fn us(t: std::time::Duration) -> f64 {
    t.as_secs_f64() * 1e6
}
fn main() {
    let traced = std::env::var_os("TRACE_WORKCHECK").is_some();
    let warmups = if traced { 0 } else { 2 };
    eprintln!("MODE traced={traced} output_oracle=true");
    let cases: Vec<String> = std::env::args().nth(1).map(|s| vec![s]).unwrap_or_else(|| {
        ["rewrite", "sparse", "dense", "alias", "degree", "cyclic"]
            .map(str::to_string)
            .to_vec()
    });
    let sizes: Vec<usize> = std::env::args()
        .nth(2)
        .map(|s| vec![s.parse().unwrap()])
        .unwrap_or(vec![8, 32, 128]);
    let reps: usize = std::env::args()
        .nth(3)
        .map(|s| s.parse().unwrap())
        .unwrap_or(11);
    println!(
        "case,n,rep,runner,init_us,runtime_output_us,engine_drop_us,output_drop_us,qualified_apps,ticks,source_us,trace_enabled"
    );
    for case in cases {
        for &n in &sizes {
            let (source, query, apps, _) = workload(&case, n).unwrap();
            let t = Instant::now();
            let ast = chr::syntax::parse_program(&source).unwrap();
            let qast = chr::syntax::parse_query(&query).unwrap();
            let parse = us(t.elapsed());
            let t = Instant::now();
            let p = Arc::new(chr::program::prepare(&ast, &qast).unwrap());
            let prep = us(t.elapsed());
            let t = Instant::now();
            let (rules, q) = convert(&ast, &qast);
            let convert_us = us(t.elapsed());
            let t = Instant::now();
            let oldp = PreparedRuleset::new(rules, None).unwrap();
            let oldprep = us(t.elapsed());
            eprintln!(
                "PREP {case} {n} parse_us={parse:.3} current_prepare_us={prep:.3} old_ast_convert_us={convert_us:.3} old_prepare_us={oldprep:.3}"
            );
            for rep in 0..reps + warmups {
                for offset in 0..3 {
                    let mode = (rep + offset) % 3;
                    if mode == 0 && std::env::var_os("SKIP_GLOBAL").is_some() {
                        continue;
                    }
                    if mode < 2 {
                        let t = Instant::now();
                        let mut e = oldp
                            .start(
                                q.clone(),
                                if mode == 0 {
                                    Policy::Global
                                } else {
                                    Policy::Active
                                },
                                Access::Indexed,
                            )
                            .unwrap();
                        if traced { e.enable_trace(); }
                        let init = us(t.elapsed());
                        let t = Instant::now();
                        let mut ticks = 0;
                        loop {
                            let status = e.advance(2048);
                            ticks += 1;
                            assert!(!status.failed && !status.pending_split);
                            if status.exhausted {
                                break;
                            }
                            assert!(ticks < 10000000);
                        }
                        let source = us(t.elapsed());
                        let a = e.observe().unwrap();
                        let runtime = us(t.elapsed());
                        if traced {
                        assert_eq!(e.trace().len(), apps, "application oracle {case} {n}");
                        let unique = e.trace().iter().collect::<BTreeSet<_>>();
                        assert_eq!(unique.len(), apps);
                        for (rule, ids) in e.trace() {
                            let arity =
                                ast.rules[*rule].kept.len() + ast.rules[*rule].removed.len();
                            assert_eq!(ids.len(), arity);
                            assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), arity);
                        }
                        let mut per_rule = BTreeMap::new();
                        for (rule, _) in e.trace() {
                            *per_rule.entry(*rule).or_insert(0usize) += 1;
                        }
                        let expected = match case.as_str() {
                            "rewrite" => BTreeMap::from([(0, n), (1, n)]),
                            "alias" => BTreeMap::from([(0, 1), (1, n)]),
                            "degree" => BTreeMap::from([(0, n), (1, 1)]),
                            _ => BTreeMap::from([(0, apps)]),
                        };
                        assert_eq!(per_rule, expected);
                        } else { assert!(e.trace().is_empty()); }
                        old_verify(&case, n, &a);
                        let t = Instant::now();
                        drop(e);
                        let ed = us(t.elapsed());
                        let t = Instant::now();
                        drop(a);
                        let od = us(t.elapsed());
                        if rep >= warmups {
                            println!(
                                "{case},{n},{},{},{init:.3},{runtime:.3},{ed:.3},{od:.3},{apps},{ticks},{source:.3},{traced}",
                                rep - warmups,
                                if mode == 0 {
                                    "old-global"
                                } else {
                                    "old-active"
                                }
                            );
                        }
                    } else {
                        let t = Instant::now();
                        let mut e = chr::engine::Engine::new(p.clone());
                        let init = us(t.elapsed());
                        let mut out = vec![];
                        let t = Instant::now();
                        let mut ticks = 0;
                        let mut source = None;
                        loop {
                            e.advance(1);
                            ticks += 1;
                            if let Some(o) = e.take_output() {
                                out.push(o);
                            }
                            if source.is_none() && e.exhausted() {
                                source = Some(us(t.elapsed()));
                            }
                            if e.delivery_done() {
                                break;
                            }
                            assert!(ticks < 500000000);
                        }
                        let runtime = us(t.elapsed());
                        assert_eq!(e.applications(), apps as u64);
                        new_verify(&case, n, &p, &out);
                        let t = Instant::now();
                        drop(e);
                        let ed = us(t.elapsed());
                        let t = Instant::now();
                        drop(out);
                        let od = us(t.elapsed());
                        if rep >= warmups {
                            println!(
                                "{case},{n},{},current,{init:.3},{runtime:.3},{ed:.3},{od:.3},{apps},{ticks},{:.3},false",
                                rep - warmups,
                                source.unwrap()
                            );
                        }
                    }
                }
            }
            let t = Instant::now();
            drop(oldp);
            let old_drop = us(t.elapsed());
            let t = Instant::now();
            drop(p);
            let current_drop = us(t.elapsed());
            let t = Instant::now();
            drop(ast);
            drop(qast);
            let ast_drop = us(t.elapsed());
            eprintln!("AST_DROP {case} {n} us={ast_drop:.3}");
            eprintln!(
                "PREP_DROP {case} {n} old_us={old_drop:.3} current_us={:.3}",
                current_drop
            );
        }
    }
}
