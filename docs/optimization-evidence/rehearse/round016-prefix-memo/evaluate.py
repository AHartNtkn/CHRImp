"""Use maintained native tests, perf runner and independent workload oracles."""
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = Path('/tmp/chrimp-round016-baseline-harness')

def run(label, args, root=ROOT):
    result = subprocess.run(args, cwd=root, text=True, capture_output=True)
    (HERE / 'logs').mkdir(exist_ok=True)
    (HERE / 'logs' / (label + '.log')).write_text(json.dumps(dict(command=args, cwd=str(root), exit=result.returncode)) + '\n' + result.stdout + result.stderr)
    print(label, result.returncode, flush=True)
    return result

def tests():
    for side, root in [('baseline', BASE), ('candidate', ROOT)]:
        artifacts = [json.loads(line) for line in Path('/tmp/round016-' + side + '-build.json').read_text().splitlines() if line.startswith('{')]
        binaries = [a for a in artifacts if a.get('reason') == 'compiler-artifact' and a.get('executable') and a['profile']['test']]
        for a in binaries:
            if side == 'candidate' or a['target']['name'] in ['shared_restrictions', 'store']:
                run(side + '-test-' + Path(a['executable']).name, ['timeout', '--kill-after=5s', '60s', a['executable']], root)
        binary = next(a['executable'] for a in binaries if a['target']['name'] == 'shared_restrictions')
        for label, name in [('growth', 'growing_prefix_versions_preserve_order_cutoffs_and_release'), ('churn', 'growing_prefix_cache_churn_preserves_answers_and_release')]:
            run(side + '-' + label, ['timeout', '--kill-after=5s', '60s', binary, '--exact', name, '--nocapture', '--test-threads=1'], root)

def perf():
    points = [
        *[(k, [f'notebook-behavior-{k}', '1']) for k in 'is'],
        *[(f'{kind}-{n}', [kind, str(n), '--rows', '8']) for kind in ['partial-join', 'partial-join-hit'] for n in [64, 256, 1024]],
        ('wide', ['wide-rewrite', '16', '--rows', '64']),
        ('archive', ['life-archive-rotate-conditional', '4', '--rows', '4', '--work', '32', '--cadence', '4']),
        ('inspect', ['life-inspections-conditional', '4', '--rows', '4', '--work', '32']),
        ('history', ['life-history-choice', '64']),
        ('runtime', ['runtime-sessions', '4', '--rows', '4', '--batch', '4']),
        ('fair', ['fair-grow', '4', '--rows', '8']),
    ]
    for name, args in points:
        for side, root in [('baseline', BASE), ('candidate', ROOT)]:
            run(side + '-perf-' + name, [sys.executable, str(ROOT / 'examples/perf.py'), '--binary', str(root / 'target/release/examples/measure'), '--repeat', '3', '--warmup', '0', '--out', str(HERE / 'raw' / side / name), '--', *args[:2], '50000000', '5', *args[2:], '--detail'])

if __name__ == '__main__':
    {'tests': tests, 'perf': perf}[sys.argv[1]]()
