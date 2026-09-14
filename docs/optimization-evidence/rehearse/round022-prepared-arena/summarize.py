#!/usr/bin/env python3
"""Summarize raw maintained-runner records; retain ranges and semantic endpoints."""
import json
from pathlib import Path
import statistics
import sys
from reproduce import matrix


def samples(path):
    return [json.loads(p.read_text()) for p in sorted(path.glob("[0-9]*.json"))]


def flatten(value, prefix=""):
    if isinstance(value, dict):
        for k, v in value.items():
            yield from flatten(v, f"{prefix}.{k}" if prefix else k)
    elif isinstance(value, list):
        for i, v in enumerate(value):
            yield from flatten(v, f"{prefix}.{i}")
    else:
        yield prefix, value


def read(sample):
    assert sample["status"] == "completed", sample["status"]
    metrics, semantic = {}, {}
    for record in sample["records"]:
        kind, d = record["kind"], record["data"]
        if kind == "allocations":
            for phase in d["phases"]:
                name = phase["phase"]
                for k in ("allocations", "deallocations", "allocated_bytes", "freed_bytes"):
                    metrics[f"alloc.{name}.{k}"] = phase[k]
                if name == "prepare":
                    metrics["retained_preparation_heap"] = phase["allocated_bytes"] - phase["freed_bytes"]
            metrics["live_final"] = d["process_live_requested_bytes"]
            metrics["peak_requested"] = d["process_peak_requested_bytes"]
            metrics["total_allocated"] = sum(p["allocated_bytes"] for p in d["phases"])
        elif kind == "result":
            assert d.get("error") is None and not d.get("censored", False)
            for k, v in flatten(d):
                if isinstance(v, (int, float)) and not isinstance(v, bool):
                    metrics[f"result.{k}"] = v
                if k in ("status", "goal", "goal_count", "prepared_released", "cleanup_done", "cleanup_in_time") or (
                    k.startswith(("expected.", "work.", "uses.")) and k.split(".")[-1] in
                    ("answers", "applications", "facts", "ports", "scalars", "complete", "prepared_owners_after_engine_drop")
                ):
                    semantic[f"result.{k}"] = v
        elif kind == "diagnostics":
            phase = d["phase"]
            for k, v in flatten(d):
                if k.startswith(("work.", "shared_restrictions.", "store_mutations.", "graph_pages.", "memory_counts.")):
                    if isinstance(v, (int, float)):
                        metrics[f"diagnostics.{phase}.{k}"] = v
            if "plan_representation_bytes" in d:
                metrics["representation_bytes"] = d["plan_representation_bytes"]
            semantic[f"applications.{phase}"] = [r["applied"] for r in d["work"]["rules"]]
        elif kind == "phase":
            phase = d["phase"]
            for k, v in flatten(d):
                if isinstance(v, (int, float)) and not isinstance(v, bool):
                    metrics[f"phase.{phase}.{k}"] = v
                if k in ("complete", "prepared_released", "continued_applications", "retained_snapshots") or k.startswith("data."):
                    semantic[f"phase.{phase}.{k}"] = v
    return metrics, semantic


def main():
    root = Path(sys.argv[1])
    report = {}
    for name, _, _ in matrix():
        sides = {}
        for side in ("baseline", "candidate"):
            records = [read(s) for s in samples(root / name / side)]
            assert len(records) >= 5, (name, side, len(records))
            values = {}
            endpoints = set()
            for metrics, semantic in records:
                endpoints.add(json.dumps(semantic, sort_keys=True))
                for k, v in metrics.items():
                    values.setdefault(k, []).append(v)
            sides[side] = {
                "samples": len(records),
                "semantic_endpoints": [json.loads(e) for e in sorted(endpoints)],
                "metrics": {k: {"median": statistics.median(v), "min": min(v), "max": max(v)} for k, v in values.items()},
            }
        sides["same_endpoints"] = sides["baseline"]["semantic_endpoints"] == sides["candidate"]["semantic_endpoints"]
        report[name] = sides
        keys = ["alloc.prepare.allocations", "alloc.prepare.allocated_bytes", "retained_preparation_heap", "alloc.engine.allocated_bytes", "alloc.cleanup.allocated_bytes"]
        print(name, sides["same_endpoints"], *[
            f"{k}=" + "/".join(str(sides[s]["metrics"].get(k, {}).get("median")) for s in ("baseline", "candidate")) for k in keys
        ])
    (root / "summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
