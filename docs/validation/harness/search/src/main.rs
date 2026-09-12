use chr_compiled::{Access, Policy, PreparedRuleset, SearchEvent};
use chr_syntax::{Query, Rule, Term, Var, c, v};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};
type Facts = Vec<(String, Vec<u64>)>;
#[derive(Default, Debug)]
struct Answer {
    bindings: BTreeMap<String, u64>,
    facts: Facts,
}
fn canonical(a: &Answer) -> String {
    let mut known = BTreeMap::new();
    for (name, id) in &a.bindings {
        known.entry(*id).or_insert(name.clone());
    }
    let fresh = a
        .facts
        .iter()
        .flat_map(|(_, a)| a)
        .filter(|id| !known.contains_key(id))
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    assert!(
        fresh.len() <= 6,
        "qualification canonicalizer only handles small fresh fixtures"
    );
    fn permute(
        ids: &[u64],
        i: usize,
        perm: &mut Vec<usize>,
        m: &mut BTreeMap<u64, String>,
        a: &Answer,
        best: &mut Option<String>,
    ) {
        if i < ids.len() {
            for j in 0..ids.len() {
                if !perm.contains(&j) {
                    perm.push(j);
                    m.insert(ids[i], format!("#{j}"));
                    permute(ids, i + 1, perm, m, a, best);
                    perm.pop();
                }
            }
            return;
        }
        let b = a
            .bindings
            .iter()
            .map(|(name, id)| format!("{name}={}", m[id]))
            .collect::<Vec<_>>();
        let mut f = a
            .facts
            .iter()
            .map(|(name, args)| {
                format!(
                    "{name}({})",
                    args.iter()
                        .map(|id| m[id].clone())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>();
        f.sort();
        let key = format!("{b:?}|{f:?}");
        if best.as_ref().is_none_or(|old| key < *old) {
            *best = Some(key);
        }
    }
    let mut best = None;
    permute(&fresh, 0, &mut vec![], &mut known, a, &mut best);
    best.unwrap()
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
        Or { items } => {
            let mut xs = items.iter().map(|b| goal(b, m)).collect::<Vec<_>>();
            let last = xs.pop().unwrap();
            xs.into_iter()
                .rev()
                .fold(last, |tail, x| chr_syntax::or(x, tail))
        }
        True => chr_syntax::Goal::True,
        Fail => chr_syntax::Goal::Fail,
    }
}
fn convert(p: &chr::syntax::Program, q: &chr::syntax::Body) -> (Vec<Rule>, Query) {
    let mut rules = p
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
        .collect::<Vec<_>>();
    let mut m = BTreeMap::new();
    let body = goal(q, &mut m);
    let args = (0..m.len() as u64).map(v).collect::<Vec<_>>();
    assert!(!format!("{p:?}{q:?}").contains("bench_admit"));
    rules.push(Rule::simplify(
        "admission",
        [c("bench_admit", args.clone())],
        body,
    ));
    (
        rules,
        Query {
            constraints: vec![c("bench_admit", args)],
            outputs: m.into_iter().map(|(s, id)| (s, Var(id))).collect(),
        },
    )
}
struct Prepared {
    new: Arc<chr::program::Prepared>,
    old: PreparedRuleset,
    query: Query,
    rule_count: usize,
    parse: f64,
    newprep: f64,
    convert: f64,
    oldprep: f64,
    astdrop: f64,
}
fn us(t: std::time::Duration) -> f64 {
    t.as_secs_f64() * 1e6
}
fn prepare(p: &str, q: &str) -> Prepared {
    let t = Instant::now();
    let ast = chr::syntax::parse_program(p).unwrap();
    let qast = chr::syntax::parse_query(q).unwrap();
    let parse = us(t.elapsed());
    let t = Instant::now();
    let new = Arc::new(chr::program::prepare(&ast, &qast).unwrap());
    let newprep = us(t.elapsed());
    let t = Instant::now();
    let (rules, query) = convert(&ast, &qast);
    let convert = us(t.elapsed());
    let t = Instant::now();
    let old = PreparedRuleset::new(rules, None).unwrap();
    let oldprep = us(t.elapsed());
    let rule_count = ast.rules.len();
    let d = Instant::now();
    drop(ast);
    drop(qast);
    let astdrop = us(d.elapsed());
    Prepared {
        new,
        old,
        query,
        rule_count,
        parse,
        newprep,
        convert,
        oldprep,
        astdrop,
    }
}
#[derive(Default)]
struct ResultData {
    answers: Vec<Answer>,
    traces: Vec<Vec<usize>>,
    failed: usize,
    splits: usize,
    ticks: u64,
    apps: u64,
    init: f64,
    runtime: f64,
    observe: f64,
    drop: f64,
    output_drop: f64,
    source_exhausted: Option<f64>,
}
fn old_run(p: &Prepared, policy: Policy, first: bool, trace: bool) -> ResultData {
    let mut result = ResultData::default();
    let t = Instant::now();
    let mut e = p
        .old
        .start_search(p.query.clone(), policy, Access::Indexed)
        .unwrap();
    if trace {
        e.enable_trace();
    }
    result.init = us(t.elapsed());
    let t = Instant::now();
    loop {
        result.ticks += 1;
        assert!(result.ticks < 5_000_000, "old finite budget");
        match e.tick() {
            SearchEvent::Split { .. } => result.splits += 1,
            SearchEvent::Complete(mut branch) => {
                // Copying trace/observing and disposing terminal engines are explicitly timed.
                if trace {
                    result
                        .traces
                        .push(branch.engine.trace().iter().map(|(r, _)| *r).collect());
                }
                let o = Instant::now();
                let a = branch.engine.observe().unwrap();
                result.observe += us(o.elapsed());
                let var = |t: Term| match t {
                    Term::Var(Var(id)) => id,
                    _ => panic!("constructor escaped"),
                };
                result.answers.push(Answer {
                    bindings: a.outputs.into_iter().map(|(n, t)| (n, var(t))).collect(),
                    facts: a
                        .residual
                        .into_iter()
                        .map(|r| (r.name, r.args.into_iter().map(var).collect()))
                        .collect(),
                });
                let d = Instant::now();
                drop(branch);
                result.drop += us(d.elapsed());
                if first {
                    break;
                }
            }
            SearchEvent::Failed(branch) => {
                result.failed += 1;
                if trace {
                    result
                        .traces
                        .push(branch.engine.trace().iter().map(|(r, _)| *r).collect());
                }
                let d = Instant::now();
                drop(branch);
                result.drop += us(d.elapsed());
            }
            SearchEvent::Exhausted => break,
            SearchEvent::Progress => (),
        }
    }
    result.runtime = us(t.elapsed()) - result.drop;
    let d = Instant::now();
    drop(e);
    result.drop += us(d.elapsed());
    result
}
fn new_run(p: &Prepared, first: bool) -> ResultData {
    use chr::observe::Output::*;
    let mut result = ResultData::default();
    let t = Instant::now();
    let mut e = chr::engine::Engine::new(p.new.clone());
    result.init = us(t.elapsed());
    let t = Instant::now();
    let diagnostic = std::env::var_os("WIDE_DIAGNOSTIC").is_some();
    let mut answer = None;
    let mut fact = None;
    loop {
        e.advance(1);
        result.ticks += 1;
        assert!(
            result.ticks < 100_000_000,
            "current finite budget: apps={} completed={}",
            e.applications(),
            result.answers.len()
        );
        if diagnostic && result.ticks % 10_000_000 == 0 {
            eprintln!(
                "PROGRESS ticks={} apps={} answers={}",
                result.ticks,
                e.applications(),
                result.answers.len()
            );
        }
        if let Some(o) = e.take_output() {
            match o {
                Begin { .. } => {
                    assert!(answer.is_none());
                    answer = Some(Answer::default());
                }
                Variable { slot, variable } => {
                    assert!(
                        answer
                            .as_mut()
                            .unwrap()
                            .bindings
                            .insert(p.new.query_variables[slot].clone(), variable)
                            .is_none()
                    );
                }
                Fact { relation, .. } => {
                    assert!(fact.is_none());
                    fact = Some((p.new.signatures[relation].name.clone(), vec![]));
                }
                Port { variable } => fact.as_mut().unwrap().1.push(variable),
                EndFact => answer.as_mut().unwrap().facts.push(fact.take().unwrap()),
                End => {
                    result.answers.push(answer.take().unwrap());
                    if first {
                        break;
                    }
                }
                _ => panic!("pending syntax in normal form"),
            }
        }
        if result.source_exhausted.is_none() && e.exhausted() {
            result.source_exhausted = Some(us(t.elapsed()));
        }
        if e.delivery_done() {
            break;
        }
    }
    assert!(answer.is_none() && fact.is_none());
    result.apps = e.applications();
    result.runtime = us(t.elapsed());
    let d = Instant::now();
    drop(e);
    result.drop = us(d.elapsed());
    result
}
fn keys(r: &ResultData) -> Vec<String> {
    let mut a = r.answers.iter().map(canonical).collect::<Vec<_>>();
    a.sort();
    a
}
fn expected(bindings: &[(&str, u64)], facts: &[(&str, &[u64])]) -> String {
    canonical(&Answer {
        bindings: bindings
            .iter()
            .map(|(n, id)| (n.to_string(), *id))
            .collect(),
        facts: facts
            .iter()
            .map(|(n, a)| (n.to_string(), a.to_vec()))
            .collect(),
    })
}
fn qualify() {
    let fixtures: Vec<(&str, &str, &str, Vec<String>)> = vec![
        (
            "raw-duplicates",
            "",
            "(p(A);p(A))",
            vec![expected(&[("A", 0)], &[("p", &[0])]); 2],
        ),
        (
            "inactive-nested",
            "",
            "(p(A);(q(A);r(A)))",
            vec![
                expected(&[("A", 0)], &[("p", &[0])]),
                expected(&[("A", 0)], &[("q", &[0])]),
                expected(&[("A", 0)], &[("r", &[0])]),
            ],
        ),
        (
            "branch-local-failure",
            "",
            "(fail;(true;true))",
            vec![expected(&[], &[]); 2],
        ),
        (
            "nonbinding",
            "p(X),q(X)==>hit(X).",
            "p(A),q(B),(true;true)",
            vec![expected(&[("A", 0), ("B", 1)], &[("p", &[0]), ("q", &[1])]); 2],
        ),
        (
            "conditional-alias-wake",
            "p(X),q(X)==>hit(X).",
            "p(A),q(B),(A=B;true)",
            vec![
                expected(
                    &[("A", 0), ("B", 0)],
                    &[("p", &[0]), ("q", &[0]), ("hit", &[0])],
                ),
                expected(&[("A", 0), ("B", 1)], &[("p", &[0]), ("q", &[1])]),
            ],
        ),
        (
            "opposite-parent-regions",
            "p(X),q(X)==>hit(X).",
            "p(A),q(C),(A=B;B=C)",
            vec![
                expected(&[("A", 0), ("B", 0), ("C", 1)], &[("p", &[0]), ("q", &[1])]),
                expected(&[("A", 0), ("B", 1), ("C", 1)], &[("p", &[0]), ("q", &[1])]),
            ],
        ),
        (
            "fresh-event-choices",
            "seed(X)<=> (link(X,Y);link(X,Y)).",
            "seed(A),seed(B)",
            vec![
                expected(
                    &[("A", 0), ("B", 1)],
                    &[("link", &[0, 2]), ("link", &[1, 3])]
                );
                4
            ],
        ),
        (
            "occurrence-propagation",
            "p(X),p(Y)==>pair(X,Y).",
            "p(A),p(A),(true;true)",
            vec![
                expected(
                    &[("A", 0)],
                    &[
                        ("p", &[0]),
                        ("p", &[0]),
                        ("pair", &[0, 0]),
                        ("pair", &[0, 0])
                    ]
                );
                2
            ],
        ),
        (
            "consuming-multiplicity",
            "p(X),p(X)<=>hit(X).",
            "p(A),p(A),(true;true)",
            vec![expected(&[("A", 0)], &[("hit", &[0])]); 2],
        ),
    ];
    for (name, source, query, mut expected) in fixtures {
        expected.sort();
        let p = prepare(source, query);
        let a = new_run(&p, false);
        assert_eq!(keys(&a), expected, "current {name}");
        for policy in [Policy::Active, Policy::Global] {
            let b = old_run(&p, policy, false, true);
            assert_eq!(keys(&b), expected, "old {policy:?} {name}");
            eprintln!(
                "QUAL {name} {policy:?} answers={} splits={} failed={} ticks={}",
                b.answers.len(),
                b.splits,
                b.failed,
                b.ticks
            );
        }
        eprintln!(
            "QUAL {name} Current answers={} apps={} ticks={}",
            a.answers.len(),
            a.apps,
            a.ticks
        );
    }
    // Competing source rules must yield ONE committed schedule, not search both.
    let p = prepare("p(X)<=>a(X). p(X)<=>b(X).", "p(A)");
    for mode in 0..3 {
        let r = if mode == 2 {
            new_run(&p, false)
        } else {
            old_run(
                &p,
                if mode == 0 {
                    Policy::Active
                } else {
                    Policy::Global
                },
                false,
                true,
            )
        };
        assert_eq!(r.answers.len(), 1);
        assert_eq!(r.splits, 0);
        let key = canonical(&r.answers[0]);
        assert!(
            [
                expected(&[("A", 0)], &[("a", &[0])]),
                expected(&[("A", 0)], &[("b", &[0])])
            ]
            .contains(&key)
        );
        eprintln!("QUAL committed-schedule mode={mode} {key}");
    }
    let p = prepare("loop(X)<=>loop(X).", "(loop(A);answer(A))");
    for mode in 0..3 {
        let r = if mode == 2 {
            new_run(&p, true)
        } else {
            old_run(
                &p,
                if mode == 0 {
                    Policy::Active
                } else {
                    Policy::Global
                },
                true,
                true,
            )
        };
        assert_eq!(keys(&r), vec![expected(&[("A", 0)], &[("answer", &[0])])]);
        eprintln!(
            "QUAL finite-beside-divergent mode={mode} ticks={} answers={}",
            r.ticks,
            r.answers.len()
        );
    }
}
fn workload(case: &str, n: usize) -> (&'static str, String) {
    let atoms = |name: &str| {
        (0..n)
            .map(|i| format!("{name}(V{i})"))
            .collect::<Vec<_>>()
            .join(",")
    };
    match case {
        "common" => (
            "p(X)<=>q(X). q(X)<=>done(X).",
            format!("(true;true),{}", atoms("p")),
        ),
        "common-wide" => (
            "p(X)<=>q(X). q(X)<=>done(X).",
            format!("({}),{}", vec!["true"; n].join(";"), atoms("p")),
        ),
        "distinct" => (
            "p(X)<=>first(X). q(X)<=>second(X).",
            format!("({};{})", atoms("p"), atoms("q")),
        ),
        "failure" => (
            "p(X)<=>q(X). q(X)<=>done(X).",
            format!("(fail;({}))", atoms("p")),
        ),
        _ => unreachable!(),
    }
}
fn verify(case: &str, n: usize, p: &Prepared, r: &ResultData, old: bool) {
    let b = (0..n)
        .map(|i| (format!("V{i}"), i as u64))
        .collect::<BTreeMap<_, _>>();
    let names = match case {
        "distinct" => vec!["first", "second"],
        "common" => vec!["done", "done"],
        "common-wide" => vec!["done"; n],
        _ => vec!["done"],
    };
    let mut want = names
        .into_iter()
        .map(|name| {
            canonical(&Answer {
                bindings: b.clone(),
                facts: (0..n).map(|i| (name.into(), vec![i as u64])).collect(),
            })
        })
        .collect::<Vec<_>>();
    want.sort();
    assert_eq!(keys(r), want, "{case} {n}");
    if old {
        assert_eq!(r.splits, if case == "common-wide" { n - 1 } else { 1 });
        assert_eq!(r.failed, usize::from(case == "failure"));
        for trace in &r.traces {
            assert_eq!(
                trace.iter().filter(|&&r| r == p.rule_count).count(),
                1,
                "one admission per projection"
            );
            let mut c = BTreeMap::new();
            for &rule in trace {
                *c.entry(rule).or_insert(0usize) += 1;
            }
            let source = trace.len() - 1;
            if source == 0 {
                assert_eq!(case, "failure");
            } else if case == "distinct" {
                assert_eq!(source, n);
                assert!(
                    (c.get(&0) == Some(&n) && !c.contains_key(&1))
                        || (c.get(&1) == Some(&n) && !c.contains_key(&0))
                );
            } else {
                assert_eq!(c.get(&0), Some(&n));
                assert_eq!(c.get(&1), Some(&n));
            }
        }
    } else {
        assert_eq!(r.apps, (2 * n) as u64);
    }
}
fn main() {
    qualify();
    if std::env::args().any(|a| a == "--qualify") {
        return;
    }
    println!(
        "case,n,rep,runner,parse_us,prepare_us,translation_us,init_us,runtime_output_us,observe_us,engine_drop_us,output_drop_us,qualified_physical_source_apps,admission_apps,ticks,ast_drop_us,source_exhausted_us,delivery_tail_us,answers,facts,ports,query_bindings"
    );
    let cases = if std::env::args().any(|a| a == "--all") {
        vec!["common", "distinct", "failure", "common-wide"]
    } else if std::env::args().any(|a| a == "--wide") {
        vec!["common-wide"]
    } else {
        vec!["common", "distinct", "failure"]
    };
    for case in cases {
        let sizes = std::env::args()
            .find_map(|a| {
                a.strip_prefix("--n=")
                    .map(|s| vec![s.parse::<usize>().unwrap()])
            })
            .unwrap_or(vec![8, 32, 128]);
        for n in sizes {
            let (source, query) = workload(case, n);
            let p = prepare(source, &query);
            let reference = [
                old_run(&p, Policy::Active, false, true),
                old_run(&p, Policy::Global, false, true),
            ];
            for r in &reference {
                verify(case, n, &p, r, true);
            }
            let measured_trace = std::env::args().any(|a| a == "--trace");
            let reps = std::env::args()
                .find_map(|a| {
                    a.strip_prefix("--reps=")
                        .map(|s| s.parse::<usize>().unwrap())
                })
                .unwrap_or(11);
            for rep in 0..reps + 2 {
                for offset in 0..3 {
                    let mode = (rep + offset) % 3;
                    let mut r = if mode == 2 {
                        new_run(&p, false)
                    } else {
                        old_run(
                            &p,
                            if mode == 0 {
                                Policy::Active
                            } else {
                                Policy::Global
                            },
                            false,
                            measured_trace,
                        )
                    };
                    verify(case, n, &p, &r, mode < 2);
                    let physical = if mode == 2 {
                        r.apps
                    } else {
                        (reference[mode]
                            .traces
                            .iter()
                            .map(|t| t.len() - 1)
                            .sum::<usize>()) as u64
                    };
                    let answers = r.answers.len();
                    let facts: usize = r.answers.iter().map(|a| a.facts.len()).sum();
                    let ports: usize = r.answers.iter().flat_map(|a| &a.facts).map(|(_,p)|p.len()).sum();
                    let bindings: usize = r.answers.iter().map(|a|a.bindings.len()).sum();
                    let d = Instant::now();
                    drop(std::mem::take(&mut r.answers));
                    drop(std::mem::take(&mut r.traces));
                    r.output_drop = us(d.elapsed());
                    if rep >= 2 {
                        println!(
                            "{case},{n},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{},{:.3},{:.3},{:.3},{},{},{},{}",
                            rep - 2,
                            ["old-active", "old-global", "current"][mode],
                            p.parse,
                            if mode == 2 { p.newprep } else { p.oldprep },
                            if mode == 2 { 0.0 } else { p.convert },
                            r.init,
                            r.runtime,
                            if mode == 2 { f64::NAN } else { r.observe },
                            r.drop,
                            r.output_drop,
                            physical,
                            usize::from(mode < 2),
                            r.ticks,
                            p.astdrop,
                            r.source_exhausted.unwrap_or(f64::NAN),
                            r.source_exhausted.map_or(f64::NAN, |s| r.runtime-s),
                            answers, facts, ports, bindings
                        );
                    }
                }
            }
            let d = Instant::now();
            drop(p.new);
            let newdrop = us(d.elapsed());
            let d = Instant::now();
            drop(p.old);
            drop(p.query);
            let olddrop = us(d.elapsed());
            eprintln!("PREP_DROP {case} {n} current_us={newdrop:.3} older_us={olddrop:.3}");
        }
    }
}
