"""Run separately built Cargo test artifacts, each under the required timeout."""
import json
from pathlib import Path
import subprocess
import sys

artifacts = []
for line in Path(sys.argv[1]).read_text().splitlines():
    r = json.loads(line)
    if r.get("reason") == "compiler-artifact" and r.get("profile", {}).get("test") and r.get("executable"):
        artifacts.append(r["executable"])
failed = []
with Path(sys.argv[2]).open("w") as log:
    for binary in sorted(set(artifacts)):
        command = ["timeout", "--kill-after=5s", "60s", binary]
        log.write("COMMAND " + " ".join(command) + "\n")
        log.flush()
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            failed.append((binary, result.returncode))
print(f"{len(set(artifacts))} test binaries; failures: {failed}")
sys.exit(bool(failed))
