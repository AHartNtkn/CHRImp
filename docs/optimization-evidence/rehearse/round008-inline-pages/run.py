"""Reuse accepted Round 007 samples; run the unchanged maintained oracles.

Usage: python3 run.py CANDIDATE_BINARY OUTPUT [comma-separated-points]
The source/cleanup limits and five-sample cohorts are the maintained Round 007
configuration, not a candidate/campaign budget. Builds are separate.
"""
import gzip
import json
from pathlib import Path
import runpy
import subprocess
import sys

HERE = Path(__file__).resolve().parent
PREVIOUS = HERE.parent / "round007-adaptive-pages"
points = runpy.run_path(str(PREVIOUS / "run.py"))["points"]

if __name__ == "__main__":
    binary, output = sys.argv[1], Path(sys.argv[2])
    selected = sys.argv[3].split(",") if len(sys.argv) > 3 else points
    accepted = json.loads(gzip.decompress((PREVIOUS / "raw-samples.json.gz").read_bytes()))
    for label in selected:
        saved = accepted[f"candidate/{label}"]
        baseline = output / f"baseline-{label}"
        baseline.mkdir(parents=True, exist_ok=True)
        (baseline / "campaign.json").write_text(json.dumps(saved["campaign"]))
        for i, sample in enumerate(saved["samples"]):
            (baseline / f"{i}.json").write_text(json.dumps(sample))
        destination = output / f"candidate-{label}"
        if (destination / "campaign.json").exists():
            assert json.loads((destination / "campaign.json").read_text())["summary"]["counts"] == {"completed": 5}
            continue
        args = points[label]
        print(label, flush=True)
        subprocess.run([sys.executable, "examples/perf.py", "--binary", binary,
                        "--out", str(destination), "--repeat", "5", "--warmup", "0",
                        "--seconds", "55", "--total-seconds", "600", "--",
                        *args[:2], "500000000", "45", *args[2:], "--detail"], check=True)
