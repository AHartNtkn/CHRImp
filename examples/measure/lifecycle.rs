//! Lifecycle probes use application milestones; runtime probes use the real API and scheduler.
use crate::allocation::{Phase, during};
use crate::observation;
use chr::{
    engine::{Engine, InspectionError, Memory, ViewId},
    notebook::Runtime,
    observe::Output,
    program::{Signature, prepare},
    syntax::{parse_program, parse_query},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

#[path = "interactions.rs"]
mod interactions;
pub use interactions::Options as InteractionOptions;

pub const CASES: &str = "life-alias life-propagation life-dependent life-snapshot life-history life-history-choice life-archive life-held-output life-archive-fixed life-archive-rotate life-inspections runtime";
const REWRITE: &str = "p(X) <=> q(X). q(X) <=> done(X).";
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
fn check(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why.into()) }
}

struct Budget {
    start: Instant,
    ticks: u64,
    limit: u64,
    timeout: Duration,
    maximum: Duration,
    collection_ticks: u64,
    validator: Duration,
    phase: Phase,
    detailed: bool,
}
impl Budget {
    fn new(limit: u64, timeout: Duration) -> Self {
        Self {
            start: Instant::now(),
            ticks: 0,
            limit,
            timeout,
            maximum: Duration::ZERO,
            collection_ticks: 0,
            validator: Duration::ZERO,
            phase: Phase::Engine,
            detailed: observation::detailed(),
        }
    }
    fn in_time(&self) -> bool {
        self.start.elapsed() < self.timeout
    }
    fn available(&self) -> bool {
        self.ticks < self.limit
            && (!self.ticks.is_multiple_of(2048) || self.start.elapsed() < self.timeout)
    }
    fn step(&mut self, e: &mut Engine) -> bool {
        if !self.available() {
            return false;
        }
        self.collection_ticks += u64::from(self.detailed && e.collecting());
        let t = observation::start(self.detailed);
        during(self.phase, || e.advance(1));
        self.maximum = self.maximum.max(observation::elapsed(t));
        self.ticks += 1;
        true
    }
    fn runtime_tick(&mut self, r: &Runtime) -> bool {
        if !self.available() {
            return false;
        }
        let t = observation::start(self.detailed);
        during(self.phase, || r.tick());
        self.maximum = self.maximum.max(observation::elapsed(t));
        self.ticks += 1;
        true
    }
}

// One answer at a time. Event and occurrence IDs are checked independently of tuples.
#[derive(Default)]
struct Reader {
    open: bool,
    bindings: Vec<u64>,
    rows: Vec<(String, Vec<u64>)>,
    fact: Option<(String, usize, Vec<u64>)>,
    occurrences: BTreeSet<u64>,
    ids: BTreeSet<(u64, u64)>,
    answers: usize,
}
impl Reader {
    fn push(
        &mut self,
        o: Output,
        signatures: &[Signature],
        expected: &[&str],
        variables: usize,
    ) -> Result<bool, String> {
        match o {
            Output::Begin {
                completion,
                alternative,
            } => {
                check(
                    !self.open && self.ids.insert((completion, alternative)),
                    "duplicate or overlapping answer",
                )?;
                self.open = true;
                self.bindings.clear();
                self.rows.clear();
                self.occurrences.clear();
            }
            Output::Variable { slot, variable } => {
                check(
                    self.open
                        && self.rows.is_empty()
                        && self.fact.is_none()
                        && slot == self.bindings.len()
                        && slot < variables,
                    "invalid binding order",
                )?;
                self.bindings.push(variable);
            }
            Output::Fact {
                occurrence,
                relation,
            } => {
                check(
                    self.open
                        && self.bindings.len() == variables
                        && self.fact.is_none()
                        && self.occurrences.insert(occurrence),
                    "invalid occurrence",
                )?;
                let s = signatures.get(relation).ok_or("unknown relation")?;
                self.fact = Some((s.name.clone(), s.arity, vec![]));
            }
            Output::Port { variable } => {
                let f = self.fact.as_mut().ok_or("port without fact")?;
                check(f.2.len() < f.1, "too many ports")?;
                f.2.push(variable);
            }
            Output::EndFact => {
                let (name, arity, ports) = self.fact.take().ok_or("end without fact")?;
                check(arity == ports.len(), "incomplete ports")?;
                self.rows.push((name, ports));
            }
            Output::End => {
                check(
                    self.open && self.fact.is_none() && self.bindings.len() == variables,
                    "incomplete answer",
                )?;
                check(
                    self.bindings.iter().collect::<BTreeSet<_>>().len() == variables,
                    "unexpected alias",
                )?;
                let mut want = expected
                    .iter()
                    .flat_map(|name| {
                        self.bindings
                            .iter()
                            .map(move |v| ((*name).to_owned(), vec![*v]))
                    })
                    .collect::<Vec<_>>();
                want.sort();
                self.rows.sort();
                check(
                    self.rows == want,
                    "incorrect residual graph or multiplicity",
                )?;
                self.open = false;
                self.answers += 1;
                return Ok(true);
            }
            // Retained snapshots can also expose unfinished syntax. These probes
            // check their known committed graph; pending syntax has separate tests.
            Output::PendingBegin { .. }
            | Output::PendingEnd
            | Output::Expression { .. }
            | Output::ExpressionRelation { .. }
            | Output::ExpressionVariable { .. }
            | Output::ExpressionEnd => return Err("unexpected pending syntax in answer".into()),
        }
        Ok(false)
    }
}

fn zero(m: Memory) -> bool {
    m.graph_nodes == 0
        && m.release_batches == 0
        && m.occurrences == 0
        && m.conditions == 0
        && m.history_nodes == 0
        && m.history_records == 0
        && m.pending_nodes == 0
        && m.obligation_descriptors == 0
        && m.choices == 0
        // The current coordinate epoch belongs to the empty engine itself.
        && m.coordinate_records == 1
        && m.snapshots == 0
        && m.inspections == 0
}
fn cancel(e: &mut Engine, limit: u64, timeout: Duration) -> bool {
    crate::report_diagnostics("source", e);
    let mut b = Budget::new(limit, timeout);
    b.phase = Phase::Cleanup;
    let t = Instant::now();
    during(Phase::Cleanup, || e.cancel());
    let request = t.elapsed();
    let apps = e.applications();
    while !e.cancel_done() && b.step(e) {}
    let elapsed = b.start.elapsed();
    let done = e.cancel_done() && elapsed < timeout;
    assert_eq!(e.applications(), apps, "application after cancel");
    println!(
        "cancel_request_ms={:.3} cancel_ms={:.3} cancel_ticks={} cancel_max_step_ms={} cancel_done={} retained={:?}",
        ms(request),
        ms(elapsed),
        b.ticks,
        observation::milliseconds(b.detailed, b.maximum),
        done,
        e.memory()
    );
    crate::report::emit(
        "phase",
        json!({"phase":"cancel", "request_ms":ms(request),
        "elapsed_ms":ms(elapsed),"ticks":b.ticks,"complete":done,
        "applications":e.applications(),"memory":crate::report::memory(crate::memory(e))}),
    );
    crate::report_diagnostics("after_cancel", e);
    done
}
fn release(e: &mut Engine, limit: u64, timeout: Duration) -> Result<bool, String> {
    let mut b = Budget::new(limit, timeout);
    b.phase = Phase::Cleanup;
    // Cancellation finishes projections, but handles remain independently owned.
    loop {
        let next = e.inspections().next();
        let Some(id) = next else {
            break;
        };
        if !b.available() {
            return Ok(false);
        }
        match e.release_inspection(id) {
            Ok(()) => b.ticks += 1,
            Err(InspectionError::Busy) => {
                if !b.step(e) {
                    return Ok(false);
                }
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    // Release all registered views before sweeping their shared roots.
    loop {
        let next = e.snapshots().next().map(|s| s.id);
        let Some(id) = next else {
            break;
        };
        if !b.available() {
            return Ok(false);
        }
        match e.release_snapshot(id) {
            Ok(()) => {
                b.ticks += 1;
            }
            Err(InspectionError::Busy) => {
                if !b.step(e) {
                    return Ok(false);
                }
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    while !e.cancel_done() && b.step(e) {}
    let done = e.cancel_done() && b.in_time();
    println!(
        "release_ms={:.3} release_work={} release_max_step_ms={} release_done={} reclaimed={:?}",
        ms(b.start.elapsed()),
        b.ticks,
        observation::milliseconds(b.detailed, b.maximum),
        done,
        e.memory()
    );
    crate::report::emit(
        "phase",
        json!({"phase":"release", "elapsed_ms":ms(b.start.elapsed()),
        "ticks":b.ticks,"complete":done,"memory":crate::report::memory(crate::memory(e))}),
    );
    if done {
        check(
            zero(e.memory()) && e.pending_tasks() == 0,
            "unowned execution memory after cleanup",
        )?;
    }
    Ok(done)
}

fn inspect(
    e: &mut Engine,
    snapshot: ViewId,
    expected_answers: usize,
    limit: u64,
    timeout: Duration,
) -> Result<bool, String> {
    let mut b = Budget::new(limit, timeout);
    b.phase = Phase::Inspection;
    let id = e
        .start_inspection(Some(snapshot), vec![])
        .map_err(|e| e.to_string())?;
    let mut r = Reader::default();
    let apps = e.applications();
    while !e.inspection_status(id).map_err(|e| e.to_string())?.done {
        if !b.available() {
            return Ok(false);
        }
        during(Phase::Inspection, || e.advance_inspection(id, 1)).map_err(|e| e.to_string())?;
        b.ticks += 1;
        if let Some(o) = e.take_inspection_output(id).map_err(|e| e.to_string())? {
            // Only the committed graph is part of this oracle. The chosen view
            // can contain the remaining loop body as pending syntax.
            if matches!(
                o,
                Output::PendingBegin { .. }
                    | Output::PendingEnd
                    | Output::Expression { .. }
                    | Output::ExpressionRelation { .. }
                    | Output::ExpressionVariable { .. }
                    | Output::ExpressionEnd
            ) {
                continue;
            }
            r.push(o, e.program().signatures(), &["keep", "loop"], 1)?;
        }
    }
    check(
        r.answers == expected_answers && !r.open && e.applications() == apps,
        "retained snapshot changed after cancel",
    )?;
    e.release_inspection(id).map_err(|e| e.to_string())?;
    let in_time = b.in_time();
    println!(
        "retained_inspection_ms={:.3} inspection_ticks={} answers={}",
        ms(b.start.elapsed()),
        b.ticks,
        r.answers
    );
    Ok(in_time)
}

fn stream(case: &str, n: usize, limit: u64, timeout: Duration) -> Result<bool, String> {
    let last = n.checked_mul(8).ok_or("application milestone overflow")? as u64;
    let retained = matches!(
        case,
        "life-snapshot" | "life-history" | "life-history-choice"
    );
    let (program, query) = match case {
        "life-alias" => ("loop(X) <=> X=Y,loop(Y).", "loop(A);done(A)"),
        "life-propagation" => (
            "p(X) ==> seen(X). p(X) \\ seen(X) <=> next(X). p(X),next(X) <=> p(Y).",
            "p(A);done(A)",
        ),
        "life-dependent" => (
            "loop() <=> loop(),(a();b()). left() \\ a() <=> true. right() \\ b() <=> true. left(),b() <=> fail. right(),a() <=> fail.",
            "((left();right()),loop());done(A)",
        ),
        "life-snapshot" | "life-history" => ("loop(X) <=> loop(X).", "keep(A),loop(A)"),
        "life-history-choice" => ("loop(X) <=> loop(X).", "keep(A),(true;true),loop(A)"),
        _ => return Err(format!("unknown lifecycle case {case}")),
    };
    let t = Instant::now();
    let code = Arc::new(
        prepare(
            &parse_program(program).map_err(|e| e.to_string())?,
            &parse_query(query).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
    );
    let mut e = Engine::with_history(code, case.starts_with("life-history"));
    println!(
        "case={case} size={n} prepare_init_ms={:.3} milestone_end={last}",
        ms(t.elapsed())
    );
    let mut b = Budget::new(limit, timeout);
    let mut reader = Reader::default();
    let mut first_event = None;
    let mut first_answer = None;
    let mut first_answer_tick = None;
    let mut held = None;
    let mut previous = (0, 0, Duration::ZERO, 0);
    let mut target = n as u64;
    let mut peak = [0; 7];
    loop {
        if !b.step(&mut e) {
            break;
        }
        if let Some(o) = e.take_output() {
            first_event.get_or_insert_with(|| b.start.elapsed());
            let t = observation::start(b.detailed);
            let end = reader.push(o, e.program().signatures(), &["done"], 1)?;
            b.validator += observation::elapsed(t);
            if end {
                first_answer.get_or_insert_with(|| b.start.elapsed());
                first_answer_tick.get_or_insert(b.ticks);
            }
            check(
                !retained && reader.answers <= 1,
                "unexpected continuing-stream answer",
            )?;
        }
        let milestone = e.applications() >= target;
        let sample = (b.detailed || b.ticks.is_multiple_of(2048) || milestone).then(|| e.memory());
        if let Some(m) = sample {
            for (p, v) in peak.iter_mut().zip([
                m.graph_nodes,
                m.occurrences,
                m.history_records,
                m.choices,
                m.coordinate_records,
                m.snapshots,
                m.release_batches,
            ]) {
                *p = (*p).max(v);
            }
        }
        if retained && held.is_none() && e.applications() >= n as u64 {
            let relation = e
                .program()
                .signatures()
                .iter()
                .position(|s| s.name == "loop")
                .unwrap();
            if e.facts(relation)
                .map_err(|e| format!("{e:?}"))?
                .next()
                .is_some()
            {
                match e.capture_snapshot() {
                    Ok(id) => held = Some(id),
                    Err(InspectionError::Busy | InspectionError::Initializing) => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
        if milestone {
            let m = sample.expect("milestone sampled");
            let elapsed = b.start.elapsed();
            println!(
                "window_apps={}..{} window_ticks={} window_ms={:.3} window_collection_ticks={} memory={m:?}",
                previous.0,
                e.applications(),
                b.ticks - previous.1,
                ms(elapsed - previous.2),
                observation::count(b.detailed, b.collection_ticks - previous.3)
            );
            previous = (e.applications(), b.ticks, elapsed, b.collection_ticks);
            target = target.saturating_mul(2);
        }
        if e.applications() >= last
            && (if retained {
                held.is_some()
            } else {
                reader.answers == 1
            })
        {
            break;
        }
    }
    let complete = b.in_time()
        && e.applications() >= last
        && !e.exhausted()
        && (if retained {
            held.is_some()
        } else {
            reader.answers == 1 && !reader.open
        });
    println!(
        "source_status={} apps={} answers={} ticks={} elapsed_ms={:.3} validator_ms={} first_event_ms={:?} first_answer_ms={:?} max_step_ms={} collection_ticks={} collections={} peak_graph_occurrences_history_choices_coordinates_snapshots_release={peak:?}",
        if complete {
            "PREFIX_COMPLETE"
        } else {
            "INCOMPLETE"
        },
        e.applications(),
        reader.answers,
        b.ticks,
        ms(b.start.elapsed()),
        observation::milliseconds(b.detailed, b.validator),
        first_event.map(ms),
        first_answer.map(ms),
        observation::milliseconds(b.detailed, b.maximum),
        observation::count(b.detailed, b.collection_ticks),
        e.collections()
    );
    crate::report::emit(
        "phase",
        json!({"phase":"source", "complete":complete,
        "applications":e.applications(),"answers":reader.answers,"ticks":b.ticks,
        "elapsed_ms":ms(b.start.elapsed()),"first_answer_tick":first_answer_tick,
        "first_answer_ms":first_answer.map(ms),"first_event_ms":first_event.map(ms),
        "validator_ms":b.detailed.then(|| ms(b.validator)),
        "max_step_ms":b.detailed.then(|| ms(b.maximum)),
        "collection_status_ticks":b.detailed.then_some(b.collection_ticks),
        "memory":crate::report::memory(crate::memory(&e))}),
    );
    println!("first_answer_tick={first_answer_tick:?}");
    let mut checkpoint_done = false;
    if complete {
        let mut gc = Budget::new(limit, timeout);
        e.request_collection();
        while e.collecting() && gc.step(&mut e) {}
        checkpoint_done = !e.collecting() && gc.in_time();
        println!(
            "checkpoint_collection_done={checkpoint_done} checkpoint_collection_ticks={} checkpoint_collection_ms={:.3} checkpoint_memory={:?}",
            gc.ticks,
            ms(gc.start.elapsed()),
            e.memory()
        );
    }
    let canceled = cancel(&mut e, limit, timeout);
    let mut cleanup = canceled;
    if canceled {
        if let Some(id) = held {
            cleanup &= inspect(
                &mut e,
                id,
                if case == "life-history-choice" { 2 } else { 1 },
                limit,
                timeout,
            )?;
        }
        if cleanup {
            cleanup &= release(&mut e, limit, timeout)?;
        }
    }
    let t = Instant::now();
    drop(e);
    let dropped = t.elapsed();
    println!(
        "cleanup_status={} engine_drop_ms={:.3}",
        if cleanup { "COMPLETE" } else { "INCOMPLETE" },
        ms(dropped)
    );
    Ok(complete && checkpoint_done && cleanup)
}

// Hold an independently sized archive, then measure a fixed amount of ordinary
// execution. Captures use public APIs; automatic history remains off throughout.
fn archive(n: usize, limit: u64, timeout: Duration) -> Result<bool, String> {
    let code = Arc::new(
        prepare(
            &parse_program("loop(X) <=> loop(X).").map_err(|e| e.to_string())?,
            &parse_query("keep(A),loop(A)").map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
    );
    let mut e = Engine::new(code);
    let relation = e
        .program()
        .signatures()
        .iter()
        .position(|s| s.name == "loop")
        .unwrap();
    let mut setup = Budget::new(limit, timeout);
    let mut views = Vec::new();
    let mut last_capture = 0;
    while views.len() < n && setup.step(&mut e) {
        check(
            e.take_output().is_none(),
            "continuing archive source emitted an answer",
        )?;
        if e.applications() > last_capture
            && e.facts(relation)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            match e.capture_snapshot() {
                Ok(id) => {
                    views.push(id);
                    last_capture = e.applications();
                }
                Err(InspectionError::Busy | InspectionError::Initializing) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    let admitted = views.len() == n && setup.in_time();
    println!(
        "case=life-archive size={n} setup_done={admitted} setup_apps={} setup_ticks={} setup_ms={:.3} retained_snapshots={}",
        e.applications(),
        setup.ticks,
        ms(setup.start.elapsed()),
        views.len()
    );
    let initial = e.applications();
    let mut b = Budget::new(limit, timeout);
    while admitted && e.applications() - initial < 2048 && b.step(&mut e) {
        check(
            e.take_output().is_none(),
            "continuing archive source emitted an answer",
        )?;
    }
    let complete = admitted && e.applications() - initial == 2048 && b.in_time();
    check(
        e.snapshots().count() == views.len(),
        "fixed archive changed size",
    )?;
    println!(
        "source_status={} continuation_apps={} continuation_ticks={} continuation_ms={:.3} collection_ticks={} memory={:?}",
        if complete {
            "PREFIX_COMPLETE"
        } else {
            "INCOMPLETE"
        },
        e.applications() - initial,
        b.ticks,
        ms(b.start.elapsed()),
        observation::count(b.detailed, b.collection_ticks),
        e.memory()
    );
    crate::report::emit(
        "phase",
        json!({"phase":"archive_source","complete":complete,
        "setup_complete":admitted,"setup_ticks":setup.ticks,"retained_snapshots":views.len(),
        "continued_applications":e.applications()-initial,"ticks":b.ticks,
        "elapsed_ms":ms(b.start.elapsed()),"memory":crate::report::memory(crate::memory(&e))}),
    );
    let mut clean = cancel(&mut e, limit, timeout);
    if clean {
        // Validate both ends of the archive after all subsequent work and cancel.
        for index in [0, views.len().saturating_sub(1)] {
            if let Some(&id) = views.get(index) {
                clean &= inspect(&mut e, id, 1, limit, timeout)?;
            }
        }
        if clean {
            clean &= release(&mut e, limit, timeout)?;
        }
    }
    Ok(complete && clean)
}

fn request(r: &Runtime, path: &str, body: &Value) -> Result<Value, String> {
    let response = r.request(path, &body.to_string());
    check(
        response.status == 200,
        &format!("{path}: {} {}", response.status, response.body),
    )?;
    Ok(response.body)
}
fn owner(r: &Runtime, boot: &Value) -> Result<Value, String> {
    let owner = request(r, "/api/reserve", &json!({"boot":boot}))?["owner"].clone();
    request(r, "/api/attach", &json!({"boot":boot,"owner":owner}))?;
    Ok(owner)
}
fn event(v: &Value) -> Result<Output, String> {
    let id = |field: &str| {
        v[field]
            .as_u64()
            .or_else(|| v[field].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| format!("invalid {field}"))
    };
    Ok(match v["kind"].as_str().ok_or("missing kind")? {
        "begin" => Output::Begin {
            completion: id("completion")?,
            alternative: id("alternative")?,
        },
        "variable" => Output::Variable {
            slot: id("slot")? as usize,
            variable: id("variable")?,
        },
        "fact" => Output::Fact {
            occurrence: id("occurrence")?,
            relation: id("relation")? as usize,
        },
        "port" => Output::Port {
            variable: id("variable")?,
        },
        "end_fact" => Output::EndFact,
        "end" => Output::End,
        _ => return Err(format!("unexpected runtime event {v}")),
    })
}
fn runtime(n: usize, limit: u64, timeout: Duration) -> Result<bool, String> {
    let r = Runtime::default();
    let boot = request(&r, "/api/hello", &json!({}))?["boot"].clone();
    let heavy = owner(&r, &boot)?;
    let tiny = owner(&r, &boot)?;
    let query = format!(
        "({}),{}",
        vec!["true"; n].join(";"),
        (0..n)
            .map(|i| format!("p(V{i})"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut model = request(&r, "/api/parse", &json!({"program":REWRITE,"query":query}))?;
    model["boot"] = boot.clone();
    model["owner"] = heavy.clone();
    model["command"] = json!(1);
    let mut tiny_model = request(&r, "/api/parse", &json!({"program":"","query":"done(A)"}))?;
    tiny_model["boot"] = boot.clone();
    tiny_model["owner"] = tiny.clone();
    tiny_model["command"] = json!(1);
    let t = Instant::now();
    let started = request(&r, "/api/start", &model)?;
    let admission = t.elapsed();
    let mut heavy_req = json!({"boot":boot,"owner":heavy,"run":started["run"],"budget":4096});
    let t = Instant::now();
    check(
        request(&r, "/api/start", &model)? == started,
        "start replay admitted another run",
    )?;
    let replay = t.elapsed();
    let tiny_start = Instant::now();
    let tiny_started = request(&r, "/api/start", &tiny_model)?;
    let tiny_admission = tiny_start.elapsed();
    let mut tiny_req = json!({"boot":boot,"owner":tiny,"run":tiny_started["run"],"budget":4096});
    let signatures = |start: &Value| -> Result<Vec<Signature>, String> {
        start["signatures"]
            .as_array()
            .ok_or("missing signatures")?
            .iter()
            .map(|s| {
                Ok(Signature {
                    name: s["name"].as_str().ok_or("missing relation name")?.into(),
                    arity: s["arity"].as_u64().ok_or("missing arity")? as usize,
                })
            })
            .collect()
    };
    let tiny_signatures = signatures(&tiny_started)?;
    let signatures = signatures(&started)?;
    let mut b = Budget::new(limit, timeout);
    let mut tiny_reader = Reader::default();
    let mut tiny_latency = None;
    let mut tiny_turn = None;
    let mut heavy_done = false;
    while b.runtime_tick(&r) {
        if tiny_latency.is_none() {
            let out = request(&r, "/api/output", &tiny_req)?;
            tiny_req["ack"] = out["sequence"].clone();
            let t = observation::start(b.detailed);
            for v in out["events"].as_array().ok_or("missing events")? {
                if tiny_reader.push(event(v)?, &tiny_signatures, &["done"], 1)? {
                    tiny_latency = Some(tiny_start.elapsed());
                    tiny_turn = Some(b.ticks);
                }
            }
            b.validator += observation::elapsed(t);
        }
        let status = request(&r, "/api/status", &heavy_req)?;
        check(status["error"].is_null(), "runtime source error")?;
        heavy_done = status["execution_done"] == true;
        if heavy_done && tiny_latency.is_some() {
            break;
        }
    }
    let delayed = b.start.elapsed();
    let production_turns = b.ticks;
    let mut reader = Reader::default();
    let mut bytes = 0;
    let mut batches = 0;
    let mut complete = false;
    let delivery = Instant::now();
    let mut first_event = None;
    let mut first_answer = None;
    // No scheduler calls during reads: all source and projection work must have
    // finished while the heavy consumer was absent.
    while heavy_done && b.available() {
        let out = request(&r, "/api/output", &heavy_req)?;
        b.ticks += 1;
        if batches == 0 {
            check(
                request(&r, "/api/output", &heavy_req)? == out,
                "output replay changed batch",
            )?;
        }
        heavy_req["ack"] = out["sequence"].clone();
        batches += 1;
        let t = observation::start(b.detailed);
        for v in out["events"].as_array().ok_or("missing events")? {
            first_event.get_or_insert_with(|| delivery.elapsed());
            bytes += serde_json::to_vec(v).map_err(|e| e.to_string())?.len() + 1;
            if reader.push(event(v)?, &signatures, &["done"], n)? {
                first_answer.get_or_insert_with(|| delivery.elapsed());
            }
        }
        b.validator += observation::elapsed(t);
        check(
            out["applications"] == json!(2 * n),
            "shared application count mismatch",
        )?;
        if out["delivery_done"] == true {
            complete = true;
            break;
        }
    }
    complete &= b.in_time();
    let delivery_elapsed = delivery.elapsed();
    if complete {
        check(
            reader.answers == n && !reader.open && tiny_reader.answers == 1 && !tiny_reader.open,
            "runtime answer multiplicity",
        )?;
    }
    println!(
        "case=runtime size={n} source_status={} admission_ms={:.3} start_replay_ms={:.3} tiny_admission_ms={:.3} tiny_answer_ms={:?} delayed_read_ms={:.3} delivery_ms={:.3} first_read_event_ms={:?} first_read_answer_ms={:?} validator_ms={} max_scheduler_tick_ms={} scheduler_and_read_work={} batches={batches} answers={} event_json_bytes={bytes}",
        if complete { "COMPLETE" } else { "INCOMPLETE" },
        ms(admission),
        ms(replay),
        ms(tiny_admission),
        tiny_latency.map(ms),
        ms(delayed),
        ms(delivery.elapsed()),
        first_event.map(ms),
        first_answer.map(ms),
        observation::milliseconds(b.detailed, b.validator),
        observation::milliseconds(b.detailed, b.maximum),
        b.ticks,
        reader.answers
    );
    let status = request(&r, "/api/status", &heavy_req)?;
    println!(
        "runtime_production_turns={production_turns} tiny_answer_turn={tiny_turn:?} source_apps={} source_memory={}",
        status["applications"], status["memory"]
    );
    let mut cleanup = Budget::new(limit, timeout);
    request(&r, "/api/close", &heavy_req)?;
    request(&r, "/api/close", &tiny_req)?;
    let mut closed = false;
    while cleanup.runtime_tick(&r) {
        if request(&r, "/api/close", &heavy_req)?["closed"] == true
            && request(&r, "/api/close", &tiny_req)?["closed"] == true
        {
            closed = true;
            break;
        }
    }
    closed &= cleanup.in_time();
    if closed {
        check(
            request(&r, "/api/retire", &json!({"boot":boot,"owner":heavy}))?["retired"] == true,
            "heavy owner retains run",
        )?;
        check(
            request(&r, "/api/retire", &json!({"boot":boot,"owner":tiny}))?["retired"] == true,
            "tiny owner retains run",
        )?;
    }
    println!(
        "cleanup_status={} close_ms={:.3} close_scheduler_ticks={} close_max_tick_ms={}",
        if closed { "COMPLETE" } else { "INCOMPLETE" },
        ms(cleanup.start.elapsed()),
        cleanup.ticks,
        observation::milliseconds(cleanup.detailed, cleanup.maximum)
    );
    crate::report::emit(
        "phase",
        json!({"phase":"runtime","complete":complete,"closed":closed,
        "answers":reader.answers,"batches":batches,"event_json_bytes":bytes,
        "production_turns":production_turns,"tiny_answer_turn":tiny_turn,
        "tiny_answer_ms":tiny_latency.map(ms),"delivery_ms":ms(delivery_elapsed),
        "close_ticks":cleanup.ticks,"close_ms":ms(cleanup.start.elapsed())}),
    );
    Ok(complete && closed)
}

pub fn run(case: &str, n: usize, max_ticks: u64, timeout: Duration) -> Result<bool, String> {
    run_options(case, n, InteractionOptions::default(), max_ticks, timeout)
}

/// SIZE selects siblings, retained snapshots or concurrent inspections.
/// Options independently select view width, continued applications and rotation cadence.
pub fn run_options(
    case: &str,
    n: usize,
    options: InteractionOptions,
    max_ticks: u64,
    timeout: Duration,
) -> Result<bool, String> {
    let result = run_inner(case, n, options, max_ticks, timeout);
    crate::report::emit(
        "result",
        json!({"case":case,"size":n,
        "rows":options.rows,"work":options.work,"cadence":options.cadence,
        "status":match &result { Ok(true) => if case == "runtime" { "COMPLETE" } else { "PREFIX_COMPLETE" }, Ok(false) => "INCOMPLETE", Err(_) => "INVALID" },
        "censored":matches!(result, Ok(false)),"error":result.as_ref().err(),
        "max_ticks":max_ticks,"timeout_seconds":timeout.as_secs_f64()}),
    );
    result
}
fn run_inner(
    case: &str,
    n: usize,
    options: InteractionOptions,
    max_ticks: u64,
    timeout: Duration,
) -> Result<bool, String> {
    check(
        n > 0 && max_ticks > 0 && !timeout.is_zero(),
        "positive size and budgets required",
    )?;
    println!(
        "lifecycle_limits apply independently to source, checkpoint collection, cancellation, inspection and release; ticks are Engine::advance(1), runtime work is scheduler turns/API reads; collection_ticks counts ticks entered with collection requested/active; timings include checks; memory is object counts, not bytes; runtime excludes HTTP/browser"
    );
    if matches!(
        case,
        "life-held-output" | "life-archive-fixed" | "life-archive-rotate" | "life-inspections"
    ) {
        interactions::run(case, n, options, max_ticks, timeout)
    } else if case == "life-archive" {
        archive(n, max_ticks, timeout)
    } else if case == "runtime" {
        runtime(n, max_ticks, timeout)
    } else {
        stream(case, n, max_ticks, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oracle_rejects_duplicate_answer_and_wrong_ports() {
        let signatures = [Signature {
            name: "done".into(),
            arity: 1,
        }];
        for port in [7, 8] {
            let mut r = Reader::default();
            for event in [
                Output::Begin {
                    completion: 1,
                    alternative: 0,
                },
                Output::Variable {
                    slot: 0,
                    variable: 7,
                },
                Output::Fact {
                    occurrence: 1,
                    relation: 0,
                },
                Output::Port { variable: port },
                Output::EndFact,
            ] {
                r.push(event, &signatures, &["done"], 1).unwrap();
            }
            assert_eq!(
                r.push(Output::End, &signatures, &["done"], 1).is_ok(),
                port == 7
            );
            if port == 7 {
                assert!(!r.open);
                assert!(
                    r.push(
                        Output::Begin {
                            completion: 1,
                            alternative: 0
                        },
                        &signatures,
                        &["done"],
                        1
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn lifecycle_and_runtime_smoke() {
        for case in CASES.split_whitespace() {
            assert!(
                run(case, 4, 2_000_000, Duration::from_secs(20)).unwrap(),
                "{case}"
            );
        }
        assert!(!run("life-alias", 4, 1, Duration::from_secs(1)).unwrap());
    }
}
