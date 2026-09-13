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

#[path = "measure/allocation.rs"]
mod allocation;
#[path = "measure/families.rs"]
mod families;
#[path = "measure/fresh.rs"]
mod fresh;
#[path = "measure/generated.rs"]
mod generated;
#[path = "measure/lifecycle.rs"]
mod lifecycle;
#[path = "measure/notebooks.rs"]
mod notebooks;
#[path = "measure/observation.rs"]
mod observation;
#[path = "measure/preparation.rs"]
mod preparation;
#[path = "measure/report.rs"]
mod report;
use allocation::{Phase, during};
use families::{Goal, Workload};

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

// Retain one answer for validation; IDs are checked independently of semantic tuples.
#[derive(Default)]
struct Reader {
    open: bool,
    slots: usize,
    ports_left: Option<usize>,
    answers: usize,
    facts: usize,
    ports: usize,
    scalars: usize,
    bindings: families::Bindings,
    rows: families::Rows,
    relation: String,
    tuple: Vec<u64>,
    ids: HashSet<(u64, u64)>,
    occurrences: HashSet<u64>,
    keys: HashSet<u64>,
    peak_bindings: usize,
    peak_rows: usize,
}
impl Reader {
    fn push(&mut self, output: Output, e: &Engine, w: &mut Workload) -> Result<(), String> {
        self.scalars += 1;
        match output {
            Output::Begin {
                completion,
                alternative,
            } if !self.open => {
                if !self.ids.insert((completion, alternative)) {
                    return Err("duplicate answer ID".into());
                }
                self.open = true;
                self.slots = 0;
                self.bindings.clear();
                self.rows.clear();
                self.occurrences.clear();
            }
            Output::Variable { slot, variable }
                if self.open
                    && self.ports_left.is_none()
                    && self.rows.is_empty()
                    && slot == self.slots
                    && slot < e.program().query_variables().len() =>
            {
                self.bindings
                    .insert(e.program().query_variables()[slot].clone(), variable);
                self.peak_bindings = self.peak_bindings.max(self.bindings.len());
                self.slots += 1;
            }
            Output::Fact {
                relation,
                occurrence,
            } if self.open
                && self.ports_left.is_none()
                && self.slots == e.program().query_variables().len() =>
            {
                if !self.occurrences.insert(occurrence) {
                    return Err("duplicate occurrence ID within answer".into());
                }
                let signature = e
                    .program()
                    .signatures()
                    .get(relation)
                    .ok_or("invalid relation ID")?;
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
                self.rows
                    .entry(self.relation.clone())
                    .or_default()
                    .push(std::mem::take(&mut self.tuple));
                self.ports_left = None;
            }
            Output::End
                if self.open
                    && self.ports_left.is_none()
                    && self.slots == e.program().query_variables().len() =>
            {
                self.peak_rows = self.peak_rows.max(self.occurrences.len());
                for rows in self.rows.values_mut() {
                    rows.sort();
                }
                if let Some(key) = w
                    .oracle
                    .check(&self.bindings, &self.rows, &mut w.expected)?
                    && !self.keys.insert(key)
                {
                    return Err("duplicate semantic alternative".into());
                }
                self.answers += 1;
                self.open = false;
                if w.answers.is_some_and(|n| self.answers > n) {
                    return Err("too many answers".into());
                }
            }
            other => return Err(format!("invalid or unexpected scalar {other:?}")),
        }
        Ok(())
    }
}
fn memory(e: &Engine) -> [usize; 14] {
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
        m.release_batches,
        m.restriction_nodes,
    ]
}
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
fn run(case: &str, n: usize, max_ticks: u64, timeout: Duration) -> Result<bool, String> {
    run_options(case, n, 1, None, max_ticks, timeout)
}
fn run_options(
    case: &str,
    n: usize,
    rows: usize,
    prefix: Option<usize>,
    max_ticks: u64,
    timeout: Duration,
) -> Result<bool, String> {
    // Input generation is outside timings; parsing and preparation remain charged.
    let w = during(Phase::Setup, || families::make(case, n, rows))?;
    run_workload(w, case, n, rows, prefix, max_ticks, timeout)
}
fn run_workload(
    mut w: Workload,
    case: &str,
    n: usize,
    rows: usize,
    prefix: Option<usize>,
    max_ticks: u64,
    timeout: Duration,
) -> Result<bool, String> {
    if let Some(count) = prefix {
        if !matches!(w.goal, Goal::Complete) || w.answers.is_some_and(|total| count > total) {
            return Err(
                "--prefix requires a finite case and cannot exceed its answer count".into(),
            );
        }
        w.goal = Goal::Answers(count);
    }
    let start = Instant::now();
    let parsed_program = during(Phase::Setup, || parse_program(&w.program))
        .map_err(|e| format!("parse program: {e:?}"))?;
    let parse_program_time = start.elapsed();
    let start = Instant::now();
    let parsed_query = during(Phase::Setup, || parse_query(&w.query))
        .map_err(|e| format!("parse query: {e:?}"))?;
    let parse_query_time = start.elapsed();
    let start = Instant::now();
    let code = during(Phase::Setup, || {
        prepare(&parsed_program, &parsed_query).map(Arc::new)
    })
    .map_err(|e| format!("prepare: {e:?}"))?;
    let prepare_time = start.elapsed();
    let start = Instant::now();
    let mut e = during(Phase::Setup, || Engine::new(code.clone()));
    let init_time = start.elapsed();
    let detailed = observation::detailed();
    let mut reader = Reader::default();
    let start = Instant::now();
    let mut validator = Duration::ZERO;
    let (mut ticks, mut collection_ticks) = (0, 0);
    let mut peak = memory(&e);
    let (mut first_event, mut first_answer, mut exhausted) = (None, None, None);
    let mut max_tick = Duration::ZERO;
    let mut error = None;
    let mut reached = false;
    let mut timed_out = false;
    let window_size = (n as u64 / 4).max(1);
    let mut next_window = window_size;
    let (mut last_apps, mut last_ticks, mut last_collection) = (0, 0, 0);
    let mut windows = Vec::new();
    while ticks < max_ticks && !e.delivery_done() {
        let collecting = detailed && e.collecting();
        let tick_start = observation::start(detailed);
        during(Phase::Engine, || e.advance(1));
        max_tick = max_tick.max(observation::elapsed(tick_start));
        ticks += 1;
        collection_ticks += u64::from(detailed && (collecting || e.collecting()));
        if let Some(output) = during(Phase::Delivery, || e.take_output()) {
            let end = matches!(output, Output::End);
            let event_time = if first_event.is_none() || (end && first_answer.is_none()) {
                start.elapsed()
            } else {
                Duration::ZERO
            };
            first_event.get_or_insert((ticks, event_time));
            let check = observation::start(detailed);
            let result = during(Phase::Validator, || reader.push(output, &e, &mut w));
            validator += observation::elapsed(check);
            if let Err(problem) = result {
                error = Some(problem);
                break;
            }
            if end {
                first_answer.get_or_insert((ticks, event_time));
            }
        }
        if exhausted.is_none() && e.exhausted() {
            exhausted.get_or_insert((ticks, start.elapsed()));
        }
        if matches!(w.goal, Goal::Applications(_)) && e.applications() >= next_window {
            windows.push((
                e.applications() - last_apps,
                ticks - last_ticks,
                collection_ticks - last_collection,
                memory(&e),
            ));
            last_apps = e.applications();
            last_ticks = ticks;
            last_collection = collection_ticks;
            next_window = last_apps.saturating_add(window_size);
        }
        reached = match w.goal {
            Goal::Complete => e.delivery_done(),
            Goal::Answers(count) => reader.answers == count,
            Goal::Applications(count) => e.applications() >= count,
        };
        if ticks % 2048 == 0 || reached || ticks == max_ticks {
            for (p, m) in peak.iter_mut().zip(memory(&e)) {
                *p = (*p).max(m);
            }
            timed_out = start.elapsed() >= timeout;
            if timed_out || reached {
                break;
            }
        }
    }
    let elapsed = start.elapsed();
    let apps = e.applications();
    let collections = e.collections();
    if reached && !timed_out && error.is_none() {
        if matches!(w.goal, Goal::Complete)
            && (reader.open
                || !w.expected.is_empty()
                || w.answers != Some(reader.answers)
                || w.apps.is_some_and(|want| want != apps))
        {
            error = Some(format!(
                "completion mismatch: answers={} expected={:?}, apps={apps} expected={:?}",
                reader.answers, w.answers, w.apps
            ));
        }
        if matches!(w.oracle, families::Oracle::Fair | families::Oracle::Stream) && e.exhausted() {
            error = Some("continuing workload unexpectedly exhausted".into());
        }
        if matches!(w.oracle, families::Oracle::Stream) && reader.scalars != 0 {
            error = Some("continuing stream emitted output".into());
        }
    }
    let before = memory(&e);
    for (p, m) in peak.iter_mut().zip(before) {
        *p = (*p).max(m);
    }
    report_diagnostics("source", &e);
    let cleanup_start = Instant::now();
    during(Phase::Cleanup, || e.cancel());
    let mut cleanup_ticks = 0;
    while !e.cancel_done() && cleanup_ticks < max_ticks {
        during(Phase::Cleanup, || {
            e.advance(1);
            e.take_output();
        });
        cleanup_ticks += 1;
        if cleanup_ticks % 2048 == 0 && cleanup_start.elapsed() >= timeout {
            break;
        }
    }
    let cleanup_time = cleanup_start.elapsed();
    let cleanup_in_time = cleanup_time < timeout;
    let after = memory(&e);
    let cleanup_done = e.cancel_done();
    // Coordinates retains the current epoch even after all execution roots are released.
    let reclaimed_all = matches!(
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
    ) && e.pending_tasks() == 0;
    if cleanup_done && !reclaimed_all {
        error = Some(format!("cleanup retained engine state: {after:?}"));
    }
    if e.applications() != apps {
        error = Some(format!(
            "source advanced during cancellation: before={apps} after={}",
            e.applications()
        ));
    }
    let reclaimed = std::array::from_fn::<_, 13, _>(|i| before[i].saturating_sub(after[i]));
    report_diagnostics("after_cancel", &e);
    let drop_start = Instant::now();
    during(Phase::Cleanup, || drop(e));
    let engine_drop = drop_start.elapsed();
    let drop_start = Instant::now();
    during(Phase::Cleanup, || drop(code));
    let prepared_drop = drop_start.elapsed();
    let success = reached && !timed_out && cleanup_done && cleanup_in_time && error.is_none();
    let status = if error.is_some() {
        "INVALID"
    } else if success {
        if matches!(w.goal, Goal::Complete) {
            "COMPLETE"
        } else {
            "PREFIX_COMPLETE"
        }
    } else {
        "INCOMPLETE"
    };
    println!(
        "case={case} size={n} rows={rows} goal={:?} status={status} source_goal_reached={reached} cleanup_done={cleanup_done} timed_out={timed_out} max_ticks={max_ticks}",
        w.goal
    );
    println!(
        "parse_program_ms={:.3} parse_query_ms={:.3} prepare_ms={:.3} engine_init_ms={:.3}",
        ms(parse_program_time),
        ms(parse_query_time),
        ms(prepare_time),
        ms(init_time)
    );
    println!(
        "source_delivery_ms={:.3} validator_ms={} source_delivery_without_validator_ms={} max_advance1_ms={}",
        ms(elapsed),
        observation::milliseconds(detailed, validator),
        observation::milliseconds(detailed, elapsed.saturating_sub(validator)),
        observation::milliseconds(detailed, max_tick)
    );
    println!(
        "first_event_ticks={:?} first_event_ms={:?} first_answer_ticks={:?} first_answer_ms={:?} source_exhausted_ticks={:?} source_exhausted_ms={:?}",
        first_event.map(|x| x.0),
        first_event.map(|x| ms(x.1)),
        first_answer.map(|x| x.0),
        first_answer.map(|x| ms(x.1)),
        exhausted.map(|x| x.0),
        exhausted.map(|x| ms(x.1))
    );
    println!(
        "advance1_ticks={ticks} collection_ticks={} collections={collections} applications={apps} expected_applications={:?} answers={} expected_answers={:?} facts={} ports={} scalars={}",
        observation::count(detailed, collection_ticks),
        w.apps,
        reader.answers,
        w.answers,
        reader.facts,
        reader.ports,
        reader.scalars
    );
    println!(
        "memory_counts [graph,occurrences,conditions,history_nodes,history_records,pending_nodes,descriptors,choices,coordinates,snapshots,inspections,tasks,release_batches,restriction_nodes] sampled_peak={peak:?} before_cleanup={before:?} after_cleanup={after:?} reclaimed={reclaimed:?}"
    );
    println!(
        "cleanup_ticks={cleanup_ticks} cleanup_in_time={cleanup_in_time} cleanup_ms={:.3} engine_drop_ms={:.3} prepared_drop_ms={:.3} validator_peak_bindings={} validator_peak_rows={} validator_answer_ids={}",
        ms(cleanup_time),
        ms(engine_drop),
        ms(prepared_drop),
        reader.peak_bindings,
        reader.peak_rows,
        reader.ids.len()
    );
    for (index, (apps, ticks, gc, memory)) in windows.iter().enumerate() {
        println!(
            "app_window={index} applications={apps} ticks={ticks} collection_ticks={} memory={memory:?}",
            observation::count(detailed, *gc)
        );
    }
    report::emit(
        "result",
        serde_json::json!({
            "case": case, "size": n, "rows": rows, "status": status,
            "goal": match w.goal { Goal::Complete => "complete", Goal::Answers(_) => "answer_prefix", Goal::Applications(_) => "application_prefix" },
            "goal_count": match w.goal { Goal::Complete => None, Goal::Answers(v) => Some(v as u64), Goal::Applications(v) => Some(v) },
            "source_goal_reached": reached, "cleanup_done": cleanup_done, "cleanup_in_time": cleanup_in_time,
            "timed_out": timed_out, "error": error, "max_ticks": max_ticks, "timeout_ms": ms(timeout),
            "times_ms": {"parse_program": ms(parse_program_time), "parse_query": ms(parse_query_time),
                "prepare": ms(prepare_time), "engine_init": ms(init_time), "source_delivery": ms(elapsed),
                "validator": detailed.then(|| ms(validator)),
                "source_delivery_without_validator": detailed.then(|| ms(elapsed.saturating_sub(validator))),
                "max_advance1": detailed.then(|| ms(max_tick)), "cleanup": ms(cleanup_time),
                "engine_drop": ms(engine_drop), "prepared_drop": ms(prepared_drop)},
            "first_event": first_event.map(|(tick, t)| serde_json::json!({"tick": tick, "ms": ms(t)})),
            "first_answer": first_answer.map(|(tick, t)| serde_json::json!({"tick": tick, "ms": ms(t)})),
            "source_exhausted": exhausted.map(|(tick, t)| serde_json::json!({"tick": tick, "ms": ms(t)})),
            "work": {"advance1_ticks": ticks, "collection_status_ticks": detailed.then_some(collection_ticks),
                "collections": collections, "applications": apps, "answers": reader.answers,
                "facts": reader.facts, "ports": reader.ports, "scalars": reader.scalars, "cleanup_ticks": cleanup_ticks},
            "expected": {"answers": w.answers, "applications": w.apps},
            "memory_counts": {"sampled_peak": report::memory(peak), "before_cleanup": report::memory(before), "after_cleanup": report::memory(after)},
            "windows": windows.iter().map(|(apps,ticks,gc,memory)| serde_json::json!({"applications": apps,"ticks": ticks,"collection_status_ticks": detailed.then_some(gc),"memory_counts": report::memory(*memory)})).collect::<Vec<_>>()
        }),
    );
    if let Some(error) = error {
        return Err(error);
    }
    Ok(success)
}
fn report_diagnostics(phase: &str, e: &Engine) {
    #[cfg(feature = "diagnostics")]
    {
        let allocation = allocation::snapshot();
        report::emit(
            "diagnostics",
            serde_json::json!({"phase": phase, "work": e.diagnostics(), "shared_restrictions": e.restriction_diagnostics(), "allocation": allocation, "memory_counts": report::memory(memory(e))}),
        );
        println!(
            "diagnostics={}",
            serde_json::json!({"phase": phase, "work": e.diagnostics(), "shared_restrictions": e.restriction_diagnostics(), "allocation": allocation, "memory_counts": memory(e)})
        );
    }
    #[cfg(not(feature = "diagnostics"))]
    let _ = (phase, e);
}
fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args == ["--help"] || args == ["--list"] {
        println!(
            "runtime-sessions: SIZE finite answers; --closed retired session count, --retained live completed sessions, --rows width, --batch scalar batch1..4096, --replay-every batch cadence (0off), --work background application target (0off)."
        );
        println!(
            "Preparation: {}; SIZE inactive rules (zero allowed); --heads N --arity N --repeats N --width N --depth N --uses N --empty. Repeated ports, body structure and sequential engine uses vary independently. Defaults 1/1/0/1/0/1, with a one-step query; --empty uses true.",
            preparation::CASES
        );
        println!(
            "Lifecycle interactions: life-held-output, life-archive-fixed, life-archive-rotate, life-inspections (each also has a -conditional variant for explicit choice turnover); SIZE counts siblings/snapshots/inspections; --rows sets residual width, --work continued applications (default 32), --cadence applications between rotations (default 1). Generated cases: {}. Options: --seed N --shape chain|ring|star|diamond|dense|random. SIZE is vertex/copy/distractor count; --rows is edge multiplicity, constraints per edge, duplicate groups or probe count. graph-bits has an exhaustive oracle limited to 16 vertices; proof-dag rejects cyclic shapes. Seed 0 is canonical order, other seeds reproducibly vary inputs and order.",
            generated::CASES
        );
        println!(
            "Usage: measure CASE SIZE [MAX_TICKS] [TIMEOUT_SECONDS] [--rows N] [--prefix N] [--detail]\nCases: {CASES} {} {} {}\nDefaults: MAX_TICKS=50000000 TIMEOUT_SECONDS=30 rows=1. SIZE positive; rows may be zero.\nanswers: SIZE alternatives, --rows residual rows (zero gives empty answers).\nbits-chain/star[-delayed]: SIZE bits; delayed constraints have an 8*SIZE+1 application gate.\nalias-consume: SIZE conditional merges plus one unmerged arm.\nfair-loop/grow: SIZE continuing siblings, --rows finite-chain length; stops at first complete answer.\nstream-fail: SIZE application prefix, four approximately equal application windows.\nrejected3: SIZE rows per head, no hits. multiport[-hit|-probes-first]: SIZE rows per bucket, --rows probes.\nsimpagation: SIZE copies, --rows depth. repeated-alias: SIZE aliases, --rows probes; raw-probes: same probes without aliases.\nreach-chain: SIZE edges. prepare: SIZE irrelevant rules; fanout: SIZE enabled propagation rules.\nlife-*: SIZE initial application milestone; windows continue through 8*SIZE. life-archive: SIZE retained snapshots, then 2048 additional applications.\nruntime: SIZE alternatives, each with SIZE residual rows.\nnotebook-arithmetic-*: SIZE arithmetic magnitude. notebook-type/behavior-*: SIZE answer-prefix count.\nnotebook-lambda: SIZE nested identity count; stops at its first validated answer.\n--prefix N stops finite core cases after N validated answers; never claims exhaustion.\nNo history or retained views in core cases. Baseline clocks phase boundaries and first events, with periodic budgets/memory checks; --detail adds per-tick latency, collection-status and validator timing. Unobserved fields are null. Validation runs in both modes.\nFirst-answer time is at End, before its validation. Collection ticks sample collecting before OR after advance.\nMemory counts sampled every 2048 ticks and at exit, not bytes or exact peaks. Validator retains one answer plus IDs.\nSource and cleanup each get the supplied tick/time limits; timeout cannot preempt a tick or drop.",
            families::CASES,
            notebooks::CASES,
            lifecycle::CASES
        );
        return ExitCode::SUCCESS;
    }
    let result = (|| {
        if args.len() < 2 {
            return Err("expected CASE SIZE".into());
        }
        let n: usize = args[1].parse().map_err(|_| "invalid size")?;
        let (mut max_ticks, mut seconds, mut rows, mut prefix) =
            (50_000_000u64, 30u64, 1usize, None);
        let (mut seed, mut shape) = (0u64, "chain".to_string());
        let is_preparation = preparation::CASES
            .split_whitespace()
            .any(|case| case == args[0]);
        let is_sessions = lifecycle::sessions::CASES
            .split_whitespace()
            .any(|case| case == args[0]);
        let mut session_options = false;
        let mut sessions = lifecycle::sessions::Options::default();
        let is_fresh = fresh::CASES.split_whitespace().any(|case| case == args[0]);
        let (mut fresh_depth, mut order) = (4usize, "grouped".to_string());
        let (mut depth_option, mut order_option) = (false, false);
        let mut prep = preparation::Options::default();
        let mut preparation_options = false;
        let mut generated_options = false;
        let mut detailed = false;
        let (mut work, mut cadence) = (32u64, 1u64);
        let mut interaction_options = false;
        let mut positional = 0;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--detail" => detailed = true,
                "--empty" => {
                    preparation_options = true;
                    prep.empty = true;
                }
                "--depth" => {
                    depth_option = true;
                    i += 1;
                    let value = args
                        .get(i)
                        .ok_or("missing depth")?
                        .parse::<usize>()
                        .map_err(|_| "invalid depth")?;
                    prep.depth = value;
                    fresh_depth = value;
                }
                "--order" => {
                    order_option = true;
                    i += 1;
                    order = args.get(i).ok_or("missing fresh order")?.clone();
                }
                "--heads" | "--arity" | "--repeats" | "--width" | "--uses" => {
                    preparation_options = true;
                    let flag = &args[i];
                    i += 1;
                    let value = args
                        .get(i)
                        .ok_or("missing preparation option")?
                        .parse::<usize>()
                        .map_err(|_| "invalid preparation option")?;
                    match flag.as_str() {
                        "--heads" => prep.heads = value,
                        "--arity" => prep.arity = value,
                        "--repeats" => prep.repeats = value,
                        "--width" => prep.width = value,
                        "--uses" => prep.uses = value,
                        _ => unreachable!(),
                    }
                }
                "--closed" | "--retained" | "--batch" | "--replay-every" => {
                    session_options = true;
                    let flag = &args[i];
                    i += 1;
                    let value = args
                        .get(i)
                        .ok_or("missing session option")?
                        .parse::<usize>()
                        .map_err(|_| "invalid session option")?;
                    match flag.as_str() {
                        "--closed" => sessions.closed = value,
                        "--retained" => sessions.retained = value,
                        "--batch" => sessions.batch = value,
                        "--replay-every" => sessions.replay_every = value,
                        _ => unreachable!(),
                    }
                }
                "--work" | "--cadence" => {
                    let flag = &args[i];
                    i += 1;
                    let value = args
                        .get(i)
                        .ok_or("missing interaction option")?
                        .parse::<u64>()
                        .map_err(|_| "invalid interaction option")?;
                    if value == 0 && !(is_sessions && flag == "--work") {
                        return Err("positive work/cadence required".into());
                    }
                    if is_sessions && flag == "--work" {
                        sessions.work = value;
                        session_options = true;
                    } else {
                        interaction_options = true;
                        if flag == "--work" {
                            work = value;
                        } else {
                            cadence = value;
                        }
                    }
                }
                "--seed" | "--shape" => {
                    generated_options = true;
                    let flag = &args[i];
                    i += 1;
                    let value = args.get(i).ok_or("missing generated option value")?;
                    if flag == "--seed" {
                        seed = value.parse().map_err(|_| "invalid seed")?;
                    } else {
                        shape.clone_from(value);
                    }
                }
                "--rows" | "--prefix" => {
                    let flag = &args[i];
                    i += 1;
                    let value: usize = args
                        .get(i)
                        .ok_or("missing option value")?
                        .parse()
                        .map_err(|_| "invalid option value")?;
                    if flag == "--rows" {
                        rows = value;
                    } else {
                        prefix = Some(value);
                    }
                }
                _ => {
                    let value = args[i]
                        .parse::<u64>()
                        .map_err(|_| "invalid limit or unknown option")?;
                    match positional {
                        0 => max_ticks = value,
                        1 => seconds = value,
                        _ => return Err("too many positional limits".into()),
                    }
                    positional += 1;
                }
            }
            i += 1;
        }
        if (n == 0 && !is_preparation)
            || max_ticks == 0
            || seconds == 0
            || prefix == Some(0)
            || n.checked_add(rows)
                .and_then(|v| v.checked_add(1))
                .and_then(|v| v.checked_mul(v))
                .and_then(|v| v.checked_mul(16))
                .is_none()
        {
            return Err(
                "positive size/limits/prefix required; dimensions must not overflow".into(),
            );
        }
        let interaction = matches!(
            args[0].strip_suffix("-conditional").unwrap_or(&args[0]),
            "life-held-output" | "life-archive-fixed" | "life-archive-rotate" | "life-inspections"
        );
        if interaction_options && !interaction {
            return Err("--work/--cadence require lifecycle interaction cases".into());
        }
        if (depth_option && !is_preparation && !is_fresh) || (order_option && !is_fresh) {
            return Err(
                "depth requires preparation/fresh cases; order requires fresh cases".into(),
            );
        }
        if is_fresh && (generated_options || interaction_options || prefix.is_some()) {
            return Err(
                "fresh cases require complete exhaustion and use depth/order/rows options".into(),
            );
        }
        if preparation_options && !is_preparation {
            return Err("preparation options require prepare-reuse or prepare-independent".into());
        }
        if is_preparation
            && (generated_options || interaction_options || prefix.is_some() || rows != 1)
        {
            return Err("preparation cases use their shape/use options, not rows/prefix/generated/lifecycle options".into());
        }
        if session_options && !is_sessions {
            return Err("session options require runtime-sessions".into());
        }
        if is_sessions && (generated_options || prefix.is_some() || interaction_options) {
            return Err("runtime-sessions requires full output and its session options".into());
        }
        sessions.rows = rows;
        observation::configure(detailed);
        report::emit(
            "configuration",
            serde_json::json!({"case": args[0], "size": n, "rows": rows, "prefix": prefix, "seed": seed, "shape": shape, "detailed": detailed, "diagnostics_feature": cfg!(feature = "diagnostics"), "max_ticks": max_ticks, "timeout_seconds": seconds, "continued_work": interaction.then_some(work), "rotation_cadence": interaction.then_some(cadence), "preparation": is_preparation.then_some(prep), "fresh_depth": is_fresh.then_some(fresh_depth), "fresh_order": is_fresh.then_some(&order), "sessions": is_sessions.then_some(sessions)}),
        );
        println!(
            "measurement_mode={} diagnostics_feature={}",
            if detailed { "detailed" } else { "baseline" },
            cfg!(feature = "diagnostics")
        );
        if is_sessions {
            let result =
                lifecycle::sessions::run(n, sessions, max_ticks, Duration::from_secs(seconds))?;
            report::emit("result", result.clone());
            if let Some(error) = result["error"].as_str() {
                return Err(error.into());
            }
            return Ok(result["status"] == "COMPLETE");
        }
        if is_fresh {
            let workload = fresh::make(&args[0], n, rows, fresh_depth, &order)?;
            return run_workload(
                workload,
                &args[0],
                n,
                rows,
                prefix,
                max_ticks,
                Duration::from_secs(seconds),
            );
        }
        if is_preparation {
            let measurement =
                preparation::run(&args[0], n, prep, max_ticks, Duration::from_secs(seconds))?;
            report::emit("result", serde_json::json!(measurement));
            if let Some(error) = measurement.error {
                return Err(error);
            }
            return Ok(measurement.status == "COMPLETE");
        }
        if generated::CASES
            .split_whitespace()
            .any(|name| name == args[0])
        {
            let w = generated::make(&args[0], n, rows, seed, &shape)?;
            println!("generator_seed={seed} generator_shape={shape}");
            return run_workload(
                w,
                &args[0],
                n,
                rows,
                prefix,
                max_ticks,
                Duration::from_secs(seconds),
            );
        }
        if generated_options {
            return Err("--seed/--shape require a generated case".into());
        }
        if lifecycle::CASES
            .split_whitespace()
            .any(|name| name == args[0])
        {
            if interaction {
                if prefix.is_some() {
                    return Err("lifecycle interactions do not use --prefix".into());
                }
                return lifecycle::run_options(
                    &args[0],
                    n,
                    lifecycle::InteractionOptions {
                        rows,
                        work,
                        cadence,
                    },
                    max_ticks,
                    Duration::from_secs(seconds),
                );
            }
            if rows != 1 || prefix.is_some() {
                return Err("lifecycle modes use SIZE; no --rows/--prefix".into());
            }
            return lifecycle::run(&args[0], n, max_ticks, Duration::from_secs(seconds));
        }
        if args[0].starts_with("notebook-") {
            if rows != 1 || prefix.is_some() {
                return Err(
                    "notebook modes use SIZE for magnitude or answer prefix; no --rows/--prefix"
                        .into(),
                );
            }
            return notebooks::run(&args[0], n, max_ticks, Duration::from_secs(seconds));
        }
        if rows == 1 && prefix.is_none() {
            run(&args[0], n, max_ticks, Duration::from_secs(seconds))
        } else {
            run_options(
                &args[0],
                n,
                rows,
                prefix,
                max_ticks,
                Duration::from_secs(seconds),
            )
        }
    })();
    #[cfg(feature = "diagnostics")]
    {
        let sample = allocation::snapshot();
        report::emit("allocations", serde_json::json!(sample));
        println!("allocations={}", serde_json::to_string(&sample).unwrap());
    }
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            report::emit("error", serde_json::json!({"message": error}));
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_answers_prefixes_and_limits_are_distinct() {
        let limit = Duration::from_secs(10);
        assert!(run_options("answers", 4, 0, None, 100_000, limit).unwrap());
        assert!(run_options("answers", 4, 1, Some(2), 100_000, limit).unwrap());
        assert!(!run_options("answers", 4, 0, Some(2), 1, limit).unwrap());
        assert!(run_options("answers", 4, 0, Some(5), 100_000, limit).is_err());
        assert!(run_options("simpagation", 4, 3, None, 500_000, limit).unwrap());
    }
    #[test]
    fn family_oracles_reject_mixed_bits_wrong_consumption_and_duplicate_rows() {
        use families::{Bindings, Rows};
        let bits = families::make("bits-chain", 2, 1).unwrap();
        let b = Bindings::from([("V0".into(), 1), ("V1".into(), 2)]);
        let mut rows = Rows::from([
            ("zero".into(), vec![vec![1], vec![2]]),
            ("link".into(), vec![vec![1, 2]]),
            ("enabled".into(), vec![vec![]]),
        ]);
        assert_eq!(bits.oracle.check(&b, &rows, &mut vec![]).unwrap(), Some(0));
        rows.get_mut("zero").unwrap().pop();
        rows.insert("one".into(), vec![vec![2]]);
        assert!(bits.oracle.check(&b, &rows, &mut vec![]).is_err());
        let alias = families::make("alias-consume", 2, 1).unwrap();
        let b = Bindings::from([("A".into(), 1), ("V0".into(), 1), ("V1".into(), 2)]);
        let mut rows = Rows::from([
            ("p".into(), vec![vec![1]]),
            ("q".into(), vec![vec![2]]),
            ("hit".into(), vec![vec![1]]),
        ]);
        assert_eq!(alias.oracle.check(&b, &rows, &mut vec![]).unwrap(), Some(0));
        rows.get_mut("q").unwrap()[0] = vec![1];
        assert!(alias.oracle.check(&b, &rows, &mut vec![]).is_err());
        let answers = families::make("answers", 1, 1).unwrap();
        let b = Bindings::from([("V0".into(), 1)]);
        let rows = Rows::from([("p".into(), vec![vec![1], vec![1]])]);
        assert!(answers.oracle.check(&b, &rows, &mut vec![]).is_err());
    }
    #[test]
    fn distinguishing_families_validate_small_runs() {
        for case in CASES
            .split_whitespace()
            .chain(families::CASES.split_whitespace())
        {
            assert!(
                run(case, 2, 2_000_000, Duration::from_secs(10)).unwrap(),
                "{case}"
            );
        }
    }
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
        let mut w = families::make("common", 1, 1).unwrap();
        let (program, query) = (&w.program, &w.query);
        let e = Engine::new(Arc::new(
            prepare(
                &parse_program(&program).unwrap(),
                &parse_query(&query).unwrap(),
            )
            .unwrap(),
        ));
        let relation = e
            .program()
            .signatures()
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
                reader.push(event, &e, &mut w).unwrap();
            }
            let end = reader.push(Output::End, &e, &mut w);
            assert_eq!(end.is_ok(), alternative < 2);
        }
        assert_eq!(reader.answers, 2);
        assert!(w.expected.is_empty());
    }
    #[test]
    fn wide_common_work_runs_once_and_delivers_every_alternative() {
        let n = 8;
        let mut w = families::make("common-wide", n, 1).unwrap();
        let (program, query, applications) = (&w.program, &w.query, w.apps.unwrap());
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
                reader.push(event, &e, &mut w).unwrap();
            }
            if e.exhausted() && w.expected.is_empty() {
                break;
            }
        }
        assert!(e.exhausted());
        assert!(w.expected.is_empty());
        assert_eq!(reader.answers, n);
        assert_eq!(e.applications(), applications as u64);
        assert_eq!(applications, (2 * n) as u64);
    }
    #[test]
    fn common_and_alias_consumption_controls_check_complete_outputs_and_sharing() {
        for case in ["common", "alias-consume"] {
            let mut w = families::make(case, 8, 1).unwrap();
            let expected_answers = w.answers.unwrap();
            let expected_apps = w.apps.unwrap();
            let mut e = Engine::new(Arc::new(
                prepare(
                    &parse_program(&w.program).unwrap(),
                    &parse_query(&w.query).unwrap(),
                )
                .unwrap(),
            ));
            let mut reader = Reader::default();
            for _ in 0..1_000_000 {
                e.advance(1);
                while let Some(event) = e.take_output() {
                    reader.push(event, &e, &mut w).unwrap();
                }
                if e.delivery_done() {
                    break;
                }
            }
            assert!(e.delivery_done() && e.exhausted(), "{case}");
            assert_eq!(reader.answers, expected_answers, "{case}");
            assert_eq!(e.applications(), expected_apps, "{case}");
            assert!(w.expected.is_empty(), "{case}");
        }
    }
}
