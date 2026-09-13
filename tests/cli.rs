use std::process::Command;
#[test]
fn program_files_run_as_a_language_and_stream_complete_graphs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/reachability.chr");
    let output = Command::new(env!("CARGO_BIN_EXE_chr"))
        .arg(&path)
        .args(["--query", "edge(A,B),edge(B,C)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["kind"], "program");
    assert_eq!(
        events[0]["signatures"],
        serde_json::json!([{ "name": "edge", "arity": 2 }, { "name": "reach", "arity": 2 }])
    );
    assert_eq!(events[0]["variables"], serde_json::json!(["A", "B", "C"]));
    assert_eq!(events[1]["kind"], "begin");
    assert_eq!(events.last().unwrap()["kind"], "end");
    assert_eq!(events.iter().filter(|e| e["kind"] == "fact").count(), 5);
    assert_eq!(events.iter().filter(|e| e["kind"] == "port").count(), 10);
    let mut facts = vec![];
    let mut current = None;
    for e in events {
        if e["kind"] == "fact" {
            current = Some((e["relation"].as_u64().unwrap(), vec![]));
        }
        if e["kind"] == "port" {
            current
                .as_mut()
                .unwrap()
                .1
                .push(e["variable"].as_u64().unwrap());
        }
        if e["kind"] == "end_fact" {
            facts.push(current.take().unwrap());
        }
    }
    facts.sort();
    assert_eq!(
        facts,
        [
            (0, vec![0, 1]),
            (0, vec![1, 2]),
            (1, vec![0, 1]),
            (1, vec![0, 2]),
            (1, vec![1, 2])
        ]
    );
    let invalid = Command::new(env!("CARGO_BIN_EXE_chr"))
        .arg(path)
        .args(["--query", "edge(a,B)"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(!invalid.stderr.is_empty());
}

#[test]
fn notebook_default_origin_is_stable_and_port_override_is_honored() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::time::Duration;

    fn launch(args: &[&str]) -> (String, std::process::Output) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_chr"))
            .arg("--notebook")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = send.send(result);
        });
        let result = receive.recv_timeout(Duration::from_secs(5));
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        reader.join().unwrap();
        let line = result.expect("notebook startup timed out").unwrap();
        (line, output)
    }

    for _ in 0..2 {
        assert_eq!(launch(&[]).0, "Notebook: http://127.0.0.1:7878\n");
    }
    // An occupied default must fail, rather than silently change storage origin.
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 7878)).unwrap();
    let (line, output) = launch(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(line.is_empty());
    let address: std::net::SocketAddr = launch(&["--port", "0"])
        .0
        .trim()
        .strip_prefix("Notebook: http://")
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(address.port(), 0);
    assert_ne!(address.port(), occupied.local_addr().unwrap().port());
    assert_eq!(
        launch(&["--port", &address.port().to_string()]).0,
        format!("Notebook: http://{address}\n")
    );
}

#[test]
fn native_query_diagnostics_are_opt_in_and_preserve_answers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/reachability.chr");
    let invoke = |diagnostics: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_chr"));
        command.arg(&path).args(["--query", "edge(A,B),edge(B,C)"]);
        if diagnostics {
            command.arg("--diagnostics");
        }
        command.output().unwrap()
    };
    let baseline = invoke(false);
    assert!(baseline.status.success());
    assert!(baseline.stderr.is_empty());
    let observed = invoke(true);
    if cfg!(feature = "diagnostics") {
        assert!(
            observed.status.success(),
            "{}",
            String::from_utf8_lossy(&observed.stderr)
        );
        assert_eq!(baseline.stdout, observed.stdout);
        let report: serde_json::Value = serde_json::from_slice(&observed.stderr).unwrap();
        assert_eq!(report["kind"], "cli_diagnostics");
        assert_eq!(report["source"]["work"]["complete_answers"], 1);
        let applications: u64 = report["source"]["work"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["applied"].as_u64().unwrap())
            .sum();
        assert_eq!(applications, 3);
        assert_eq!(
            report["after_cancel"]["work"]["rules"],
            report["source"]["work"]["rules"]
        );
        assert_eq!(report["after_cancel"]["pending_tasks"], 0);
        // A live Engine owns its current coordinate epoch even after source cancellation.
        for (name, value) in report["after_cancel"]["memory_counts"].as_object().unwrap() {
            assert_eq!(
                value.as_u64().unwrap(),
                u64::from(name == "coordinate_records"),
                "{name}"
            );
        }
        assert_eq!(report["source"]["exhausted"], true);
    } else {
        assert_eq!(observed.status.code(), Some(2));
        assert!(observed.stdout.is_empty());
        assert!(String::from_utf8_lossy(&observed.stderr).contains("--features diagnostics"));
    }
}
