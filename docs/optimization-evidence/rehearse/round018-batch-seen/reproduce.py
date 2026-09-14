#!/usr/bin/env python3
"""Round 018 orchestration of existing test/measurement runners, not a benchmark."""
import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]


def run(command, path, env=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w") as out:
        result = subprocess.run(command, cwd=ROOT, stdout=out,
                                stderr=subprocess.STDOUT, env=env)
    if result.returncode:
        raise RuntimeError(f"{command}: exit {result.returncode}; {path}")


def probes(binary, out, repeats):
    cases = set()
    for n in [0, 1, 2, 8, 9, 16, 64, 256]:
        for distinct in {1, max(1, n // 4), max(1, n),
                         min(max(n, 1), 7), min(max(n, 1), 8), min(max(n, 1), 9)}:
            for layout in ["dense", "sparse"]:
                for mode in ["insert", "mixed", "noop", "delete"]:
                    cases.add(f"{n}/{distinct}/{layout}/{mode}")
    observations = []
    for case in sorted(cases):
        for repeat in range(repeats):
            path = out / (case.replace("/", "-") + f"-{repeat}.log")
            env = dict(os.environ, CHRIMP_BATCH_CASE=case)
            run(["timeout", "--kill-after=5s", "60s", str(binary), "--exact",
                 "batch_order_and_lifecycle_matrix", "--nocapture",
                 "--test-threads=1"], path, env)
            rows = [json.loads(line.split("batch_probe=", 1)[1]) for line in
                    path.read_text().splitlines() if "batch_probe=" in line]
            assert len(rows) == 1
            observations.append(dict(case=case, repeat=repeat, **rows[0]))
    (out / "observations.json").write_text(json.dumps(observations, indent=2) + "\n")


def maintained(binary, out, repeats):
    points = []
    for arity in [0, 1, 8, 16, 64, 256]:
        points.append(("wide-arity-" + str(arity),
                       ["wide-rewrite", "4", "--rows", str(arity)]))
    for n in [16, 64, 256]:
        for case, options in [("wide-rewrite", ["--rows", "16"]),
                              ("repeated-alias", ["--rows", "8"]),
                              ("alias-consume", [])]:
            points.append((f"{case}-{n}", [case, str(n), *options]))
    for case, options in [
            ("life-archive-rotate-conditional", ["--rows", "8", "--work", "32", "--cadence", "4"]),
            ("life-inspections-conditional", ["--rows", "8", "--work", "32"]),
            ("life-pending-cancel", []),
            ("life-history-choice", []),
            ("fair-grow", ["--rows", "8"]),
            ("partial-join-hit", ["--rows", "8"]),
            ("notebook-type-i", []),
            ("notebook-type-s", [])]:
        points.append((case, [case, "1" if case.startswith("notebook") else "4", *options]))
    for name, args in points:
        workload = [*args[:2], "50000000", "3", *args[2:], "--detail"]
        run(["python3", "examples/perf.py", "--binary", str(binary),
             "--out", str(out / name), "--repeat", str(repeats), "--warmup", "0",
             "--seconds", "10", "--total-seconds", "45", "--", *workload],
            out / (name + ".log"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["probes", "maintained"])
    parser.add_argument("binary", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--repeat", type=int, default=3)
    args = parser.parse_args()
    globals()[args.mode](args.binary.resolve(), args.out.resolve(), args.repeat)
