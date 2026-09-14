"""Short paired check using the maintained runner; binaries must be prebuilt."""
import json
from pathlib import Path
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(sys.argv[1])
BASE = Path(sys.argv[2])
CASES = [
    ("small", 8, 1, 1, 0, 0, 0, False),
    ("stream", 128, 16, 1, 0, 0, 0, False),
    ("prefill", 128, 16, 1, 0, 0, 0, True),
    ("batch", 128, 16, 4096, 1, 0, 0, True),
    ("large-tail", 512, 16, 1, 0, 0, 0, True),
    ("retained", 8, 512, 4096, 1, 4, 0, False),
    ("background", 32, 8, 1, 1, 2, 32, False),
    ("background-batch", 128, 16, 4096, 1, 0, 32, False),
]
OUT.mkdir(parents=True, exist_ok=True)
summary = []
for name, n, rows, batch, replay, retained, work, prefill in CASES:
    if len(sys.argv) > 3 and name not in sys.argv[3:]:
        continue
    pair = {}
    for variant, root in [("before", BASE), ("after", ROOT)]:
        dest = OUT / name / variant
        cmd = ["timeout", "--kill-after=5s", "60s", "python3", str(ROOT / "examples/perf.py"),
               "--binary", str(root / "target/release/examples/measure"), "--out", str(dest),
               "--repeat", "3", "--warmup", "0", "--seconds", "20", "--total-seconds", "55",
               "--", "runtime-sessions", str(n), "5000000", "15", "--rows", str(rows),
               "--batch", str(batch), "--replay-every", str(replay), "--retained", str(retained),
               "--work", str(work), "--detail"] + (["--prefill"] if prefill else [])
        subprocess.run(cmd, cwd=root, check=True, stdout=subprocess.DEVNULL)
        samples = []
        for i in range(3):
            raw = json.loads((dest / f"{i}.json").read_text())
            assert raw["status"] == "completed", raw
            r = next(x["data"] for x in raw["records"] if x["kind"] == "result")
            assert r["status"] == "COMPLETE"
            metrics = [s["metrics"] for s in r["before_close_spool_work"]]
            sample = dict(logical=r["before_close_spools"]["logical_bytes"],
                          disk=r["before_close_spools"]["allocated_bytes"],
                          source_ms=r["source_ms"], read_ms=r["source"]["read_ms"],
                          cleanup_ms=r["cleanup"]["elapsed_ms"], drop_ms=r["runtime_drop_ms"],
                          rss_kib=r["native_peak_rss_estimate_kib"],
                          process_rss_kib=raw["process"]["max_rss_kib"],
                          hash=r["source_and_setup"]["event_hash"],
                          applications=r["finite_applications"], background=r["background_applications"],
                          bytes=r["source_and_setup"]["delivered_event_json_bytes"])
            for key in ("copied_bytes", "reclaimed_bytes", "compactions", "write_ns", "flush_ns", "read_ns", "compaction_ns"):
                sample[key] = sum(m[key] for m in metrics)
            for phase in ("after_close_spools", "after_drop_spools"):
                assert all(value == 0 for value in r[phase].values())
            samples.append(sample)
        pair[variant] = samples
    for key in ("hash", "applications", "background", "bytes"):
        assert len({s[key] for values in pair.values() for s in values}) == 1, (name, key)
    medians = {v: {k: statistics.median(s[k] for s in ss) for k in ss[0] if k != "hash"}
               for v, ss in pair.items()}
    summary.append(dict(case=name, medians=medians, samples=pair))
    print(name, json.dumps(medians), flush=True)
    (OUT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
