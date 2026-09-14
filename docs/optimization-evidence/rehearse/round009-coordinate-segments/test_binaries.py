"""Run prebuilt Cargo test executables with the required per-invocation guard.

Usage: python3 test_binaries.py BUILD_LOG OUTPUT_LOG [name-fragment]
Build log comes from cargo test --all-targets --no-run, separately from tests.
"""
import re
from pathlib import Path
import subprocess
import sys

binaries = re.findall(r"Executable .* \(([^)]+)\)", Path(sys.argv[1]).read_text())
assert binaries, "no test binaries in build log"
selected = sys.argv[3] if len(sys.argv) > 3 else ""
failed = []
with Path(sys.argv[2]).open("w") as log:
    for binary in binaries:
        if selected not in binary:
            continue
        command = ["timeout", "--kill-after=5s", "60s", binary]
        print(" ".join(command), file=log, flush=True)
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        print(f"EXIT {result.returncode}: {binary}", file=log, flush=True)
        print(f"EXIT {result.returncode}: {binary}", flush=True)
        if result.returncode:
            failed.append((binary, result.returncode))
print(f"Failures: {failed}")
sys.exit(bool(failed))
