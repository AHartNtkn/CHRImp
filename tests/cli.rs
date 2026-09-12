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
