"""Run the maintained Round 007/008 workload matrix for Round 009.

Usage: python3 run.py BASELINE_BINARY CANDIDATE_BINARY OUTPUT [points]
All workload limits/repeats are inherited from the maintained prior campaign;
there is no candidate time budget. Build binaries separately. Baseline cohorts
may be copied from /tmp/chrimp-round009-measured (the exact accepted revision).
"""
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
points = runpy.run_path(str(HERE.parent / "round007-adaptive-pages/run.py"))["points"]

if __name__ == "__main__":
    output = Path(sys.argv[3])
    selected = sys.argv[4].split(",") if len(sys.argv) > 4 else points
    for label in selected:
        for side, binary in zip(("baseline", "candidate"), sys.argv[1:3]):
            out = output / f"{side}-{label}"
            for prior_root in ("/tmp/chrimp-round009-verified", "/tmp/chrimp-round009-final", "/tmp/chrimp-round009-measured"):
                prior = Path(prior_root) / out.name
                if side == "baseline" and not out.exists() and (prior / "campaign.json").exists():
                    saved = json.loads((prior / "campaign.json").read_text())
                    if saved.get("summary", {}).get("counts") == {"completed": 5}:
                        shutil.copytree(prior, out)
            if (out / "campaign.json").exists():
                counts = json.loads((out / "campaign.json").read_text())["summary"]["counts"]
                assert sum(counts.values()) == 5 and not set(counts) - {"completed", "censored"}, out
                continue
            args = points[label]
            print(side, label, flush=True)
            result = subprocess.run([sys.executable, "examples/perf.py", "--binary", binary,
                            "--out", str(out), "--repeat", "5", "--warmup", "0",
                            "--seconds", "55", "--total-seconds", "600", "--",
                            *args[:2], "500000000", "45", *args[2:], "--detail"])
            assert result.returncode in (0, 2), out
            counts = json.loads((out / "campaign.json").read_text())["summary"]["counts"]
            assert sum(counts.values()) == 5 and not set(counts) - {"completed", "censored"}, out
