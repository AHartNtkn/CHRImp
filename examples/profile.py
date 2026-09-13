#!/usr/bin/env python3
"""Build and sample a native CHR workload, retaining an interactive CPU flame graph.

python3 examples/profile.py --out /tmp/chr-profile -- notebook-behavior-i 1 50000000 15
python3 examples/profile.py --cli --out /tmp/chr-query -- examples/proofs.chr --query '...'
Requires Linux perf, GNU c++filt, inferno-collapse-perf and inferno-flamegraph.
"""
import argparse
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
from urllib.parse import quote
from supervise import run as supervise

ROOT = Path(__file__).resolve().parents[1]


def normalize_stacks(text):
    # Inferno's perf parser requires a whitespace-free DSO token. Keep raw stacks
    # separately; encode only DSO paths, never symbol text or sample headers.
    text = re.sub(r" \(([^\n]*)\)$", lambda m: " (" + quote(m[1], safe="/._-[]") + ")", text, flags=re.M)
    # GNU's Rust demangler cannot parse LLVM's internal cloning suffix.
    return re.sub(r"(?<=\w)\.llvm\.\d+(?=[+ ]|$)", "", text, flags=re.M)


def folded_weight(text):
    total = 0
    for line in text.splitlines():
        stack, weight = line.rsplit(" ", 1)
        weight = int(weight)
        if not stack or weight <= 0:
            raise ValueError("invalid folded stack weight")
        total += weight
    if not total:
        raise ValueError("no CPU samples: use a longer workload; no flame graph was validated")
    return total


def validate_samples(raw, folded):
    samples = folded_weight(folded)
    observed = len(re.findall(r"^[^\s].* cpu-clock:u:\s*$", raw, flags=re.M))
    if samples != observed:
        raise ValueError(f"sample accounting mismatch: perf={observed}, folded={samples}")
    return samples


def hardware_counters(text):
    result=[]
    for line in text.splitlines():
        if not line.strip(): continue
        row=json.loads(line)
        event=row['event'];raw=row['counter-value']
        if not isinstance(event,str): raise ValueError('invalid counter event')
        if raw in ('<not counted>','<not supported>'):
            value=None;status=raw[1:-1].replace(' ','_')
        else:
            value=float(raw);status='measured'
            if not math.isfinite(value) or value<0: raise ValueError('invalid counter value')
        runtime=float(row['event-runtime']);running=float(row['pcnt-running'])
        if not math.isfinite(runtime) or runtime<0 or not math.isfinite(running) or not 0<=running<=100:
            raise ValueError('invalid counter exposure')
        result.append(dict(event=event,value=value,status=status,unit=row['unit'],event_runtime_ns=runtime,percent_running=running))
    if not result: raise ValueError('no hardware counter records')
    return result


def capture(command, **kwargs):
    return subprocess.run(command, cwd=ROOT, check=True, text=True, capture_output=True, timeout=60, **kwargs).stdout


def outcome(resources, cli):
    code=resources['returncode'];limited=resources['limit_reason'] is not None
    if not resources['group_cleanup_complete'] or (not limited and resources['descendants_signaled_after_exit']):
        return "failed"
    if limited or (code == 2 and not cli):
        return "censored"
    return "completed" if code == 0 else "failed"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, type=Path, help="new directory for raw profile, metadata and flame.svg")
    parser.add_argument("--counters", action="store_true", help="collect instruction/cycle/cache/branch counters instead of sampled stacks")
    parser.add_argument("--cli", action="store_true", help="profile the language CLI instead of the measure example")
    parser.add_argument("--seconds", type=float, default=40, help="external wall limit including workload cleanup")
    parser.add_argument("--memory-mib", type=int, default=4096, help="per-process address-space ceiling")
    parser.add_argument("--frequency", type=int, default=499, help="user-space CPU samples per second")
    parser.add_argument("args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    workload = args.args[1:] if args.args[:1] == ["--"] else args.args
    if not workload or (not math.isfinite(args.seconds) or args.seconds <= 0) or args.memory_mib <= 0 or args.frequency <= 0:
        parser.error("a workload and positive limits are required")
    required=["cargo","rustc","perf"] + ([] if args.counters else ["c++filt","inferno-collapse-perf","inferno-flamegraph"])
    for tool in required:
        if not shutil.which(tool):
            parser.error(f"required executable is unavailable: {tool}")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C force-frame-pointers=yes").strip()
    build = ["cargo", "build", "--offline", "--profile", "profiling", *( ["--bin", "chr"] if args.cli else ["--example", "measure"] )]
    with (out / "build.log").open("w") as log:
        subprocess.run(build, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=300)
    target = Path(env.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target.is_absolute():
        target = ROOT / target
    binary = target / "profiling" / ("chr" if args.cli else "examples/measure")
    command = ["perf", "record", "-q", "-e", "cpu-clock:u", "-F", str(args.frequency), "--call-graph", "fp", "-o", str(out / "perf.data"), "--", str(binary), *workload]
    if args.counters:
        command=["perf","stat","-j","-e","cycles:u,instructions:u,cache-misses:u,branches:u,branch-misses:u","-o",str(out/"counters.jsonl"),"--",str(binary),*workload]
    metadata = {
        "schema": 1, "kind": "cpu_flamegraph", "command": command, "build": build,
        "rustflags": env["RUSTFLAGS"],
        "revision": capture(["git", "rev-parse", "HEAD"]).strip(),
        "working_tree": capture(["git", "status", "--short"]),
        "rustc": capture(["rustc", "-Vv"]), "host": platform.platform(),
        "external_wall_seconds": args.seconds, "memory_mib": args.memory_mib,
        "measurement": "Sampled user-space CPU call stacks across the entire process, including preparation, validation, delivery and cleanup; not baseline wall timing or kernel/I/O wait time.",
        "status": "recording", "profile_validated": False,
    }
    if args.counters:
        metadata.update(kind="hardware_counters",measurement="User-space hardware event counts for the native workload, including preparation, validation and cleanup. Keep PMU domains separate; values may be perf-scaled. Runtime/running percentage expose scheduling and multiplexing. Not logical engine work or a sum across heterogeneous PMUs.")
    meta = out / "profile.json"
    meta.write_text(json.dumps(metadata, indent=2) + "\n")
    try:
        with (out / "workload.log").open("w") as stdout, (out / "perf.log").open("w") as stderr:
            resources = supervise(command, ROOT, stdout, stderr, args.seconds, args.memory_mib, grace=5)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        metadata.update(status="supervisor_error", error=str(error))
        meta.write_text(json.dumps(metadata, indent=2) + "\n")
        raise
    code = resources["returncode"]
    limited = resources["limit_reason"] is not None
    status = outcome(resources, args.cli)
    metadata.update(returncode=code, external_limit=limited, elapsed_seconds=resources["wall_seconds"],
                    status=status, process_resources=resources)
    meta.write_text(json.dumps(metadata, indent=2) + "\n")
    if metadata["status"] == "failed":
        raise RuntimeError(f"profiling/workload failed with status {code}; see {out / 'perf.log'} and workload.log")
    if args.counters:
        try: counters=hardware_counters((out/"counters.jsonl").read_text())
        except (ValueError,KeyError,TypeError) as error:
            metadata['counter_error']=str(error);meta.write_text(json.dumps(metadata,indent=2)+'\n');raise
        available=any(c['value'] is not None for c in counters)
        metadata.update(counters=counters,counters_available=available,profile_validated=True)
        meta.write_text(json.dumps(metadata,indent=2)+'\n')
        print(json.dumps(dict(status=metadata['status'],counters_available=available,metadata=str(meta))))
        return 2 if metadata['status']=='censored' or not available else 0
    raw = capture(["perf", "script", "-i", str(out / "perf.data"), "-F", "comm,pid,tid,time,event,ip,sym,dso"])
    (out / "stacks.txt").write_text(raw)
    readable = capture(["c++filt", "-s", "rust", "-i"], input=normalize_stacks(raw))
    collapsed = subprocess.run(["inferno-collapse-perf"], input=readable, text=True, capture_output=True, check=True, timeout=60)
    (out / "collapse.log").write_text(collapsed.stderr)
    if "Weird stack line" in collapsed.stderr:
        raise RuntimeError("stack parsing lost frames; see collapse.log")
    samples = validate_samples(raw, collapsed.stdout)
    (out / "stacks.folded").write_text(collapsed.stdout)
    svg = capture(["inferno-flamegraph", "--title", "CHR CPU profile", "--subtitle", " ".join(workload), "--countname", "samples", "--colors", "rust", "--deterministic"], input=collapsed.stdout)
    import xml.etree.ElementTree as ET
    if ET.fromstring(svg).tag != "{http://www.w3.org/2000/svg}svg":
        raise ValueError("flame graph renderer did not produce SVG")
    (out / "flame.svg").write_text(svg)
    metadata.update(samples=samples, flamegraph="flame.svg", profile_validated=True,
                    uncertainty="Sampling uncertainty applies; short runs with few samples cannot support precise percentage comparisons.")
    meta.write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"status": metadata["status"], "samples": samples, "flamegraph": str(out / "flame.svg"), "metadata": str(meta)}))
    return 2 if metadata["status"] == "censored" else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
