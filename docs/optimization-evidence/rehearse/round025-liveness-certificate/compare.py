"""Paired invocations of the maintained measure oracles; retain every raw record."""
import hashlib
import json
import pathlib
import subprocess
import sys
import argparse

parser = argparse.ArgumentParser()
parser.add_argument("out")
parser.add_argument("--only", action="append")
parser.add_argument("--repeat", type=int, default=3)
args = parser.parse_args()
OUT = pathlib.Path(args.out)
OUT.mkdir(parents=True, exist_ok=True)
ROOTS = {
    "before": pathlib.Path("/tmp/chrimp-round024-baseline-harness"),
    "after": pathlib.Path("/tmp/chrimp-opt-round025-liveness-certificate"),
}
CASES = [
    (f"rotate-r{rows}-w{work}-o{owners}-c{cadence}", "life-archive-rotate-conditional", owners,
     ["--rows", str(rows), "--work", str(work), "--cadence", str(cadence)])
    for rows, work, owners, cadence in [(8, 32, 4, 1), (32, 32, 4, 1), (128, 32, 4, 1),
                                      (8, 128, 4, 1), (8, 32, 1, 1), (8, 32, 8, 1),
                                      (8, 32, 4, 4)]
] + [
    (f"{case}-{n}", case, n, extra)
    for case, sizes, extra in [
        ("rewrite", (8, 64, 256), []),
        ("alias-consume", (8, 32), []),
        ("fresh-contract", (4, 16), ["--rows", "4", "--depth", "4", "--order", "interleaved"]),
        ("wide-rewrite", (8, 32), ["--rows", "32"]),
        ("bits-chain", (4, 8), []),
        ("bits-star-delayed", (4, 8), []),
        ("reach-chain", (8, 32), []),
        ("duplicate-heads", (4,), ["--rows", "3"]),
        ("answers", (32,), ["--rows", "0"]),
        ("fair-loop", (2, 8), ["--rows", "8"]),
        ("fair-grow", (2, 8), ["--rows", "8"]),
        ("notebook-behavior-i", (1,), []),
        ("notebook-behavior-k", (1,), []),
        ("notebook-arithmetic-forward", (8,), []),
        ("life-alias", (32,), []),
        ("life-archive-fixed", (4,), ["--rows", "1", "--work", "32"]),
        ("life-archive-rotate", (4,), ["--rows", "1", "--work", "32"]),
        ("life-archive-fixed-conditional", (4,), ["--rows", "8", "--work", "32"]),
        ("life-inspections-conditional", (4,), ["--rows", "8", "--work", "32"]),
        ("life-held-output-conditional", (4,), ["--rows", "8", "--work", "32"]),
        ("life-pending-snapshot", (8, 64), []),
        ("life-pending-cancel", (8, 64), []),
        ("runtime-sessions", (8,), ["--rows", "8", "--work", "32"]),
    ] for n in sizes
]
if args.only:
    CASES = [case for case in CASES if case[0] in args.only]
    assert len(CASES) == len(args.only), "unknown or duplicate case name"

manifest = {side: {"root": str(root), "revision": subprocess.check_output(
    ["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "diff": subprocess.check_output(["git", "diff"], cwd=root, text=True),
    "binary_sha256": hashlib.sha256((root / "target/release/examples/measure").read_bytes()).hexdigest()}
    for side, root in ROOTS.items()}
manifest_path = OUT / "manifest.json"
if manifest_path.exists():
    old = json.loads(manifest_path.read_text())
    assert all(old[side]["binary_sha256"] == manifest[side]["binary_sha256"] for side in ROOTS), "changed binary: use a new output directory"
else:
    manifest_path.write_text(json.dumps(manifest, indent=2))
summary = []
for repeat in range(args.repeat):
    for name, case, n, extra in CASES:
        row = {"name": name, "case": case, "size": n, "extra": extra, "repeat": repeat}
        for side in (list(ROOTS) if repeat % 2 == 0 else list(reversed(ROOTS))):
            root = ROOTS[side]
            prefix = OUT / f"{repeat}-{name}-{side}"
            cmd = ["timeout", "--kill-after=5s", "60s", str(root / "target/release/examples/measure"),
                   case, str(n), "50000000", "10", "--detail", *extra]
            if prefix.with_suffix(".command.json").exists():
                invocation = json.loads(prefix.with_suffix(".command.json").read_text())
                assert invocation["command"] == cmd and invocation["cwd"] == str(root), "changed workload: use a new output directory"
            else:
                with prefix.with_suffix(".log").open("w") as log:
                    result = subprocess.run(cmd, cwd=root, stdout=log, stderr=subprocess.STDOUT)
                invocation = {"command": cmd, "cwd": str(root), "exit": result.returncode}
                prefix.with_suffix(".command.json").write_text(json.dumps(invocation, indent=2))
            records = [json.loads(line.removeprefix("measurement=")) for line in
                       prefix.with_suffix(".log").read_text().splitlines() if line.startswith("measurement=")]
            row[side] = {"invocation": invocation, "records": records}
            print(repeat, name, side, invocation["exit"], flush=True)
        summary.append(row)
        (OUT / "summary.json").write_text(json.dumps(summary, indent=2))
if any(row[side]["invocation"]["exit"] != 0 for row in summary for side in ROOTS):
    sys.exit(1)
