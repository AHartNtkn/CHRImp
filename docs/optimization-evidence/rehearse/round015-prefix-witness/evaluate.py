#!/usr/bin/env python3
"""Round 015 uses maintained perf.py and native semantic/resource oracles."""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = pathlib.Path('/tmp/chrimp-round015-baseline-harness')

def command(label, args, cwd=ROOT):
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True)
    (HERE / 'logs').mkdir(exist_ok=True)
    (HERE / 'logs' / (label + '.log')).write_text(
        json.dumps({'command': args, 'cwd': str(cwd), 'returncode': result.returncode})
        + '\n' + result.stdout + result.stderr)
    print(label, result.returncode, flush=True)
    return result

def tests(side, root):
    result = command(side + '-test-build', ['cargo', 'test', '--offline', '--release',
        '--features', 'diagnostics', '--no-run', '--message-format=json'], root)
    if result.returncode: raise RuntimeError(result.stderr)
    artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    for a in artifacts:
        if a.get('reason') != 'compiler-artifact' or not a.get('executable') or not a['profile']['test']:
            continue
        name = pathlib.Path(a['executable']).name
        command(side + '-test-' + name, ['timeout', '--kill-after=5s', '60s', a['executable']], root)
    binary = next(a['executable'] for a in artifacts if a.get('target', {}).get('name') == 'shared_restrictions' and a.get('executable'))
    command(side + '-growth', ['timeout', '--kill-after=5s', '60s', binary,
        '--exact', 'growing_prefix_versions_preserve_order_cutoffs_and_release', '--nocapture', '--test-threads=1'], root)

def perf():
    points = [
        *[(k, [f'notebook-behavior-{k}', '1']) for k in 'is'],
        *[(f'partial-{kind}-{n}', [kind, str(n), '--rows', '8'])
          for kind in ('partial-join', 'partial-join-hit') for n in (64, 256, 1024)],
        ('wide', ['wide-rewrite', '16', '--rows', '64']),
        ('archive', ['life-archive-rotate-conditional', '4', '--rows', '4', '--work', '32', '--cadence', '4']),
        ('inspect', ['life-inspections-conditional', '4', '--rows', '4', '--work', '32']),
        ('history', ['life-history-choice', '64']),
        ('runtime', ['runtime-sessions', '4', '--rows', '4', '--batch', '4']),
        ('fair', ['fair-grow', '4', '--rows', '8']),
    ]
    for name, args in points:
        for side, root in [('baseline', BASE), ('candidate', ROOT)]:
            out = HERE / 'raw' / side / name
            command(side + '-perf-' + name, [sys.executable, str(ROOT / 'examples/perf.py'),
                '--binary', str(root / 'target/release/examples/measure'), '--repeat', '3',
                '--warmup', '0', '--out', str(out), '--',
                *args[:2], '50000000', '5', *args[2:], '--detail'])

if __name__ == '__main__':
    mode = sys.argv[1]
    if mode == 'tests':
        for side, root in [('baseline', BASE), ('candidate', ROOT)]: tests(side, root)
    elif mode == 'perf': perf()
