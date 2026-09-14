"""Package existing perf.py records; all raw endpoints/metrics are retained."""
import gzip
import json
from pathlib import Path
import sys
from run import points

root = Path(sys.argv[1])
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parent
out.mkdir(parents=True, exist_ok=True)
archive, summary = {}, {}
for label in points:
    if not (root / f"candidate-{label}" / "campaign.json").exists():
        continue
    summary[label] = {}
    for side in ("baseline", "candidate"):
        directory = root / f"{side}-{label}"
        campaign = json.loads((directory / "campaign.json").read_text())
        assert campaign["summary"]["counts"] == {"completed": 5}, directory
        samples = [json.loads((directory / f"{i}.json").read_text()) for i in range(5)]
        assert all(s["status"] == "completed" for s in samples)
        archive[f"{side}/{label}"] = {"campaign": campaign, "samples": samples}
        metrics = campaign["summary"]["metrics"]
        summary[label][side] = {
            "command": campaign["command"],
            "medians": {k: v["median"] for k, v in metrics.items()},
            "varying_ranges": {k: [v["min"], v["max"]] for k, v in metrics.items() if v["min"] != v["max"]},
            "incomplete_or_missing": {k: v["incomplete_or_missing"] for k, v in metrics.items() if v["incomplete_or_missing"]},
            "first_sample_endpoints": [r for r in samples[0]["records"] if r["kind"] in ("result", "phase")],
        }
(out / "raw-samples.json.gz").write_bytes(gzip.compress(json.dumps(archive, sort_keys=True).encode(), mtime=0))
(out / "measurements.json").write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n")
print(f"{len(summary)} paired points, {len(archive)*5} samples (half reused baseline)")
