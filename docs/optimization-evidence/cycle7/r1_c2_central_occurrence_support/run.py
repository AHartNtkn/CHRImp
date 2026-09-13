"""Local evidence command recorder; no benchmark or acceptance logic."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent
name = sys.argv[1]
command = sys.argv[2:]
out = root / name
out.mkdir(parents=True, exist_ok=False)
record = {
    "command": command,
    "cwd": os.getcwd(),
    "started": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "dirty": subprocess.check_output(["git", "status", "--short"], text=True),
}
(out / "run.json").write_text(json.dumps(record, indent=2) + "\n")
with (out / "stdout.log").open("w") as stdout, (out / "stderr.log").open("w") as stderr:
    result = subprocess.run(command, stdout=stdout, stderr=stderr)
record.update(exit_status=result.returncode, finished=datetime.datetime.now(datetime.timezone.utc).isoformat())
(out / "run.json").write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps(record))
sys.exit(result.returncode)
