#!/usr/bin/env python3
"""Round 022 selection of maintained perf.py workloads; no new benchmark engine.

Build each binary separately. Every runner invocation has the required 60s guard.
Usage: reproduce.py BASELINE_WORKTREE CANDIDATE_WORKTREE OUT [--ordinary]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess


def matrix():
    cases = []

    def add(name, case, n, *options, env=None):
        cases.append((name, [case, str(n), "50000000", "3", *map(str, options)], env or {}))

    for n in (0, 1, 16, 64, 256):
        add(f"prepare-tiny-{n}", "prepare-reuse", n)
    for n in (16, 64, 256):
        add(f"prepare-shape-{n}", "prepare-reuse", n, "--heads", 4, "--arity", 4,
            "--repeats", 4, "--width", 16, "--depth", 8, "--uses", 4)
    add("prepare-wide", "prepare-reuse", 64, "--width", 256)
    add("prepare-heads", "prepare-reuse", 16, "--heads", 64, "--arity", 8)
    add("prepare-deep", "prepare-reuse", 64, "--depth", 120)
    add("prepare-independent", "prepare-independent", 64, "--heads", 4, "--arity", 4,
        "--repeats", 4, "--width", 16, "--depth", 8, "--uses", 4)
    for n in (32, 128):
        add(f"rejected3-{n}", "rejected3", n)
        add(f"repeated-alias-{n}", "repeated-alias", n, "--rows", n)
        add(f"answers-{n}", "answers", n, "--rows", 0)
        add(f"archive-{n}", "life-archive", n)
    add("answers-wide", "answers", 32, "--rows", 16)
    for n in (2, 8):
        add(f"inspections-structural-{n}", "life-inspections-structural", n, "--rows", 16, "--work", 64)
    add("archive-structural", "life-archive-fixed-structural", 4, "--rows", 32, "--work", 64)
    add("archive-conditional", "life-archive-rotate-conditional", 4,
        "--rows", 16, "--work", 64, "--cadence", 4)
    for n in (8, 64):
        for mode in ("snapshot", "cancel"):
            add(f"pending-{mode}-{n}", f"life-pending-{mode}", n)
    add("fair-loop", "fair-loop", 4, "--rows", 8)
    for n in (64, 256):
        add(f"fanout-{n}", "fanout", n)
    for case in ("notebook-behavior-i", "notebook-type-i"):
        add(case, case, 1)
    return cases


def main():
    p = argparse.ArgumentParser(__doc__)
    p.add_argument("baseline", type=Path)
    p.add_argument("candidate", type=Path)
    p.add_argument("out", type=Path)
    p.add_argument("--ordinary", action="store_true")
    p.add_argument("--repeat", type=int, default=5)
    p.add_argument("--resume", action="store_true")
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=args.resume)
    runs = json.loads((args.out / "runs.json").read_text()) if args.resume else []
    # Alternate side order by point; do not overlap measured processes.
    for index, (name, workload, env) in enumerate(matrix()):
        for side in (("baseline", "candidate") if index % 2 == 0 else ("candidate", "baseline")):
            if any(r["name"] == name and r["side"] == side and r["returncode"] == 0 for r in runs):
                continue
            root = getattr(args, side).resolve()
            binary = root / "target/release/examples/measure"
            if args.ordinary:
                binary = root / "target/ordinary/release/examples/measure"
            destination = args.out.resolve() / name / side
            command = ["timeout", "--kill-after=5s", "60s", "python3", "examples/perf.py",
                       "--binary", str(binary), "--out", str(destination), "--repeat", str(args.repeat),
                       "--warmup", "0", "--seconds", "10", "--total-seconds", "50", "--", *workload]
            if not args.ordinary:
                command.append("--detail")
            result = subprocess.run(command, cwd=root, env={**os.environ, **env}, text=True, capture_output=True)
            runs.append(dict(name=name, side=side, command=command, cwd=str(root), env=env,
                             returncode=result.returncode, stdout=result.stdout, stderr=result.stderr))
            (args.out / "runs.json").write_text(json.dumps(runs, indent=2) + "\n")
            print(name, side, result.returncode, flush=True)
            if result.returncode:
                raise SystemExit(result.stderr or result.stdout or str(result.returncode))


if __name__ == "__main__":
    main()
