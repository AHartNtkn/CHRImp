//! Typed measurement records. Display text is not a machine-readable authority.
use serde_json::{Value, json};

pub fn emit(kind: &str, mut data: Value) {
    if kind == "result" {
        data["native_peak_rss_estimate_kib"] = json!(native_peak_rss_estimate_kib());
    }
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

/// Linux's current address-space high-water mark, excluding the pre-exec image.
/// Includes the whole native harness, allocator overhead and reporting so far.
/// VmHWM uses approximate kernel RSS accounting; it is not an exact byte gauge.
fn native_peak_rss_estimate_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut fields = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace();
    fields.next()?;
    let value = fields.next()?.parse().ok()?;
    (fields.next()? == "kB").then_some(value)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn native_peak_tracks_touched_memory_and_survives_release() {
        if std::env::var_os("CHR_RSS_CALIBRATION").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "report::tests::native_peak_tracks_touched_memory_and_survives_release",
                ])
                .env("CHR_RSS_CALIBRATION", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        let before = super::native_peak_rss_estimate_kib().unwrap();
        let mut bytes = vec![0_u8; 32 * 1024 * 1024];
        for index in (0..bytes.len()).step_by(4096) {
            // Volatile writes ensure actual committed pages, even in optimized tests.
            unsafe {
                std::ptr::write_volatile(bytes.as_mut_ptr().add(index), 1);
            }
        }
        let held = super::native_peak_rss_estimate_kib().unwrap();
        assert!(held >= before + 30 * 1024, "before={before}, held={held}");
        drop(bytes);
        let released = super::native_peak_rss_estimate_kib().unwrap();
        // VmHWM is approximate and may decrease slightly when accounting catches up.
        // Both observations must expose the known 32 MiB touched allocation.
        assert!(
            released >= before + 30 * 1024,
            "before={before}, held={held}, released={released}"
        );
    }
}
