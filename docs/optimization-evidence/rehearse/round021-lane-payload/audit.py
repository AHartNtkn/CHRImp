"""Compare maintained perf.py records, retaining every phase and allocation sample.

No workloads or semantic oracles are supplied here. Usage: audit.py MATRIX OUT.
The output gzip includes full typed results, phase records, diagnostics and
allocations, plus every numeric diagnostic difference from baseline sample zero.
"""
import gzip
import json
from pathlib import Path
import statistics
import sys


def flat(value, path=""):
    if isinstance(value, dict):
        for k, v in value.items():
            yield from flat(v, f"{path}.{k}" if path else k)
    elif isinstance(value, list):
        for k, v in enumerate(value):
            yield from flat(v, f"{path}.{k}")
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        yield path, value


def read(path):
    campaign = json.loads((path / "campaign.json").read_text())
    assert not campaign["aggregate_censored"], path
    runs = []
    for i in range(campaign["requested_samples"]):
        sample = json.loads((path / f"{i}.json").read_text())
        assert sample["status"] == "completed", path
        r = sample["records"]
        runs.append({
            "diagnostics": {x["data"]["phase"]: x["data"] for x in r if x["kind"] == "diagnostics"},
            "phases": [x["data"] for x in r if x["kind"] == "phase"],
            "result": next(x["data"] for x in r if x["kind"] == "result"),
            "allocations": next(x["data"] for x in r if x["kind"] == "allocations"),
        })
    return campaign["command"], runs


def median_range(values):
    return [min(values), statistics.median(values), max(values)]


root, output = map(Path, sys.argv[1:])
report = {"points": {}}
for folder in sorted(root.glob("before-*")):
    label = folder.name.removeprefix("before-")
    cb, before = read(folder)
    ca, after = read(root / f"after-{label}")
    reference = before[0]["diagnostics"]
    differences, summary = {}, {}
    for side, runs in [("before", before), ("after", after)]:
        for i, run in enumerate(runs):
            assert run["diagnostics"].keys() == reference.keys(), label
            for phase, d in run["diagnostics"].items():
                assert d["work"]["rules"] == reference[phase]["work"]["rules"], (label, side, i, phase, "rules")
                for key in ["tasks_created", "tasks_completed", "tasks_canceled", "task_parks",
                            "task_requeues", "task_wakes", "choice_births", "fail_applications",
                            "fail_support_changes", "body_posts", "body_merges", "merge_support_changes",
                            "certificates_published", "output_events", "complete_answers",
                            "syntax_promotions", "syntax_promotion_tasks", "syntax_descriptors_materialized"]:
                    assert d["work"][key] == reference[phase]["work"][key], (label, side, i, phase, key)
                for key in ["requests", "enqueued", "granted", "entries", "peak_entries", "payload_moves",
                            "discard_steps", "discarded_tasks", "trace_tasks"]:
                    assert d["work"]["waiters"][key] == reference[phase]["work"]["waiters"][key], (label, side, i, phase, key)
                if side == "after":
                    w = d["work"]["waiters"]
                    assert w["direct_handoffs"] == d["work"]["task_wakes"]
                    assert w["payload_moves"] == d["work"]["task_wakes"] + d["work"]["task_parks"]
                    if phase == "after_cancel":
                        assert w["entries"] == w["capacity_bytes"] == 0
                base = dict(flat(reference[phase]))
                diff = {k: [base.get(k), v] for k, v in flat(d)
                        if not k.startswith("allocation.") and not k.endswith("_ns") and base.get(k) != v}
                allowed = {"condition_slab.3", "condition_slab.6", "condition_slab.16", "condition_slab.17",
                           "work.advance_iterations", "work.dispatch.collection", "work.dispatch.cancellation",
                           "work.shared.collection.arena", "work.shared.coordinates.cleanup_probes"}
                assert all(k in allowed or k.startswith("work.waiters.") for k in diff), (label, diff)
                delta = lambda k: diff.get(k, [0, 0])[1] - diff.get(k, [0, 0])[0]
                gc = delta("work.shared.collection.arena")
                cancel = delta("work.dispatch.cancellation")
                assert delta("work.dispatch.collection") == gc, (label, side, i, phase)
                assert delta("work.advance_iterations") == gc + cancel, (label, side, i, phase)
                assert delta("work.shared.coordinates.cleanup_probes") == gc + cancel, (label, side, i, phase)
                assert gc == 0 or label.startswith("archive-conditional-"), (label, gc)
                assert cancel == 0 or (phase == "after_cancel" and side == "after"), (label, cancel)
                if phase == "after_cancel" and side == "after" and "source" in reference:
                    w = reference["source"]["work"]["waiters"]
                    assert cancel == sum(w["enqueued"][j] - w["granted"][j] for j in [1, 2]), label
                if diff:
                    differences[f"{side}/{i}/{phase}"] = diff
            for p in run["phases"]:
                if p["phase"] == "release":
                    assert p["complete"]
                    assert all(v == (1 if k == "coordinate_records" else 0)
                               for k, v in p["memory"].items()), (label, p)
        values = {}
        for key in ("process_peak_requested_bytes", "process_live_requested_bytes"):
            values[key] = median_range([r["allocations"][key] for r in runs])
        for phase in runs[0]["allocations"]["phases"]:
            name = phase["phase"]
            for key in ["allocations", "deallocations", "allocated_bytes", "freed_bytes"]:
                values[f"{name}.{key}"] = median_range([
                    next(p[key] for p in r["allocations"]["phases"] if p["phase"] == name) for r in runs])
        summary[side] = values
    assert summary["before"]["process_live_requested_bytes"] == summary["after"]["process_live_requested_bytes"], label
    report["points"][label] = {"commands": [cb, ca], "summary": summary,
                                "differences": differences, "before": before, "after": after}
    print(label, "engine bytes", summary["before"]["engine.allocated_bytes"], "->", summary["after"]["engine.allocated_bytes"],
          "peak", summary["before"]["process_peak_requested_bytes"], "->", summary["after"]["process_peak_requested_bytes"])
report["validated_runs"] = sum(len(p["before"]) + len(p["after"]) for p in report["points"].values())
with gzip.open(output, "wt") as f:
    json.dump(report, f, separators=(",", ":"))
print("validated runs", report["validated_runs"])
