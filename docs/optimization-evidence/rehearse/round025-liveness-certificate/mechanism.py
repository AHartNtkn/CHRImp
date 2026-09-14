"""Public Graph lifecycle fixture with the maintained requested-allocation observer."""
import json
import os
import pathlib
import subprocess
import sys

out = pathlib.Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
roots = {"before": pathlib.Path("/tmp/chrimp-round024-baseline-harness"),
         "after": pathlib.Path("/tmp/chrimp-opt-round025-liveness-certificate")}
summary = []
cancel = "--cancel" in sys.argv[2:]
for n in ((128, 4096) if cancel else (128, 1024, 4096)):
    for changed in ((1, 8, 65) if cancel else (0, 1, 8, 32, 65)):
        for side, root in roots.items():
            name = f"{n}-{changed}-{side}"
            cmd = ["timeout", "--kill-after=5s", "60s", "cargo", "test", "--offline",
                   "--release", "--features", "diagnostics", "--test", "liveness_certificate",
                   "sparse_dense_and_archive_certificate_lifecycle", "--", "--exact",
                   "--nocapture", "--test-threads=1"]
            env = dict(os.environ, CHRIMP_LIVENESS_CASE=f"{n}/{changed}" + ("/cancel" if cancel else ""))
            with (out / f"{name}.log").open("w") as log:
                result = subprocess.run(cmd, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
            # libtest prefixes the first stdout line with the test name.
            records = [json.loads(line.split("liveness_case=", 1)[1]) for line in
                       (out / f"{name}.log").read_text().splitlines() if "liveness_case=" in line]
            expected = ["prepared", "mutated", "canceled", "dropped"] if cancel else ["prepared", "mutated", "pruned", "retained", "repeated", "released", "dropped"]
            assert [r["phase"] for r in records] == expected
            row = {"n": n, "changed": changed, "cancel": cancel, "side": side, "command": cmd,
                   "env": {"CHRIMP_LIVENESS_CASE": env["CHRIMP_LIVENESS_CASE"]},
                   "cwd": str(root), "exit": result.returncode, "records": records}
            summary.append(row)
            (out / "summary.json").write_text(json.dumps(summary, indent=2))
            print(name, result.returncode, flush=True)
            if result.returncode: sys.exit(result.returncode)
