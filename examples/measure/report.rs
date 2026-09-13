//! Typed measurement records. Display text is not a machine-readable authority.
use serde_json::{Value, json};

pub fn emit(kind: &str, data: Value) {
    println!(
        "measurement={}",
        json!({"schema": 1, "kind": kind, "data": data})
    );
}

pub fn memory(values: [usize; 13]) -> Value {
    const NAMES: [&str; 13] = [
        "graph_nodes",
        "occurrences",
        "conditions",
        "history_nodes",
        "history_records",
        "pending_nodes",
        "obligation_descriptors",
        "choices",
        "coordinate_records",
        "snapshots",
        "inspections",
        "pending_tasks",
        "release_batches",
    ];
    NAMES
        .into_iter()
        .zip(values)
        .map(|(k, v)| (k.to_owned(), json!(v)))
        .collect()
}
