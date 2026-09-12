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

#[path = "measure/families.rs"]
mod families;
#[path = "measure/lifecycle.rs"]
mod lifecycle;
#[path = "measure/notebooks.rs"]
mod notebooks;
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
fn memory(e: &Engine) -> [usize; 13] {
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
    let mut w = families::make(case, n, rows)?;
    if let Some(count) = prefix {
        if !matches!(w.goal, Goal::Complete) || w.answers.is_some_and(|total| count > total) {
            return Err(
                "--prefix requires a finite case and cannot exceed its answer count".into(),
            );
        }
        w.goal = Goal::Answers(count);
    }
    let start = Instant::now();
    let parsed_program = parse_program(&w.program).map_err(|e| format!("parse program: {e:?}"))?;
    let parse_program_time = start.elapsed();
    let start = Instant::now();
    let parsed_query = parse_query(&w.query).map_err(|e| format!("parse query: {e:?}"))?;
    let parse_query_time = start.elapsed();
    let start = Instant::now();
    let code =
        Arc::new(prepare(&parsed_program, &parsed_query).map_err(|e| format!("prepare: {e:?}"))?);
    let prepare_time = start.elapsed();
    let start = Instant::now();
    let mut e = Engine::new(code.clone());
    let init_time = start.elapsed();
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
        let collecting = e.collecting();
        let tick_start = Instant::now();
        e.advance(1);
        max_tick = max_tick.max(tick_start.elapsed());
        ticks += 1;
        collection_ticks += u64::from(collecting || e.collecting());
        if let Some(output) = e.take_output() {
            let event_time = start.elapsed();
            first_event.get_or_insert((ticks, event_time));
            let end = matches!(output, Output::End);
            let check = Instant::now();
            let result = reader.push(output, &e, &mut w);
            validator += check.elapsed();
            if let Err(problem) = result {
                error = Some(problem);
                break;
            }
            if end {
                first_answer.get_or_insert((ticks, event_time));
            }
        }
        if e.exhausted() {
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
    let cleanup_start = Instant::now();
    e.cancel();
    let mut cleanup_ticks = 0;
    while !e.cancel_done() && cleanup_ticks < max_ticks {
        e.advance(1);
        e.take_output();
        cleanup_ticks += 1;
        if cleanup_ticks % 2048 == 0 && cleanup_start.elapsed() >= timeout {
            break;
        }
    }
    let cleanup_time = cleanup_start.elapsed();
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
    let drop_start = Instant::now();
    drop(e);
    let engine_drop = drop_start.elapsed();
    let drop_start = Instant::now();
    drop(code);
    let prepared_drop = drop_start.elapsed();
    let success = reached && !timed_out && cleanup_done && error.is_none();
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
        "source_delivery_ms={:.3} validator_ms={:.3} source_delivery_without_validator_ms={:.3} max_advance1_ms={:.3}",
        ms(elapsed),
        ms(validator),
        ms(elapsed.saturating_sub(validator)),
        ms(max_tick)
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
        "advance1_ticks={ticks} collection_ticks={collection_ticks} collections={collections} applications={apps} expected_applications={:?} answers={} expected_answers={:?} facts={} ports={} scalars={}",
        w.apps, reader.answers, w.answers, reader.facts, reader.ports, reader.scalars
    );
    println!(
        "memory_counts [graph,occurrences,conditions,history_nodes,history_records,pending_nodes,descriptors,choices,coordinates,snapshots,inspections,tasks,release_batches] sampled_peak={peak:?} before_cleanup={before:?} after_cleanup={after:?} reclaimed={reclaimed:?}"
    );
    println!(
        "cleanup_ticks={cleanup_ticks} cleanup_ms={:.3} engine_drop_ms={:.3} prepared_drop_ms={:.3} validator_peak_bindings={} validator_peak_rows={} validator_answer_ids={}",
        ms(cleanup_time),
        ms(engine_drop),
        ms(prepared_drop),
        reader.peak_bindings,
        reader.peak_rows,
        reader.ids.len()
    );
    for (index, (apps, ticks, gc, memory)) in windows.iter().enumerate() {
        println!(
            "app_window={index} applications={apps} ticks={ticks} collection_ticks={gc} memory={memory:?}"
        );
    }
    if let Some(error) = error {
        return Err(error);
    }
    Ok(success)
}
fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args == ["--help"] || args == ["--list"] {
        println!(
            "Usage: measure CASE SIZE [MAX_TICKS] [TIMEOUT_SECONDS] [--rows N] [--prefix N]\nCases: {CASES} {} {} {}\nDefaults: MAX_TICKS=50000000 TIMEOUT_SECONDS=30 rows=1. SIZE positive; rows may be zero.\nanswers: SIZE alternatives, --rows residual rows (zero gives empty answers).\nbits-chain/star[-delayed]: SIZE bits; delayed constraints have an 8*SIZE+1 application gate.\nalias-consume: SIZE conditional merges plus one unmerged arm.\nfair-loop/grow: SIZE continuing siblings, --rows finite-chain length; stops at first complete answer.\nstream-fail: SIZE application prefix, four approximately equal application windows.\nrejected3: SIZE rows per head, no hits. multiport[-hit|-probes-first]: SIZE rows per bucket, --rows probes.\nsimpagation: SIZE copies, --rows depth. repeated-alias: SIZE aliases, --rows probes; raw-probes: same probes without aliases.\nreach-chain: SIZE edges. prepare: SIZE irrelevant rules; fanout: SIZE enabled propagation rules.\nlife-*: SIZE initial application milestone; windows continue through 8*SIZE. life-archive: SIZE retained snapshots, then 2048 additional applications.\nruntime: SIZE alternatives, each with SIZE residual rows.\nnotebook-arithmetic-*: SIZE arithmetic magnitude. notebook-type/behavior-*: SIZE answer-prefix count.\nnotebook-lambda: SIZE nested identity count; stops at its first validated answer.\n--prefix N stops finite core cases after N validated answers; never claims exhaustion.\nNo history or retained views in core cases. Wall times include instrumentation; validator time separately charged.\nFirst-answer time is at End, before its validation. Collection ticks sample collecting before OR after advance.\nMemory counts sampled every 2048 ticks and at exit, not bytes or exact peaks. Validator retains one answer plus IDs.\nSource and cleanup each get the supplied tick/time limits; timeout cannot preempt a tick or drop.",
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
        let mut positional = 0;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
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
        if n == 0
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
        if lifecycle::CASES
            .split_whitespace()
            .any(|name| name == args[0])
        {
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
}
