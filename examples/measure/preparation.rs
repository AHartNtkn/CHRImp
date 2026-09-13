//! Preparation shape and sequential reuse, through the public text/prepare/Engine APIs.
//!
//! `families::prepare` varies unary inactive rule count. Here SIZE still counts
//! inactive rules; Options vary heads per rule, arity, repeated head ports, body
//! width/nesting, and engine uses independently. Heads of a rule share one
//! absent relation; no query can activate them. Repeating ports exercises the
//! variable-use/merge activation planning in program::prepare, while distinct
//! ports exercise slot assignment. Body width adds duplicate zero-port posts;
//! explicit singleton conjunctions preserve nesting in syntax::Body and the
//! prepared instruction tree. Body code is prepared but never executed.
//!
//! Both cases generate/parse once. `prepare-independent` prepares the same AST
//! for each fresh engine; `prepare-reuse` prepares once and clones its Arc for
//! each fresh engine. Engine destruction precedes the next use in both cases.
//! A retained harness Arc separates engine destruction from final Prepared
//! destruction. No cancellation sweep precedes these direct drops.
//!
//! Timing: generation, parsing, each preparation, engine initialization, full
//! delivery/validation, direct engine drop, final Prepared drops, and syntax/
//! source drop are separate. Use time includes scalar reading and validation;
//! validator/per-step clocks require observation::detailed(). The aggregate
//! tick budget counts source advances across all uses; the wall budget covers
//! the whole call, including direct destruction. Checks cannot interrupt parse,
//! prepare, a tick, or Drop: campaigns still require external supervision.
use crate::{
    Reader,
    allocation::{Phase, during},
    families::{self, Workload},
    ms, observation,
};
use chr::{
    engine::Engine,
    program::{Prepared, prepare},
    syntax::{parse_program, parse_query},
};
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub const CASES: &str = "prepare-reuse prepare-independent";

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Options {
    pub heads: usize,
    pub arity: usize,
    /// Extra uses of X0 among all head ports: 0 means every port is distinct.
    /// Must be less than heads*arity; only 0 is valid when arity is zero.
    pub repeats: usize,
    /// Number of inactive sink() posts; zero uses true.
    pub width: usize,
    /// Singleton conjunction containers above the flat body. Total <=128.
    pub depth: usize,
    /// Number of sequential fresh engines, independently of SIZE.
    pub uses: usize,
    /// true queries one empty answer; false rewrites start(A) to p(A).
    pub empty: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            heads: 1,
            arity: 1,
            repeats: 0,
            width: 1,
            depth: 0,
            uses: 1,
            empty: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Use {
    pub complete: bool,
    pub ticks: u64,
    pub applications: u64,
    pub answers: usize,
    pub engine_init_ms: f64,
    pub use_ms: f64,
    pub engine_drop_ms: f64,
    pub validator_ms: Option<f64>,
    pub max_step_ms: Option<f64>,
    pub prepared_owners_after_engine_drop: usize,
}
#[derive(Debug, Serialize)]
pub struct Measurement {
    pub case: String,
    pub size: usize,
    pub options: Options,
    pub status: &'static str,
    pub censored: bool,
    pub error: Option<String>,
    pub max_ticks: u64,
    pub timeout_seconds: f64,
    pub program_bytes: usize,
    pub query_bytes: usize,
    pub generation_ms: f64,
    pub parse_program_ms: Option<f64>,
    pub parse_query_ms: Option<f64>,
    pub prepare_ms: Vec<f64>,
    pub uses: Vec<Use>,
    pub prepared_drop_ms: Vec<f64>,
    pub prepared_released: bool,
    pub syntax_source_drop_ms: f64,
    pub elapsed_ms: f64,
}
fn timed<T>(phase: Phase, f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let result = during(phase, f);
    (result, ms(start.elapsed()))
}

/// Legal source, with all dimensions checked before allocating generated text.
/// SIZE=0 is the no-inactive-rule control; heads and uses must remain positive.
pub fn source(n: usize, o: Options) -> Result<(String, &'static str), String> {
    let ports = o.heads.checked_mul(o.arity).ok_or("head ports overflow")?;
    let containers = o
        .depth
        .checked_add(usize::from(o.width > 1))
        .ok_or("body depth overflow")?;
    if o.heads == 0
        || o.uses == 0
        || containers > 128
        || (ports == 0 && o.repeats != 0)
        || (ports > 0 && o.repeats >= ports)
    {
        return Err("positive heads/uses, valid repeated-port count, and at most 128 body containers required".into());
    }
    // Check the generated shape's arithmetic, not an arbitrary benchmark cap.
    n.checked_mul(
        o.heads
            .checked_add(ports)
            .and_then(|v| v.checked_add(o.width))
            .and_then(|v| v.checked_add(o.depth))
            .and_then(|v| v.checked_add(1))
            .ok_or("rule shape overflow")?,
    )
    .ok_or("program shape overflow")?;
    if n == 0 {
        return Ok((
            if o.empty {
                String::new()
            } else {
                "start(X) <=> p(X).".into()
            },
            if o.empty { "true" } else { "start(A)" },
        ));
    }
    let mut body = if o.width == 0 {
        "true".into()
    } else {
        vec!["sink()"; o.width].join(",")
    };
    if o.width > 1 {
        body = format!("({body})");
    }
    for _ in 0..o.depth {
        body = format!("({body},)");
    }
    let mut program = if o.empty {
        String::new()
    } else {
        "start(X) <=> p(X).".into()
    };
    for rule in 0..n {
        let heads = (0..o.heads)
            .map(|head| {
                let args = (0..o.arity)
                    .map(|port| {
                        let position = head * o.arity + port;
                        let slot = position.saturating_sub(o.repeats);
                        format!("X{slot}")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("unused{rule}({args})")
            })
            .collect::<Vec<_>>()
            .join(",");
        program += &format!("{heads} ==> {body}.");
    }
    Ok((program, if o.empty { "true" } else { "start(A)" }))
}

fn oracle(empty: bool) -> Workload {
    // Existing analytic oracles require one empty answer or exactly p(A).
    // Their expected result never comes from the generated inactive rules.
    families::make(
        if empty { "answers" } else { "prepare" },
        usize::from(empty),
        0,
    )
    .unwrap()
}
fn execute(
    code: &Arc<Prepared>,
    empty: bool,
    remaining: u64,
    start: Instant,
    timeout: Duration,
) -> (Use, Option<String>) {
    let mut workload = during(Phase::Validator, || oracle(empty));
    let (mut e, engine_init_ms) = timed(Phase::Engine, || Engine::new(Arc::clone(code)));
    let detailed = observation::detailed();
    let mut reader = Reader::default();
    let mut ticks = 0u64;
    let mut validator = Duration::ZERO;
    let mut maximum = Duration::ZERO;
    let mut error = None;
    let use_start = Instant::now();
    while !e.delivery_done() && ticks < remaining {
        if ticks.is_multiple_of(2048) && start.elapsed() >= timeout {
            break;
        }
        let tick = observation::start(detailed);
        during(Phase::Engine, || e.advance(1));
        maximum = maximum.max(observation::elapsed(tick));
        ticks += 1;
        if let Some(event) = during(Phase::Delivery, || e.take_output()) {
            let t = observation::start(detailed);
            let result = during(Phase::Validator, || reader.push(event, &e, &mut workload));
            validator += observation::elapsed(t);
            if let Err(why) = result {
                error = Some(why);
                break;
            }
        }
    }
    if e.delivery_done() && error.is_none() && (reader.answers != 1 || reader.open) {
        error = Some("expected exactly one complete normal form".into());
    }
    if e.delivery_done() && error.is_none() && e.applications() != u64::from(!empty) {
        error = Some("expected zero empty-query applications or one start rewrite".into());
    }
    let complete = e.delivery_done() && error.is_none() && start.elapsed() < timeout;
    let use_ms = ms(use_start.elapsed());
    let applications = e.applications();
    let (_, engine_drop_ms) = timed(Phase::Cleanup, || drop(e));
    let owners = Arc::strong_count(code);
    if owners != 1 {
        error = Some("engine destruction retained Prepared ownership".into());
    }
    (
        Use {
            complete,
            ticks,
            applications,
            answers: reader.answers,
            engine_init_ms,
            use_ms,
            engine_drop_ms,
            validator_ms: detailed.then(|| ms(validator)),
            max_step_ms: detailed.then(|| ms(maximum)),
            prepared_owners_after_engine_drop: owners,
        },
        error,
    )
}
fn drop_prepared(code: Arc<Prepared>, m: &mut Measurement) {
    let weak = Arc::downgrade(&code);
    let (released, elapsed) = timed(Phase::Cleanup, || {
        drop(code);
        let released = weak.strong_count() == 0;
        // Include the final Arc allocation release in this destruction phase.
        drop(weak);
        released
    });
    m.prepared_drop_ms.push(elapsed);
    m.prepared_released &= released;
}

/// Returns typed data for PM's `report::emit("result", json!(measurement))`.
/// Configuration/parse errors are Err; use failures and censoring retain samples.
pub fn run(
    case: &str,
    n: usize,
    options: Options,
    max_ticks: u64,
    timeout: Duration,
) -> Result<Measurement, String> {
    if !CASES.split_whitespace().any(|c| c == case) || max_ticks == 0 || timeout.is_zero() {
        return Err("known preparation case and positive budgets required".into());
    }
    let start = Instant::now();
    let (generated, generation_ms) = timed(Phase::Setup, || source(n, options));
    let (program, query) = generated?;
    let mut m = Measurement {
        case: case.into(),
        size: n,
        options,
        status: "INCOMPLETE",
        censored: true,
        error: None,
        max_ticks,
        timeout_seconds: timeout.as_secs_f64(),
        program_bytes: program.len(),
        query_bytes: query.len(),
        generation_ms,
        parse_program_ms: None,
        parse_query_ms: None,
        prepare_ms: vec![],
        uses: vec![],
        prepared_drop_ms: vec![],
        prepared_released: true,
        syntax_source_drop_ms: 0.0,
        elapsed_ms: 0.0,
    };
    let mut ast = None;
    let mut query_ast = None;
    if start.elapsed() < timeout {
        let (parsed, elapsed) = timed(Phase::Setup, || parse_program(&program));
        m.parse_program_ms = Some(elapsed);
        ast = Some(parsed.map_err(|e| e.to_string())?);
    }
    if ast.is_some() && start.elapsed() < timeout {
        let (parsed, elapsed) = timed(Phase::Setup, || parse_query(query));
        m.parse_query_ms = Some(elapsed);
        query_ast = Some(parsed.map_err(|e| e.to_string())?);
    }
    let mut shared = None;
    let mut remaining = max_ticks;
    for _ in 0..options.uses {
        if remaining == 0 || start.elapsed() >= timeout || query_ast.is_none() {
            break;
        }
        if shared.is_none() {
            let (prepared, elapsed) = timed(Phase::Setup, || {
                prepare(ast.as_ref().unwrap(), query_ast.as_ref().unwrap()).map(Arc::new)
            });
            m.prepare_ms.push(elapsed);
            match prepared {
                Ok(code) => shared = Some(code),
                Err(e) => {
                    m.error = Some(e.to_string());
                    break;
                }
            }
        }
        if start.elapsed() >= timeout {
            break;
        }
        let (sample, error) = execute(
            shared.as_ref().unwrap(),
            options.empty,
            remaining,
            start,
            timeout,
        );
        remaining -= sample.ticks;
        let complete = sample.complete;
        m.uses.push(sample);
        m.error = error;
        if case == "prepare-independent" {
            drop_prepared(shared.take().unwrap(), &mut m);
        }
        if !complete || m.error.is_some() {
            break;
        }
    }
    if let Some(code) = shared {
        drop_prepared(code, &mut m);
    }
    let (_, elapsed) = timed(Phase::Cleanup, || drop((ast, query_ast, program)));
    m.syntax_source_drop_ms = elapsed;
    if !m.prepared_released {
        m.error = Some("final Prepared owner retained".into());
    }
    m.elapsed_ms = ms(start.elapsed());
    let complete = m.uses.len() == options.uses
        && m.uses.iter().all(|s| s.complete)
        && start.elapsed() < timeout;
    m.status = if m.error.is_some() {
        "INVALID"
    } else if complete {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    };
    m.censored = m.status == "INCOMPLETE";
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chr::program::Instruction;

    #[test]
    fn shape_axes_reach_public_preparation_plans() {
        let options = Options {
            heads: 3,
            arity: 2,
            repeats: 3,
            width: 4,
            depth: 5,
            empty: true,
            ..Options::default()
        };
        let (p, q) = source(2, options).unwrap();
        let code = prepare(&parse_program(&p).unwrap(), &parse_query(q).unwrap()).unwrap();
        assert_eq!(code.rules().len(), 2);
        for rule in code.rules() {
            assert_eq!(rule.heads.len(), 3);
            assert_eq!(
                rule.heads
                    .iter()
                    .map(|h| h.args.clone())
                    .collect::<Vec<_>>(),
                [vec![0, 0], vec![0, 0], vec![1, 2]]
            );
            let mut body = rule.body;
            for _ in 0..5 {
                let Instruction::And(items) = &code.instructions()[body] else {
                    panic!("missing nesting");
                };
                assert_eq!(items.len(), 1);
                body = items[0];
            }
            let Instruction::And(items) = &code.instructions()[body] else {
                panic!("missing width");
            };
            assert_eq!(items.len(), 4);
        }
        assert!(
            source(
                1,
                Options {
                    depth: 129,
                    ..Options::default()
                }
            )
            .is_err()
        );
        assert!(
            source(
                1,
                Options {
                    repeats: 1,
                    ..Options::default()
                }
            )
            .is_err()
        );
        let (p, q) = source(
            1,
            Options {
                depth: 128,
                width: 0,
                arity: 0,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(prepare(&parse_program(&p).unwrap(), &parse_query(q).unwrap()).is_ok());
    }

    #[test]
    fn both_modes_validate_every_fresh_engine_and_release_prepared() {
        for case in CASES.split_whitespace() {
            for empty in [false, true] {
                for n in [0, 2] {
                    let options = Options {
                        heads: 2,
                        arity: 3,
                        repeats: 2,
                        width: 3,
                        depth: 2,
                        uses: 3,
                        empty,
                    };
                    let m = run(case, n, options, 100_000, Duration::from_secs(10)).unwrap();
                    assert_eq!(m.status, "COMPLETE", "{m:?}");
                    assert!(
                        m.uses
                            .iter()
                            .all(|s| s.answers == 1 && s.prepared_owners_after_engine_drop == 1)
                    );
                    assert_eq!(
                        m.prepare_ms.len(),
                        if case == "prepare-reuse" { 1 } else { 3 }
                    );
                    assert_eq!(m.prepared_drop_ms.len(), m.prepare_ms.len());
                    assert!(m.prepared_released);
                }
            }
            let m = run(case, 1, Options::default(), 1, Duration::from_secs(10)).unwrap();
            assert_eq!(m.status, "INCOMPLETE");
            assert!(m.censored && m.prepared_released);
        }
    }

    #[test]
    fn expired_generation_does_not_admit_parsing() {
        let m = run(
            "prepare-reuse",
            64,
            Options::default(),
            100_000,
            Duration::from_nanos(1),
        )
        .unwrap();
        assert_eq!(m.status, "INCOMPLETE");
        assert!(m.parse_program_ms.is_none() && m.parse_query_ms.is_none());
        assert!(m.uses.is_empty() && m.prepare_ms.is_empty());
    }

    #[test]
    fn zero_rule_control_does_not_construct_unused_body() {
        let (program, query) = source(
            0,
            Options {
                width: usize::MAX / 2,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(program, "start(X) <=> p(X).");
        assert_eq!(query, "start(A)");
    }

    #[test]
    fn work_oracle_rejects_redundant_application_with_same_residual() {
        let code = Arc::new(
            prepare(
                &parse_program("start(X) <=> p(X). p(X) ==> true.").unwrap(),
                &parse_query("start(A)").unwrap(),
            )
            .unwrap(),
        );
        let (sample, error) = execute(
            &code,
            false,
            100_000,
            Instant::now(),
            Duration::from_secs(10),
        );
        assert_eq!(sample.answers, 1);
        assert_eq!(sample.applications, 2);
        assert!(error.is_some() && !sample.complete);
    }

    #[test]
    fn semantic_oracle_rejects_an_extra_residual() {
        let code = Arc::new(
            prepare(
                &parse_program("start(X) <=> p(X),leak().").unwrap(),
                &parse_query("start(A)").unwrap(),
            )
            .unwrap(),
        );
        let (sample, error) = execute(
            &code,
            false,
            100_000,
            Instant::now(),
            Duration::from_secs(10),
        );
        assert!(error.is_some());
        assert!(!sample.complete);
        assert_eq!(Arc::strong_count(&code), 1);
    }
}
