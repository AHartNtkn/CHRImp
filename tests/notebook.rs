use chr::notebook::Runtime;
use serde_json::{Value, json};
fn ok(runtime: &Runtime, path: &str, body: Value) -> Value {
    let response = runtime.request(path, &body.to_string());
    assert_eq!(response.status, 200, "{path}: {:?}", response.body);
    response.body
}
fn next(runtime: &Runtime, path: &str, mut body: Value, ack: &mut Option<u64>) -> Value {
    body["ack"] = json!(*ack);
    let batch = ok(runtime, path, body);
    *ack = Some(batch["sequence"].as_u64().unwrap());
    batch
}
fn start(runtime: &Runtime, program: &str, query: &str, history: bool) -> Value {
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
fn retry(runtime: &Runtime, path: &str, body: Value) -> Value {
    for _ in 0..1000 {
        let response = runtime.request(path, &body.to_string());
        if response.status == 200 {
            return response.body;
        }
        assert_eq!(response.body["retry"], true, "{path}: {:?}", response.body);
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
    let runtime = Runtime::default();
    let metadata = start(&runtime, "edge(X,Y) ==> reverse(Y,X).", "edge(A,B)", true);
    assert_eq!(metadata["variables"], json!(["A", "B"]));
    let run = metadata["run"].clone();
    let mut ack = None;
    let mut events = vec![];
    let mut finished = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/advance",
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
    let runtime = Runtime::default();
    let run = start(&runtime, "p(X) <=> q(X). q(X) <=> p(X).", "p(A)", true)["run"].clone();
    retry(&runtime, "/api/step", json!({"run":run}));
    let mut ack = None;
    let mut finished = false;
    for _ in 0..1000 {
        let batch = next(
            &runtime,
            "/api/advance",
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
            "/api/advance",
            json!({"run":run,"budget":4096}),
            &mut ack
        )["applications"],
        1
    );
    retry(&runtime, "/api/resume", json!({"run":run}));
    assert!(
        next(
            &runtime,
            "/api/advance",
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
    let runtime = Runtime::default();
    assert_eq!(runtime.request("/api/start",r#"{"program":{"rules":[]},"query":{"kind":"atom","atom":{"relation":"p","args":[42]}}}"#).status,400);
    let deep = format!(
        r#"{{"program":{{"rules":[]}},"query":{}{{"kind":"true"}}{}}}"#,
        r#"{"kind":"and","items":["#.repeat(1000),
        "]}".repeat(1000)
    );
    assert_eq!(runtime.request("/api/start", &deep).status, 400);
    let run = start(&runtime, "", "true", false)["run"].clone();
    assert_eq!(run, 1, "invalid input must not create runs");
    assert_eq!(
        runtime
            .request(
                "/api/advance",
                &json!({"run":run,"budget":4097}).to_string()
            )
            .status,
        400
    );
    assert_eq!(
        runtime
            .request(
                "/api/inspect",
                &json!({"run":run,"choices":{"-1":true}}).to_string()
            )
            .status,
        400
    );
    assert_eq!(
        runtime.request("/api/advance", r#"{"run":999}"#).status,
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
        socket.read_to_string(&mut result).unwrap();
        result
    };
    let home = send(format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n"));
    assert!(home.starts_with("HTTP/1.1 200"));
    assert!(home.contains("CHR · Language notebook"));
    assert!(home.contains("Content-Security-Policy:"));
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
    let runtime = Runtime::default();
    let closing = start(&runtime, "p(X) <=> p(X).", "p(A)", true)["run"].clone();
    let kept = start(&runtime, "", "kept(X)", false)["run"].clone();
    let mut ack = None;
    for _ in 0..50 {
        next(
            &runtime,
            "/api/advance",
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
            .request("/api/inspect", &json!({"run":closing}).to_string())
            .status,
        409
    );
    let mut released = false;
    for _ in 0..10000 {
        runtime.tick();
        if runtime
            .request("/api/status", &json!({"run":closing}).to_string())
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
            .request("/api/status", &json!({"run":kept}).to_string())
            .status,
        200
    );
}

#[test]
fn deep_containers_are_rejected_as_ids_without_recursive_buffering() {
    let runtime = Runtime::default();
    for field in [
        "snapshot",
        "inspection",
        "after_snapshot",
        "after_choice",
        "before_snapshot",
        "before_choice",
    ] {
        let source = format!(
            "{{\"run\":1,\"{field}\":{}0{}}}",
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
    let runtime = Runtime::default();
    let run = start(&runtime, "", "(p(A);p(A))", true)["run"].clone();
    let mut ack = None;
    let first = next(
        &runtime,
        "/api/advance",
        json!({"run":run,"budget":4096}),
        &mut ack,
    );
    assert!(!first["events"].as_array().unwrap().is_empty());
    for _ in 0..3 {
        assert_eq!(
            ok(&runtime, "/api/advance", json!({"run":run,"budget":1})),
            first
        );
    }
    let second = next(
        &runtime,
        "/api/advance",
        json!({"run":run,"budget":4096}),
        &mut ack,
    );
    assert_eq!(second["sequence"], 2);
    assert!(second["events"].as_array().unwrap().is_empty());
    assert_eq!(
        runtime
            .request("/api/advance", &json!({"run":run,"ack":99}).to_string())
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
}

#[test]
fn releasing_inspection_zero_preserves_source_recovery() {
    let runtime = Runtime::default();
    let run = start(&runtime, "", "p(A)", false)["run"].clone();
    let batch = ok(&runtime, "/api/advance", json!({"run":run,"budget":4096}));
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
        ok(&runtime, "/api/advance", json!({"run":run,"budget":1})),
        batch
    );
}

#[test]
fn closing_discards_inspections_created_after_source_cancellation() {
    let runtime = Runtime::default();
    let run = start(&runtime, "", "p(A)", true)["run"].clone();
    ok(&runtime, "/api/advance", json!({"run":run,"budget":4096}));
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
            .request("/api/status", &json!({"run":run}).to_string())
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
    let runtime = Runtime::default();
    let query = std::iter::repeat_n("(true;true)", 70)
        .collect::<Vec<_>>()
        .join(",");
    let run = start(&runtime, "", &query, true)["run"].clone();
    let mut ack = None;
    let mut first = Value::Null;
    for _ in 0..100 {
        next(
            &runtime,
            "/api/advance",
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
            runtime.request("/api/views", &request.to_string()).status,
            400
        );
    }
    ok(&runtime, "/api/cancel", json!({"run":run}));
}

#[test]
fn logical_step_inspection_streams_the_pending_rhs_with_string_variable_ids() {
    let runtime = Runtime::default();
    let run = start(&runtime, "p(X) <=> q(X). q(X) <=> r(X).", "p(A)", false)["run"].clone();
    retry(&runtime, "/api/step", json!({"run":run}));
    let mut ack = None;
    let mut event = Value::Null;
    for _ in 0..100 {
        let batch = next(
            &runtime,
            "/api/advance",
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
