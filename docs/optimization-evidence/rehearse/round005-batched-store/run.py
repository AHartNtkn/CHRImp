"""Invoke the maintained runner on the declared paired points; resume completed cohorts.

Build both binaries first, as documented in README.md. This script supplies
workload arguments only; perf.py owns supervision, validation and statistics.
"""
import json
from pathlib import Path
import subprocess
import sys

points = {
    "rewrite32": ["rewrite", "32"],
    "rewrite128": ["rewrite", "128"],
    "answers256": ["answers", "256", "--rows", "0"],
    "fresh4": ["fresh-contract", "4", "--rows", "4", "--depth", "4"],
    "partial32": ["partial-join-hit", "32", "--rows", "4"],
    "duplicates8": ["duplicate-heads", "8", "--rows", "4"],
    "behavior-i": ["notebook-behavior-i", "1"],
    "behavior-s": ["notebook-behavior-s", "1"],
    "history64": ["life-history-choice", "64"],
    "alias128": ["life-alias", "128"],
    "fair-grow4": ["fair-grow", "4", "--rows", "8"],
}
for groups, arity in [(4, 0), (4, 4), (4, 16), (4, 64), (16, 4), (16, 16), (16, 64), (64, 16)]:
    points[f"wide{groups}-a{arity}"] = ["wide-rewrite", str(groups), "--rows", str(arity)]
for case in ("held-output", "archive-fixed", "inspections"):
    for rows in (16, 64):
        points[f"{case}{rows}"] = [f"life-{case}-conditional", "4", "--rows", str(rows), "--work", "64", "--cadence", "4"]
for mode in ("cancel", "snapshot"):
    for size in (64, 256):
        points[f"pending-{mode}{size}"] = [f"life-pending-{mode}", str(size)]
for name, args in points.items():
    for side, binary in [("baseline", "target/round005-baseline/release/examples/measure"), ("final", "target/release/examples/measure")]:
        out = Path("evidence/round005") / side / name
        if (out / "campaign.json").exists():
            report = json.loads((out / "campaign.json").read_text())
            assert report["summary"]["counts"] == {"completed": 5}, out
            continue
        subprocess.run([sys.executable, "examples/perf.py", "--binary", binary,
                        "--out", str(out), "--repeat", "5", "--warmup", "0",
                        "--seconds", "10", "--total-seconds", "60", "--",
                        *args[:2], "50000000", "3", *args[2:], "--detail"], check=True)
