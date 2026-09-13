"""Capture a scoped validation/build command with immutable source metadata."""
import datetime
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent / sys.argv[1]
OUT.mkdir()
command = sys.argv[2:]
meta = dict(command=command, cwd=str(ROOT), timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat(),
            source=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            dirty=subprocess.check_output(['git', 'status', '--short'], cwd=ROOT, text=True))
with (OUT / 'stdout.log').open('w') as stdout, (OUT / 'stderr.log').open('w') as stderr:
    result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
meta['exit_code'] = result.returncode
(OUT / 'run.json').write_text(json.dumps(meta, indent=2) + '\n')
print(json.dumps(meta))
print((OUT / 'stderr.log').read_text()[-5000:])
print((OUT / 'stdout.log').read_text()[-5000:])
sys.exit(result.returncode)
