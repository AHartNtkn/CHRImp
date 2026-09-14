"""Package existing perf.py records; no new measurement or acceptance logic.

Run from the experiment root after the cohorts in README.md have completed.
The readable file selects existing statistics. The gzip preserves every raw
typed sample and its process outcome, plus each campaign's command/provenance.
"""
import gzip
import hashlib
import json
from pathlib import Path

ROOT = Path("evidence/round004")
OUT = Path(__file__).resolve().parent
archive = {}
summary = {}
for final in sorted((ROOT / "final").glob("*/campaign.json")):
    name = final.parent.name
    summary[name] = {}
    for side in ("baseline", "final"):
        path = ROOT / side / name
        campaign = json.loads((path / "campaign.json").read_text())
        assert campaign["summary"]["counts"] == {"completed": 5}, path
        samples = [json.loads((path / f"{i}.json").read_text()) for i in range(5)]
        assert all(s["status"] == "completed" for s in samples), path
        archive[f"{side}/{name}"] = {"campaign": campaign, "samples": samples}
        metrics = campaign["summary"]["metrics"]
        selected = {k: v for k, v in metrics.items() if (
            k.startswith(("allocations.", "workload.", "phase."))
            or (k.startswith(("diagnostics.source.", "diagnostics.after_cancel."))
                and ".work." in k and ".rules." not in k)
        )}
        summary[name][side] = {
            "path": str(path), "command": campaign["command"],
            "completed_samples": len(samples),
            "medians": {k: v["median"] for k, v in selected.items()},
            "varying_ranges": {k: [v["min"], v["max"]] for k, v in selected.items()
                               if v["min"] != v["max"]},
            "incomplete_or_missing": {k: v["incomplete_or_missing"] for k, v in selected.items()
                                      if v["incomplete_or_missing"]},
            "first_sample_endpoints": [r for r in samples[0]["records"]
                                       if r["kind"] in ("result", "phase")],
        }
payload = json.dumps(archive, sort_keys=True, separators=(",", ":")).encode()
(OUT / "raw-samples.json.gz").write_bytes(gzip.compress(payload, mtime=0))
(OUT / "measurements.json").write_text(json.dumps(summary, sort_keys=True, indent=2) + "\n")
comparisons = {p.stem: json.loads(p.read_text()) for p in sorted((ROOT / "comparisons").glob("*.json"))}
(OUT / "comparisons.json").write_text(json.dumps(comparisons, sort_keys=True, indent=2) + "\n")
print(f"{len(summary)} paired points, {len(archive) * 5} completed samples")
print("raw uncompressed SHA256", hashlib.sha256(payload).hexdigest())
