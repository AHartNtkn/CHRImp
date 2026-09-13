//! Loopback notebook transport. The scheduler owns execution and retained output.
use crate::engine::{Engine, InspectionError, SnapshotKind, ViewId};
use crate::observe::Output;
use crate::program::prepare;
use crate::syntax::{
    Body, Program, format_program, format_query, parse_program, parse_query, validate_program,
    validate_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, hash_map::RandomState};
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::Duration;

const BODY_LIMIT: usize = 4 * 1024 * 1024;
const BUDGET_LIMIT: usize = 4096;
const MAX_SAFE_ID: u64 = 9_007_199_254_740_991;
static NEXT_BOOT: AtomicU64 = AtomicU64::new(0);

pub struct Runtime {
    boot: String,
    runs: Mutex<Runs>,
    batches: Mutex<BTreeMap<(u64, Option<u64>), Value>>,
    owners: Mutex<Owners>,
}
impl Default for Runtime {
    fn default() -> Self {
        // RandomState uses OS-seeded keys in std. Domain-separated hashes and
        // an ordinal distinguish incarnations without clocks or persistent files.
        // This is an opaque identity, not an authentication credential.
        let ordinal = NEXT_BOOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("runtime incarnation counter exhausted");
        let random = RandomState::new();
        let boot = format!(
            "{:016x}{:016x}",
            random.hash_one((ordinal, 0u8)),
            random.hash_one((ordinal, 1u8))
        );
        Self {
            boot,
            runs: Mutex::default(),
            batches: Mutex::default(),
            owners: Mutex::default(),
        }
    }
}
#[derive(Default)]
struct Owners {
    issued: u64,
    admitted: u64,
    entries: BTreeMap<u64, Arc<Owner>>,
    runs: BTreeMap<u64, u64>,
}
#[derive(Default)]
struct Owner {
    waiting: AtomicUsize,
    state: Mutex<OwnerState>,
}
impl Owner {
    fn request(&self) -> MutexGuard<'_, OwnerState> {
        // Register before blocking so maintenance cannot repeatedly reacquire
        // the mutex ahead of an HTTP worker woken at the end of a quantum.
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let state = self.state.lock().unwrap();
        self.waiting.fetch_sub(1, Ordering::SeqCst);
        state
    }
}
#[derive(Default)]
struct OwnerState {
    retired: bool,
    runs: usize,
    receipt: Option<Receipt>,
}
struct Receipt {
    command: u64,
    path: String,
    request: String,
    response: Value,
}
impl OwnerState {
    fn replay(&self, command: u64, path: &str, request: &str) -> Result<Option<Value>, Response> {
        if let Some(receipt) = &self.receipt
            && command == receipt.command
            && path == receipt.path
            && request == receipt.request
        {
            return Ok(Some(receipt.response.clone()));
        }
        let expected = self
            .receipt
            .as_ref()
            .map_or(Some(1), |r| r.command.checked_add(1));
        if expected != Some(command) {
            return Err(Response {
                status: 409,
                body: json!({"code":"command_conflict", "error":"command is not the next command or an exact replay", "retry":false}),
            });
        }
        Ok(None)
    }
    fn remember(&mut self, command: u64, path: &str, request: &str, response: &Value) {
        self.receipt = Some(Receipt {
            command,
            path: path.into(),
            request: request.into(),
            response: response.clone(),
        });
    }
}
fn safe_id(value: u64, name: &str) -> Result<u64, Response> {
    if value == 0 || value > MAX_SAFE_ID {
        Err(Response::error(400, format!("invalid {name}")))
    } else {
        Ok(value)
    }
}
fn control(path: &str) -> bool {
    matches!(
        path,
        "/api/start"
            | "/api/inspect"
            | "/api/step"
            | "/api/resume"
            | "/api/pause"
            | "/api/snapshot"
    )
}
fn unknown_owner() -> Response {
    Response {
        status: 409,
        body: json!({"code":"unknown_owner", "error":"notebook owner is not attached or has retired", "retry":false}),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerRequest {
    boot: String,
    owner: Option<u64>,
}
#[derive(Default)]
struct Runs {
    closing: BTreeMap<u64, bool>,
    next: u64,
    entries: BTreeMap<u64, Arc<Mutex<Run>>>,
    after: Option<u64>,
    round: Option<u64>,
    active: bool,
}
// Output is retained until explicit close. Only one bounded delivery batch lives
// in RAM; acknowledgements replay that batch and never control source execution.
struct Spool {
    writer: BufWriter<File>,
    reader: File,
    path: PathBuf,
    written: u64,
    read: u64,
}
impl Spool {
    fn new(boot: &str, run: u64) -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!("chr-{boot}-{run}.jsonl"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        let reader = File::open(&path)?;
        // The open handles own this process-local spool, including on abrupt exit.
        #[cfg(unix)]
        std::fs::remove_file(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            reader,
            path,
            written: 0,
            read: 0,
        })
    }
    fn append(&mut self, event: Value) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&event)?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }
    fn read(&mut self, limit: usize) -> io::Result<Vec<Value>> {
        self.writer.flush()?;
        self.reader.seek(SeekFrom::Start(self.read))?;
        let mut reader = BufReader::new((&mut self.reader).take(self.written - self.read));
        let mut events = Vec::new();
        let mut consumed = 0;
        let mut line = String::new();
        for _ in 0..limit {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            events.push(serde_json::from_str(&line)?);
            consumed += n as u64;
        }
        self.read += consumed;
        Ok(events)
    }
    fn drained(&self) -> bool {
        self.read == self.written
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
struct Run {
    engine: Engine,
    spool: Spool,
    running: bool,
    stepping: bool,
    error: Option<String>,
}
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Value,
}
impl Response {
    fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }
    fn error(status: u16, message: impl ToString) -> Self {
        Self {
            status,
            body: json!({"error": message.to_string()}),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    program: String,
    query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    #[serde(default)]
    boot: Option<String>,
    owner: Option<u64>,
    command: Option<u64>,
    program: Program,
    query: Body,
    #[serde(default)]
    record_history: bool,
    #[serde(default)]
    paused: bool,
}
#[derive(Clone, Copy)]
struct Id(u64);
impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Scalar;
        impl serde::de::Visitor<'_> for Scalar {
            type Value = Id;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an unsigned ID or decimal string")
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Id, E> {
                Ok(Id(value))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Id, E> {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(E::custom("invalid unsigned ID"));
                }
                value.parse().map(Id).map_err(E::custom)
            }
        }
        d.deserialize_any(Scalar)
    }
}
impl Id {
    fn view(self) -> ViewId {
        ViewId(self.0)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    boot: String,
    owner: u64,
    command: Option<u64>,
    run: u64,
    #[serde(default)]
    continue_step: bool,
    #[serde(default)]
    ack: Option<u64>,
    #[serde(default = "budget")]
    budget: usize,
    #[serde(default)]
    snapshot: Option<Id>,
    #[serde(default)]
    inspection: Option<Id>,
    #[serde(default)]
    choices: BTreeMap<String, bool>,
    #[serde(default)]
    after_choice: Option<Id>,
    #[serde(default)]
    after_snapshot: Option<Id>,
    #[serde(default)]
    before_choice: Option<Id>,
    #[serde(default)]
    before_snapshot: Option<Id>,
}
fn budget() -> usize {
    1024
}
fn decode<T: serde::de::DeserializeOwned>(source: &str) -> Result<T, Response> {
    if source.len() > BODY_LIMIT {
        return Err(Response::error(413, "request body exceeds 4 MiB"));
    }
    // All request shapes are fixed or bounded by Body's semantic-depth decoder.
    let mut d = serde_json::Deserializer::from_str(source);
    d.disable_recursion_limit();
    T::deserialize(&mut d)
        .and_then(|v| d.end().map(|()| v))
        .map_err(|e| Response::error(400, e))
}
fn choices(request: &RunRequest) -> Result<Vec<(u64, bool)>, Response> {
    request
        .choices
        .iter()
        .map(|(id, &positive)| {
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Response::error(400, "invalid choice ID"));
            }
            id.parse()
                .map(|id| (id, positive))
                .map_err(|e| Response::error(400, e))
        })
        .collect()
}
fn event(output: Output) -> Value {
    // IDs are decimal strings on the browser wire, avoiding JS integer rounding.
    match output {
        Output::Begin {
            completion,
            alternative,
        } => {
            json!({"kind":"begin", "completion":completion.to_string(), "alternative":alternative.to_string()})
        }
        Output::Variable { slot, variable } => {
            json!({"kind":"variable", "slot":slot, "variable":variable.to_string()})
        }
        Output::Fact {
            occurrence,
            relation,
        } => json!({"kind":"fact", "occurrence":occurrence.to_string(), "relation":relation}),
        Output::Port { variable } => json!({"kind":"port", "variable":variable.to_string()}),
        Output::EndFact => json!({"kind":"end_fact"}),
        Output::PendingBegin { event } => json!({"kind":"pending_begin","event":event.to_string()}),
        Output::Expression { operator } => json!({"kind":"expression","operator":operator}),
        Output::ExpressionRelation { relation } => {
            json!({"kind":"expression_relation","relation":relation})
        }
        Output::ExpressionVariable { variable } => {
            json!({"kind":"expression_variable","variable":variable.to_string()})
        }
        Output::ExpressionEnd => json!({"kind":"expression_end"}),
        Output::PendingEnd => json!({"kind":"pending_end"}),
        Output::End => json!({"kind":"end"}),
    }
}
fn view_result<T>(value: Result<T, InspectionError>) -> Result<T, Response> {
    value.map_err(|e| match e {
        InspectionError::Busy | InspectionError::Initializing => Response {
            status: 409,
            body: json!({"error":e.to_string(), "retry":true}),
        },
        _ => Response::error(400, e),
    })
}
impl Runtime {
    pub fn request(&self, path: &str, body: &str) -> Response {
        self.dispatch(path, body)
            .unwrap_or_else(|response| response)
    }
    fn owner(&self, id: u64) -> Result<Arc<Owner>, Response> {
        self.owners
            .lock()
            .unwrap()
            .entries
            .get(&id)
            .cloned()
            .ok_or_else(unknown_owner)
    }
    fn validate_boot(&self, boot: &str) -> Result<(), Response> {
        if boot.len() != 32
            || !boot
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Response::error(
                400,
                "boot must be 32 lowercase hexadecimal characters",
            ));
        }
        if boot != self.boot {
            return Err(Response {
                status: 409,
                body: json!({"code":"stale_boot", "error":"The notebook server restarted. This execution is no longer available.", "retry":false}),
            });
        }
        Ok(())
    }
    fn dispatch(&self, path: &str, body: &str) -> Result<Response, Response> {
        match path {
            "/api/hello" => {
                let input: BTreeMap<String, String> = decode(body)?;
                if !input.is_empty() {
                    return Err(Response::error(400, "hello expects an empty object"));
                }
                return Ok(Response::ok(json!({"boot":self.boot})));
            }
            "/api/reserve" | "/api/attach" | "/api/retire" => {
                let request: OwnerRequest = decode(body)?;
                self.validate_boot(&request.boot)?;
                let mut owners = self.owners.lock().unwrap();
                if path == "/api/reserve" {
                    if request.owner.is_some() {
                        return Err(Response::error(400, "reserve takes no owner"));
                    }
                    owners.issued = owners
                        .issued
                        .checked_add(1)
                        .filter(|&n| n <= MAX_SAFE_ID)
                        .ok_or_else(|| Response::error(503, "owner IDs exhausted"))?;
                    return Ok(Response::ok(json!({"owner":owners.issued})));
                }
                let owner = safe_id(
                    request
                        .owner
                        .ok_or_else(|| Response::error(400, "missing owner"))?,
                    "owner",
                )?;
                if path == "/api/attach" {
                    if let Some(entry) = owners.entries.get(&owner).cloned() {
                        drop(owners);
                        if entry.request().retired {
                            return Err(unknown_owner());
                        }
                        return Ok(Response::ok(json!({})));
                    }
                    if owner <= owners.admitted || owner > owners.issued {
                        return Err(unknown_owner());
                    }
                    owners.admitted = owner;
                    owners.entries.insert(owner, Arc::new(Owner::default()));
                    return Ok(Response::ok(json!({})));
                }
                if let Some(entry) = owners.entries.get(&owner).cloned() {
                    drop(owners);
                    let mut entry = entry.request();
                    if entry.runs != 0 {
                        return Err(Response {
                            status: 409,
                            body: json!({"code":"owner_busy", "error":"close owned executions before retiring the notebook owner", "retry":false}),
                        });
                    }
                    // Fence requests that obtained the Arc before registry removal.
                    entry.retired = true;
                    self.owners.lock().unwrap().entries.remove(&owner);
                } else if owner > owners.admitted {
                    return Err(unknown_owner());
                }
                return Ok(Response::ok(json!({"retired":true})));
            }
            "/api/parse" => {
                let input: Source = decode(body)?;
                let program = parse_program(&input.program).map_err(|e| Response::error(400, e))?;
                let query = parse_query(&input.query).map_err(|e| Response::error(400, e))?;
                return Ok(Response::ok(json!({"program":program, "query":query})));
            }
            "/api/format" | "/api/start" => {
                let input: Model = decode(body)?;
                if path == "/api/start" {
                    self.validate_boot(
                        input
                            .boot
                            .as_deref()
                            .ok_or_else(|| Response::error(400, "missing boot"))?,
                    )?;
                }
                if path == "/api/format" {
                    validate_program(&input.program)
                        .and_then(|()| validate_query(&input.query))
                        .map_err(|e| Response::error(400, e))?;
                    return Ok(Response::ok(
                        json!({"program":format_program(&input.program), "query":format_query(&input.query)}),
                    ));
                }
                let owner = safe_id(
                    input
                        .owner
                        .ok_or_else(|| Response::error(400, "missing owner"))?,
                    "owner",
                )?;
                let command = safe_id(
                    input
                        .command
                        .ok_or_else(|| Response::error(400, "missing command"))?,
                    "command",
                )?;
                let entry = self.owner(owner)?;
                let mut entry = entry.request();
                if entry.retired {
                    return Err(unknown_owner());
                }
                if let Some(response) = entry.replay(command, path, body)? {
                    return Ok(Response::ok(response));
                }
                let code = Arc::new(
                    prepare(&input.program, &input.query).map_err(|e| Response::error(400, e))?,
                );
                let engine = Engine::with_history(code.clone(), input.record_history);
                let mut runs = self.runs.lock().unwrap();
                if runs.next == 9_007_199_254_740_991 {
                    return Err(Response::error(503, "run IDs exhausted"));
                }
                runs.next += 1;
                let run = runs.next;
                let spool = Spool::new(&self.boot, run).map_err(|e| Response::error(500, e))?;
                runs.entries.insert(
                    run,
                    Arc::new(Mutex::new(Run {
                        engine,
                        spool,
                        running: !input.paused,
                        stepping: false,
                        error: None,
                    })),
                );
                self.owners.lock().unwrap().runs.insert(run, owner);
                entry.runs += 1;
                drop(runs);
                let response = json!({"run":run,"signatures":code.signatures(),"variables":code.query_variables()});
                entry.remember(command, path, body, &response);
                return Ok(Response::ok(response));
            }
            _ => {}
        }
        let request: RunRequest = decode(body)?;
        self.validate_boot(&request.boot)?;
        if request.budget > BUDGET_LIMIT {
            return Err(Response::error(400, "work budget exceeds 4096"));
        }
        if request.continue_step && path != "/api/resume" {
            return Err(Response::error(
                400,
                "continue_step is only valid for resume",
            ));
        }
        let owner = safe_id(request.owner, "owner")?;
        let command = if control(path) {
            Some(safe_id(
                request
                    .command
                    .ok_or_else(|| Response::error(400, "missing command"))?,
                "command",
            )?)
        } else {
            if request.command.is_some() {
                return Err(Response::error(400, "this route takes no command"));
            }
            None
        };
        // Never hold the registry while waiting on an owner. Only this owner's
        // commands are serialized across engine effects and receipt publication.
        let entry = self.owner(owner)?;
        let mut entry = entry.request();
        if entry.retired {
            return Err(unknown_owner());
        }
        if let Some(command) = command
            && let Some(response) = entry.replay(command, path, body)?
        {
            return Ok(Response::ok(response));
        }
        let mapped = self.owners.lock().unwrap().runs.get(&request.run).copied();
        match mapped {
            Some(actual) if actual == owner => {}
            Some(_) => {
                return Err(Response {
                    status: 403,
                    body: json!({"code":"wrong_owner", "error":"execution belongs to another notebook owner", "retry":false}),
                });
            }
            None => {
                return if path == "/api/close" {
                    Ok(Response::ok(json!({"closed":true})))
                } else {
                    Err(Response::error(404, "unknown run"))
                };
            }
        }
        let run = self.runs.lock().unwrap().entries.get(&request.run).cloned();
        let Some(run) = run else {
            return if path == "/api/close" {
                Ok(Response::ok(json!({"closed":true})))
            } else {
                Err(Response::error(404, "unknown run"))
            };
        };
        let mut e = run.lock().unwrap();
        {
            let runs = self.runs.lock().unwrap();
            if !runs.entries.contains_key(&request.run) {
                return if path == "/api/close" {
                    Ok(Response::ok(json!({"closed":true})))
                } else {
                    Err(Response::error(404, "unknown run"))
                };
            }
            if runs.closing.contains_key(&request.run)
                && !matches!(path, "/api/status" | "/api/close" | "/api/cancel")
            {
                return Err(Response::error(409, "run is closing"));
            }
        }
        let stream = match path {
            "/api/output" => Some((request.run, None)),
            "/api/inspect_advance" => Some((
                request.run,
                Some(
                    request
                        .inspection
                        .ok_or_else(|| Response::error(400, "missing inspection"))?
                        .0,
                ),
            )),
            _ => None,
        };
        let mut sequence = 1;
        if let Some(key) = stream {
            if let Some(batch) = self.batches.lock().unwrap().get(&key) {
                let previous = batch["sequence"].as_u64().unwrap();
                if request.ack.is_some_and(|ack| ack > previous) {
                    return Err(Response::error(400, "acknowledgement is ahead of delivery"));
                }
                if request.ack != Some(previous) {
                    if request
                        .ack
                        .map_or(previous == 1, |ack| ack.checked_add(1) == Some(previous))
                    {
                        return Ok(Response::ok(batch.clone()));
                    }
                    return Err(Response::error(
                        409,
                        "acknowledgement is outside the retained delivery window",
                    ));
                }
                sequence = previous
                    .checked_add(1)
                    .filter(|&n| n <= 9_007_199_254_740_991)
                    .ok_or_else(|| Response::error(503, "batch IDs exhausted"))?;
            } else if request.ack.is_some() {
                return Err(Response::error(400, "unknown acknowledgement"));
            }
        }
        let raw_request = body;
        let mut body = match path {
            "/api/output" => {
                let events = match e.spool.read(request.budget) {
                    Ok(events) => events,
                    Err(error) => {
                        e.error = Some(error.to_string());
                        e.running = false;
                        e.engine.cancel();
                        return Err(Response::error(500, error));
                    }
                };
                json!({"events":events,"applications":e.engine.applications(),"exhausted":e.engine.exhausted(),"delivery_done":e.engine.delivery_done() && e.spool.drained(),"canceled":e.engine.canceled(),"step":e.engine.step_status(),"running":e.running,"output_drained":e.spool.drained(),"error":e.error})
            }
            "/api/pause" => {
                e.running = false;
                json!({})
            }
            "/api/step" => {
                view_result(e.engine.request_step(choices(&request)?))?;
                e.running = true;
                e.stepping = true;
                json!({"step":e.engine.step_status()})
            }
            "/api/resume" => {
                if e.engine.canceled() {
                    return Err(Response::error(400, InspectionError::Canceled));
                }
                if request.continue_step {
                    if !e.stepping {
                        return Err(Response::error(400, "no admitted step to continue"));
                    }
                    e.running = !e.engine.step_status().done;
                } else {
                    view_result(e.engine.resume())?;
                    e.stepping = false;
                    e.running = !e.engine.delivery_done();
                }
                json!({})
            }
            "/api/maintenance" => {
                e.engine.maintain(request.budget);
                json!({"collecting":e.engine.collecting()})
            }
            "/api/inspect" => {
                let id = view_result(
                    e.engine
                        .start_inspection(request.snapshot.map(Id::view), choices(&request)?),
                )?;
                json!({"inspection":id.0.to_string()})
            }
            "/api/inspect_advance" => {
                let id = request
                    .inspection
                    .ok_or_else(|| Response::error(400, "missing inspection"))?
                    .view();
                let mut events = vec![];
                for _ in 0..request.budget {
                    view_result(e.engine.advance_inspection(id, 1))?;
                    if let Some(output) = view_result(e.engine.take_inspection_output(id))? {
                        events.push(event(output));
                    }
                    if view_result(e.engine.inspection_status(id))?.done {
                        break;
                    }
                }
                let status = view_result(e.engine.inspection_status(id))?;
                json!({"events":events,"done":status.done,"canceled":status.canceled,"error":status.error})
            }
            "/api/inspect_cancel" => {
                let id = request
                    .inspection
                    .ok_or_else(|| Response::error(400, "missing inspection"))?
                    .view();
                view_result(e.engine.cancel_inspection(id))?;
                view_result(e.engine.advance_inspection(id, request.budget))?;
                let pending = self
                    .batches
                    .lock()
                    .unwrap()
                    .get(&(request.run, Some(id.0)))
                    .cloned();
                json!({"done":view_result(e.engine.inspection_status(id))?.done,"pending":pending})
            }
            "/api/inspect_release" => {
                let id = request
                    .inspection
                    .ok_or_else(|| Response::error(400, "missing inspection"))?
                    .view();
                match e.engine.release_inspection(id) {
                    Ok(()) | Err(InspectionError::UnknownInspection) => {}
                    Err(error) => {
                        view_result::<()>(Err(error))?;
                    }
                }
                self.batches
                    .lock()
                    .unwrap()
                    .remove(&(request.run, Some(id.0)));
                json!({})
            }
            "/api/snapshot" => {
                let snapshot = view_result(e.engine.capture_snapshot())?;
                json!({"snapshot":snapshot.0.to_string()})
            }
            "/api/snapshot_release" => {
                let snapshot = request
                    .snapshot
                    .ok_or_else(|| Response::error(400, "missing snapshot"))?;
                match e.engine.release_snapshot(snapshot.view()) {
                    Ok(()) | Err(InspectionError::UnknownSnapshot) => {}
                    Err(error) => {
                        view_result::<()>(Err(error))?;
                    }
                }
                json!({})
            }
            "/api/views" => {
                let cutoff = if let Some(id) = request.inspection {
                    view_result(e.engine.inspection_snapshot_info(id.view()))?.last_choice
                } else if let Some(id) = request.snapshot {
                    view_result(e.engine.snapshot_info(id.view()))?.last_choice
                } else {
                    e.engine.choices().next_back().map(|(&id, _)| id)
                };
                if (request.after_choice.is_some() && request.before_choice.is_some())
                    || (request.after_snapshot.is_some() && request.before_snapshot.is_some())
                {
                    return Err(Response::error(400, "choose one page direction"));
                }
                let choice_rows: Vec<_> = if let Some(before) = request.before_choice {
                    e.engine
                        .choices_before(before.0, cutoff)
                        .take(64)
                        .map(|(&id, _)| id)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect()
                } else {
                    e.engine
                        .choices_after(request.after_choice.map(|id| id.0), cutoff)
                        .take(64)
                        .map(|(&id, _)| id)
                        .collect()
                };
                let prev_choice = choice_rows
                    .first()
                    .filter(|&&id| e.engine.choices_before(id, cutoff).next().is_some())
                    .map(|id| id.to_string());
                let next_choice = choice_rows
                    .last()
                    .filter(|&&id| e.engine.choices_after(Some(id), cutoff).next().is_some())
                    .map(|id| id.to_string());
                let choices: Vec<_> = choice_rows
                    .into_iter()
                    .map(|id| {
                        let birth = e.engine.choices_after(id.checked_sub(1), Some(id)).next().unwrap().1;
                        json!({"id":id.to_string(),"label":format!(
                            "Choice {} (event {}): First arms {}–{}; Second arms {}–{}",
                            id+1, birth.event, birth.start+1, birth.split, birth.split+1, birth.end)})
                    })
                    .collect();
                let snapshot_rows: Vec<_> = if let Some(before) = request.before_snapshot {
                    e.engine
                        .snapshots_before(before.view())
                        .take(64)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect()
                } else {
                    e.engine
                        .snapshots_after(request.after_snapshot.map(Id::view))
                        .take(64)
                        .collect()
                };
                let prev_snapshot = snapshot_rows
                    .first()
                    .filter(|s| e.engine.snapshots_before(s.id).next().is_some())
                    .map(|s| s.id.0.to_string());
                let next_snapshot = snapshot_rows
                    .last()
                    .filter(|s| e.engine.snapshots_after(Some(s.id)).next().is_some())
                    .map(|s| s.id.0.to_string());
                let snapshots: Vec<_> = snapshot_rows.into_iter().map(|s| {
                    let action = match s.kind { SnapshotKind::Initial=>"initial query",SnapshotKind::Requested=>"retained view",SnapshotKind::Application{..}=>"rule application",SnapshotKind::Post{..}=>"relation posted",SnapshotKind::Merge=>"variables merged",SnapshotKind::Choice=>"disjunction",SnapshotKind::Failure=>"failure",SnapshotKind::NormalForm=>"normal form" };
                    let mut row = json!({"id":s.id.0.to_string(),"label":format!("State {} · {action}",s.id.0)});
                    if let SnapshotKind::Application { rule, .. } = s.kind {
                        let name = e.engine.program().rules()[rule].name.as_deref();
                        row["rule"] = json!(rule);
                        row["name"] = json!(name);
                        row["label"] = json!(format!("State {} · rule {}{}", s.id.0, rule + 1,
                            name.map(|n| format!(" · {n}")).unwrap_or_default()));
                    }
                    row
                }).collect();
                json!({"choices":choices,"snapshots":snapshots,"next_choice":next_choice,"next_snapshot":next_snapshot,"prev_choice":prev_choice,"prev_snapshot":prev_snapshot})
            }
            "/api/cancel" => {
                e.running = false;
                e.engine.cancel();
                let pending = self
                    .batches
                    .lock()
                    .unwrap()
                    .get(&(request.run, None))
                    .cloned();
                json!({"done":e.engine.cancel_done(),"pending":pending,"output_pending":!e.spool.drained()})
            }
            "/api/close" => {
                e.running = false;
                e.engine.cancel();
                self.runs
                    .lock()
                    .unwrap()
                    .closing
                    .entry(request.run)
                    .or_insert(false);
                json!({})
            }
            "/api/status" => {
                json!({"applications":e.engine.applications(),"canceled":e.engine.canceled(),"cancel_done":e.engine.cancel_done(),"memory":e.engine.memory(),"running":e.running,"execution_done":e.engine.delivery_done(),"error":e.error})
            }
            _ => return Err(Response::error(404, "unknown API route")),
        };
        if let Some(key) = stream {
            body["sequence"] = json!(sequence);
            self.batches.lock().unwrap().insert(key, body.clone());
        }
        if let Some(command) = command {
            entry.remember(command, path, raw_request, &body);
        }
        Ok(Response::ok(body))
    }
    /// One run per maintenance turn, with a frozen admission boundary for fairness.
    pub fn tick(&self) -> bool {
        let run = {
            let mut runs = self.runs.lock().unwrap();
            if runs.round.is_none() {
                runs.round = runs.entries.last_key_value().map(|(&id, _)| id);
                runs.active = false;
            }
            let Some(last) = runs.round else {
                return false;
            };
            let next = runs
                .entries
                .range((runs.after.map_or(Unbounded, Excluded), Included(last)))
                .next()
                .map(|(&id, run)| (id, run.clone()));
            if let Some((id, run)) = next {
                runs.after = Some(id);
                Some((id, run, runs.closing.get(&id).copied()))
            } else {
                runs.after = None;
                runs.round = None;
                return runs.active;
            }
        };
        let Some((id, run, closing)) = run else {
            return true;
        };
        let owner = {
            let owners = self.owners.lock().unwrap();
            owners
                .runs
                .get(&id)
                .and_then(|owner| owners.entries.get(owner))
                .cloned()
        };
        let Some(owner) = owner else {
            return false;
        };
        // ponytail: request floods can defer this owner's maintenance; use FIFO
        // admission if fairness under a continuous request flood is required.
        if owner.waiting.load(Ordering::SeqCst) == 0
            && let Ok(mut owner) = owner.state.try_lock()
            && !owner.retired
            && let Ok(mut e) = run.try_lock()
        {
            // Another maintenance caller may have reclaimed the selected run
            // between the registry lookup and this nonblocking owner lock.
            if !self.owners.lock().unwrap().runs.contains_key(&id) {
                return false;
            }
            let active = closing.is_some()
                || e.running
                || (e.engine.canceled() && !e.engine.cancel_done())
                || (!e.engine.canceled()
                    && e.engine.query_variables().len()
                        != e.engine.program().query_variables().len());
            self.runs.lock().unwrap().active |= active;
            match closing {
                Some(false) => {
                    e.engine.advance(512);
                    if e.engine.cancel_done() {
                        self.runs.lock().unwrap().closing.insert(id, true);
                    }
                }
                Some(true) => {
                    // Drop view roots in bounded batches BEFORE another sweep.
                    // Sweeping after every individual release would rescan the
                    // remaining history once per snapshot.
                    let mut released_all = false;
                    for _ in 0..512 {
                        let snapshot = e.engine.snapshots().next().map(|s| s.id);
                        let inspection = e.engine.inspections().next();
                        if let Some(snapshot) = snapshot {
                            if e.engine.release_snapshot(snapshot).is_err() {
                                break;
                            }
                        } else if let Some(inspection) = inspection {
                            if e.engine
                                .inspection_status(inspection)
                                .is_ok_and(|s| !s.done)
                            {
                                let _ = e.engine.discard_inspection(inspection, 512);
                                let _ = e.engine.release_inspection(inspection);
                                break;
                            }
                            if e.engine.release_inspection(inspection).is_err() {
                                break;
                            }
                        } else {
                            released_all = true;
                            break;
                        }
                    }
                    if released_all {
                        e.engine.advance(512);
                        if e.engine.cancel_done()
                            && e.engine.memory().graph_nodes == 0
                            && e.engine.memory().conditions == 0
                        {
                            let mut batches = self.batches.lock().unwrap();
                            let batch = batches
                                .range((id, None)..=(id, Some(u64::MAX)))
                                .next()
                                .map(|(&key, _)| key);
                            if let Some(key) = batch {
                                batches.remove(&key);
                                return false;
                            }
                            drop(batches);
                            drop(e);
                            let mut runs = self.runs.lock().unwrap();
                            runs.entries.remove(&id);
                            runs.closing.remove(&id);
                            self.owners
                                .lock()
                                .unwrap()
                                .runs
                                .remove(&id)
                                .expect("owned run");
                            owner.runs -= 1;
                        }
                    }
                }
                None => {
                    if e.engine.canceled() {
                        e.running = false;
                        e.engine.advance(512);
                    } else if e.running {
                        let Run {
                            engine,
                            spool,
                            error,
                            ..
                        } = &mut *e;
                        if let Err(failure) = crate::runtime::drive(engine, 4096, |output| {
                            spool.append(event(output))
                        }) {
                            *error = Some(failure.to_string());
                        }
                        if e.engine.delivery_done()
                            || e.engine.canceled()
                            || (e.stepping && e.engine.step_status().done)
                        {
                            e.running = false;
                        }
                    } else if e.engine.query_variables().len()
                        != e.engine.program().query_variables().len()
                    {
                        // Admission initializes identifiers even for an initially paused run.
                        for _ in 0..512 {
                            if e.engine.query_variables().len()
                                == e.engine.program().query_variables().len()
                            {
                                break;
                            }
                            let Run {
                                engine,
                                spool,
                                error,
                                ..
                            } = &mut *e;
                            if let Err(failure) = crate::runtime::drive(engine, 1, |output| {
                                spool.append(event(output))
                            }) {
                                *error = Some(failure.to_string());
                                break;
                            }
                        }
                    } else {
                        e.engine.maintain(512);
                    }
                }
            }
        }
        true
    }
}

pub fn serve(listener: TcpListener) -> io::Result<()> {
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "notebook must bind loopback",
        ));
    }
    let runtime = Arc::new(Runtime::default());
    let maintenance = runtime.clone();
    std::thread::spawn(move || {
        loop {
            if !maintenance.tick() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });
    let (send, receive) = mpsc::sync_channel::<TcpStream>(64);
    let receive = Arc::new(Mutex::new(receive));
    for _ in 0..4 {
        let receive = receive.clone();
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            loop {
                let Ok(stream) = receive.lock().unwrap().recv() else {
                    break;
                };
                let _ = connection(stream, &runtime, address.port());
            }
        });
    }
    for stream in listener.incoming() {
        send.send(stream?).map_err(io::Error::other)?;
    }
    Ok(())
}
fn connection(mut stream: TcpStream, runtime: &Runtime, port: u16) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = vec![];
    loop {
        let before = head.len();
        let n = reader
            .by_ref()
            .take((65537 - head.len()) as u64)
            .read_until(b'\n', &mut head)?;
        if n == 0 || head.len() > 65536 {
            return write_json_error(&mut stream, 400, "invalid HTTP headers");
        }
        if &head[before..] == b"\r\n" {
            break;
        }
    }
    let Ok(head) = std::str::from_utf8(&head) else {
        return write_json_error(&mut stream, 400, "invalid HTTP headers");
    };
    let mut lines = head.split("\r\n");
    let parts = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return write_json_error(&mut stream, 400, "expected HTTP/1.1");
    }
    let (method, path) = (parts[0], parts[1]);
    let mut headers = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            return write_json_error(&mut stream, 400, "invalid header");
        };
        if headers
            .insert(key.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return write_json_error(&mut stream, 400, "duplicate header");
        }
    }
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host = headers.get("host").copied().unwrap_or_default();
    if !hosts.iter().any(|h| h == host)
        || headers
            .get("origin")
            .is_some_and(|origin| **origin != format!("http://{host}"))
    {
        return write_json_error(&mut stream, 403, "notebook requests must be same-origin");
    }
    if method == "GET" {
        let asset = match path {
            "/" => Some((
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html").as_slice(),
            )),
            "/notebook.css" => Some((
                "text/css; charset=utf-8",
                include_bytes!("../web/notebook.css").as_slice(),
            )),
            "/notebook.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/notebook.mjs").as_slice(),
            )),
            "/connection.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/connection.mjs").as_slice(),
            )),
            "/answers.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/answers.mjs").as_slice(),
            )),
            "/documents.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/documents.mjs").as_slice(),
            )),
            "/graph.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/graph.mjs").as_slice(),
            )),
            "/examples/arithmetic.chrnb" => Some((
                "application/json",
                include_bytes!("../examples/arithmetic.chrnb").as_slice(),
            )),
            "/examples/type-synthesis.chrnb" => Some((
                "application/json",
                include_bytes!("../examples/type-synthesis.chrnb").as_slice(),
            )),
            "/examples/behavior-synthesis.chrnb" => Some((
                "application/json",
                include_bytes!("../examples/behavior-synthesis.chrnb").as_slice(),
            )),
            "/examples/lambda.chrnb" => Some((
                "application/json",
                include_bytes!("../examples/lambda.chrnb").as_slice(),
            )),
            _ => None,
        };
        return match asset {
            Some((mime, body)) => write_response(&mut stream, 200, mime, body),
            None => write_response(&mut stream, 404, "text/plain", b"Not found"),
        };
    }
    if method != "POST"
        || headers.contains_key("transfer-encoding")
        || headers
            .get("content-type")
            .is_none_or(|value| value.split(';').next() != Some("application/json"))
    {
        return write_json_error(&mut stream, 400, "expected JSON POST with Content-Length");
    }
    let Some(length) = headers
        .get("content-length")
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return write_json_error(&mut stream, 400, "missing Content-Length");
    };
    if length > BODY_LIMIT {
        return write_json_error(&mut stream, 413, "request too large");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let response = match std::str::from_utf8(&body) {
        Ok(body) => runtime.request(path, body),
        Err(_) => Response::error(400, "expected UTF-8"),
    };
    write_response(
        &mut stream,
        response.status,
        "application/json",
        &serde_json::to_vec(&response.body)?,
    )
}
fn write_json_error(stream: &mut TcpStream, status: u16, message: &str) -> io::Result<()> {
    write_response(
        stream,
        status,
        "application/json",
        &serde_json::to_vec(&Response::error(status, message).body)?,
    )
}
fn write_response(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    fn attach(runtime: &Runtime) -> u64 {
        let owner = runtime
            .request("/api/reserve", &json!({"boot":runtime.boot}).to_string())
            .body["owner"]
            .as_u64()
            .unwrap();
        assert_eq!(
            runtime
                .request(
                    "/api/attach",
                    &json!({"boot":runtime.boot,"owner":owner}).to_string()
                )
                .status,
            200
        );
        owner
    }
    fn start_body(runtime: &Runtime, owner: u64) -> String {
        json!({"boot":runtime.boot,"owner":owner,"command":1,"program":{"rules":[]},"query":{"kind":"true"}}).to_string()
    }
    #[test]
    fn scheduler_executes_without_output_requests_and_retains_results() {
        let runtime = Runtime::default();
        let owner = attach(&runtime);
        let run = runtime
            .request("/api/start", &start_body(&runtime, owner))
            .body["run"]
            .clone();
        for _ in 0..1000 {
            runtime.tick();
        }
        let request = json!({"boot":runtime.boot,"owner":owner,"run":run}).to_string();
        let status = runtime.request("/api/status", &request);
        assert_eq!(status.body["execution_done"], true);
        let result = runtime.request("/api/output", &request);
        assert_eq!(result.status, 200);
        assert_eq!(
            result.body["events"].as_array().unwrap().last().unwrap()["kind"],
            "end"
        );
        assert_eq!(result.body["delivery_done"], true);
        assert_eq!(runtime.request("/api/output", &request).body, result.body);
    }
    #[test]
    fn continuing_a_completed_step_preserves_its_boundary() {
        let runtime = Runtime::default();
        let owner = attach(&runtime);
        let mut model = runtime
            .request(
                "/api/parse",
                &json!({"program":"p <=> q. q <=> p.","query":"p"}).to_string(),
            )
            .body;
        model["boot"] = json!(runtime.boot);
        model["owner"] = json!(owner);
        model["command"] = json!(1);
        model["paused"] = json!(true);
        let run = runtime.request("/api/start", &model.to_string()).body["run"].clone();
        let mut request = json!({"boot":runtime.boot,"owner":owner,"run":run,"command":2});
        assert_eq!(
            runtime.request("/api/step", &request.to_string()).status,
            200
        );
        request["command"] = json!(3);
        assert_eq!(
            runtime.request("/api/pause", &request.to_string()).status,
            200
        );
        for _ in 0..1000 {
            runtime.tick();
        }
        let identity = json!({"boot":runtime.boot,"owner":owner,"run":run}).to_string();
        assert_eq!(
            runtime.request("/api/status", &identity).body["applications"],
            0
        );
        request["command"] = json!(4);
        request["continue_step"] = json!(true);
        assert_eq!(
            runtime.request("/api/resume", &request.to_string()).status,
            200
        );
        for _ in 0..1000 {
            runtime.tick();
        }
        request["command"] = json!(5);
        request["continue_step"] = json!(true);
        assert_eq!(
            runtime.request("/api/resume", &request.to_string()).status,
            200
        );
        for _ in 0..1000 {
            runtime.tick();
        }
        let status = runtime.request(
            "/api/status",
            &json!({"boot":runtime.boot,"owner":owner,"run":run}).to_string(),
        );
        assert_eq!(status.body["applications"], 1);
        assert_eq!(status.body["running"], false);
    }
    #[test]
    fn output_reads_never_drive_source_and_spool_flush_failure_is_reported() {
        let runtime = Runtime::default();
        let owner = attach(&runtime);
        let run = runtime
            .request("/api/start", &start_body(&runtime, owner))
            .body["run"]
            .as_u64()
            .unwrap();
        let mut request = json!({"boot":runtime.boot,"owner":owner,"run":run});
        for _ in 0..10 {
            let response = runtime.request("/api/output", &request.to_string());
            assert_eq!(response.body["events"], json!([]));
            assert_eq!(response.body["delivery_done"], false);
            request["ack"] = response.body["sequence"].clone();
        }
        let entry = runtime.runs.lock().unwrap().entries[&run].clone();
        {
            let mut run = entry.lock().unwrap();
            // A read-only writer accepts this tiny answer into its buffer;
            // publication must surface the later flush failure.
            run.spool.writer = BufWriter::new(run.spool.reader.try_clone().unwrap());
        }
        for _ in 0..1000 {
            runtime.tick();
        }
        assert!(entry.lock().unwrap().engine.delivery_done());
        assert_eq!(
            runtime.request("/api/output", &request.to_string()).status,
            500
        );
        let run = entry.lock().unwrap();
        assert!(run.error.is_some());
        assert!(run.engine.canceled());
        assert!(!run.running);
    }
    #[test]
    fn spool_retains_more_than_ram_capacity_and_pages_exactly() {
        let runtime = Runtime::default();
        let mut spool = Spool::new(&runtime.boot, 1).unwrap();
        for n in 0..20000 {
            spool
                .append(json!({"kind":"port","variable":n.to_string()}))
                .unwrap();
        }
        assert!(spool.written > spool.writer.capacity() as u64);
        assert!(spool.writer.buffer().len() <= spool.writer.capacity());
        for n in 0..20000 {
            assert_eq!(
                spool.read(1).unwrap(),
                vec![json!({"kind":"port","variable":n.to_string()})]
            );
        }
        assert!(spool.drained());
    }
    #[test]
    fn waiting_requests_defer_only_their_owners_maintenance() {
        let runtime = Runtime::default();
        let a = attach(&runtime);
        let b = attach(&runtime);
        let runs: Vec<_> = [a, b]
            .into_iter()
            .map(|owner| {
                let id = runtime
                    .request("/api/start", &start_body(&runtime, owner))
                    .body["run"]
                    .as_u64()
                    .unwrap();
                runtime.runs.lock().unwrap().entries[&id].clone()
            })
            .collect();
        let owner = runtime.owner(a).unwrap();
        // A request is preempted after registering, before acquiring the mutex.
        owner.waiting.store(1, Ordering::SeqCst);
        for _ in 0..100 {
            runtime.tick();
        }
        assert!(!runs[0].lock().unwrap().engine.delivery_done());
        assert!(runs[1].lock().unwrap().engine.delivery_done());
        owner.waiting.store(0, Ordering::SeqCst);
        for _ in 0..100 {
            runtime.tick();
        }
        assert!(runs[0].lock().unwrap().engine.delivery_done());
    }
    #[test]
    fn independent_owners_and_reservation_do_not_wait_for_a_busy_owner() {
        let runtime = Arc::new(Runtime::default());
        let a = attach(&runtime);
        let b = attach(&runtime);
        let entry = runtime.owner(a).unwrap();
        let guard = entry.request();
        let (send, receive) = mpsc::channel();
        let other = runtime.clone();
        let worker = std::thread::spawn(move || {
            let started = other.request("/api/start", &start_body(&other, b));
            let reserved = other.request("/api/reserve", &json!({"boot":other.boot}).to_string());
            other.tick();
            send.send((started.status, reserved.status)).unwrap();
        });
        let completed = receive.recv_timeout(Duration::from_secs(2));
        drop(guard);
        worker.join().unwrap();
        assert_eq!(completed.unwrap(), (200, 200));
    }
    #[test]
    fn engine_entry_without_published_owner_mapping_is_never_executable() {
        let runtime = Runtime::default();
        let a = attach(&runtime);
        let b = attach(&runtime);
        let run = runtime.request("/api/start", &start_body(&runtime, b)).body["run"]
            .as_u64()
            .unwrap();
        runtime.owners.lock().unwrap().runs.remove(&run);
        // The engine entry is visible in the publication window, but not its owner mapping.
        let request = json!({"boot":runtime.boot,"owner":a,"run":run}).to_string();
        assert_eq!(runtime.request("/api/cancel", &request).status, 404);
        assert_eq!(runtime.request("/api/close", &request).body["closed"], true);
        assert!(
            !runtime.runs.lock().unwrap().entries[&run]
                .lock()
                .unwrap()
                .engine
                .canceled()
        );
        runtime.owners.lock().unwrap().runs.insert(run, b);
        assert_eq!(runtime.request("/api/cancel", &request).status, 403);
    }
    #[test]
    fn receipts_and_reservation_storage_stay_bounded_and_do_not_pin_closed_runs() {
        let runtime = Runtime::default();
        let owner = attach(&runtime);
        let run = runtime
            .request("/api/start", &start_body(&runtime, owner))
            .body["run"]
            .clone();
        for command in 2..=4096 {
            let body =
                json!({"boot":runtime.boot,"owner":owner,"run":run,"command":command}).to_string();
            assert_eq!(runtime.request("/api/resume", &body).status, 200);
            let entry = runtime.owner(owner).unwrap();
            let entry = entry.request();
            let receipt = entry.receipt.as_ref().unwrap();
            assert_eq!(receipt.command, command);
            assert_eq!(receipt.request, body);
            assert_eq!(receipt.response, json!({}));
        }
        for _ in 0..4096 {
            assert_eq!(
                runtime
                    .request("/api/reserve", &json!({"boot":runtime.boot}).to_string())
                    .status,
                200
            );
        }
        assert_eq!(runtime.owners.lock().unwrap().entries.len(), 1);
        let request = json!({"boot":runtime.boot,"owner":owner,"run":run}).to_string();
        runtime.request("/api/close", &request);
        for _ in 0..10000 {
            runtime.tick();
            if runtime.runs.lock().unwrap().entries.is_empty() {
                break;
            }
        }
        assert!(runtime.runs.lock().unwrap().entries.is_empty());
        let retained = runtime.owner(owner).unwrap();
        assert_eq!(retained.request().runs, 0);
        let retire = json!({"boot":runtime.boot,"owner":owner}).to_string();
        assert_eq!(runtime.request("/api/retire", &retire).status, 200);
        assert!(retained.request().retired);
        assert!(runtime.owners.lock().unwrap().entries.is_empty());
        assert!(runtime.owners.lock().unwrap().runs.is_empty());
        assert_eq!(
            runtime
                .request("/api/start", &start_body(&runtime, owner))
                .status,
            409
        );
    }
    #[test]
    fn retirement_and_start_are_fenced_under_the_same_owner_lock() {
        for _ in 0..64 {
            let runtime = Arc::new(Runtime::default());
            let owner = attach(&runtime);
            let gate = Arc::new(std::sync::Barrier::new(3));
            let jobs: Vec<_> = [false, true]
                .into_iter()
                .map(|retire| {
                    let runtime = runtime.clone();
                    let gate = gate.clone();
                    std::thread::spawn(move || {
                        gate.wait();
                        if retire {
                            runtime
                                .request(
                                    "/api/retire",
                                    &json!({"boot":runtime.boot,"owner":owner}).to_string(),
                                )
                                .status
                        } else {
                            runtime
                                .request("/api/start", &start_body(&runtime, owner))
                                .status
                        }
                    })
                })
                .collect();
            gate.wait();
            let results: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
            assert!(
                results == [200, 409] || results == [409, 200],
                "{results:?}"
            );
            let owners = runtime.owners.lock().unwrap();
            assert_eq!(owners.runs.len(), usize::from(results[0] == 200));
        }
    }
    #[test]
    fn busy_control_does_not_consume_its_command_and_owner_ids_never_wrap() {
        let runtime = Runtime::default();
        let owner = attach(&runtime);
        let run = runtime
            .request("/api/start", &start_body(&runtime, owner))
            .body["run"]
            .as_u64()
            .unwrap();
        let engine = runtime.runs.lock().unwrap().entries[&run].clone();
        {
            let mut engine = engine.lock().unwrap();
            engine.engine.request_collection();
            engine.engine.advance(1);
            assert!(engine.engine.collecting());
        }
        let request = json!({"boot":runtime.boot,"owner":owner,"run":run,"command":2}).to_string();
        let busy = runtime.request("/api/snapshot", &request);
        assert_eq!(busy.status, 409);
        assert_eq!(busy.body["retry"], true);
        assert_eq!(
            runtime
                .owner(owner)
                .unwrap()
                .request()
                .receipt
                .as_ref()
                .unwrap()
                .command,
            1
        );
        for _ in 0..1000 {
            runtime.request(
                "/api/maintenance",
                &json!({"boot":runtime.boot,"owner":owner,"run":run,"budget":4096}).to_string(),
            );
            if !engine.lock().unwrap().engine.collecting() {
                break;
            }
        }
        assert_eq!(runtime.request("/api/snapshot", &request).status, 200);
        runtime.owners.lock().unwrap().issued = MAX_SAFE_ID;
        assert_eq!(
            runtime
                .request("/api/reserve", &json!({"boot":runtime.boot}).to_string())
                .status,
            503
        );
        assert_eq!(runtime.owners.lock().unwrap().issued, MAX_SAFE_ID);
    }
}
