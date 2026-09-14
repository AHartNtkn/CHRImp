"""Run already-built test artifacts, with an individual hard limit per binary."""
import json
import pathlib
import subprocess
import sys

artifacts, log = map(pathlib.Path, sys.argv[1:])
failed = []
seen = set()
with log.open('w') as out:
    for line in artifacts.read_text().splitlines():
        record = json.loads(line)
        binary = record.get('executable')
        if not binary or not record.get('profile', {}).get('test') or binary in seen:
            continue
        seen.add(binary)
        command = ['timeout', '--kill-after=5s', '60s', binary]
        out.write(' '.join(command) + '\n')
        out.flush()
        result = subprocess.run(command, stdout=out, stderr=subprocess.STDOUT)
        print(pathlib.Path(binary).name, result.returncode, flush=True)
        if result.returncode:
            failed.append([binary, result.returncode])
    out.write(f'Binaries={len(seen)} failures={failed}\n')
assert not failed, failed
