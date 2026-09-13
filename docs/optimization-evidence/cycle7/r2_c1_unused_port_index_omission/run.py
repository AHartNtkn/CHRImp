"""Small campaign driver; maintained supervisor and native semantic oracles.

Run from the assigned worktree: python3 <this file> baseline|candidate BINARY
or: python3 <this file> check LABEL COMMAND ...
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path.cwd()
EVIDENCE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT / "examples"))
import supervise


def execute(label, command, seconds=15):
    out = EVIDENCE / label
    out.mkdir(parents=True, exist_ok=False)
    with (out / "stdout.log").open("w") as stdout, (out / "stderr.log").open("w") as stderr:
        record = supervise.run(command, ROOT, stdout, stderr, seconds=seconds, memory_mib=2048)
    record["source"] = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    record["dirty"] = subprocess.check_output(["git", "status", "--short"], text=True)
    (out / "run.json").write_text(json.dumps(record, indent=2) + "\n")
    print(label, record, flush=True)
    return out


if sys.argv[1] == "check":
    execute(sys.argv[2], sys.argv[3:], seconds=300)
else:
    variant, binary = sys.argv[1:]
    binary = str(Path(binary).resolve())
    cases = {
        "fresh64": ["fresh-contract", "64", "--rows", "4", "--depth", "4"],
        "fresh256": ["fresh-contract", "256", "--rows", "4", "--depth", "4"],
        "unmerged64": ["fresh-unmerged", "64", "--rows", "4", "--depth", "4"],
        "onecopy64": ["fresh-contract", "64", "--rows", "1", "--depth", "4"],
        "depthzero64": ["fresh-contract", "64", "--rows", "4", "--depth", "0"],
        "rewrite128": ["rewrite", "128"],
        "partial128": ["partial-join", "128", "--rows", "8"],
        "retained": ["life-archive-fixed", "2", "--rows", "3", "--work", "24", "--cadence", "4"],
    }
    manifest = {"binary": binary, "sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                "toolchain": subprocess.check_output(["rustc", "--version"], text=True).strip(),
                "diagnostics": True, "observations_per_case": 1, "runtime_verdict": False}
    summary = {}
    for name, args in cases.items():
        args = args[:2] + ["50000000", "5"] + args[2:]
        out = execute(f"{variant}-{name}", [binary, *args])
        records = [json.loads(line.removeprefix("measurement="))
                   for line in (out / "stdout.log").read_text().splitlines()
                   if line.startswith("measurement=")]
        summary[name] = records
    (EVIDENCE / f"{variant}.json").write_text(json.dumps({"manifest": manifest, "cases": summary}, indent=2) + "\n")
