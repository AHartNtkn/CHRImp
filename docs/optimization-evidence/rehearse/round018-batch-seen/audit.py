#!/usr/bin/env python3
"""Audit exact completed obligations and report all allocation/work differences."""
import json
from pathlib import Path
import statistics
import sys

ROOT = Path(sys.argv[1])


def read(path):
    return json.loads(path.read_text())


def total(checkpoint, field):
    return sum(phase[field] for phase in checkpoint["phases"])


def probes():
    before = read(ROOT / "probes/baseline/observations.json")
    after = read(ROOT / "probes/candidate/observations.json")
    assert len(before) == len(after)
    output = []
    for a, b in zip(before, after):
        assert (a["case"], a["repeat"], a["semantic"]) == (b["case"], b["repeat"], b["semantic"])
        for key in ["nodes_allocated", "pages_before", "pages_after", "mutations", "cleanup_ticks"]:
            assert a[key] == b[key], (a["case"], key)
        for key in ["calls", "input_writes", "membership_checks", "retained_writes", "max_input_writes"]:
            assert a["batch"][key] == b["batch"][key], (a["case"], key)
        row = dict(case=a["case"], repeat=a["repeat"])
        for label, p in [("baseline", a), ("candidate", b)]:
            c = p["checkpoints"]
            # Rust's test harness can allocate concurrently even with one test
            # thread. Keep that unscoped traffic visible, and independently
            # require the explicitly scoped fixture to balance after its drops.
            def other_live(s):
                o = next(v for v in s["phases"] if v["phase"] == "other")
                return o["allocated_bytes"] - o["freed_bytes"]
            other_delta = other_live(c["dropped"]) - other_live(c["start"])
            live_delta = c["dropped"]["process_live_requested_bytes"] - c["start"]["process_live_requested_bytes"]
            assert live_delta - other_delta == p["retained_report_bytes"], p["case"]
            metrics = dict(p["batch"])
            metrics["final_unscoped_live_delta"] = other_delta
            metrics["final_fixture_live_excluding_report"] = live_delta - other_delta - p["retained_report_bytes"]
            for phase in ["prepared", "updated", "validated", "released", "dropped"]:
                metrics[phase] = {
                    "allocations_since_start": total(c[phase], "allocations") - total(c["start"], "allocations"),
                    "allocated_bytes_since_start": total(c[phase], "allocated_bytes") - total(c["start"], "allocated_bytes"),
                    "freed_bytes_since_start": total(c[phase], "freed_bytes") - total(c["start"], "freed_bytes"),
                    "live_bytes_since_start": c[phase]["process_live_requested_bytes"] - c["start"]["process_live_requested_bytes"],
                    "process_peak_bytes": c[phase]["process_peak_requested_bytes"],
                }
            for field in ["allocations", "allocated_bytes", "freed_bytes"]:
                metrics["batch_" + field] = total(c["updated"], field) - total(c["prepared"], field)
            row[label] = metrics
        if b["distinct"] <= 8:
            assert b["batch"]["scratch_tables"] == b["batch"]["hash_requests"] == 0
            for field in ["batch_allocations", "batch_allocated_bytes"]:
                assert row["baseline"][field] == row["candidate"][field]
        output.append(row)
    return output


def differences(a, b, path=""):
    if isinstance(a, dict) and isinstance(b, dict) and a.keys() == b.keys():
        return [d for key in a for d in differences(a[key], b[key], path + "." + key)]
    return [] if a == b else [dict(path=path, baseline=a, candidate=b)]


def maintained(folder):
    output = []
    for campaign in sorted((ROOT / folder / "baseline").glob("*/campaign.json")):
        name = campaign.parent.name
        other = ROOT / folder / "candidate" / name
        summary_a = read(campaign)["summary"]
        summary_b = read(other / "campaign.json")["summary"]
        assert summary_a["counts"] == summary_b["counts"] == {"completed": 3}
        point = dict(name=name, samples=[], allocation_metrics={})
        for sample in sorted(campaign.parent.glob("[0-9]*.json")):
            a, b = read(sample), read(other / sample.name)
            assert a["status"] == b["status"] == "completed"
            ra, rb = a["records"], b["records"]
            assert [r["kind"] for r in ra] == [r["kind"] for r in rb]
            result = dict(sample=sample.name, variations=[], results=[], diagnostics=[])
            for x, y in zip(ra, rb):
                kind = x["kind"]
                x, y = x["data"], y["data"]
                if kind == "configuration":
                    assert x == y
                if kind == "result":
                    for key in ["status", "error", "expected", "source_goal_reached", "cleanup_done"]:
                        assert x.get(key) == y.get(key), (name, key)
                    if isinstance(x.get("work"), dict):
                        for key in ["answers", "applications", "facts", "ports", "scalars", "cleanup_ticks"]:
                            assert x["work"].get(key) == y["work"].get(key), (name, key)
                    else:
                        assert x.get("work") == y.get("work"), name
                    if "memory_counts" in x:
                        for phase in ["before_cleanup", "after_cleanup"]:
                            assert x["memory_counts"][phase] == y["memory_counts"][phase], (name, phase)
                    result["results"].append(dict(baseline=x, candidate=y))
                if kind == "diagnostics":
                    for key in ["field_updates", "graph_allocations", "graph_pages", "memory_counts",
                                "normalization", "prefix_lookup", "prefix_memo", "shared_restrictions",
                                "store_mutations", "store_batches"]:
                        assert x[key] == y[key], (name, key)
                    assert x["work"]["rules"] == y["work"]["rules"], name
                    # Preserve all work counters, including collector variations;
                    # do not silently normalize them into claims of identical work.
                    result["diagnostics"].append(dict(phase=x["phase"],
                        baseline_work=x["work"], candidate_work=y["work"],
                        store_batches=x["store_batches"]))
                    xx = {k: v for k, v in x.items() if k != "allocation"}
                    yy = {k: v for k, v in y.items() if k != "allocation"}
                    result["variations"].extend(differences(xx, yy, x["phase"]))
            point["samples"].append(result)
        for key in summary_a["metrics"]:
            if key.startswith("allocations."):
                a, b = summary_a["metrics"][key], summary_b["metrics"][key]
                point["allocation_metrics"][key] = dict(baseline=a, candidate=b,
                    median_delta=b["median"] - a["median"])
        output.append(point)
    assert output
    return output


result = dict(probes=probes(), maintained=maintained("maintained"), suite=maintained("suite"))
(ROOT / "audit.json").write_text(json.dumps(result, indent=2) + "\n")
print("PASS:", len(result["probes"]), "exact synthetic pairs;",
      sum(len(p["samples"]) for p in result["maintained"]), "maintained pairs;",
      sum(len(p["samples"]) for p in result["suite"]), "suite pairs")
