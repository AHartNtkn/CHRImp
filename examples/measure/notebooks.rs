//! Actual notebook workloads; only one scalar-delivered answer is retained.
use crate::allocation::{Phase, during};
use crate::{memory, ms, observation};
use chr::{
    engine::Engine,
    observe::Output,
    program::prepare,
    syntax::{Atom, Body, Program},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};

pub const CASES: &str = "notebook-lambda notebook-arithmetic-forward notebook-arithmetic-reverse notebook-arithmetic-decompose notebook-type-i notebook-type-k notebook-type-ki notebook-type-s notebook-type-t notebook-type-b notebook-type-c notebook-type-w notebook-behavior-i notebook-behavior-k notebook-behavior-ki notebook-behavior-s notebook-behavior-t notebook-behavior-m notebook-behavior-b notebook-behavior-c notebook-behavior-w";

#[derive(Default)]
struct Answer {
    variables: Vec<u64>,
    rows: Vec<(usize, Vec<u64>)>,
    occurrences: BTreeSet<u64>,
    ports: usize,
}
#[derive(Default)]
struct Reader {
    answer: Option<Answer>,
    row: Option<(usize, Vec<u64>)>,
    ids: BTreeSet<(u64, u64)>,
    peak_rows: usize,
    peak_ports: usize,
    scalars: usize,
}
impl Reader {
    fn push(&mut self, output: Output, e: &Engine) -> Result<Option<Answer>, String> {
        self.scalars += 1;
        match output {
            Output::Begin {
                completion,
                alternative,
            } if self.answer.is_none() => {
                if !self.ids.insert((completion, alternative)) {
                    return Err("duplicate answer event identity".into());
                }
                self.answer = Some(Answer::default());
            }
            Output::Variable { slot, variable } if self.row.is_none() => {
                let a = self.answer.as_mut().ok_or("variable outside answer")?;
                if slot != a.variables.len()
                    || slot >= e.program().query_variables().len()
                    || !a.rows.is_empty()
                {
                    return Err("invalid variable slot/order".into());
                }
                a.variables.push(variable);
            }
            Output::Fact {
                occurrence,
                relation,
            } if self.row.is_none() => {
                let a = self.answer.as_mut().ok_or("fact outside answer")?;
                if a.variables.len() != e.program().query_variables().len()
                    || relation >= e.program().signatures().len()
                    || !a.occurrences.insert(occurrence)
                {
                    return Err("invalid relation, occurrence identity, or missing bindings".into());
                }
                self.row = Some((relation, vec![]));
            }
            Output::Port { variable } => {
                let (r, ports) = self.row.as_mut().ok_or("port outside fact")?;
                if ports.len() >= e.program().signatures()[*r].arity {
                    return Err("too many ports".into());
                }
                ports.push(variable);
                let a = self.answer.as_mut().ok_or("port outside answer")?;
                a.ports += 1;
                self.peak_ports = self.peak_ports.max(a.ports);
            }
            Output::EndFact => {
                let row = self.row.take().ok_or("end outside fact")?;
                if row.1.len() != e.program().signatures()[row.0].arity {
                    return Err("wrong fact arity".into());
                }
                self.answer
                    .as_mut()
                    .ok_or("fact outside answer")?
                    .rows
                    .push(row);
                self.peak_rows = self.peak_rows.max(self.answer.as_ref().unwrap().rows.len());
            }
            Output::PendingBegin { event } => {
                return Err(format!("unresolved pending obligation event={event}"));
            }
            Output::End if self.row.is_none() => {
                let a = self.answer.take().ok_or("end outside answer")?;
                if a.variables.len() != e.program().query_variables().len() {
                    return Err("missing answer bindings".into());
                }
                self.peak_rows = self.peak_rows.max(a.rows.len());
                self.peak_ports = self.peak_ports.max(a.rows.iter().map(|r| r.1.len()).sum());
                return Ok(Some(a));
            }
            other => return Err(format!("invalid scalar order: {other:?}")),
        }
        Ok(None)
    }
}
fn root(e: &Engine, a: &Answer, name: &str) -> Result<u64, String> {
    e.program()
        .query_variables()
        .iter()
        .position(|v| v == name)
        .and_then(|i| a.variables.get(i).copied())
        .ok_or_else(|| format!("missing {name} root"))
}
fn unary(e: &Engine, a: &Answer, mut node: u64) -> Result<usize, String> {
    let mut seen = BTreeSet::new();
    let mut depth = 0;
    loop {
        if !seen.insert(node) {
            return Err("cyclic unary graph".into());
        }
        let mut zero = false;
        let mut children = BTreeSet::new();
        for (r, ports) in &a.rows {
            if ports.first() != Some(&node) {
                continue;
            }
            match e.program().signatures()[*r].name.as_str() {
                "zero" => zero = true,
                "succ" => {
                    children.insert(ports[1]);
                }
                _ => {}
            }
        }
        if children.len() > 1 || (zero && !children.is_empty()) {
            return Err("conflicting unary constructors".into());
        }
        if zero {
            return Ok(depth);
        }
        node = *children.first().ok_or("unresolved unary tail")?;
        depth += 1;
    }
}
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
fn apps(f: Term, xs: impl IntoIterator<Item = Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn term(e: &Engine, a: &Answer, node: u64, path: &mut Vec<u64>) -> Result<Term, String> {
    if path.contains(&node) {
        return Err("cyclic term graph".into());
    }
    // ponytail: bounded recursive oracle; use an explicit stack for larger answer terms.
    if path.len() >= 512 {
        return Err("validator term depth limit (512)".into());
    }
    path.push(node);
    let mut value = None;
    for (r, p) in &a.rows {
        if p.first() != Some(&node) {
            continue;
        }
        let t = match e.program().signatures()[*r].name.as_str() {
            "k" => K,
            "s" => S,
            "app" => app(term(e, a, p[1], path)?, term(e, a, p[2], path)?),
            "constant" => term(e, a, p[1], path)?,
            "symbol_x" => Symbol(0),
            "symbol_y" => Symbol(1),
            "symbol_z" => Symbol(2),
            _ => continue,
        };
        if value.as_ref().is_some_and(|old| old != &t) {
            return Err("conflicting term constructors".into());
        }
        value = Some(t);
    }
    path.pop();
    Ok(value.unwrap_or(Hole(node)))
}
// Leftmost outermost reduction discards unused arguments before visiting them.
fn reduce(t: Term, fuel: &mut usize, depth: usize) -> Result<Term, String> {
    if *fuel == 0 || depth >= 512 {
        return Err("normal-order oracle budget exceeded".into());
    }
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
        return reduce(apps(first, args), fuel, depth + 1);
    }
    if head == S && args.len() >= 3 {
        let f = args.remove(0);
        let g = args.remove(0);
        let x = args.remove(0);
        return reduce(
            apps(app(app(f, x.clone()), app(g, x)), args),
            fuel,
            depth + 1,
        );
    }
    let mut result = head;
    for x in args {
        result = app(result, reduce(x, fuel, depth + 1)?);
    }
    Ok(result)
}
fn expectation(target: &str) -> (Vec<Term>, Term) {
    let (x, y, z) = (Symbol(0), Symbol(1), Symbol(2));
    match target {
        "i" => (vec![x.clone()], x),
        "k" => (vec![x.clone(), y], x),
        "ki" => (vec![x, y.clone()], y),
        "s" => (
            vec![x.clone(), y.clone(), z.clone()],
            apps(x, [z.clone(), app(y, z)]),
        ),
        "t" => (vec![x.clone(), y.clone()], app(y, x)),
        "m" => (vec![x.clone()], app(x.clone(), x)),
        "b" => (vec![x.clone(), y.clone(), z.clone()], app(x, app(y, z))),
        "c" => (vec![x.clone(), y.clone(), z.clone()], apps(x, [z, y])),
        "w" => (vec![x.clone(), y.clone()], apps(x, [y.clone(), y])),
        _ => unreachable!(),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Var(usize),
    Atom(u8),
    Arrow(Box<Ty>, Box<Ty>),
}
fn arrow(a: Ty, b: Ty) -> Ty {
    Ty::Arrow(Box::new(a), Box::new(b))
}
#[derive(Default)]
struct Types {
    bindings: Vec<Option<Ty>>,
    holes: BTreeMap<u64, Ty>,
}
impl Types {
    fn fresh(&mut self) -> Ty {
        let i = self.bindings.len();
        self.bindings.push(None);
        Ty::Var(i)
    }
    fn resolve(&self, t: Ty) -> Ty {
        match t {
            Ty::Var(i) => self.bindings[i]
                .clone()
                .map_or(Ty::Var(i), |v| self.resolve(v)),
            Ty::Arrow(a, b) => arrow(self.resolve(*a), self.resolve(*b)),
            _ => t,
        }
    }
    fn unify(&mut self, a: Ty, b: Ty) -> Result<(), String> {
        let (a, b) = (self.resolve(a), self.resolve(b));
        if a == b {
            return Ok(());
        }
        fn occurs(i: usize, t: &Ty) -> bool {
            match t {
                Ty::Var(j) => i == *j,
                Ty::Arrow(a, b) => occurs(i, a) || occurs(i, b),
                _ => false,
            }
        }
        match (a, b) {
            (Ty::Var(i), t) | (t, Ty::Var(i)) if !occurs(i, &t) => {
                self.bindings[i] = Some(t);
                Ok(())
            }
            (Ty::Arrow(a, b), Ty::Arrow(c, d)) => {
                self.unify(*a, *c)?;
                self.unify(*b, *d)
            }
            _ => Err("independent type mismatch/occurs check".into()),
        }
    }
    fn infer(&mut self, t: &Term) -> Result<Ty, String> {
        Ok(match t {
            K => {
                let a = self.fresh();
                let b = self.fresh();
                arrow(a.clone(), arrow(b, a))
            }
            S => {
                let a = self.fresh();
                let b = self.fresh();
                let c = self.fresh();
                arrow(
                    arrow(a.clone(), arrow(b.clone(), c.clone())),
                    arrow(arrow(a.clone(), b), arrow(a, c)),
                )
            }
            App(f, x) => {
                let f = self.infer(f)?;
                let x = self.infer(x)?;
                let r = self.fresh();
                self.unify(f, arrow(x, r.clone()))?;
                r
            }
            Hole(id) => {
                if let Some(t) = self.holes.get(id) {
                    t.clone()
                } else {
                    let t = self.fresh();
                    self.holes.insert(*id, t.clone());
                    t
                }
            }
            Symbol(_) => return Err("symbol in synthesized SK program".into()),
        })
    }
}
fn target_type(target: &str) -> Ty {
    let (a, b, c) = (Ty::Atom(0), Ty::Atom(1), Ty::Atom(2));
    match target {
        "i" => arrow(a.clone(), a),
        "k" => arrow(a.clone(), arrow(b, a)),
        "ki" => arrow(a, arrow(b.clone(), b)),
        "s" => arrow(
            arrow(a.clone(), arrow(b.clone(), c.clone())),
            arrow(arrow(a.clone(), b), arrow(a, c)),
        ),
        "t" => arrow(a.clone(), arrow(arrow(a, b.clone()), b)),
        "b" => arrow(
            arrow(b.clone(), c.clone()),
            arrow(arrow(a.clone(), b), arrow(a, c)),
        ),
        "c" => arrow(
            arrow(a.clone(), arrow(b.clone(), c.clone())),
            arrow(b, arrow(a, c)),
        ),
        "w" => arrow(arrow(a.clone(), arrow(a.clone(), b.clone())), arrow(a, b)),
        _ => unreachable!(),
    }
}
fn sk_only(t: &Term) -> bool {
    match t {
        K | S | Hole(_) => true,
        App(f, x) => sk_only(f) && sk_only(x),
        Symbol(_) => false,
    }
}
fn validate_synthesis(e: &Engine, a: &Answer, kind: &str, target: &str) -> Result<(), String> {
    for (r, _) in &a.rows {
        let name = &e.program().signatures()[*r].name;
        if matches!(
            name.as_str(),
            "infer" | "apply_type" | "eval" | "fold" | "apply_k" | "apply_s" | "apply_s_two"
        ) {
            return Err(format!("unresolved {name} obligation"));
        }
    }
    let p = term(e, a, root(e, a, "Program")?, &mut vec![])?;
    if !sk_only(&p) {
        return Err("non-SK symbol in synthesized program".into());
    }
    let (args, expected) = expectation(target);
    if reduce(apps(p.clone(), args), &mut 100_000, 0)? != expected {
        return Err("independent SK behavior mismatch (including forced holes)".into());
    }
    if kind == "type" {
        let mut types = Types::default();
        let inferred = types.infer(&p)?;
        types.unify(inferred, target_type(target))?;
    } else if term(e, a, root(e, a, "Output")?, &mut vec![])? != expected {
        return Err("output target mismatch".into());
    }
    Ok(())
}
fn atom(name: &str, args: Vec<String>) -> Body {
    Body::Atom {
        atom: Atom {
            relation: name.into(),
            args,
        },
    }
}
fn arithmetic_query(mode: &str, n: usize) -> Result<Body, String> {
    let mut rows = vec![];
    let mut nat = |root: &str, count: usize| {
        for i in 0..count {
            rows.push(atom(
                "succ",
                vec![
                    if i == 0 {
                        root.into()
                    } else {
                        format!("{root}{i}")
                    },
                    format!("{root}{}", i + 1),
                ],
            ));
        }
        rows.push(atom(
            "zero",
            vec![if count == 0 {
                root.into()
            } else {
                format!("{root}{count}")
            }],
        ));
    };
    match mode {
        "forward" => {
            nat("X", n);
            nat("Y", n);
        }
        "reverse" => {
            nat("Y", n);
            nat("Z", n.checked_mul(2).ok_or("magnitude overflow")?);
        }
        "decompose" => nat("Z", n),
        _ => return Err("unknown arithmetic mode".into()),
    }
    rows.push(atom("add", vec!["X".into(), "Y".into(), "Z".into()]));
    Ok(Body::And { items: rows })
}
fn lambda_query(n: usize) -> Body {
    let mut rows = vec![atom("var", vec!["Original".into()])];
    let mut input = "Original".to_string();
    for i in 0..n {
        let binder = format!("Binder{i}");
        let lambda = format!("Lambda{i}");
        let next = format!("Input{i}");
        rows.push(atom("var", vec![binder.clone()]));
        rows.push(atom("lam", vec![lambda.clone(), binder.clone(), binder]));
        rows.push(atom("app", vec![next.clone(), lambda, input]));
        input = next;
    }
    rows.push(atom("lamEq", vec![input, "Output".into()]));
    Body::And { items: rows }
}
fn validate_lambda(e: &Engine, a: &Answer, n: usize) -> Result<(), String> {
    let original = root(e, a, "Original")?;
    if root(e, a, "Output")? != original {
        return Err("lambda Output lost original wire identity".into());
    }
    let actual: BTreeSet<_> = a
        .rows
        .iter()
        .map(|(r, p)| (e.program().signatures()[*r].name.as_str(), p.clone()))
        .collect();
    if actual.len() != a.rows.len() {
        return Err("duplicate lambda residual occurrence".into());
    }
    let mut expected = BTreeSet::from([("var", vec![original])]);
    let mut identities = BTreeSet::from([original]);
    let mut non_apps = BTreeSet::from([original]);
    let mut lambdas = vec![];
    let mut current = vec![];
    let mut input = original;
    for i in 0..n {
        let binder = root(e, a, &format!("Binder{i}"))?;
        let lambda = root(e, a, &format!("Lambda{i}"))?;
        let next = root(e, a, &format!("Input{i}"))?;
        if !identities.insert(binder) || !identities.insert(lambda) || !identities.insert(next) {
            return Err("lambda input wires unexpectedly aliased".into());
        }
        non_apps.extend([binder, lambda]);
        expected.extend([
            ("var", vec![binder]),
            ("lam", vec![lambda, binder, binder]),
            ("app", vec![next, lambda, input]),
        ]);
        lambdas.push(lambda);
        current.push(next);
        input = next;
    }
    let indexes: BTreeMap<_, _> = lambdas.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut counts = vec![0; n];
    let mut apps = BTreeMap::new();
    let mut app_roots = BTreeSet::new();
    for (_, ports) in actual.iter().filter(|(name, _)| *name == "app") {
        let &[id, f, x] = ports.as_slice() else {
            return Err("invalid application arity".into());
        };
        let &i = indexes
            .get(&f)
            .ok_or("intermediate application has unknown function")?;
        if non_apps.contains(&id) || !app_roots.insert(id) || apps.insert((f, x), id).is_some() {
            return Err("conflicting or repeated lambda application structure".into());
        }
        counts[i] += 1;
    }
    // In this workload only identity beta and argument-context steps can succeed.
    // Each inner reduction before identity i creates exactly one new app of i.
    // Thus count[i]-1 is i's insertion position in the order of smaller identities;
    // this reconstructs a permutation without searching its n! possible histories.
    let mut order = vec![];
    for (i, count) in counts.into_iter().enumerate() {
        if count == 0 || count > i + 1 {
            return Err("impossible number of intermediate lambda applications".into());
        }
        order.insert(count - 1, i);
    }
    let mut remaining: Vec<_> = (0..n).collect();
    for i in order {
        let position = remaining.iter().position(|&j| i == j).unwrap();
        let mut argument = if position == 0 {
            original
        } else {
            current[position - 1]
        };
        remaining.remove(position);
        current.remove(position);
        for j in position..remaining.len() {
            let function = lambdas[remaining[j]];
            let &next = apps
                .get(&(function, argument))
                .ok_or("missing argument-context application")?;
            if identities.contains(&next)
                || !expected.insert(("app", vec![next, function, argument]))
            {
                return Err("argument-context result must have a fresh wire".into());
            }
            identities.insert(next);
            current[j] = next;
            argument = next;
        }
    }
    // Structural propagation retains exactly the strict descendant closure, once
    // per pair. No norm/neq/step/lamEq obligation survives these identity histories.
    let children: BTreeMap<_, _> = expected
        .iter()
        .filter(|(name, _)| matches!(*name, "lam" | "app"))
        .map(|(_, p)| (p[0], p[1..].to_vec()))
        .collect();
    for (&parent, direct) in &children {
        let mut pending = direct.clone();
        let mut seen = BTreeSet::new();
        while let Some(child) = pending.pop() {
            if seen.insert(child) {
                expected.insert(("below", vec![parent, child]));
                pending.extend(children.get(&child).into_iter().flatten().copied());
            }
        }
    }
    if actual != expected {
        return Err(format!(
            "lambda residual mismatch: missing={:?}, unexpected={:?}",
            expected.difference(&actual).next(),
            actual.difference(&expected).next()
        ));
    }
    Ok(())
}

pub fn run(case: &str, n: usize, max_ticks: u64, timeout: Duration) -> Result<bool, String> {
    let suffix = case
        .strip_prefix("notebook-")
        .ok_or("not a notebook case")?;
    let (kind, target) = if suffix == "lambda" {
        ("lambda", "identity")
    } else {
        suffix.split_once('-').ok_or("missing notebook target")?
    };
    let source = match kind {
        "lambda" => include_str!("../lambda.chrnb"),
        "arithmetic" => include_str!("../arithmetic.chrnb"),
        "type" => include_str!("../type-synthesis.chrnb"),
        "behavior" => include_str!("../behavior-synthesis.chrnb"),
        _ => return Err("unknown notebook kind".into()),
    };
    if n == 0 || max_ticks == 0 || timeout.is_zero() {
        return Err("size and budgets must be positive".into());
    }
    let start = Instant::now();
    let document: serde_json::Value = serde_json::from_str(source).map_err(|e| e.to_string())?;
    let program: Program =
        serde_json::from_value(document["program"].clone()).map_err(|e| e.to_string())?;
    // Lambda measures one complete answer of the nested workload, not exhaustion
    // or the factorial number of successful reduction histories.
    let finite = kind == "arithmetic";
    let expected = if target == "decompose" {
        n.checked_add(1).ok_or("magnitude overflow")?
    } else if finite || kind == "lambda" {
        1
    } else {
        n
    };
    let query: Body = if kind == "lambda" {
        lambda_query(n)
    } else if kind == "arithmetic" {
        arithmetic_query(target, n)?
    } else {
        let index = match target {
            "i" => 0,
            "k" => 1,
            "ki" => 2,
            "s" => 3,
            "t" => 7,
            "m" if kind == "behavior" => 8,
            "b" => 4,
            "c" => 5,
            "w" => 6,
            _ => return Err("unknown synthesis target".into()),
        };
        serde_json::from_value(document["queries"][index]["body"].clone())
            .map_err(|e| e.to_string())?
    };
    let parse_time = start.elapsed();
    let start = Instant::now();
    let code = during(Phase::Setup, || prepare(&program, &query).map(Arc::new))
        .map_err(|e| format!("prepare: {e:?}"))?;
    let prepare_time = start.elapsed();
    let start = Instant::now();
    let mut e = during(Phase::Setup, || Engine::new(code));
    let init_time = start.elapsed();
    let detailed = observation::detailed();
    let mut reader = Reader::default();
    let mut triples = BTreeSet::new();
    let mut ticks = 0;
    let mut answers = 0;
    let mut collection_ticks = 0;
    let mut validator = Duration::ZERO;
    let mut first_event = None;
    let mut first_answer = None;
    let mut first_answer_value = None;
    let mut peak = memory(&e);
    let mut error = None;
    let start = Instant::now();
    while ticks < max_ticks && !e.delivery_done() && (finite || answers < expected) {
        if ticks % 2048 == 0 && start.elapsed() >= timeout {
            break;
        }
        let collecting = detailed && e.collecting();
        during(Phase::Engine, || e.advance(1));
        ticks += 1;
        collection_ticks += u64::from(detailed && (collecting || e.collecting()));
        while let Some(output) = during(Phase::Delivery, || e.take_output()) {
            let arrival = if first_event.is_none()
                || (matches!(output, Output::End) && first_answer.is_none())
            {
                start.elapsed()
            } else {
                Duration::ZERO
            };
            first_event.get_or_insert((ticks, arrival));
            let v = observation::start(detailed);
            let result = during(Phase::Validator, || {
                reader.push(output, &e).and_then(|a| {
                    if let Some(a) = a {
                        first_answer.get_or_insert((ticks, arrival));
                        if kind == "lambda" {
                            validate_lambda(&e, &a, n)?;
                            first_answer_value.get_or_insert_with(|| {
                                format!("Output=Original({})", root(&e, &a, "Original").unwrap())
                            });
                        } else if kind == "arithmetic" {
                            for (r, _) in &a.rows {
                                if !matches!(
                                    e.program().signatures()[*r].name.as_str(),
                                    "zero" | "succ" | "predecessor"
                                ) {
                                    return Err("unresolved arithmetic obligation".into());
                                }
                            }
                            let xyz = [
                                unary(&e, &a, root(&e, &a, "X")?)?,
                                unary(&e, &a, root(&e, &a, "Y")?)?,
                                unary(&e, &a, root(&e, &a, "Z")?)?,
                            ];
                            let valid = if target == "decompose" {
                                xyz[2] == n && xyz[0].checked_add(xyz[1]) == Some(n)
                            } else {
                                xyz == [n, n, n.checked_mul(2).ok_or("magnitude overflow")?]
                            };
                            first_answer_value.get_or_insert_with(|| format!("{xyz:?}"));
                            if !valid || !triples.insert(xyz) {
                                return Err(format!("incorrect triple/multiplicity: {xyz:?}"));
                            }
                        } else {
                            validate_synthesis(&e, &a, kind, target)?;
                            if first_answer_value.is_none() {
                                first_answer_value = Some(format!(
                                    "{:?}",
                                    term(&e, &a, root(&e, &a, "Program")?, &mut vec![])?
                                ));
                            }
                        }
                        answers += 1;
                    }
                    Ok(())
                })
            });
            validator += observation::elapsed(v);
            if let Err(problem) = result {
                error = Some(problem);
                break;
            }
        }
        if ticks % 2048 == 0 {
            for (p, m) in peak.iter_mut().zip(memory(&e)) {
                *p = (*p).max(m);
            }
        }
        if error.is_some() {
            break;
        }
    }
    let elapsed = start.elapsed();
    let before = memory(&e);
    for (p, m) in peak.iter_mut().zip(before) {
        *p = (*p).max(m);
    }
    let exhausted = e.exhausted();
    let delivery_done = e.delivery_done();
    let applications = e.applications();
    let collections = e.collections();
    if finite
        && delivery_done
        && error.is_none()
        && (answers != expected || reader.answer.is_some())
    {
        error = Some(format!("expected {expected} answers, received {answers}"));
    }
    let achieved =
        error.is_none() && answers == expected && (!finite || delivery_done) && elapsed < timeout;
    // Cleanup has its own identical budget, so source limits do not skip reclamation.
    crate::report_diagnostics("source", &e);
    during(Phase::Cleanup, || e.cancel());
    let cleanup_start = Instant::now();
    let mut cleanup_ticks = 0;
    while !e.cancel_done() && cleanup_ticks < max_ticks {
        if cleanup_ticks % 2048 == 0 && cleanup_start.elapsed() >= timeout {
            break;
        }
        during(Phase::Cleanup, || e.advance(1));
        cleanup_ticks += 1;
    }
    let cleanup_time = cleanup_start.elapsed();
    let cleanup_in_time = cleanup_time < timeout;
    crate::report_diagnostics("after_cancel", &e);
    let cleanup_done = e.cancel_done();
    let after = memory(&e);
    if e.applications() != applications {
        error = Some("source advanced during cancellation".into());
    }
    if cleanup_done
        && (e.pending_tasks() != 0
            || !matches!(
                e.memory(),
                chr::engine::Memory {
                    graph_nodes: 0,
                    release_batches: 0,
                    occurrences: 0,
                    conditions: 0,
                    history_nodes: 0,
                    history_records: 0,
                    pending_nodes: 0,
                    obligation_descriptors: 0,
                    choices: 0,
                    coordinate_records: 0 | 1,
                    snapshots: 0,
                    inspections: 0,
                    restriction_nodes: 0,
                }
            ))
    {
        error = Some(format!("cleanup retained unowned state: {after:?}"));
    }
    let reclaimed: Vec<_> = before
        .iter()
        .zip(after)
        .map(|(b, a)| b.saturating_sub(a))
        .collect();
    let status = if error.is_some() {
        "INVALID"
    } else if !achieved || !cleanup_done || !cleanup_in_time {
        "INCOMPLETE"
    } else if finite {
        "COMPLETE"
    } else {
        "PREFIX_COMPLETE"
    };
    println!(
        "case={case} size={n} status={status} goal={} answers={answers}/{expected} search_exhausted={exhausted} delivery_done={delivery_done}",
        if finite {
            "complete"
        } else if kind == "lambda" {
            "first-answer-prefix"
        } else {
            "answer-prefix"
        }
    );
    println!(
        "parse_ms={:.3} prepare_ms={:.3} engine_init_ms={:.3} source_delivery_ms={:.3} source_delivery_without_validator_ms={} validator_ms={}",
        ms(parse_time),
        ms(prepare_time),
        ms(init_time),
        ms(elapsed),
        observation::milliseconds(detailed, elapsed.saturating_sub(validator)),
        observation::milliseconds(detailed, validator)
    );
    println!(
        "first_event_ticks={:?} first_event_ms={:?} first_answer_ticks={:?} first_answer_ms={:?}",
        first_event.map(|v| v.0),
        first_event.map(|v| ms(v.1)),
        first_answer.map(|v| v.0),
        first_answer.map(|v| ms(v.1))
    );
    println!("first_complete_answer={first_answer_value:?}");
    println!(
        "advance1_ticks={ticks} applications={applications} collection_ticks={} collections={collections} scalars={} max_ticks={max_ticks} timeout_ms={:.3}",
        observation::count(detailed, collection_ticks),
        reader.scalars,
        ms(timeout)
    );
    println!(
        "sampled_peak={peak:?} before_cleanup={before:?} after_cleanup={after:?} reclaimed={reclaimed:?} memory_order=graph,occurrences,conditions,history_nodes,history_records,pending_nodes,descriptors,choices,coordinates,snapshots,inspections,tasks,release_batches,restriction_nodes (counts; sampled every 2048 ticks and at stop)"
    );
    println!(
        "cleanup_ticks={cleanup_ticks} cleanup_in_time={cleanup_in_time} cleanup_ms={:.3} cleanup_status={} validator_peak_rows={} validator_peak_ports={} validator_answer_ids={} validator_triples={} (one answer retained; identities and arithmetic multiplicity metadata retained)",
        ms(cleanup_time),
        if cleanup_done && cleanup_in_time {
            "COMPLETE"
        } else {
            "INCOMPLETE"
        },
        reader.peak_rows,
        reader.peak_ports,
        reader.ids.len(),
        triples.len()
    );
    if !achieved && error.is_none() {
        println!(
            "limit_reached={}",
            if elapsed >= timeout {
                "timeout"
            } else if ticks >= max_ticks {
                "max_ticks"
            } else {
                "search-ended-before-prefix"
            }
        );
    }
    crate::report::emit(
        "result",
        serde_json::json!({
            "case": case, "size": n, "status": status, "goal": if finite { "complete" } else { "answer_prefix" },
            "goal_count": expected, "source_goal_reached": answers == expected && (!finite || delivery_done),
            "timed_out": elapsed >= timeout,
            "censor_reason": if elapsed >= timeout { Some("source_timeout") } else if !cleanup_in_time { Some("cleanup_timeout") } else if !cleanup_done { Some("cleanup_ticks") } else if !achieved { Some(if ticks >= max_ticks { "source_ticks" } else { "search_ended_before_prefix" }) } else { None }, "cleanup_done": cleanup_done,
            "cleanup_in_time": cleanup_in_time, "error": error, "search_exhausted": exhausted,
            "delivery_done": delivery_done, "max_ticks": max_ticks, "timeout_ms": ms(timeout),
            "times_ms": {"parse": ms(parse_time), "prepare": ms(prepare_time), "engine_init": ms(init_time),
                "source_delivery": ms(elapsed), "validator": detailed.then(|| ms(validator)),
                "source_delivery_without_validator": detailed.then(|| ms(elapsed.saturating_sub(validator))), "cleanup": ms(cleanup_time)},
            "first_event": first_event.map(|(tick,t)| serde_json::json!({"tick":tick,"ms":ms(t)})),
            "first_answer": first_answer.map(|(tick,t)| serde_json::json!({"tick":tick,"ms":ms(t)})),
            "first_complete_answer": first_answer_value,
            "work": {"advance1_ticks":ticks,"applications":applications,"collections":collections,
                "collection_status_ticks":detailed.then_some(collection_ticks),"answers":answers,
                "scalars":reader.scalars,"cleanup_ticks":cleanup_ticks},
            "memory_counts": {"sampled_peak":crate::report::memory(peak),"before_cleanup":crate::report::memory(before),"after_cleanup":crate::report::memory(after)}
        }),
    );
    if let Some(error) = error {
        return Err(error);
    }
    Ok(achieved && cleanup_done && cleanup_in_time)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lambda_answer(inner_first: bool) -> (Engine, Answer) {
        let doc: serde_json::Value = serde_json::from_str(include_str!("../lambda.chrnb")).unwrap();
        let program: Program = serde_json::from_value(doc["program"].clone()).unwrap();
        let e = Engine::new(Arc::new(prepare(&program, &lambda_query(2)).unwrap()));
        let variables = e
            .program()
            .query_variables()
            .iter()
            .map(|name| match name.as_str() {
                "Original" | "Output" => 1,
                "Binder0" => 2,
                "Lambda0" => 3,
                "Input0" => 4,
                "Binder1" => 5,
                "Lambda1" => 6,
                "Input1" => 7,
                _ => panic!("unexpected query variable"),
            })
            .collect();
        let mut rows = vec![
            ("var", vec![1]),
            ("var", vec![2]),
            ("var", vec![5]),
            ("lam", vec![3, 2, 2]),
            ("lam", vec![6, 5, 5]),
            ("app", vec![4, 3, 1]),
            ("app", vec![7, 6, 4]),
        ];
        for (parent, children) in [
            (3, vec![2]),
            (6, vec![5]),
            (4, vec![3, 1, 2]),
            (7, vec![6, 4, 5, 3, 1, 2]),
        ] {
            rows.extend(
                children
                    .into_iter()
                    .map(|child| ("below", vec![parent, child])),
            );
        }
        if inner_first {
            rows.push(("app", vec![99, 6, 1]));
            rows.extend([6, 1, 5].map(|child| ("below", vec![99, child])));
        }
        let rows = rows
            .into_iter()
            .map(|(name, ports)| {
                (
                    e.program()
                        .signatures()
                        .iter()
                        .position(|s| s.name == name)
                        .unwrap(),
                    ports,
                )
            })
            .collect();
        (
            e,
            Answer {
                variables,
                rows,
                ..Answer::default()
            },
        )
    }

    #[test]
    fn lambda_oracle_checks_exact_residuals_and_both_reduction_orders() {
        for inner_first in [false, true] {
            let (e, mut a) = lambda_answer(inner_first);
            validate_lambda(&e, &a, 2).unwrap();
            for name in ["var", "lam", "app", "below"] {
                let index = a
                    .rows
                    .iter()
                    .position(|(r, _)| e.program().signatures()[*r].name == name)
                    .unwrap();
                let row = a.rows.remove(index);
                assert!(validate_lambda(&e, &a, 2).is_err(), "missing {name}");
                a.rows.insert(index, row.clone());
                a.rows.push(row);
                assert!(validate_lambda(&e, &a, 2).is_err(), "duplicate {name}");
                a.rows.pop();
            }
            if inner_first {
                let index = a
                    .rows
                    .iter()
                    .position(|(r, p)| e.program().signatures()[*r].name == "app" && p[0] == 99)
                    .unwrap();
                // Keep a consistent closure and application count, but substitute
                // a binder for the argument: no permitted reduction produces it.
                a.rows[index].1[2] = 2;
                let edge = a
                    .rows
                    .iter_mut()
                    .find(|(r, p)| e.program().signatures()[*r].name == "below" && p == &[99, 1])
                    .unwrap();
                edge.1[1] = 2;
                assert!(
                    validate_lambda(&e, &a, 2).is_err(),
                    "impossible intermediate application"
                );
            }
        }
    }

    #[test]
    fn independent_oracles_reject_wrong_terms_and_preserve_unused_holes() {
        let i = apps(S, [K, K]);
        let hidden_symbol = apps(K, [i.clone(), Symbol(2)]);
        let ignored_hole = apps(K, [i.clone(), Hole(99)]);
        assert!(!sk_only(&hidden_symbol));
        assert!(sk_only(&ignored_hole));
        assert_eq!(
            reduce(app(hidden_symbol, Symbol(0)), &mut 1000, 0).unwrap(),
            Symbol(0)
        );
        assert_eq!(
            reduce(app(ignored_hole, Symbol(0)), &mut 1000, 0).unwrap(),
            Symbol(0)
        );
        assert_eq!(reduce(app(i, Symbol(0)), &mut 1000, 0).unwrap(), Symbol(0));
        assert_eq!(
            reduce(apps(K, [Symbol(0), Hole(99)]), &mut 1000, 0).unwrap(),
            Symbol(0)
        );
        assert_ne!(
            reduce(app(Hole(99), Symbol(0)), &mut 1000, 0).unwrap(),
            Symbol(0)
        );
        let mut ts = Types::default();
        let t = ts.infer(&K).unwrap();
        assert!(ts.unify(t, target_type("i")).is_err());
        let mut ts = Types::default();
        let v = ts.fresh();
        assert!(ts.unify(v.clone(), arrow(v, Ty::Atom(0))).is_err());
    }
    #[test]
    fn additional_synthesis_targets_have_independent_behavior_and_type_oracles() {
        let identity = apps(S, [K, K]);
        let composition = apps(S, [app(K, S), K]);
        let swap = apps(S, [apps(composition.clone(), [composition, S]), app(K, K)]);
        for (target, witness) in [
            ("ki", app(K, identity.clone())),
            ("s", S),
            ("t", app(swap, identity.clone())),
            ("m", apps(S, [identity.clone(), identity])),
        ] {
            let (args, expected) = expectation(target);
            assert_eq!(
                reduce(apps(witness.clone(), args.clone()), &mut 100_000, 0).unwrap(),
                expected
            );
            assert_ne!(reduce(apps(K, args), &mut 100_000, 0).unwrap(), expected);
            if target != "m" {
                let mut types = Types::default();
                let inferred = types.infer(&witness).unwrap();
                types.unify(inferred, target_type(target)).unwrap();
            }
        }
    }

    #[test]
    fn lambda_measures_a_validated_first_answer_prefix() {
        assert!(run("notebook-lambda", 1, 20_000_000, Duration::from_secs(30)).unwrap());
    }
    #[test]
    fn finite_notebook_arithmetic() {
        for mode in ["forward", "reverse", "decompose"] {
            assert!(
                run(
                    &format!("notebook-arithmetic-{mode}"),
                    2,
                    2_000_000,
                    Duration::from_secs(10)
                )
                .unwrap()
            );
        }
    }
}
