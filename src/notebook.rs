//! Loopback notebook transport. Each execution request has a finite work budget.
use crate::engine::{Engine, InspectionError, SnapshotKind, ViewId};
use crate::observe::Output;
use crate::program::prepare;
use crate::syntax::{
    Body, Program, format_program, format_query, parse_program, parse_query, validate_program,
    validate_query,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const BODY_LIMIT: usize = 4 * 1024 * 1024;
const BUDGET_LIMIT: usize = 4096;
#[derive(Default)]
pub struct Runtime {
    runs: Mutex<Runs>,
    batches: Mutex<BTreeMap<(u64, Option<u64>), Value>>,
}
#[derive(Default)]
struct Runs {
    closing: BTreeMap<u64, bool>,
    next: u64,
    entries: BTreeMap<u64, Arc<Mutex<Engine>>>,
    after: Option<u64>,
    round: Option<u64>,
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
    program: Program,
    query: Body,
    #[serde(default)]
    record_history: bool,
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
    run: u64,
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
    fn dispatch(&self, path: &str, body: &str) -> Result<Response, Response> {
        match path {
            "/api/parse" => {
                let input: Source = decode(body)?;
                let program = parse_program(&input.program).map_err(|e| Response::error(400, e))?;
                let query = parse_query(&input.query).map_err(|e| Response::error(400, e))?;
                return Ok(Response::ok(json!({"program":program, "query":query})));
            }
            "/api/format" | "/api/start" => {
                let input: Model = decode(body)?;
                validate_program(&input.program)
                    .and_then(|()| validate_query(&input.query))
                    .map_err(|e| Response::error(400, e))?;
                if path == "/api/format" {
                    return Ok(Response::ok(
                        json!({"program":format_program(&input.program), "query":format_query(&input.query)}),
                    ));
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
                runs.entries.insert(run, Arc::new(Mutex::new(engine)));
                return Ok(Response::ok(
                    json!({"run":run,"signatures":code.signatures,"variables":code.query_variables}),
                ));
            }
            _ => {}
        }
        let request: RunRequest = decode(body)?;
        if request.budget > BUDGET_LIMIT {
            return Err(Response::error(400, "work budget exceeds 4096"));
        }
        let run = self
            .runs
            .lock()
            .unwrap()
            .entries
            .get(&request.run)
            .cloned()
            .ok_or_else(|| Response::error(404, "unknown run"))?;
        let mut e = run.lock().unwrap();
        if self.runs.lock().unwrap().closing.contains_key(&request.run)
            && !matches!(path, "/api/status" | "/api/close" | "/api/cancel")
        {
            return Err(Response::error(409, "run is closing"));
        }
        let stream = match path {
            "/api/advance" => Some((request.run, None)),
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
        let mut body = match path {
            "/api/advance" => {
                let mut events = vec![];
                for _ in 0..request.budget {
                    e.advance(1);
                    if let Some(output) = e.take_output() {
                        events.push(event(output));
                    }
                }
                json!({"events":events,"applications":e.applications(),"exhausted":e.exhausted(),"delivery_done":e.delivery_done(),"canceled":e.canceled(),"step":e.step_status()})
            }
            "/api/step" => {
                view_result(e.request_step(choices(&request)?))?;
                json!({"step":e.step_status()})
            }
            "/api/resume" => {
                view_result(e.resume())?;
                json!({})
            }
            "/api/maintenance" => {
                e.maintain(request.budget);
                json!({"collecting":e.collecting()})
            }
            "/api/inspect" => {
                // Initial identifier allocation is the only source work allowed by an inspection retry.
                if !e.canceled() && e.query_variables().len() != e.program().query_variables.len() {
                    for _ in 0..request.budget {
                        if e.query_variables().len() == e.program().query_variables.len() {
                            break;
                        }
                        e.advance(1);
                    }
                }
                let id = view_result(
                    e.start_inspection(request.snapshot.map(Id::view), choices(&request)?),
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
                    view_result(e.advance_inspection(id, 1))?;
                    if let Some(output) = view_result(e.take_inspection_output(id))? {
                        events.push(event(output));
                    }
                    if view_result(e.inspection_status(id))?.done {
                        break;
                    }
                }
                let status = view_result(e.inspection_status(id))?;
                json!({"events":events,"done":status.done,"canceled":status.canceled,"error":status.error})
            }
            "/api/inspect_cancel" => {
                let id = request
                    .inspection
                    .ok_or_else(|| Response::error(400, "missing inspection"))?
                    .view();
                view_result(e.cancel_inspection(id))?;
                view_result(e.advance_inspection(id, request.budget))?;
                json!({"done":view_result(e.inspection_status(id))?.done})
            }
            "/api/inspect_release" => {
                let id = request
                    .inspection
                    .ok_or_else(|| Response::error(400, "missing inspection"))?
                    .view();
                match e.release_inspection(id) {
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
            "/api/snapshot" => json!({"snapshot":view_result(e.capture_snapshot())?.0.to_string()}),
            "/api/snapshot_release" => {
                view_result(
                    e.release_snapshot(
                        request
                            .snapshot
                            .ok_or_else(|| Response::error(400, "missing snapshot"))?
                            .view(),
                    ),
                )?;
                json!({})
            }
            "/api/views" => {
                let cutoff = if let Some(id) = request.inspection {
                    view_result(e.inspection_snapshot_info(id.view()))?.last_choice
                } else if let Some(id) = request.snapshot {
                    view_result(e.snapshot_info(id.view()))?.last_choice
                } else {
                    e.choices().last().map(|(&id, _)| id)
                };
                let mut choices = e
                    .choices_after(request.after_choice.map(|id| id.view().0), cutoff)
                    .take(65)
                    .map(|(&id, _)| json!({"id":id.to_string(),"label":format!("Choice {}",id+1)}))
                    .collect::<Vec<_>>();
                let next_choice = (choices.len() > 64).then(|| choices[63]["id"].clone());
                choices.truncate(64);
                let mut snapshots = e.snapshots_after(request.after_snapshot.map(Id::view)).take(65).map(|s| {
                    let action = match s.kind { SnapshotKind::Initial=>"initial query",SnapshotKind::Requested=>"retained view",SnapshotKind::Application{..}=>"rule application",SnapshotKind::Post{..}=>"relation posted",SnapshotKind::Merge=>"variables merged",SnapshotKind::Choice=>"disjunction",SnapshotKind::Failure=>"failure",SnapshotKind::NormalForm=>"normal form" };
                    json!({"id":s.id.0.to_string(),"label":format!("State {} · {action}",s.id.0)})
                }).collect::<Vec<_>>();
                let next_snapshot = (snapshots.len() > 64).then(|| snapshots[63]["id"].clone());
                snapshots.truncate(64);
                json!({"choices":choices,"snapshots":snapshots,"next_choice":next_choice,"next_snapshot":next_snapshot})
            }
            "/api/cancel" => {
                e.cancel();
                let pending = self
                    .batches
                    .lock()
                    .unwrap()
                    .get(&(request.run, None))
                    .cloned();
                json!({"done":e.cancel_done(),"pending":pending})
            }
            "/api/close" => {
                e.cancel();
                self.runs
                    .lock()
                    .unwrap()
                    .closing
                    .entry(request.run)
                    .or_insert(false);
                json!({})
            }
            "/api/status" => {
                json!({"applications":e.applications(),"canceled":e.canceled(),"cancel_done":e.cancel_done(),"memory":e.memory()})
            }
            _ => return Err(Response::error(404, "unknown API route")),
        };
        if let Some(key) = stream {
            body["sequence"] = json!(sequence);
            self.batches.lock().unwrap().insert(key, body.clone());
        }
        Ok(Response::ok(body))
    }
    /// One run per maintenance turn, with a frozen admission boundary for fairness.
    pub fn tick(&self) {
        let run = {
            let mut runs = self.runs.lock().unwrap();
            if runs.round.is_none() {
                runs.round = runs.entries.last_key_value().map(|(&id, _)| id);
            }
            let Some(last) = runs.round else {
                return;
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
                None
            }
        };
        if let Some((id, run, closing)) = run
            && let Ok(mut e) = run.try_lock()
        {
            match closing {
                Some(false) => {
                    e.advance(512);
                    if e.cancel_done() {
                        self.runs.lock().unwrap().closing.insert(id, true);
                    }
                }
                Some(true) => {
                    // Drop view roots in bounded batches BEFORE another sweep.
                    // Sweeping after every individual release would rescan the
                    // remaining history once per snapshot.
                    let mut released_all = false;
                    for _ in 0..512 {
                        let snapshot = e.snapshots().next().map(|s| s.id);
                        let inspection = e.inspections().next();
                        if let Some(snapshot) = snapshot {
                            if e.release_snapshot(snapshot).is_err() {
                                break;
                            }
                        } else if let Some(inspection) = inspection {
                            if e.inspection_status(inspection).is_ok_and(|s| !s.done) {
                                let _ = e.discard_inspection(inspection, 512);
                                let _ = e.release_inspection(inspection);
                                break;
                            }
                            if e.release_inspection(inspection).is_err() {
                                break;
                            }
                        } else {
                            released_all = true;
                            break;
                        }
                    }
                    if released_all {
                        e.advance(512);
                        if e.cancel_done()
                            && e.memory().graph_nodes == 0
                            && e.memory().conditions == 0
                        {
                            let mut batches = self.batches.lock().unwrap();
                            let batch = batches
                                .range((id, None)..=(id, Some(u64::MAX)))
                                .next()
                                .map(|(&key, _)| key);
                            if let Some(key) = batch {
                                batches.remove(&key);
                                return;
                            }
                            drop(batches);
                            drop(e);
                            let mut runs = self.runs.lock().unwrap();
                            runs.entries.remove(&id);
                            runs.closing.remove(&id);
                        }
                    }
                }
                None => {
                    if e.canceled() {
                        e.advance(512);
                    }
                }
            }
        }
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
            maintenance.tick();
            std::thread::sleep(Duration::from_millis(5));
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
            return write_response(
                &mut stream,
                400,
                "application/json",
                br#"{"error":"invalid HTTP headers"}"#,
            );
        }
        if &head[before..] == b"\r\n" {
            break;
        }
    }
    let Ok(head) = std::str::from_utf8(&head) else {
        return write_response(
            &mut stream,
            400,
            "application/json",
            br#"{"error":"invalid HTTP headers"}"#,
        );
    };
    let mut lines = head.split("\r\n");
    let parts = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return write_response(
            &mut stream,
            400,
            "application/json",
            br#"{"error":"expected HTTP/1.1"}"#,
        );
    }
    let (method, path) = (parts[0], parts[1]);
    let mut headers = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once(':') else {
            return write_response(
                &mut stream,
                400,
                "application/json",
                br#"{"error":"invalid header"}"#,
            );
        };
        if headers
            .insert(key.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return write_response(
                &mut stream,
                400,
                "application/json",
                br#"{"error":"duplicate header"}"#,
            );
        }
    }
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host = headers.get("host").copied().unwrap_or_default();
    if !hosts.iter().any(|h| h == host)
        || headers
            .get("origin")
            .is_some_and(|origin| **origin != format!("http://{host}"))
    {
        return write_response(
            &mut stream,
            403,
            "application/json",
            br#"{"error":"notebook requests must be same-origin"}"#,
        );
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
            "/graph.mjs" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("../web/graph.mjs").as_slice(),
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
        return write_response(
            &mut stream,
            400,
            "application/json",
            br#"{"error":"expected JSON POST with Content-Length"}"#,
        );
    }
    let Some(length) = headers
        .get("content-length")
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return write_response(
            &mut stream,
            400,
            "application/json",
            br#"{"error":"missing Content-Length"}"#,
        );
    };
    if length > BODY_LIMIT {
        return write_response(
            &mut stream,
            413,
            "application/json",
            br#"{"error":"request too large"}"#,
        );
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
fn write_response(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} Response\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
