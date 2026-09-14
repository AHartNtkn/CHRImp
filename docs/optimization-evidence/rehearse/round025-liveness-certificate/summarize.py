"""Preserve raw paired records and summarize the evaluated component costs."""
import gzip
import json
import pathlib
import statistics
import sys
import re
import hashlib

source = pathlib.Path(sys.argv[1])
dest = pathlib.Path(__file__).resolve().parent
paired = json.loads((source / "owner-verified/summary.json").read_text())
mechanism = json.loads((source / "mechanism-owner/summary.json").read_text())
cancel = json.loads((source / "cancel-owner/summary.json").read_text())
assert len(paired) == 126
for row in paired:
    for side in ("before", "after"):
        assert row[side]["invocation"]["exit"] == 0
        results = [r["data"] for r in row[side]["records"] if r["kind"] == "result"]
        assert len(results) == 1 and not results[0].get("censored") and not results[0].get("error")
for row in mechanism + cancel:
    assert row["exit"] == 0
raw = {"paired": paired, "mechanism": mechanism, "cancel": cancel,
       "manifest": json.loads((source / "owner-verified/manifest.json").read_text())}
storage = {name: json.loads((source / name / "summary.json").read_text()) for name in ("storage-control", "storage-confirm", "storage-owner-confirm")}
for rows in storage.values():
    for row in rows:
        for side in ("before", "after"):
            assert row[side]["invocation"]["exit"] == 0
raw["storage"] = storage
(dest / "raw.json.gz").write_bytes(gzip.compress(json.dumps(raw, separators=(",", ":")).encode(), mtime=0))

def snapshot(row, side):
    records = row[side]["records"]
    diagnostics = [r["data"] for r in records if r["kind"] == "diagnostics"]
    d = next((r for r in diagnostics if r["phase"] == "source"), diagnostics[0] if diagnostics else {})
    metrics = {}
    work = d.get("work", {})
    for key, value in work.get("shared", {}).get("collection", {}).items():
        if isinstance(value, (int, float)): metrics["collection." + key] = value
    if "graph_pages" in d: metrics["scalar_fallback"] = d["graph_pages"][9]
    if "graph_allocations" in d: metrics["graph_allocations"] = d["graph_allocations"]
    if "store_mutations" in d:
        for i, (unique, copied) in enumerate(d["store_mutations"]):
            metrics[f"store{i}.unique"] = unique
            metrics[f"store{i}.copied"] = copied
    for key, value in d.get("field_updates", {}).items():
        if key.startswith("liveness_"): metrics[key] = value
    alloc = next(r["data"] for r in records if r["kind"] == "allocations")
    metrics["peak_requested_bytes"] = alloc["process_peak_requested_bytes"]
    metrics["total_allocated_bytes"] = sum(p["allocated_bytes"] for p in alloc["phases"])
    for phase in alloc["phases"]:
        for key in ("allocated_bytes", "allocations", "freed_bytes", "deallocations"):
            metrics[f"allocation.{phase['phase']}.{key}"] = phase[key]
    rules = work.get("rules", [])
    semantics = {key: work[key] for key in ("body_posts", "body_merges", "choice_births", "fail_applications", "fail_support_changes", "output_answers") if key in work}
    semantics["rule_applications"] = [r["applied"] for r in rules]
    semantics["total_applications"] = sum(semantics["rule_applications"])
    phases = [{k: v for k, v in r["data"].items() if k in ("phase", "applications", "complete", "prepared_released", "data")} for r in records if r["kind"] == "phase"]
    return {"metrics": metrics, "semantics": semantics, "phase_oracles": phases,
            "source_memory": d.get("memory_counts"), "final_memory": diagnostics[-1].get("memory_counts") if diagnostics else None}

cases = []
for name in dict.fromkeys(r["name"] for r in paired):
    rows = [r for r in paired if r["name"] == name]
    case = {"name": name, "case": rows[0]["case"], "size": rows[0]["size"], "extra": rows[0]["extra"]}
    for side in ("before", "after"):
        samples = [snapshot(r, side) for r in rows]
        keys = samples[0]["metrics"].keys()
        ranges = {key: {"median": statistics.median(s["metrics"][key] for s in samples),
                        "min": min(s["metrics"][key] for s in samples),
                        "max": max(s["metrics"][key] for s in samples)} for key in keys}
        case[side] = {"ranges": ranges, "samples": samples}
    case["same_source_semantics"] = all(snapshot(r, "before")["semantics"] == snapshot(r, "after")["semantics"] for r in rows)
    case["same_phase_oracles"] = all(snapshot(r, "before")["phase_oracles"] == snapshot(r, "after")["phase_oracles"] for r in rows)
    assert case["same_source_semantics"] and case["same_phase_oracles"], name
    cases.append(case)
storage_summary = {}
for campaign, rows in storage.items():
    storage_summary[campaign] = {}
    for name in dict.fromkeys(r["name"] for r in rows):
        storage_summary[campaign][name] = {}
        for side in ("before", "after"):
            values = [snapshot(r, side)["metrics"]["peak_requested_bytes"] for r in rows if r["name"] == name]
            storage_summary[campaign][name][side] = {"values": values, "min": min(values), "median": statistics.median(values), "mean": statistics.mean(values), "max": max(values)}
(dest / "summary.json").write_text(json.dumps({"recommendation": "KEEP", "baseline": "0a4ff81aadfb52d9c0c0b6e7563a12fe303916f0", "samples": 252, "cases": cases, "storage_followup": storage_summary}, indent=2) + "\n")
(dest / "mechanism.json").write_text(json.dumps({"normal": mechanism, "cancel": cancel}, indent=2) + "\n")
print("source-semantic differences:", [c["name"] for c in cases if not c["same_source_semantics"]])
logs = {}
validation = {}
for mode, expected in (("diagnostic", 554), ("ordinary", 508)):
    path = pathlib.Path(f"/tmp/round025-owner-{mode}-tests.log")
    log = path.read_text()
    counts = re.findall(r"test result: ok\. (\d+) passed", log)
    assert sum(map(int, counts)) == expected and "FAILED" not in log
    logs[str(path)] = log
    command = ["timeout", "--kill-after=5s", "60s", "cargo", "test", "--offline", "--release"]
    if mode == "diagnostic": command += ["--features", "diagnostics"]
    command += ["--all-targets"]
    validation[mode] = {"command": command, "passed": expected, "binaries": len(counts), "exit": 0,
                        "log": str(path), "sha256": hashlib.sha256(log.encode()).hexdigest()}
for filename in ("round025-owner-clippy.log", "round025-build-owner-verified.log", "round025-build-owner-ordinary.log", "round025-build-compact.log", "round025-build-journal-test.log", "round025-red.log", "round025-frontier-red.log", "round025-delta-red.log", "round025-release-red.log", "round025-reorder-test.log", "round025-owner-red.log"):
    path = pathlib.Path("/tmp") / filename
    logs[str(path)] = path.read_text()
(dest / "validation.json").write_text(json.dumps(validation, indent=2) + "\n")
(dest / "validation-logs.json.gz").write_bytes(gzip.compress(json.dumps(logs).encode(), mtime=0))
