//! Full-engine measurements; run with `cargo run --release --example measure -- CASE SIZE`.
use chr::{
    engine::Engine,
    observe::Output,
    program::prepare,
    syntax::{parse_program, parse_query},
};
use std::{
    collections::{BTreeMap, HashSet},
    env,
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

type Counts = BTreeMap<String, usize>;
const CASES: &str = "rewrite sparse dense alias degree common common-wide failure distinct cyclic";
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
        "common-wide" => (
            rewrite.into(),
            format!("({}),{p}", vec!["true"; n].join(";")),
            2 * n,
            vec![counts(&[("done", n)]); n],
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
        "rewrite" | "common" | "common-wide" | "failure" | "distinct" => {
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
fn consume_tuple(tuples: &mut Tuples, relation: &str, ports: &[u64]) -> Result<(), String> {
    if tuples
        .get_mut(relation)
        .is_some_and(|expected| expected.remove(ports))
    {
        Ok(())
    } else {
        Err(format!("unexpected or duplicate tuple {relation}{ports:?}"))
    }
}

// Per-answer query bindings, remaining expected tuples, and one in-flight fact.
#[derive(Default)]
struct Reader {
    open: bool,
    slots: usize,
    ports_left: Option<usize>,
    counts: Counts,
    answers: usize,
    facts: usize,
    ports: usize,
    scalars: usize,
    bindings: BTreeMap<String, u64>,
    remaining: Option<Tuples>,
    relation: String,
    tuple: Vec<u64>,
    peak_bindings: usize,
    peak_tuple_capacity: usize,
}
impl Reader {
    fn push(
        &mut self,
        output: Output,
        e: &Engine,
        expected: &mut Vec<Counts>,
        case: &str,
        n: usize,
    ) -> Result<(), String> {
        self.scalars += 1;
        match output {
            Output::Begin { .. } if !self.open => {
                self.open = true;
                self.slots = 0;
                self.counts.clear();
                self.bindings.clear();
                self.remaining = None;
            }
            Output::Variable { slot, variable }
                if self.open
                    && self.ports_left.is_none()
                    && self.counts.is_empty()
                    && slot == self.slots
                    && slot < e.program().query_variables.len() =>
            {
                self.bindings
                    .insert(e.program().query_variables[slot].clone(), variable);
                self.peak_bindings = self.peak_bindings.max(self.bindings.len());
                self.slots += 1;
            }
            Output::Fact { relation, .. }
                if self.open
                    && self.ports_left.is_none()
                    && self.slots == e.program().query_variables.len() =>
            {
                if self.remaining.is_none() {
                    let tuples = expected_tuples(case, n, &self.bindings)?;
                    self.peak_tuple_capacity = self
                        .peak_tuple_capacity
                        .max(tuples.values().map(HashSet::capacity).sum());
                    self.remaining = Some(tuples);
                }
                let signature = e
                    .program()
                    .signatures
                    .get(relation)
                    .ok_or("invalid relation ID")?;
                *self.counts.entry(signature.name.clone()).or_default() += 1;
                self.relation.clone_from(&signature.name);
                self.tuple.clear();
                self.ports_left = Some(signature.arity);
                self.facts += 1;
            }
            Output::Port { variable } if self.ports_left.is_some_and(|left| left > 0) => {
                *self.ports_left.as_mut().unwrap() -= 1;
                self.tuple.push(variable);
                self.ports += 1;
            }
            Output::EndFact if self.ports_left == Some(0) => {
                consume_tuple(
                    self.remaining.as_mut().ok_or("missing tuple oracle")?,
                    &self.relation,
                    &self.tuple,
                )?;
                self.ports_left = None;
            }
            Output::End
                if self.open
                    && self.ports_left.is_none()
                    && self.slots == e.program().query_variables.len() =>
            {
                let index = expected
                    .iter()
                    .position(|want| want == &self.counts)
                    .ok_or_else(|| {
                        format!(
                            "unexpected answer relations {:?}; remaining {expected:?}",
                            self.counts
                        )
                    })?;
                for relation in self.counts.keys() {
                    if !self.remaining.as_ref().unwrap()[relation].is_empty() {
                        return Err(format!("missing expected {relation} tuples"));
                    }
                }
                expected.swap_remove(index);
                self.answers += 1;
                self.open = false;
            }
            other => return Err(format!("invalid or unexpected scalar {other:?}")),
        }
        Ok(())
    }
}
fn memory(e: &Engine) -> [usize; 12] {
    let m = e.memory();
    [
        m.graph_nodes,
        m.occurrences,
        m.conditions,
        m.history_nodes,
        m.history_records,
        m.pending_nodes,
        m.obligation_descriptors,
        m.choices,
        m.coordinate_records,
        m.snapshots,
        m.inspections,
        e.pending_tasks(),
    ]
}
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
fn run(case: &str, n: usize, max_ticks: u64, timeout: Duration) -> Result<bool, String> {
    // Generation is deliberately outside every reported timing interval.
    let (program, query, apps, mut expected) = workload(case, n)?;
    let expected_answers = expected.len();
    let expected_facts: usize = expected.iter().flat_map(|row| row.values()).sum();
    let start = Instant::now();
    let parsed_program = parse_program(&program).map_err(|e| format!("parse program: {e:?}"))?;
    let parse_program_time = start.elapsed();
    let start = Instant::now();
    let parsed_query = parse_query(&query).map_err(|e| format!("parse query: {e:?}"))?;
    let parse_query_time = start.elapsed();
    let start = Instant::now();
    let code =
        Arc::new(prepare(&parsed_program, &parsed_query).map_err(|e| format!("prepare: {e:?}"))?);
    let prepare_time = start.elapsed();
    let expected_ports: usize = expected
        .iter()
        .flat_map(|row| row.iter())
        .map(|(name, count)| {
            let signature = code.signatures.iter().find(|s| &s.name == name).unwrap();
            count * signature.arity
        })
        .sum();
    let start = Instant::now();
    let mut e = Engine::new(code.clone());
    let init_time = start.elapsed();
    let mut reader = Reader::default();
    let mut ticks = 0;
    let mut peak = memory(&e);
    let mut max_batch = Duration::ZERO;
    let mut source_done = None;
    let mut batch_start = Instant::now();
    let mut error = None;
    let mut timed_out = false;
    while !e.delivery_done() && ticks < max_ticks {
        e.advance(1);
        ticks += 1;
        while let Some(output) = e.take_output() {
            if let Err(problem) = reader.push(output, &e, &mut expected, case, n) {
                error = Some(problem);
                break;
            }
        }
        if e.exhausted() && source_done.is_none() {
            source_done = Some(start.elapsed());
        }
        if ticks % 2048 == 0 || e.delivery_done() || ticks == max_ticks || error.is_some() {
            max_batch = max_batch.max(batch_start.elapsed());
            for (peak, now) in peak.iter_mut().zip(memory(&e)) {
                *peak = (*peak).max(now);
            }
            batch_start = Instant::now();
            timed_out = start.elapsed() >= timeout;
            if timed_out || error.is_some() {
                break;
            }
        }
    }
    let elapsed = start.elapsed();
    let complete = e.delivery_done() && error.is_none() && !timed_out;
    let actual_apps = e.applications();
    let collections = e.collections();
    if complete
        && (reader.open
            || !expected.is_empty()
            || reader.answers != expected_answers
            || reader.facts != expected_facts
            || reader.ports != expected_ports
            || actual_apps != apps as u64)
    {
        error = Some(format!(
            "semantic mismatch: expected answers={expected_answers}, applications={apps}, facts={expected_facts}, ports={expected_ports}"
        ));
    }
    let disposal = Instant::now();
    drop(e);
    let engine_drop = disposal.elapsed();
    let disposal = Instant::now();
    drop(code);
    let prepared_drop = disposal.elapsed();
    let status = if error.is_some() {
        "INVALID"
    } else if complete {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    };
    println!(
        "case={case} size={n} status={status} max_ticks={max_ticks} timeout_ms={:.3}",
        ms(timeout)
    );
    if !complete && error.is_none() {
        println!(
            "limit_reached={}",
            if timed_out { "timeout" } else { "max_ticks" }
        );
    }
    println!(
        "parse_program_ms={:.3} parse_query_ms={:.3} prepare_ms={:.3} engine_init_ms={:.3}",
        ms(parse_program_time),
        ms(parse_query_time),
        ms(prepare_time),
        ms(init_time)
    );
    println!(
        "source_delivery_ms={:.3} source_exhausted_ms={:?} delivery_tail_ms={:?} engine_drop_ms={:.3} prepared_drop_ms={:.3}",
        ms(elapsed),
        source_done.map(ms),
        source_done.map(|t| ms(elapsed.saturating_sub(t))),
        ms(engine_drop),
        ms(prepared_drop)
    );
    println!(
        "advance1_ticks={ticks} max_2048_tick_batch_ms={:.3} collections={collections} applications={actual_apps}/{apps} answers={}/{expected_answers} facts={}/{expected_facts} ports={}/{expected_ports} scalars={}",
        ms(max_batch),
        reader.answers,
        reader.facts,
        reader.ports,
        reader.scalars
    );
    println!(
        "sampled_peak [graph,occurrences,conditions,history_nodes,history_records,pending_nodes,descriptors,choices,coordinates,snapshots,inspections,tasks]={peak:?}"
    );
    println!(
        "validator_peak_bindings={} validator_peak_tuple_capacity={} (separate from engine memory; counts, not bytes)",
        reader.peak_bindings, reader.peak_tuple_capacity
    );
    if let Some(error) = error {
        return Err(error);
    }
    Ok(complete)
}
fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args == ["--help"] || args == ["--list"] {
        println!(
            "Usage: cargo run --release --example measure -- CASE SIZE [MAX_TICKS] [TIMEOUT_SECONDS]\nCases: {CASES}\nDefaults: MAX_TICKS=50000000 TIMEOUT_SECONDS=30\nSize must be positive. Dense join produces n² hits. History is off; no snapshots.\nSource/delivery overlap; source_exhausted_ms marks exhaustion, delivery_tail_ms the remainder.\nTimings include exact ordered-tuple validation. Validator retains input-sized bindings and one answer's expected tuple hash sets (dense O(n²)); reported separately from engine memory. Dense p/q use distinct variable sets to expose reversed ports. Memory is sampled every 2048 ticks and at exit, not an exact peak.\nTimeout is checked at batch boundaries; one Engine tick or disposal cannot be preempted."
        );
        return ExitCode::SUCCESS;
    }
    let result = (|| {
        if !(2..=4).contains(&args.len()) {
            return Err("expected CASE SIZE [MAX_TICKS] [TIMEOUT_SECONDS]".into());
        }
        let n: usize = args[1].parse().map_err(|_| "invalid size")?;
        if n == 0
            || n.checked_mul(n)
                .and_then(|square| square.checked_mul(16))
                .is_none()
        {
            return Err("size must be positive without arithmetic overflow".into());
        }
        let max_ticks = args
            .get(2)
            .map_or(Ok(50_000_000), |v| v.parse::<u64>())
            .map_err(|_| "invalid max ticks")?;
        let seconds = args
            .get(3)
            .map_or(Ok(30), |v| v.parse::<u64>())
            .map_err(|_| "invalid timeout seconds")?;
        if max_ticks == 0 || seconds == 0 {
            return Err("limits must be positive".into());
        }
        run(&args[0], n, max_ticks, Duration::from_secs(seconds))
    })();
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bindings(items: &[(&str, u64)]) -> BTreeMap<String, u64> {
        items
            .iter()
            .map(|(name, id)| ((*name).into(), *id))
            .collect()
    }
    #[test]
    fn dense_rejects_reversed_ports_and_duplicate_tuples() {
        let b = bindings(&[("V0", 10), ("V1", 11), ("W0", 20), ("W1", 21)]);
        let mut tuples = expected_tuples("dense", 2, &b).unwrap();
        assert!(consume_tuple(&mut tuples, "hit", &[20, 10]).is_err());
        assert!(consume_tuple(&mut tuples, "hit", &[10, 20]).is_ok());
        assert!(consume_tuple(&mut tuples, "hit", &[10, 20]).is_err());
        for row in [[10, 21], [11, 20], [11, 21]] {
            consume_tuple(&mut tuples, "hit", &row).unwrap();
        }
        assert!(tuples["hit"].is_empty());
    }
    #[test]
    fn aliases_and_selective_join_are_checked() {
        assert!(expected_tuples("alias", 1, &bindings(&[("V0", 10), ("V1", 11)])).is_err());
        assert!(expected_tuples("alias", 1, &bindings(&[("V0", 10), ("V1", 10)])).is_ok());
        let mut sparse =
            expected_tuples("sparse", 2, &bindings(&[("V0", 10), ("V1", 11)])).unwrap();
        assert!(consume_tuple(&mut sparse, "hit", &[11]).is_err());
        let mut degree =
            expected_tuples("degree", 1, &bindings(&[("A", 10), ("B", 10), ("V0", 11)])).unwrap();
        assert!(consume_tuple(&mut degree, "spoke", &[11, 10]).is_err());
        consume_tuple(&mut degree, "spoke", &[10, 11]).unwrap();
    }
    #[test]
    fn cyclic_accepts_only_directed_rotations() {
        let mut tuples = expected_tuples(
            "cyclic",
            1,
            &bindings(&[("A0", 10), ("B0", 11), ("C0", 12)]),
        )
        .unwrap();
        assert!(consume_tuple(&mut tuples, "triangle", &[10, 12, 11]).is_err());
        for row in [[10, 11, 12], [11, 12, 10], [12, 10, 11]] {
            consume_tuple(&mut tuples, "triangle", &row).unwrap();
        }
        assert!(tuples["triangle"].is_empty());
    }
    #[test]
    fn common_reader_resets_tuples_but_requires_exact_answer_multiplicity() {
        let (program, query, _, mut expected) = workload("common", 1).unwrap();
        let e = Engine::new(Arc::new(
            prepare(
                &parse_program(&program).unwrap(),
                &parse_query(&query).unwrap(),
            )
            .unwrap(),
        ));
        let relation = e
            .program()
            .signatures
            .iter()
            .position(|s| s.name == "done")
            .unwrap();
        let mut reader = Reader::default();
        for alternative in 0..3 {
            for event in [
                Output::Begin {
                    completion: 0,
                    alternative,
                },
                Output::Variable {
                    slot: 0,
                    variable: 42,
                },
                Output::Fact {
                    occurrence: 0,
                    relation,
                },
                Output::Port { variable: 42 },
                Output::EndFact,
            ] {
                reader.push(event, &e, &mut expected, "common", 1).unwrap();
            }
            let end = reader.push(Output::End, &e, &mut expected, "common", 1);
            assert_eq!(end.is_ok(), alternative < 2);
        }
        assert_eq!(reader.answers, 2);
        assert!(expected.is_empty());
    }
    #[test]
    fn wide_common_work_runs_once_and_delivers_every_alternative() {
        let n = 8;
        let (program, query, applications, mut expected) = workload("common-wide", n).unwrap();
        let mut e = Engine::new(Arc::new(
            prepare(
                &parse_program(&program).unwrap(),
                &parse_query(&query).unwrap(),
            )
            .unwrap(),
        ));
        let mut reader = Reader::default();
        for _ in 0..1_000_000 {
            e.advance(1);
            while let Some(event) = e.take_output() {
                reader
                    .push(event, &e, &mut expected, "common-wide", n)
                    .unwrap();
            }
            if e.exhausted() && expected.is_empty() {
                break;
            }
        }
        assert!(e.exhausted());
        assert!(expected.is_empty());
        assert_eq!(reader.answers, n);
        assert_eq!(e.applications(), applications as u64);
        assert_eq!(applications, 2 * n);
    }
}
