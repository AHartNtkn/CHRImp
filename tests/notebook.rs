use chr::notebook::Runtime;
use serde_json::{Value, json};
// Each fixture owns one attached notebook and admits commands only after success.
struct Client {
    runtime: Runtime,
    owner: u64,
    next: std::sync::atomic::AtomicU64,
}
impl Default for Client {
    fn default() -> Self {
        let runtime = Runtime::default();
        let token = boot(&runtime);
        let reserved = runtime.request("/api/reserve", &json!({"boot":token}).to_string());
        assert_eq!(reserved.status, 200, "{:?}", reserved.body);
        let owner = reserved.body["owner"].as_u64().unwrap();
        assert_eq!(
            runtime
                .request(
                    "/api/attach",
                    &json!({"boot":token,"owner":owner}).to_string()
                )
                .status,
            200
        );
        Self {
            runtime,
            owner,
            next: std::sync::atomic::AtomicU64::new(1),
        }
    }
}
impl std::ops::Deref for Client {
    type Target = Runtime;
    fn deref(&self) -> &Runtime {
        &self.runtime
    }
}
fn command_body(client: &Client, path: &str, mut body: Value) -> Value {
    if matches!(
        path,
        "/api/start"
            | "/api/inspect"
            | "/api/step"
            | "/api/resume"
            | "/api/pause"
            | "/api/snapshot"
    ) && body.get("command").is_none()
    {
        body["command"] = json!(client.next.load(std::sync::atomic::Ordering::Relaxed));
    }
    body
}
fn admitted(client: &Client, body: &Value) {
    if let Some(command) = body["command"].as_u64() {
        client
            .next
            .fetch_max(command + 1, std::sync::atomic::Ordering::Relaxed);
    }
}
fn boot(runtime: &Runtime) -> Value {
    let response = runtime.request("/api/hello", "{}");
    assert_eq!(response.status, 200);
    response.body["boot"].clone()
}
fn execution_body(runtime: &Client, mut body: Value) -> String {
    body["boot"] = boot(runtime);
    body["owner"] = json!(runtime.owner);
    body.to_string()
}
fn ok(runtime: &Client, path: &str, body: Value) -> Value {
    // The fixture supplies independent scheduler turns; transport only reads.
    if path == "/api/output" {
        for _ in 0..32 {
            runtime.tick();
        }
    }
    let command = command_body(runtime, path, body);
    let body = if matches!(path, "/api/parse" | "/api/format" | "/api/hello") {
        command.to_string()
    } else {
        execution_body(runtime, command.clone())
    };
    let response = runtime.request(path, &body);
    assert_eq!(response.status, 200, "{path}: {:?}", response.body);
    admitted(runtime, &command);
    response.body
}
fn next(runtime: &Client, path: &str, mut body: Value, ack: &mut Option<u64>) -> Value {
    body["ack"] = json!(*ack);
    let batch = ok(runtime, path, body);
    *ack = Some(batch["sequence"].as_u64().unwrap());
    batch
}
fn start(runtime: &Client, program: &str, query: &str, history: bool) -> Value {
    let mut model = ok(
        runtime,
        "/api/parse",
        json!({"program":program,"query":query}),
    );
    let text = ok(runtime, "/api/format", model.clone());
    assert_eq!(ok(runtime, "/api/parse", text), model);
    model["record_history"] = json!(history);
    ok(runtime, "/api/start", model)
}
fn retry(runtime: &Client, path: &str, body: Value) -> Value {
    let body = command_body(runtime, path, body);
    for _ in 0..1000 {
        let response = runtime.request(path, &execution_body(runtime, body.clone()));
        if response.status == 200 {
            admitted(runtime, &body);
            return response.body;
        }
        assert_eq!(response.body["retry"], true, "{path}: {:?}", response.body);
        runtime.tick();
        ok(
            runtime,
            "/api/maintenance",
            json!({"run":body["run"],"budget":4096}),
        );
    }
    panic!("request did not finish")
}
#[test]
fn notebook_protocol_executes_and_replays_a_recorded_graph_in_bounded_batches() {
    let runtime = Client::default();
    let metadata = start(&runtime, "edge(X,Y) ==> reverse(Y,X).", "edge(A,B)", true);
    assert_eq!(metadata["variables"], json!(["A", "B"]));
    let run = metadata["run"].clone();
    let mut ack = None;
    let mut events = vec![];
    let mut finished = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":17}),
            &mut ack,
        );
        assert!(batch["events"].as_array().unwrap().len() <= 17);
        events.extend(batch["events"].as_array().unwrap().clone());
        if batch["delivery_done"] == true {
            finished = true;
            break;
        }
    }
    assert!(finished);
    assert_eq!(events.iter().filter(|e| e["kind"] == "end").count(), 1);
    assert_eq!(events.iter().filter(|e| e["kind"] == "fact").count(), 2);
    assert!(
        events
            .iter()
            .filter_map(|e| e.get("variable"))
            .all(Value::is_string)
    );
    let views = ok(&runtime, "/api/views", json!({"run":run}));
    let snapshot = views["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["label"].as_str().unwrap().ends_with("normal form"))
        .unwrap()["id"]
        .clone();
    let inspection = retry(
        &runtime,
        "/api/inspect",
        json!({"run":run,"snapshot":snapshot}),
    )["inspection"]
        .clone();
    let mut inspect_ack = None;
    let mut inspected = vec![];
    let mut done = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/inspect_advance",
            json!({"run":run,"inspection":inspection,"budget":7}),
            &mut inspect_ack,
        );
        assert!(batch["events"].as_array().unwrap().len() <= 7);
        inspected.extend(batch["events"].as_array().unwrap().clone());
        if batch["done"] == true {
            done = true;
            break;
        }
    }
    assert!(done);
    assert_eq!(&events[1..], &inspected[1..]);
    retry(
        &runtime,
        "/api/inspect_release",
        json!({"run":run,"inspection":inspection}),
    );
}
#[test]
fn step_stops_at_a_logical_application_and_cancel_progresses_without_client_polling() {
    let runtime = Client::default();
    let run = start(&runtime, "p(X) <=> q(X). q(X) <=> p(X).", "p(A)", true)["run"].clone();
    retry(&runtime, "/api/step", json!({"run":run}));
    let mut ack = None;
    let mut finished = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":19}),
            &mut ack,
        );
        if batch["step"]["done"] == true {
            assert_eq!(batch["applications"], 1);
            finished = true;
            break;
        }
    }
    assert!(finished);
    assert_eq!(
        next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":4096}),
            &mut ack
        )["applications"],
        1
    );
    retry(&runtime, "/api/resume", json!({"run":run}));
    assert!(
        next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":4096}),
            &mut ack
        )["applications"]
            .as_u64()
            .unwrap()
            > 1
    );
    let before = ok(&runtime, "/api/status", json!({"run":run}))["applications"].clone();
    ok(&runtime, "/api/cancel", json!({"run":run}));
    for _ in 0..1000 {
        runtime.tick();
        if ok(&runtime, "/api/status", json!({"run":run}))["cancel_done"] == true {
            break;
        }
    }
    let status = ok(&runtime, "/api/status", json!({"run":run}));
    assert_eq!(status["applications"], before);
    assert_eq!(status["cancel_done"], true);
    assert!(status["memory"]["snapshots"].as_u64().unwrap() > 0);
}
#[test]
fn malformed_requests_are_rejected_before_creating_or_mutating_a_run() {
    let runtime = Client::default();
    assert_eq!(runtime.request("/api/start", &execution_body(&runtime, json!({"program":{"rules":[]},"query":{"kind":"atom","atom":{"relation":"p","args":[42]}}}))).status,400);
    let deep = format!(
        r#"{{"boot":{},"program":{{"rules":[]}},"query":{}{{"kind":"true"}}{}}}"#,
        boot(&runtime),
        r#"{"kind":"and","items":["#.repeat(1000),
        "]}".repeat(1000)
    );
    assert_eq!(runtime.request("/api/start", &deep).status, 400);
    let run = start(&runtime, "", "true", false)["run"].clone();
    assert_eq!(run, 1, "invalid input must not create runs");
    assert_eq!(
        runtime
            .request(
                "/api/output",
                &execution_body(&runtime, json!({"run":run,"budget":4097}))
            )
            .status,
        400
    );
    assert_eq!(
        runtime
            .request(
                "/api/inspect",
                &execution_body(
                    &runtime,
                    json!({"run":run,"command":2,"choices":{"-1":true}})
                )
            )
            .status,
        400
    );
    assert_eq!(
        runtime
            .request("/api/output", &execution_body(&runtime, json!({"run":999})))
            .status,
        404
    );
    assert_eq!(
        runtime
            .request("/api/start", &" ".repeat(4 * 1024 * 1024 + 1))
            .status,
        413
    );
}

#[test]
fn loopback_http_serves_the_notebook_and_enforces_request_boundaries() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::process::{Command, Stdio};
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_chr"))
            .args(["--notebook", "--port", "0"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let address = line.trim().strip_prefix("Notebook: http://").unwrap();
    let send = |request: String| {
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        socket.write_all(request.as_bytes()).unwrap();
        let mut result = String::new();
        socket.read_to_string(&mut result).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; response so far: {result}",
                request.lines().next().unwrap()
            )
        });
        result
    };
    let post = |route: &str, body: Value| {
        let body = body.to_string();
        let response = send(format!(
            "POST /api/{route} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ));
        assert!(response.starts_with("HTTP/1.1 200"), "{route}: {response}");
        serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    };
    let token = post("hello", json!({}))["boot"].clone();
    let owner = post("reserve", json!({"boot":token}))["owner"].clone();
    post("attach", json!({"boot":token,"owner":owner}));
    let mut model = post("parse", json!({"program":"", "query":"p(A);q(A)"}));
    model["boot"] = token.clone();
    model["owner"] = owner.clone();
    model["command"] = json!(1);
    let run = post("start", model)["run"].clone();
    // No HTTP requests, including status or delivery, during this interval.
    std::thread::sleep(std::time::Duration::from_millis(150));
    let identity = json!({"boot":token,"owner":owner,"run":run});
    assert_eq!(post("status", identity.clone())["execution_done"], true);
    let mut request = identity.clone();
    request["budget"] = json!(1);
    let first = post("output", request.clone());
    assert_eq!(
        post("output", request.clone()),
        first,
        "lost delivery replays exactly"
    );
    let mut batch = first;
    let mut events = Vec::new();
    loop {
        events.extend(batch["events"].as_array().unwrap().clone());
        if batch["delivery_done"] == true {
            break;
        }
        request["ack"] = batch["sequence"].clone();
        batch = post("output", request.clone());
    }
    assert_eq!(events.iter().filter(|e| e["kind"] == "end").count(), 2);
    assert_eq!(events.iter().filter(|e| e["kind"] == "fact").count(), 2);
    post("close", identity);

    let mut model = post("parse", json!({"program":"p <=> q. q <=> p.", "query":"p"}));
    model["boot"] = token.clone();
    model["owner"] = owner.clone();
    model["command"] = json!(2);
    model["paused"] = json!(true);
    let run = post("start", model)["run"].clone();
    let identity = json!({"boot":token,"owner":owner,"run":run});
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(post("status", identity.clone())["applications"], 0);
    let mut control = identity.clone();
    control["command"] = json!(3);
    post("step", control.clone());
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(post("status", identity.clone())["applications"], 1);
    assert_eq!(post("status", identity.clone())["running"], false);
    control["command"] = json!(4);
    post("resume", control.clone());
    std::thread::sleep(std::time::Duration::from_millis(100));
    // Continuous execution must hand the owner lock to waiting HTTP requests.
    for _ in 0..16 {
        assert_eq!(post("status", identity.clone())["running"], true);
    }
    control["command"] = json!(5);
    post("pause", control.clone());
    let paused = post("status", identity.clone())["applications"]
        .as_u64()
        .unwrap();
    assert!(paused > 1);
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(post("status", identity.clone())["applications"], paused);
    assert_eq!(post("pause", control.clone()), json!({}));
    control["command"] = json!(6);
    post("resume", control);
    std::thread::sleep(std::time::Duration::from_millis(50));
    post("cancel", identity.clone());
    let canceled = post("status", identity.clone())["applications"].clone();
    assert!(canceled.as_u64().unwrap() > paused);
    std::thread::sleep(std::time::Duration::from_millis(100));
    let status = post("status", identity.clone());
    assert_eq!(status["applications"], canceled);
    assert_eq!(status["cancel_done"], true);
    post("close", identity);
    let home = send(format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n"));
    assert!(home.starts_with("HTTP/1.1 200"));
    assert!(home.contains("CHR · Language notebook"));
    assert!(home.contains("Content-Security-Policy:"));
    let hello_body = "{}";
    let hello = send(format!(
        "POST /api/hello HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{hello_body}",
        hello_body.len()
    ));
    assert!(hello.starts_with("HTTP/1.1 200"));
    let hello: Value = serde_json::from_str(hello.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_ne!(
        hello["boot"],
        boot(&Client::default()),
        "separate server processes have separate incarnations"
    );
    let boot = hello["boot"].as_str().unwrap();
    assert_eq!(boot.len(), 32);
    assert!(
        boot.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let body = r#"{"program":"","query":"p(X)"}"#;
    let response = send(format!(
        "POST /api/parse HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ));
    assert!(response.starts_with("HTTP/1.1 200"));
    let model: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(model["query"]["atom"]["args"], json!(["X"]));
    assert!(send(format!("POST /api/parse HTTP/1.1\r\nHost: {address}\r\nOrigin: https://example.com\r\nContent-Length: 0\r\n\r\n")).starts_with("HTTP/1.1 403"));
    assert!(send("GET / HTTP/1.1\r\nHost: example.com\r\n\r\n".into()).starts_with("HTTP/1.1 403"));
    assert!(send(format!("POST /api/parse HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n")).starts_with("HTTP/1.1 400"));
    assert!(send(format!("POST /api/parse HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: 4194305\r\n\r\n")).starts_with("HTTP/1.1 413"));
}

#[test]
fn releasing_a_recorded_execution_reclaims_its_run_without_losing_other_runs() {
    let runtime = Client::default();
    let closing = start(&runtime, "p(X) <=> p(X).", "p(A)", true)["run"].clone();
    let kept = start(&runtime, "", "kept(X)", false)["run"].clone();
    let mut ack = None;
    for _ in 0..50 {
        next(
            &runtime,
            "/api/output",
            json!({"run":closing,"budget":4096}),
            &mut ack,
        );
    }
    assert!(
        ok(&runtime, "/api/status", json!({"run":closing}))["memory"]["snapshots"]
            .as_u64()
            .unwrap()
            > 100
    );
    ok(&runtime, "/api/close", json!({"run":closing}));
    assert_eq!(
        runtime
            .request(
                "/api/inspect",
                &execution_body(&runtime, json!({"run":closing,"command":3}))
            )
            .status,
        409
    );
    let mut released = false;
    for _ in 0..10000 {
        runtime.tick();
        if runtime
            .request(
                "/api/status",
                &execution_body(&runtime, json!({"run":closing})),
            )
            .status
            == 404
        {
            released = true;
            break;
        }
    }
    assert!(released, "closed execution must finish cleanup");
    assert_eq!(
        runtime
            .request(
                "/api/status",
                &execution_body(&runtime, json!({"run":kept}))
            )
            .status,
        200
    );
}

#[test]
fn deep_containers_are_rejected_as_ids_without_recursive_buffering() {
    let runtime = Client::default();
    for field in [
        "snapshot",
        "inspection",
        "after_snapshot",
        "after_choice",
        "before_snapshot",
        "before_choice",
    ] {
        let source = format!(
            "{{\"boot\":{},\"run\":1,\"{field}\":{}0{}}}",
            boot(&runtime),
            "[".repeat(30000),
            "]".repeat(30000)
        );
        assert_eq!(runtime.request("/api/inspect", &source).status, 400);
    }
    let run = start(&runtime, "", "true", false)["run"].clone();
    assert_eq!(run, 1);
}
#[test]
fn unacknowledged_output_batches_are_replayed_exactly_after_response_loss() {
    let runtime = Client::default();
    let run = start(&runtime, "", "(p(A);p(A))", true)["run"].clone();
    let mut ack = None;
    let first = next(
        &runtime,
        "/api/output",
        json!({"run":run,"budget":4096}),
        &mut ack,
    );
    assert!(!first["events"].as_array().unwrap().is_empty());
    for _ in 0..3 {
        assert_eq!(
            ok(&runtime, "/api/output", json!({"run":run,"budget":1})),
            first
        );
    }
    let second = next(
        &runtime,
        "/api/output",
        json!({"run":run,"budget":4096}),
        &mut ack,
    );
    assert_eq!(second["sequence"], 2);
    assert!(second["events"].as_array().unwrap().is_empty());
    assert_eq!(
        runtime
            .request(
                "/api/output",
                &execution_body(&runtime, json!({"run":run,"ack":99}))
            )
            .status,
        400
    );
    let views = ok(&runtime, "/api/views", json!({"run":run}));
    let snapshot = views["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["label"].as_str().unwrap().ends_with("normal form"))
        .unwrap()["id"]
        .clone();
    let inspection = retry(
        &runtime,
        "/api/inspect",
        json!({"run":run,"snapshot":snapshot}),
    )["inspection"]
        .clone();
    let first = ok(
        &runtime,
        "/api/inspect_advance",
        json!({"run":run,"inspection":inspection,"budget":4096}),
    );
    assert_eq!(
        ok(
            &runtime,
            "/api/inspect_advance",
            json!({"run":run,"inspection":inspection,"budget":1})
        ),
        first
    );
    for _ in 0..2 {
        let canceled = retry(
            &runtime,
            "/api/inspect_cancel",
            json!({"run":run,"inspection":inspection,"budget":4096}),
        );
        assert_eq!(canceled["pending"], first);
    }
}

#[test]
fn releasing_inspection_zero_preserves_source_recovery() {
    let runtime = Client::default();
    let run = start(&runtime, "", "p(A)", false)["run"].clone();
    let batch = ok(&runtime, "/api/output", json!({"run":run,"budget":4096}));
    let inspection = retry(&runtime, "/api/inspect", json!({"run":run}))["inspection"].clone();
    // Release is idempotent, including an already absent inspector zero.
    retry(
        &runtime,
        "/api/inspect_release",
        json!({"run":run,"inspection":"0"}),
    );
    let mut ack = None;
    loop {
        if next(
            &runtime,
            "/api/inspect_advance",
            json!({"run":run,"inspection":inspection,"budget":4096}),
            &mut ack,
        )["done"]
            == true
        {
            break;
        }
    }
    retry(
        &runtime,
        "/api/inspect_release",
        json!({"run":run,"inspection":inspection}),
    );
    assert_eq!(
        ok(&runtime, "/api/output", json!({"run":run,"budget":1})),
        batch
    );
}

#[test]
fn closing_discards_inspections_created_after_source_cancellation() {
    let runtime = Client::default();
    let run = start(&runtime, "", "p(A)", true)["run"].clone();
    ok(&runtime, "/api/output", json!({"run":run,"budget":4096}));
    let views = ok(&runtime, "/api/views", json!({"run":run}));
    let snapshot = views["snapshots"].as_array().unwrap().last().unwrap()["id"].clone();
    ok(&runtime, "/api/cancel", json!({"run":run}));
    let mut canceled = false;
    for _ in 0..10000 {
        runtime.tick();
        if ok(&runtime, "/api/status", json!({"run":run}))["cancel_done"] == true {
            canceled = true;
            break;
        }
    }
    assert!(canceled);
    retry(
        &runtime,
        "/api/inspect",
        json!({"run":run,"snapshot":snapshot}),
    );
    ok(&runtime, "/api/close", json!({"run":run}));
    for _ in 0..10000 {
        runtime.tick();
        if runtime
            .request("/api/status", &execution_body(&runtime, json!({"run":run})))
            .status
            == 404
        {
            return;
        }
    }
    panic!("closing must discard inspections admitted after cancellation");
}

#[test]
fn metadata_pages_round_trip_without_replaying_prefixes() {
    let runtime = Client::default();
    let query = std::iter::repeat_n("(true;true)", 70)
        .collect::<Vec<_>>()
        .join(",");
    let run = start(&runtime, "", &query, true)["run"].clone();
    let mut ack = None;
    let mut first = Value::Null;
    for _ in 0..100 {
        next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":4096}),
            &mut ack,
        );
        first = ok(&runtime, "/api/views", json!({"run":run}));
        if !first["next_choice"].is_null() && !first["next_snapshot"].is_null() {
            break;
        }
    }
    for kind in ["choice", "snapshot"] {
        let plural = if kind == "choice" {
            "choices"
        } else {
            "snapshots"
        };
        let after = format!("after_{kind}");
        let before = format!("before_{kind}");
        assert!(!first[format!("next_{kind}")].is_null());
        assert_eq!(first[plural].as_array().unwrap().len(), 64);
        let mut request = json!({"run":run});
        request[&after] = first[format!("next_{kind}")].clone();
        let second = ok(&runtime, "/api/views", request.clone());
        assert!(second[plural].as_array().unwrap().len() <= 64);
        assert_ne!(second[plural][0], first[plural][0]);
        let mut back = json!({"run":run});
        back[&before] = second[format!("prev_{kind}")].clone();
        assert_eq!(ok(&runtime, "/api/views", back)[plural], first[plural]);
        request[&before] = json!("1");
        assert_eq!(
            runtime
                .request("/api/views", &execution_body(&runtime, request))
                .status,
            400
        );
    }
    ok(&runtime, "/api/cancel", json!({"run":run}));
}

#[test]
fn logical_step_inspection_streams_the_pending_rhs_with_string_variable_ids() {
    let runtime = Client::default();
    let run = start(&runtime, "p(X) <=> q(X). q(X) <=> r(X).", "p(A)", false)["run"].clone();
    retry(&runtime, "/api/step", json!({"run":run}));
    let mut ack = None;
    let mut event = Value::Null;
    for _ in 0..100 {
        let batch = next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":4096}),
            &mut ack,
        );
        if batch["step"]["done"] == true {
            event = batch["step"]["event"].clone();
            break;
        }
    }
    assert!(!event.is_null());
    let inspection = retry(&runtime, "/api/inspect", json!({"run":run}))["inspection"].clone();
    let mut events = vec![];
    let mut ack = None;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/inspect_advance",
            json!({"run":run,"inspection":inspection,"budget":1}),
            &mut ack,
        );
        assert!(batch["events"].as_array().unwrap().len() <= 1);
        events.extend(batch["events"].as_array().unwrap().clone());
        if batch["done"] == true {
            break;
        }
    }
    assert_eq!(
        events
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "begin",
            "variable",
            "pending_begin",
            "expression_relation",
            "expression_variable",
            "expression_end",
            "pending_end",
            "end"
        ]
    );
    assert_eq!(events[2]["event"], event.to_string());
    assert_eq!(events[3]["relation"], 1);
    assert_eq!(events[4]["variable"], "0");
    retry(
        &runtime,
        "/api/inspect_release",
        json!({"run":run,"inspection":inspection}),
    );
}

#[test]
fn snapshot_capture_and_release_replay_after_lost_responses() {
    let runtime = Client::default();
    let run = start(&runtime, "", "true", false)["run"].as_u64().unwrap();
    let first = ok(&runtime, "/api/snapshot", json!({"run":run,"command":2}));
    // The caller can discard any response and recover the same allocation.
    for _ in 0..8 {
        assert_eq!(
            ok(&runtime, "/api/snapshot", json!({"run":run,"command":2})),
            first
        );
    }
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":run}))["memory"]["snapshots"],
        1
    );
    for _ in 0..3 {
        retry(
            &runtime,
            "/api/snapshot_release",
            json!({"run":run,"snapshot":first["snapshot"]}),
        );
    }
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":run}))["memory"]["snapshots"],
        0
    );
    // A released request remains replayable without allocating another view.
    assert_eq!(
        ok(&runtime, "/api/snapshot", json!({"run":run,"command":2})),
        first
    );
    let second = retry(&runtime, "/api/snapshot", json!({"run":run,"command":3}));
    assert_ne!(first, second);
    let stale = runtime.request(
        "/api/snapshot",
        &execution_body(&runtime, json!({"run":run,"command":2})),
    );
    assert_eq!(stale.status, 409);
    for command in [json!(null), json!(0), json!(9_007_199_254_740_992_u64)] {
        assert_eq!(
            runtime
                .request(
                    "/api/snapshot",
                    &execution_body(&runtime, json!({"run":run,"command":command}))
                )
                .status,
            400
        );
    }
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":run}))["memory"]["snapshots"],
        1
    );
}

#[test]
fn concurrent_capture_retries_allocate_one_snapshot_per_run() {
    let runtime = std::sync::Arc::new(Client::default());
    let run = start(&runtime, "", "true", false)["run"].as_u64().unwrap();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let runtime = runtime.clone();
            std::thread::spawn(move || {
                ok(&runtime, "/api/snapshot", json!({"run":run,"command":2}))
            })
        })
        .collect();
    let responses: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(responses.iter().all(|response| *response == responses[0]));
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":run}))["memory"]["snapshots"],
        1
    );
    let other = start(&runtime, "", "true", false)["run"].as_u64().unwrap();
    assert_ne!(
        ok(&runtime, "/api/snapshot", json!({"run":other})),
        responses[0]
    );
    ok(&runtime, "/api/close", json!({"run":run}));
    for _ in 0..1000 {
        runtime.tick();
        if runtime
            .request("/api/status", &execution_body(&runtime, json!({"run":run})))
            .status
            == 404
        {
            break;
        }
    }
    assert_eq!(
        runtime
            .request(
                "/api/snapshot",
                &execution_body(&runtime, json!({"run":run,"command":5}))
            )
            .status,
        404
    );
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":other}))["memory"]["snapshots"],
        1
    );
}

#[test]
fn close_replay_after_reclamation_is_terminal_and_still_validates_requests() {
    let runtime = Client::default();
    let run = start(&runtime, "", "true", false)["run"].clone();
    ok(&runtime, "/api/close", json!({"run":run})); // The caller loses this accepted response.
    for _ in 0..10000 {
        runtime.tick();
        if runtime
            .request("/api/status", &execution_body(&runtime, json!({"run":run})))
            .status
            == 404
        {
            break;
        }
    }
    assert_eq!(
        runtime
            .request("/api/status", &execution_body(&runtime, json!({"run":run})))
            .status,
        404
    );
    assert_eq!(
        ok(&runtime, "/api/close", json!({"run":run}))["closed"],
        true
    );
    for body in [
        json!({}),
        json!({"run":run,"budget":4097}),
        json!({"run":run,"extra":true}),
        json!({"run":"invalid"}),
    ] {
        assert_eq!(
            runtime
                .request("/api/close", &execution_body(&runtime, body))
                .status,
            400
        );
    }
    let next = start(&runtime, "", "next()", false)["run"].clone();
    assert_ne!(next, run);
    assert_eq!(
        runtime
            .request(
                "/api/status",
                &execution_body(&runtime, json!({"run":next}))
            )
            .status,
        200
    );
}

#[test]
fn server_incarnation_rejects_stale_execution_requests_before_lookup_or_mutation() {
    let old = Client::default();
    let current = Client::default();
    let old_boot = old.request("/api/hello", "{}").body["boot"].clone();
    let current_boot = current.request("/api/hello", "{}").body["boot"].clone();
    let token = current_boot.as_str().expect("hello returns boot");
    assert_eq!(token.len(), 32);
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_ne!(old_boot, current_boot);
    assert_eq!(
        current.request("/api/hello", "{}").body["boot"],
        current_boot
    );
    let old_run = start(&old, "", "old()", false)["run"].clone();
    let current_run = start(&current, "", "current()", false)["run"].clone();
    assert_eq!(old_run, current_run, "exercise reused numeric run IDs");
    let before = ok(&current, "/api/status", json!({"run":current_run}));
    for route in [
        "output",
        "cancel",
        "close",
        "inspect",
        "inspect_advance",
        "inspect_cancel",
        "inspect_release",
        "step",
        "resume",
        "maintenance",
        "status",
        "snapshot",
        "snapshot_release",
        "views",
    ] {
        for run in [current_run.clone(), json!(999)] {
            let response = current.request(
                &format!("/api/{route}"),
                &json!({"boot":old_boot,"owner":old.owner,"run":run}).to_string(),
            );
            assert_eq!(response.status, 409, "{route}: {:?}", response.body);
            assert_eq!(response.body["code"], "stale_boot");
            assert_eq!(response.body["retry"], false);
            assert_eq!(
                response.body["error"],
                "The notebook server restarted. This execution is no longer available."
            );
        }
    }
    let model = ok(
        &current,
        "/api/parse",
        json!({"program":"", "query":"new()"}),
    );
    let mut stale_start = model.clone();
    stale_start["boot"] = old_boot;
    assert_eq!(
        current
            .request("/api/start", &stale_start.to_string())
            .status,
        409
    );
    assert_eq!(
        ok(&current, "/api/status", json!({"run":current_run})),
        before
    );
    assert_eq!(
        start(&current, "", "next()", false)["run"],
        2,
        "stale start allocated no owner"
    );
    for boot in [
        None,
        Some(json!(null)),
        Some(json!(0)),
        Some(json!("")),
        Some(json!("A".repeat(32))),
        Some(json!("0".repeat(31))),
    ] {
        let mut start_body = model.clone();
        let mut run_body = json!({"run":current_run});
        if let Some(boot) = boot {
            start_body["boot"] = boot.clone();
            run_body["boot"] = boot;
        }
        assert_eq!(
            current
                .request("/api/start", &start_body.to_string())
                .status,
            400
        );
        assert_eq!(
            current.request("/api/close", &run_body.to_string()).status,
            400
        );
    }
    assert_eq!(current.request("/api/hello", r#"{"extra":1}"#).status, 400);
    assert_eq!(current.request("/api/hello", "[]").status, 400);
    assert_eq!(
        current.request("/api/format", &model.to_string()).status,
        200
    );
}

#[test]
fn owner_reservations_and_control_replay_are_admitted_once() {
    let runtime = Runtime::default();
    let boot = boot(&runtime);
    let reserved = runtime.request("/api/reserve", &json!({"boot":boot}).to_string());
    assert_eq!(reserved.status, 200, "{:?}", reserved.body);
    let owner = reserved.body["owner"].clone();
    assert_eq!(owner, 1);
    let attach = json!({"boot":boot,"owner":owner}).to_string();
    assert_eq!(runtime.request("/api/attach", &attach).status, 200);
    let body = json!({"boot":boot,"owner":owner,"command":1,"program":{"rules":[]},"query":{"kind":"true"}}).to_string();
    let first = runtime.request("/api/start", &body);
    assert_eq!(first.status, 200);
    assert_eq!(runtime.request("/api/start", &body).body, first.body);
    assert_eq!(
        runtime.request("/api/start", &(body.clone() + " ")).status,
        409
    );
    assert_eq!(runtime.request("/api/retire", &attach).status, 409);
}

#[test]
fn lost_control_responses_replay_without_new_inspections_or_step_controllers() {
    let runtime = Client::default();
    let run = start(&runtime, "p() <=> p().", "p()", false)["run"].clone();
    let inspect = execution_body(&runtime, json!({"run":run,"command":2}));
    let first = runtime.request("/api/inspect", &inspect);
    assert_eq!(first.status, 200);
    for _ in 0..8 {
        assert_eq!(runtime.request("/api/inspect", &inspect).body, first.body);
    }
    assert_eq!(
        ok(&runtime, "/api/status", json!({"run":run}))["memory"]["inspections"],
        1
    );
    assert_eq!(
        runtime.request("/api/step", &inspect).status,
        409,
        "route is part of request identity"
    );
    let step = execution_body(&runtime, json!({"run":run,"command":3}));
    let first_step = runtime.request("/api/step", &step);
    assert_eq!(first_step.status, 200);
    assert_eq!(runtime.request("/api/step", &step).body, first_step.body);
    let skipped = execution_body(&runtime, json!({"run":run,"command":5}));
    assert_eq!(runtime.request("/api/resume", &skipped).status, 409);
    let mut ack = None;
    let mut done = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":4096}),
            &mut ack,
        );
        if batch["step"]["done"] == true {
            done = true;
            break;
        }
    }
    assert!(done);
    let before = ok(&runtime, "/api/status", json!({"run":run}));
    assert_eq!(runtime.request("/api/step", &step).body, first_step.body);
    let batch = next(
        &runtime,
        "/api/output",
        json!({"run":run,"budget":4096}),
        &mut ack,
    );
    assert_eq!(batch["step"]["done"], true);
    assert_eq!(
        batch["applications"], before["applications"],
        "replay did not replace completed step"
    );
    let resume = execution_body(&runtime, json!({"run":run,"command":4}));
    let first_resume = runtime.request("/api/resume", &resume);
    assert_eq!(first_resume.status, 200);
    assert_eq!(
        runtime.request("/api/resume", &resume).body,
        first_resume.body
    );
    assert_eq!(
        runtime.request("/api/step", &step).status,
        409,
        "older receipt no longer replayable"
    );
}

#[test]
fn owner_retirement_fences_reservations_and_cross_owner_execution() {
    let runtime = Client::default();
    let run = start(&runtime, "", "true", false)["run"].clone();
    let token = boot(&runtime);
    let reserved = |runtime: &Runtime| {
        runtime
            .request("/api/reserve", &json!({"boot":token}).to_string())
            .body["owner"]
            .clone()
    };
    let skipped = reserved(&runtime);
    let other = reserved(&runtime);
    let owner_body = |owner: &Value| json!({"boot":token,"owner":owner}).to_string();
    assert_eq!(
        runtime.request("/api/attach", &owner_body(&other)).status,
        200
    );
    assert_eq!(
        runtime.request("/api/attach", &owner_body(&skipped)).status,
        409
    );
    for future in [json!(other.as_u64().unwrap() + 1), json!(999)] {
        assert_eq!(
            runtime.request("/api/attach", &owner_body(&future)).status,
            409
        );
        assert_eq!(
            runtime.request("/api/retire", &owner_body(&future)).status,
            409
        );
    }
    let before = ok(&runtime, "/api/status", json!({"run":run}));
    for route in [
        "cancel",
        "close",
        "output",
        "inspect",
        "step",
        "resume",
        "snapshot",
        "status",
        "maintenance",
        "views",
        "snapshot_release",
        "inspect_release",
        "inspect_advance",
        "inspect_cancel",
    ] {
        let mut body = json!({"boot":token,"owner":other,"run":run});
        if matches!(route, "inspect" | "step" | "resume" | "snapshot") {
            body["command"] = json!(1);
        }
        assert_eq!(
            runtime
                .request(&format!("/api/{route}"), &body.to_string())
                .status,
            403,
            "{route}"
        );
    }
    assert_eq!(ok(&runtime, "/api/status", json!({"run":run})), before);
    assert_eq!(
        runtime
            .request("/api/retire", &owner_body(&json!(runtime.owner)))
            .status,
        409
    );
    ok(&runtime, "/api/close", json!({"run":run}));
    for _ in 0..10000 {
        runtime.tick();
        if runtime
            .request("/api/status", &execution_body(&runtime, json!({"run":run})))
            .status
            == 404
        {
            break;
        }
    }
    for _ in 0..3 {
        assert_eq!(
            runtime
                .request("/api/retire", &owner_body(&json!(runtime.owner)))
                .body["retired"],
            true
        );
    }
    assert_eq!(
        runtime
            .request("/api/attach", &owner_body(&json!(runtime.owner)))
            .status,
        409
    );
    let stale_start = execution_body(
        &runtime,
        json!({"command":1,"program":{"rules":[]},"query":{"kind":"true"}}),
    );
    assert_eq!(runtime.request("/api/start", &stale_start).status, 409);
    assert_eq!(
        runtime.request("/api/attach", &owner_body(&other)).status,
        200,
        "empty owners are not automatically retired"
    );
    let invalid = runtime.request("/api/start", &json!({"boot":token,"owner":other,"command":1,"program":{"rules":[]},"query":{"kind":"atom","atom":{"relation":"Bad","args":[]}}}).to_string());
    assert_eq!(invalid.status, 400);
    let valid = runtime.request("/api/start", &json!({"boot":token,"owner":other,"command":1,"program":{"rules":[]},"query":{"kind":"true"}}).to_string());
    assert_eq!(
        valid.status, 200,
        "failed admission did not consume command"
    );
}

#[test]
fn owner_requests_validate_boot_and_safe_ids_before_admission() {
    let runtime = Client::default();
    let stale = boot(&Runtime::default());
    for route in ["reserve", "attach", "retire"] {
        let mut body = json!({"boot":stale});
        if route != "reserve" {
            body["owner"] = json!(runtime.owner);
        }
        let result = runtime.request(&format!("/api/{route}"), &body.to_string());
        assert_eq!(result.status, 409);
        assert_eq!(result.body["code"], "stale_boot");
    }
    for owner in [
        json!(0),
        json!(9_007_199_254_740_992u64),
        json!("1"),
        json!(null),
    ] {
        for route in ["attach", "retire"] {
            assert_eq!(
                runtime
                    .request(
                        &format!("/api/{route}"),
                        &json!({"boot":boot(&runtime),"owner":owner}).to_string()
                    )
                    .status,
                400
            );
        }
    }
    let run = start(&runtime, "", "true", false)["run"].clone();
    for command in [
        json!(0),
        json!(9_007_199_254_740_992u64),
        json!("2"),
        json!(null),
    ] {
        assert_eq!(
            runtime
                .request(
                    "/api/resume",
                    &execution_body(&runtime, json!({"run":run,"command":command}))
                )
                .status,
            400
        );
    }
    assert_eq!(
        runtime
            .request(
                "/api/status",
                &execution_body(&runtime, json!({"run":run,"command":2}))
            )
            .status,
        400
    );
    assert_eq!(
        runtime
            .request(
                "/api/snapshot",
                &execution_body(&runtime, json!({"run":run,"capture":2}))
            )
            .status,
        400
    );
    assert_eq!(
        runtime
            .request("/api/reserve", &json!({"boot":boot(&runtime)}).to_string())
            .body["owner"],
        2,
        "stale reservation did not advance issuance"
    );
}

#[test]
fn flat_choice_metadata_labels_the_actual_contiguous_partitions() {
    let runtime = Client::default();
    let model = start(&runtime, "", "a(A);b(A);c(A);d(A);e(A);f(A);g(A)", true);
    let run = model["run"].clone();
    let mut ack = None;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/output",
            json!({"run":run,"budget":128}),
            &mut ack,
        );
        if batch["delivery_done"] == true {
            break;
        }
    }
    let views = ok(&runtime, "/api/views", json!({"run":run}));
    let choices = views["choices"].as_array().unwrap();
    assert_eq!(choices.len(), 6);
    assert!(
        choices[0]["label"]
            .as_str()
            .unwrap()
            .ends_with("First arms 1–3; Second arms 4–7")
    );
    let labels = choices
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect::<Vec<_>>();
    for suffix in [
        "First arms 1–1; Second arms 2–3",
        "First arms 4–5; Second arms 6–7",
        "First arms 2–2; Second arms 3–3",
        "First arms 4–4; Second arms 5–5",
        "First arms 6–6; Second arms 7–7",
    ] {
        assert_eq!(labels.iter().filter(|s| s.ends_with(suffix)).count(), 1);
    }
}

#[test]
fn specialized_source_step_keeps_named_rule_and_pending_equality_in_notebook() {
    for history in [false, true] {
        let runtime = Client::default();
        let run = start(
            &runtime,
            "fields @ app(T,A,B) \\ app(T,C,D) <=> A=C,B=D.",
            "app(T,A,B),app(T,C,D)",
            history,
        )["run"]
            .clone();
        retry(&runtime, "/api/step", json!({"run":run}));
        let mut ack = None;
        let mut done = false;
        for _ in 0..1000 {
            let batch = next(
                &runtime,
                "/api/output",
                json!({"run":run,"budget":32}),
                &mut ack,
            );
            if batch["step"]["done"] == true {
                assert_eq!(batch["step"]["rule"], 0);
                assert_eq!(batch["applications"], 1);
                done = true;
                break;
            }
        }
        assert!(done);
        if history {
            let views = retry(&runtime, "/api/views", json!({"run":run}));
            assert!(
                views["snapshots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["rule"] == 0
                        && s["name"] == "fields"
                        && s["label"].as_str().unwrap().contains("fields"))
            );
        }
        let inspection = retry(&runtime, "/api/inspect", json!({"run":run}))["inspection"].clone();
        let mut ack = None;
        let mut events = vec![];
        for _ in 0..1000 {
            let batch = next(
                &runtime,
                "/api/inspect_advance",
                json!({"run":run,"inspection":inspection,"budget":32}),
                &mut ack,
            );
            events.extend(batch["events"].as_array().unwrap().clone());
            if batch["done"] == true {
                break;
            }
        }
        assert!(events.iter().any(|e| e["kind"] == "pending_begin"));
        assert!(
            events
                .iter()
                .any(|e| e["kind"] == "expression" && e["operator"] == "equal")
        );
        retry(
            &runtime,
            "/api/inspect_release",
            json!({"run":run,"inspection":inspection}),
        );
        ok(&runtime, "/api/close", json!({"run":run}));
    }
}
