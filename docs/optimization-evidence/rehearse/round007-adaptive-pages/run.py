"""Run existing semantic/lifecycle measurements; perf.py owns all oracles.

Build each binary separately. Arguments: baseline-binary candidate-binary output.
An optional fourth argument selects a comma-separated subset. Completed cohorts
are reused; unfinished cohorts are errors requiring investigation.
"""
import json
from pathlib import Path
import subprocess
import sys

points = {
    "graph-bits7": ["graph-bits", "7", "--shape", "random", "--seed", "17"],
    "duplicate4": ["duplicate-heads", "4", "--rows", "3", "--seed", "17"],
    "partial32": ["partial-join-hit", "32", "--rows", "4"],
    "rewrite128": ["rewrite", "128"],
    "answers256": ["answers", "256", "--rows", "0"],
    "behavior-i": ["notebook-behavior-i", "1"],
    "behavior-s": ["notebook-behavior-s", "1"],
    "lambda2": ["notebook-lambda", "2"],
    "history64": ["life-history-choice", "64"],
    "fair-grow4": ["fair-grow", "4", "--rows", "8"],
    "pending-cancel64": ["life-pending-cancel", "64"],
    "pending-snapshot64": ["life-pending-snapshot", "64"],
    "runtime": ["runtime-sessions", "8", "--closed", "8", "--retained", "4", "--rows", "8", "--batch", "1", "--replay-every", "2", "--work", "32"],
}
for n in (64, 256):
    for order in ("grouped", "interleaved", "reverse"):
        points[f"fresh{n}-{order}"] = ["fresh-contract", str(n), "--rows", "4", "--depth", "4", "--order", order]
for kind in ("held-output", "archive-fixed", "archive-rotate", "inspections"):
    points[kind] = [f"life-{kind}-conditional", "4", "--rows", "64", "--work", "64", "--cadence", "4"]
for n in (4, 16):
    points[f"wide{n}"] = ["wide-rewrite", str(n), "--rows", "64"]

if __name__ == "__main__":
    output = Path(sys.argv[3])
    selected = sys.argv[4].split(",") if len(sys.argv) > 4 else points
    for label in selected:
        args = points[label]
        for side, binary in zip(("baseline", "candidate"), sys.argv[1:3]):
            out = output / f"{side}-{label}"
            if (out / "campaign.json").exists():
                assert json.loads((out / "campaign.json").read_text())["summary"]["counts"] == {"completed": 5}, out
                continue
            command = [sys.executable, "examples/perf.py", "--binary", binary,
                       "--out", str(out), "--repeat", "5", "--warmup", "0",
                       "--seconds", "55", "--total-seconds", "600", "--",
                       *args[:2], "500000000", "45", *args[2:], "--detail"]
            print(side, label, flush=True)
            subprocess.run(command, check=True)
