//! Native Runtime session populations and paced/replayed scalar delivery.
//!
//! Integrate as lifecycle::sessions so the existing Reader/Budget/event helpers
//! remain the single protocol/semantic validator. SIZE is finite answer count.
//! Closed sessions are completed, drained, closed and retired serially before
//! retained completed sessions are admitted. Both populations use `rows` facts.
//! The measured fresh pair is a SIZE-answer run and one tiny answer; optional
//! loop execution continues while batches are read/replayed. No browser/HTTP.
//!
//! Runtime::tick services one run per frozen scheduler round. Output budget is
//! scalar events (1..=4096), not bytes or engine ticks. Replay uses the SAME ack
//! after a background application advances, then checks the entire frozen batch
//! and live application invariance without any intervening tick. Replay responses
//! never enter the answer reader twice. work is an application lower bound:
//! Runtime scheduler quanta and replay progress may exceed it; actuals are kept.
//!
//! Spool files persist after delivery until close (notebook::Spool/Runtime::tick).
//! Linux /proc/self/fd observations are scoped by this Runtime's boot prefix;
//! bytes sum unique files, not both descriptors. These are logical file lengths,
//! not resident memory. Public memory objects, run invalidation, owner retirement
//! and scoped descriptor release complement each other. Other platforms expose
//! null spool evidence. Detailed API/tick/validator timing is opt-in.
//!
//! Setup/source share one work/time budget. Cleanup gets a separate equal budget
//! and also runs after censoring/errors. Work counts requests plus scheduler
//! turns. Synchronous requests/ticks/drop still need external supervision.
use super::{Budget, Reader, check, event};
use crate::{
    allocation::{Phase, during},
    ms, observation,
};
use chr::{
    notebook::{Response, Runtime},
    program::Signature,
};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const CASES: &str = "runtime-sessions";
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Options {
    pub closed: usize,
    pub retained: usize,
    pub rows: usize,
    pub batch: usize,
    /// Replay every Nth nonempty main batch; zero disables replays.
    pub replay_every: usize,
    /// Background application target; zero omits the background execution.
    pub work: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            closed: 0,
            retained: 0,
            rows: 1,
            batch: 1,
            replay_every: 0,
            work: 0,
        }
    }
}
#[derive(Debug)]
enum Stop {
    Censored,
    Invalid(String),
}
impl From<String> for Stop {
    fn from(s: String) -> Self {
        Self::Invalid(s)
    }
}
type Result<T> = std::result::Result<T, Stop>;

struct Meter {
    b: Budget,
    calls: u64,
    turns: u64,
    reads: u64,
    replays: u64,
    read_ms: Duration,
    replay_ms: Duration,
    bytes: usize,
}
impl Meter {
    fn new(limit: u64, timeout: Duration) -> Self {
        Self {
            b: Budget::new(limit, timeout),
            calls: 0,
            turns: 0,
            reads: 0,
            replays: 0,
            read_ms: Duration::ZERO,
            replay_ms: Duration::ZERO,
            bytes: 0,
        }
    }
    fn raw(&mut self, r: &Runtime, path: &str, body: &Value) -> Result<Response> {
        if !self.b.available() {
            return Err(Stop::Censored);
        }
        self.b.ticks += 1;
        self.calls += 1;
        let phase = if matches!(self.b.phase, Phase::Cleanup) {
            Phase::Cleanup
        } else {
            match path {
                "/api/parse" | "/api/start" | "/api/reserve" | "/api/attach" => Phase::Setup,
                "/api/output" => Phase::Delivery,
                "/api/status" => Phase::Validator,
                "/api/close" | "/api/retire" => Phase::Cleanup,
                _ => Phase::Engine,
            }
        };
        Ok(during(phase, || r.request(path, &body.to_string())))
    }
    fn call(&mut self, r: &Runtime, path: &str, body: &Value) -> Result<Value> {
        let response = self.raw(r, path, body)?;
        check(
            response.status == 200,
            &format!("{path}: {} {}", response.status, response.body),
        )?;
        Ok(response.body)
    }
    fn tick(&mut self, r: &Runtime) -> Result<()> {
        if !self.b.runtime_tick(r) {
            return Err(Stop::Censored);
        }
        self.turns += 1;
        Ok(())
    }
    fn data(&self) -> Value {
        json!({"work":self.b.ticks,"requests":self.calls,"scheduler_turns":self.turns,
            "elapsed_ms":ms(self.b.start.elapsed()),"reads":self.reads,"replays":self.replays,
            "delivered_event_json_bytes":self.bytes,
            "read_ms":self.b.detailed.then(||ms(self.read_ms)),
            "replay_ms":self.b.detailed.then(||ms(self.replay_ms)),
            "validator_ms":self.b.detailed.then(||ms(self.b.validator)),
            "max_tick_ms":self.b.detailed.then(||ms(self.b.maximum))})
    }
}
struct Session {
    req: Value,
    signatures: Vec<Signature>,
    reader: Reader,
    answers: usize,
    rows: usize,
    apps: u64,
    done: bool,
    batches: usize,
}
fn admit(
    r: &Runtime,
    boot: &Value,
    program: &str,
    query: &str,
    m: &mut Meter,
    owned: &mut Vec<Value>,
) -> Result<(Value, Value)> {
    let mut model = m.call(r, "/api/parse", &json!({"program":program,"query":query}))?;
    let owner = m.call(r, "/api/reserve", &json!({"boot":boot}))?["owner"].clone();
    m.call(r, "/api/attach", &json!({"boot":boot,"owner":owner}))?;
    let identity = json!({"boot":boot,"owner":owner,"run":0});
    owned.push(identity);
    model["boot"] = boot.clone();
    model["owner"] = owner;
    model["command"] = json!(1);
    let started = m.call(r, "/api/start", &model)?;
    owned.last_mut().unwrap()["run"] = started["run"].clone();
    Ok((owned.last().unwrap().clone(), started))
}
fn finite(
    r: &Runtime,
    boot: &Value,
    answers: usize,
    rows: usize,
    batch: usize,
    m: &mut Meter,
    owned: &mut Vec<Value>,
) -> Result<Session> {
    let query = format!(
        "({}),{}",
        vec!["true"; answers].join(";"),
        (0..rows)
            .map(|i| format!("p(V{i})"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let (mut req, started) = admit(r, boot, "p(X) <=> done(X).", &query, m, owned)?;
    req["budget"] = json!(batch);
    let signatures = started["signatures"]
        .as_array()
        .ok_or_else(|| Stop::Invalid("missing signatures".into()))?
        .iter()
        .map(|s| {
            Ok(Signature {
                name: s["name"]
                    .as_str()
                    .ok_or_else(|| Stop::Invalid("missing relation".into()))?
                    .into(),
                arity: s["arity"]
                    .as_u64()
                    .ok_or_else(|| Stop::Invalid("missing arity".into()))?
                    as usize,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Session {
        req,
        signatures,
        reader: Reader::default(),
        answers,
        rows,
        apps: rows as u64,
        done: false,
        batches: 0,
    })
}
fn status(r: &Runtime, id: &Value, m: &mut Meter) -> Result<Value> {
    let v = m.call(r, "/api/status", id)?;
    check(v["error"].is_null(), "runtime source error")?;
    Ok(v)
}
fn apps(v: &Value) -> Result<u64> {
    v["applications"]
        .as_u64()
        .ok_or_else(|| Stop::Invalid("missing application count".into()))
}
fn progress(r: &Runtime, id: &Value, target: u64, m: &mut Meter) -> Result<u64> {
    loop {
        let s = status(r, id, m)?;
        check(
            s["execution_done"] == false,
            "background source unexpectedly completed",
        )?;
        let current = apps(&s)?;
        if current >= target {
            return Ok(current);
        }
        m.tick(r)?;
    }
}
fn read(
    r: &Runtime,
    s: &mut Session,
    background: Option<&Value>,
    replay_every: usize,
    m: &mut Meter,
) -> Result<()> {
    let before = apps(&status(r, &s.req, m)?)?;
    let t = observation::start(m.b.detailed);
    let out = m.call(r, "/api/output", &s.req)?;
    m.read_ms += observation::elapsed(t);
    m.reads += 1;
    check(
        apps(&status(r, &s.req, m)?)? == before && apps(&out)? == before,
        "output read advanced source",
    )?;
    check(out["error"].is_null(), "output source error")?;
    let events = out["events"]
        .as_array()
        .ok_or_else(|| Stop::Invalid("missing events".into()))?;
    check(
        events.len() <= s.req["budget"].as_u64().unwrap() as usize,
        "oversized output batch",
    )?;
    if !events.is_empty() {
        s.batches += 1;
    }
    if !events.is_empty() && replay_every > 0 && s.batches.is_multiple_of(replay_every) {
        if let Some(bg) = background {
            let initial = apps(&status(r, bg, m)?)?;
            progress(
                r,
                bg,
                initial
                    .checked_add(1)
                    .ok_or_else(|| Stop::Invalid("application overflow".into()))?,
                m,
            )?;
        }
        let source_before = apps(&status(r, &s.req, m)?)?;
        let bg_before = background
            .map(|id| status(r, id, m).and_then(|v| apps(&v)))
            .transpose()?;
        let t = observation::start(m.b.detailed);
        let replay = m.call(r, "/api/output", &s.req)?;
        m.replay_ms += observation::elapsed(t);
        m.replays += 1;
        check(
            replay == out,
            "replay changed frozen scalar batch or metadata",
        )?;
        check(
            apps(&status(r, &s.req, m)?)? == source_before,
            "replay advanced finite source",
        )?;
        let bg_after = background
            .map(|id| status(r, id, m).and_then(|v| apps(&v)))
            .transpose()?;
        check(bg_before == bg_after, "replay advanced background source")?;
    }
    let t = observation::start(m.b.detailed);
    let valid = during(Phase::Validator, || -> std::result::Result<(), String> {
        for v in events {
            s.reader.push(event(v)?, &s.signatures, &["done"], s.rows)?;
        }
        check(s.reader.answers <= s.answers, "too many session answers")
    });
    m.b.validator += observation::elapsed(t);
    valid?;
    // Event bytes are measured once, excluding transport envelope and replays.
    m.bytes += events
        .iter()
        .map(|v| v.to_string().len() + 1)
        .sum::<usize>();
    s.req["ack"] = out["sequence"].clone();
    s.done = out["delivery_done"] == true;
    if s.done {
        check(
            s.reader.answers == s.answers && !s.reader.open && before == s.apps,
            "session final answer/applications mismatch",
        )?;
    }
    Ok(())
}
fn finish(r: &Runtime, s: &mut Session, m: &mut Meter) -> Result<()> {
    while !s.done {
        m.tick(r)?;
        read(r, s, None, 0, m)?;
    }
    Ok(())
}
fn close(r: &Runtime, id: &Value, m: &mut Meter) -> Result<()> {
    if id["run"] != json!(0) {
        loop {
            if m.call(r, "/api/close", id)?["closed"] == true {
                break;
            }
            m.tick(r)?;
        }
        check(
            m.raw(r, "/api/status", id)?.status == 404,
            "closed run still publicly owned",
        )?;
    }
    check(
        m.call(
            r,
            "/api/retire",
            &json!({"boot":id["boot"],"owner":id["owner"]}),
        )?["retired"]
            == true,
        "owner retains execution after close",
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
struct Spools {
    files: usize,
    descriptors: usize,
    logical_bytes: u64,
}
fn spools(boot: &Value) -> std::result::Result<Option<Spools>, String> {
    #[cfg(target_os = "linux")]
    {
        let prefix = format!("chr-{}-", boot.as_str().ok_or("invalid boot")?);
        let mut files = BTreeMap::new();
        let mut descriptors = 0;
        for entry in std::fs::read_dir("/proc/self/fd").map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let Ok(target) = std::fs::read_link(entry.path()) else {
                continue;
            };
            if target
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            {
                descriptors += 1;
                files.insert(
                    target,
                    std::fs::metadata(entry.path())
                        .map_err(|e| e.to_string())?
                        .len(),
                );
            }
        }
        Ok(Some(Spools {
            files: files.len(),
            descriptors,
            logical_bytes: files.values().sum(),
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = boot;
        Ok(None)
    }
}
fn sample(boot: &Value, expected: usize) -> Result<Value> {
    let sample = spools(boot)?;
    if let Some(s) = &sample {
        check(
            s.files == expected && (expected != 0 || s.descriptors == 0),
            "runtime spool ownership mismatch",
        )?;
    }
    Ok(json!(sample))
}

/// PM: lifecycle::sessions::run(SIZE, Options, MAX_WORK, TIMEOUT), then emit the
/// returned Value as result; COMPLETE => Ok(true), INCOMPLETE => Ok(false),
/// INVALID => Err(error). Configuration errors return Err immediately.
pub fn run(
    n: usize,
    o: Options,
    limit: u64,
    timeout: Duration,
) -> std::result::Result<Value, String> {
    if n == 0 || o.rows == 0 || o.batch == 0 || o.batch > 4096 || limit == 0 || timeout.is_zero() {
        return Err("positive answers/rows/budgets and batch 1..=4096 required".into());
    }
    o.closed
        .checked_add(o.retained)
        .and_then(|v| v.checked_add(3))
        .and_then(|v| v.checked_mul(o.rows))
        .and_then(|v| v.checked_add(n))
        .ok_or("session dimensions overflow")?;
    let total = Instant::now();
    let r = Runtime::default();
    let mut m = Meter::new(limit, timeout);
    let mut owned = Vec::new();
    let mut boot = Value::Null;
    let mut result = json!({"case":"runtime-sessions","size":n,"options":o,"max_work":limit,
        "timeout_seconds":timeout.as_secs_f64(),"status":"INCOMPLETE","censored":true,
        "error":null,"history_completed":0,"retained_completed":0,"answers":0,"tiny_answers":0,
        "background_applications":0,"cleanup_complete":false,"closed_verified":0});
    let mut source_begin = None;
    let attempt = (|| -> Result<()> {
        boot = m.call(&r, "/api/hello", &json!({}))?["boot"].clone();
        for _ in 0..o.closed {
            let mut s = finite(&r, &boot, 1, o.rows, o.batch, &mut m, &mut owned)?;
            finish(&r, &mut s, &mut m)?;
            close(&r, owned.last().unwrap(), &mut m)?;
            owned.pop();
            result["history_completed"] = json!(result["history_completed"].as_u64().unwrap() + 1);
        }
        result["after_history_spools"] = sample(&boot, 0)?;
        for _ in 0..o.retained {
            let mut s = finite(&r, &boot, 1, o.rows, o.batch, &mut m, &mut owned)?;
            finish(&r, &mut s, &mut m)?;
            let reply = m.raw(
                &r,
                "/api/retire",
                &json!({"boot":boot,"owner":s.req["owner"]}),
            )?;
            check(
                reply.status == 409 && reply.body["code"] == "owner_busy",
                "completed retained owner lost execution",
            )?;
            result["retained_completed"] =
                json!(result["retained_completed"].as_u64().unwrap() + 1);
        }
        result["retained_spools"] = sample(&boot, o.retained)?;
        result["setup"] = m.data();
        let source_start = Instant::now();
        source_begin = Some((
            source_start,
            m.calls,
            m.turns,
            m.reads,
            m.replays,
            m.bytes,
            m.read_ms,
            m.replay_ms,
        ));
        let background = if o.work > 0 {
            Some(
                admit(
                    &r,
                    &boot,
                    "loop(X) <=> loop(X).",
                    "loop(A)",
                    &mut m,
                    &mut owned,
                )?
                .0,
            )
        } else {
            None
        };
        if let Some(bg) = &background {
            progress(&r, bg, 1, &mut m)?;
        }
        let initial = background
            .as_ref()
            .map(|id| status(&r, id, &mut m).and_then(|v| apps(&v)))
            .transpose()?
            .unwrap_or(0);
        let mut heavy = finite(&r, &boot, n, o.rows, o.batch, &mut m, &mut owned)?;
        let tiny_start = Instant::now();
        let mut tiny = finite(&r, &boot, 1, 1, 1, &mut m, &mut owned)?;
        let mut tiny_latency = None;
        let mut background_apps = initial;
        while !heavy.done || !tiny.done || background_apps.saturating_sub(initial) < o.work {
            m.tick(&r)?;
            if !tiny.done {
                let read_result = read(&r, &mut tiny, None, 0, &mut m);
                if tiny.reader.answers == 1 && tiny_latency.is_none() {
                    tiny_latency = Some(ms(tiny_start.elapsed()));
                    result["tiny_answer_work"] = json!(m.b.ticks);
                }
                result["tiny_answers"] = json!(tiny.reader.answers);
                result["tiny_answer_ms"] = json!(tiny_latency);
                read_result?;
            }
            if !heavy.done {
                let read_result = read(&r, &mut heavy, background.as_ref(), o.replay_every, &mut m);
                result["answers"] = json!(heavy.reader.answers);
                read_result?;
            }
            result["answers"] = json!(heavy.reader.answers);
            result["tiny_answers"] = json!(tiny.reader.answers);
            result["tiny_answer_ms"] = json!(tiny_latency);
            if let Some(bg) = &background {
                let current = status(&r, bg, &mut m)?;
                check(
                    current["execution_done"] == false,
                    "continuing background completed",
                )?;
                background_apps = apps(&current)?;
            }
            result["background_applications"] = json!(background_apps - initial);
        }
        if let Some(bg) = &background {
            let out = m.call(&r, "/api/output", bg)?;
            check(
                out["events"] == json!([]) && apps(&out)? == background_apps,
                "continuing background output or read-driven progress",
            )?;
            check(
                apps(&status(&r, bg, &mut m)?)? == background_apps,
                "background read advanced source",
            )?;
        }
        result["source_ms"] = json!(ms(source_start.elapsed()));
        result["tiny_answer_ms"] = json!(tiny_latency);
        result["answers"] = json!(heavy.reader.answers);
        result["tiny_answers"] = json!(tiny.reader.answers);
        result["background_applications"] = json!(background_apps - initial);
        result["finite_applications"] = json!(apps(&status(&r, &heavy.req, &mut m)?)?);
        let mut memories = Vec::new();
        for id in &owned {
            let role = if id["run"] == heavy.req["run"] {
                "main"
            } else if id["run"] == tiny.req["run"] {
                "tiny"
            } else if background.as_ref().is_some_and(|bg| bg["run"] == id["run"]) {
                "background"
            } else {
                "retained"
            };
            memories.push(json!({"role":role,"memory":status(&r,id,&mut m)?["memory"]}));
        }
        result["before_close_memory"] = json!(memories);
        result["before_close_spools"] = sample(&boot, owned.len())?;
        if !m.b.in_time() {
            return Err(Stop::Censored);
        }
        Ok(())
    })();
    result["source_and_setup"] = m.data();
    if let Some((start, calls, turns, reads, replays, bytes, read_time, replay_time)) = source_begin
    {
        result["source"] = json!({"elapsed_ms":ms(start.elapsed()),"requests":m.calls-calls,
            "scheduler_turns":m.turns-turns,"reads":m.reads-reads,"replays":m.replays-replays,
            "delivered_event_json_bytes":m.bytes-bytes,
            "read_ms":m.b.detailed.then(||ms(m.read_ms-read_time)),
            "replay_ms":m.b.detailed.then(||ms(m.replay_ms-replay_time))});
    }
    let mut cleanup = Meter::new(limit, timeout);
    cleanup.b.phase = Phase::Cleanup;
    let cleanup_result = (|| -> Result<()> {
        for id in &owned {
            close(&r, id, &mut cleanup)?;
            result["closed_verified"] = json!(result["closed_verified"].as_u64().unwrap() + 1);
        }
        result["after_close_spools"] = if boot.is_null() {
            Value::Null
        } else {
            sample(&boot, 0)?
        };
        if !cleanup.b.in_time() {
            return Err(Stop::Censored);
        }
        Ok(())
    })();
    result["cleanup_complete"] = json!(cleanup_result.is_ok());
    result["cleanup"] = cleanup.data();
    let t = Instant::now();
    during(Phase::Cleanup, || drop(r));
    result["runtime_drop_ms"] = json!(ms(t.elapsed()));
    result["elapsed_ms"] = json!(ms(total.elapsed()));
    let drop_evidence = if boot.is_null() {
        Ok(Value::Null)
    } else {
        sample(&boot, 0)
    };
    if let Ok(sample) = &drop_evidence {
        result["after_drop_spools"] = sample.clone();
    }
    let drop_result = drop_evidence.map(|_| ());
    result["elapsed_ms"] = json!(ms(total.elapsed()));
    let invalid = [&attempt, &cleanup_result, &drop_result]
        .into_iter()
        .find_map(|r| match r {
            Err(Stop::Invalid(s)) => Some(s.clone()),
            _ => None,
        });
    let done = attempt.is_ok() && cleanup_result.is_ok() && cleanup.b.in_time();
    result["status"] = json!(if invalid.is_some() {
        "INVALID"
    } else if done {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    });
    result["censored"] = json!(invalid.is_none() && !done);
    result["error"] = json!(invalid);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_populations_batches_replay_and_continuing_work() {
        for o in [
            Options::default(),
            Options {
                closed: 2,
                retained: 0,
                rows: 2,
                batch: 1,
                replay_every: 1,
                work: 4,
            },
            Options {
                closed: 0,
                retained: 2,
                rows: 2,
                batch: 3,
                replay_every: 2,
                work: 7,
            },
            Options {
                closed: 2,
                retained: 2,
                rows: 1,
                batch: 4096,
                replay_every: 1,
                work: 0,
            },
        ] {
            let m = run(3, o, 100_000, Duration::from_secs(15)).unwrap();
            assert_eq!(m["status"], "COMPLETE", "{m}");
            assert_eq!(m["answers"], 3);
            assert_eq!(m["tiny_answers"], 1);
            assert_eq!(m["cleanup_complete"], true);
            assert_eq!(
                m["closed_verified"],
                o.retained + 2 + usize::from(o.work > 0)
            );
            assert!(m["background_applications"].as_u64().unwrap() >= o.work);
            assert_eq!(m["history_completed"], o.closed);
            assert_eq!(m["retained_completed"], o.retained);
            if o.replay_every > 0 {
                assert!(m["source_and_setup"]["replays"].as_u64().unwrap() > 0);
            }
            #[cfg(target_os = "linux")]
            {
                assert_eq!(m["retained_spools"]["files"], o.retained);
                assert_eq!(m["after_close_spools"]["descriptors"], 0);
                assert_eq!(m["after_drop_spools"]["descriptors"], 0);
                if o.retained > 0 {
                    assert!(m["retained_spools"]["logical_bytes"].as_u64().unwrap() > 0);
                }
            }
        }
    }
    #[test]
    fn budget_at_tiny_answer_preserves_validated_progress() {
        let options = Options {
            rows: 8,
            batch: 1,
            work: 4,
            ..Options::default()
        };
        let complete = run(8, options, 100_000, Duration::from_secs(10)).unwrap();
        assert_eq!(complete["status"], "COMPLETE");
        let boundary = complete["tiny_answer_work"].as_u64().unwrap();
        let stopped = run(8, options, boundary, Duration::from_secs(10)).unwrap();
        assert_eq!(stopped["status"], "INCOMPLETE");
        assert_eq!(stopped["tiny_answers"], 1);
        assert!(stopped["tiny_answer_ms"].is_number());
        assert!(stopped["answers"].as_u64().unwrap() < 8);
    }

    #[test]
    fn initial_deadline_is_censored_without_admission() {
        let m = run(2, Options::default(), 100_000, Duration::from_nanos(1)).unwrap();
        assert_eq!(m["status"], "INCOMPLETE");
        assert_eq!(m["censored"], true);
        assert_eq!(m["source_and_setup"]["requests"], 0);
        assert_eq!(m["closed_verified"], 0);
    }

    #[test]
    fn censored_admission_still_closes_spool_and_owner() {
        let m = run(2, Options::default(), 10, Duration::from_secs(10)).unwrap();
        assert_eq!(m["status"], "INCOMPLETE");
        assert_eq!(m["censored"], true);
        // Cleanup can also censor, but direct Runtime destruction must release
        // the admitted spool descriptors independently of status reporting.
        #[cfg(target_os = "linux")]
        {
            assert_eq!(m["after_drop_spools"]["files"], 0);
            assert_eq!(m["after_drop_spools"]["descriptors"], 0);
        }
        assert!(
            run(
                1,
                Options {
                    batch: 4097,
                    ..Options::default()
                },
                100,
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
