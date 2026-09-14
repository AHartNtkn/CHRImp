#!/usr/bin/env python3
"""Execute prebuilt Cargo test artifacts, with one 60-second guard per binary."""
import json
from pathlib import Path
import re
import subprocess
import sys

build, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
results = []
for binary in dict.fromkeys(re.findall(r"Executable.*\(([^)]+)\)", build.read_text())):
    command = ["timeout", "--kill-after=5s", "60s", binary, "--test-threads=1"]
    path = output / (Path(binary).name + ".log")
    with path.open("w") as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    text = path.read_text()
    summaries = re.findall(r"test result:.*", text)
    row = dict(command=command, exit=result.returncode, summaries=summaries)
    results.append(row)
    print(Path(binary).name, result.returncode, *summaries, flush=True)
(output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
assert results, "build log must contain tests"
sys.exit(int(any(r["exit"] for r in results)))
