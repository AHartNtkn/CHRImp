"""Package maintained perf.py evidence without dropping raw outcomes or phases.

Usage: python3 collect.py OUTPUT_ROOT [EVIDENCE_DIRECTORY]
"""
import gzip
import hashlib
import json
from pathlib import Path
import sys
import subprocess
from run import points

root = Path(sys.argv[1])
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parent
out.mkdir(parents=True, exist_ok=True)
archive, summary = {}, {}
def selected_metric(key):
    return key.startswith(("allocations.", "workload.")) or ".work.shared.coordinates." in key

def endpoints(sample):
    result = []
    for record in sample["records"]:
        kind, data = record["kind"], record["data"]
        if kind == "result":
            result.append({k: data[k] for k in ("status", "answers", "first_complete_answer", "source_goal_reached", "cleanup_done", "prepared_released") if k in data})
            work = data.get("work", {})
            if isinstance(work, dict):
                result.append({k: work[k] for k in ("applications", "answers", "scalars") if k in work})
        if kind == "phase":
            result.append({k: data[k] for k in ("phase", "applications", "data") if k in data})
        if kind == "diagnostics" and data["phase"] == "source":
            coordinates = data["work"]["shared"]["coordinates"]
            result.append({k: coordinates[k] for k in ("publications", "assignments_published")})
    return json.dumps(result, sort_keys=True)

audit = {}
for label in points:
    summary[label] = {}
    for side in ("baseline", "candidate"):
        directory = root / f"{side}-{label}"
        campaign = json.loads((directory / "campaign.json").read_text())
        counts = campaign["summary"]["counts"]
        assert sum(counts.values()) == 5 and not set(counts) - {"completed", "censored"}, directory
        samples = [json.loads((directory / f"{i}.json").read_text()) for i in range(5)]
        assert all(s["status"] in ("completed", "censored") for s in samples)
        archive[f"{side}/{label}"] = {
            "campaign": campaign, "samples": samples,
            "stdout": [(directory / f"{i}.stdout").read_text() for i in range(5)],
            "stderr": [(directory / f"{i}.stderr").read_text() for i in range(5)],
        }
        metrics = {k: v for k, v in campaign["summary"]["metrics"].items() if selected_metric(k)}
        summary[label][side] = {
            "counts": counts,
            "command": campaign["command"],
            "medians": {k: v["median"] for k, v in metrics.items()},
            "varying_ranges": {k: [v["min"], v["max"]] for k, v in metrics.items() if v["min"] != v["max"]},
            "incomplete_or_missing": {k: v["incomplete_or_missing"] for k, v in metrics.items() if v["incomplete_or_missing"]},
            "first_sample_endpoints": [r for r in samples[0]["records"] if r["kind"] in ("result", "phase")],
        }
    signatures = {side: sorted({endpoints(s) for s in archive[f"{side}/{label}"]["samples"] if s["status"] == "completed"})
                  for side in ("baseline", "candidate")}
    audit[label] = {"completed_endpoint_signatures_equal": signatures["baseline"] == signatures["candidate"],
                    **{side: [json.loads(s) for s in values] for side, values in signatures.items()}}
serial = root / "candidate-lambda2-serial"
if (serial / "campaign.json").exists():
    campaign = json.loads((serial / "campaign.json").read_text())
    assert campaign.get("summary"), "serial cohort is still running"
    samples = [json.loads((serial / f"{i}.json").read_text()) for i in range(5)]
    archive["candidate/lambda2-serial"] = {
        "campaign": campaign, "samples": samples,
        "stdout": [(serial / f"{i}.stdout").read_text() for i in range(5)],
        "stderr": [(serial / f"{i}.stderr").read_text() for i in range(5)],
    }
    summary["lambda2-serial"] = {"counts": campaign["summary"]["counts"], "command": campaign["command"],
                                 "metrics": {k: v for k, v in campaign["summary"]["metrics"].items() if selected_metric(k)}}
(out / "raw-samples.json.gz").write_bytes(gzip.compress(json.dumps(archive, sort_keys=True).encode(), mtime=0))
(out / "measurements.json").write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n")
(out / "endpoint-audit.json").write_text(json.dumps(audit, sort_keys=True, indent=2) + "\n")
logs = ["red", "memo-red", "bounded-red", "scaling", "final-observation", "final-clippy",
        "final-diagnostics-tests", "final-native-tests", "tested-diagnostics-build", "tested-native-build",
        "final-diagnostics-cli-loopback", "final-diagnostics-notebook-loopback",
        "final-native-cli-loopback", "final-native-notebook-loopback", "bounded-measurements", "lambda-serial", "doctests",
        "target-mutation-red", "target-green-diagnostics", "target-green-native"]
for name in logs:
    (out / f"{name}.log").write_bytes(Path(f"/tmp/chrimp-round009-{name}.log").read_bytes())
for name in ("behavior-i", "behavior-s"):
    (out / f"{name}-comparison.json").write_bytes(Path(f"/tmp/chrimp-round009-{name}-comparison.json").read_bytes())
environment = {
    "baseline_revision": "8e025bc82ea4ed229b53a62804ffa9bb6183e584",
    "implementation_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
    "binaries": {},
    "build_commands": [
        "cargo build --offline --release --features diagnostics --example measure --target-dir /tmp/chrimp-round009-baseline-target",
        "cargo build --offline --release --features diagnostics --example measure --target-dir /tmp/chrimp-round009-bounded-target",
        "cargo test --offline --release --features diagnostics --all-targets --no-run",
        "cargo test --offline --release --all-targets --no-run --target-dir /tmp/chrimp-round009-native-target",
    ],
}
for binary in ("/tmp/chrimp-round009-baseline-target/release/examples/measure", "/tmp/chrimp-round009-bounded-target/release/examples/measure"):
    environment["binaries"][binary] = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
(out / "environment.json").write_text(json.dumps(environment, indent=2) + "\n")
print(f"{len(points)} paired points, {len(archive)*5} samples; outcomes preserved")
