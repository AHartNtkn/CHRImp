"""Audit existing perf.py records; no workloads, timing or alternate oracles.

Usage: python3 .../audit.py /tmp/chrimp-round020-evaluation > audit.json
The maintained runners validate each result. This checks equal source work,
attributes cleanup differences, and retains every allocation phase/checkpoint.
"""
import copy
import json
from pathlib import Path
import statistics
import sys


def numeric(value, prefix=""):
    if isinstance(value, dict):
        for key, child in value.items():
            yield from numeric(child, prefix + "." + key if prefix else key)
    elif isinstance(value, list):
        for i, child in enumerate(value):
            yield from numeric(child, f"{prefix}.{i}")
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        yield prefix, value


def diagnostic(data):
    data = copy.deepcopy(data)
    data.pop("allocation")
    data.pop("phase")
    # Preparation clocks are not architectural work counters.
    for key in list(data["preparation"]):
        if key.endswith("_ns"):
            del data["preparation"][key]
    return data


def read(path):
    campaign = json.loads((path / "campaign.json").read_text())
    runs = [json.loads((path / f"{i}.json").read_text()) for i in range(3)]
    assert not campaign["aggregate_censored"]
    assert all(run["status"] == "completed" for run in runs)
    result = []
    for run in runs:
        records = run["records"]
        diags = {r["data"]["phase"]: r["data"] for r in records if r["kind"] == "diagnostics"}
        phases = [r["data"] for r in records if r["kind"] == "phase"]
        outcome = next(r["data"] for r in records if r["kind"] == "result")
        final = next(r["data"] for r in records if r["kind"] == "allocations")
        if "release" in {p["phase"] for p in phases}:
            released = next(p for p in phases if p["phase"] == "release")
            assert released["complete"]
            assert all(v == (1 if k == "coordinate_records" else 0)
                       for k, v in released["memory"].items())
        result.append({"diagnostics": diags, "phases": phases,
                       "result": outcome, "allocations": final})
    return campaign["command"], result


root = Path(sys.argv[1])
report = {"samples_per_side": 3, "points": {}}
for path in sorted((root / "matrix").glob("before-*")):
    label = path.name.removeprefix("before-")
    # The initial raw-probes SIZE variation did not grow the workload; ROWS does.
    folder = root / ("control" if label.startswith("no-choice-") and (root / "control").exists() else "matrix")
    command_b, before = read(folder / f"before-{label}")
    command_a, after = read(folder / f"after-{label}")
    reference = {p: diagnostic(d) for p, d in before[0]["diagnostics"].items()}
    allowed = {"work.advance_iterations", "work.dispatch.cancellation",
               "work.shared.coordinates.cleanup_probes"}
    changes = {}
    for side, runs in [("before", before), ("after", after)]:
        for i, run in enumerate(runs):
            assert run["diagnostics"].keys() == reference.keys()
            for phase, d in run["diagnostics"].items():
                flat = dict(numeric(diagnostic(d)))
                base = dict(numeric(reference[phase]))
                differences = {k: [base[k], v] for k, v in flat.items() if v != base[k]}
                # HashMap capacity's usable count varies with its random seed;
                # allocation checkpoints still charge actual backing bytes.
                for key in differences:
                    assert key == "condition_slab.3" or (phase == "after_cancel" and key in allowed), (label, phase, key)
                if differences:
                    changes[f"{side}/{i}/{phase}"] = differences
            if "source" in reference:
                assert run["diagnostics"]["source"]["work"] == reference["source"]["work"]
                end = run["diagnostics"]["after_cancel"]["work"]
                assert end["waiters"]["entries"] == 0
                assert end["tasks_created"] == end["tasks_completed"] + end["tasks_canceled"]
                if side == "after":
                    old = reference["after_cancel"]["work"]
                    saved = reference["source"]["work"]["waiters"]["entries"]
                    assert old["dispatch"]["cancellation"] - end["dispatch"]["cancellation"] == saved
                    assert old["advance_iterations"] - end["advance_iterations"] == saved
            # Preserve all phase allocations, live/peak samples and result oracles.
            run["checkpoint_allocations"] = {p: d["allocation"] for p, d in run.pop("diagnostics").items()}
    summary = {}
    for side, runs in [("before", before), ("after", after)]:
        values = {"allocated_bytes": [], "allocations": [], "peak_bytes": [], "final_live_bytes": []}
        for run in runs:
            a = run["allocations"]
            values["allocated_bytes"].append(sum(p["allocated_bytes"] for p in a["phases"]))
            values["allocations"].append(sum(p["allocations"] for p in a["phases"]))
            values["peak_bytes"].append(a["process_peak_requested_bytes"])
            values["final_live_bytes"].append(a["process_live_requested_bytes"])
        summary[side] = {k: {"min": min(v), "median": statistics.median(v), "max": max(v)} for k, v in values.items()}
    report["points"][label] = {"commands": [command_b, command_a],
        "baseline_diagnostics": reference, "diagnostic_differences": changes,
        "summary": summary, "before": before, "after": after}

report["validated_runs"] = len(report["points"]) * 6
print(json.dumps(report, separators=(",", ":")))
