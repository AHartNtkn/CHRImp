"""Retain commands and raw results from maintained tests and perf runner."""
import json
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = Path('/tmp/chrimp-round017-baseline-harness')

def run(label, args, root=ROOT):
    result = subprocess.run(args, cwd=root, text=True, capture_output=True)
    (HERE / 'logs').mkdir(exist_ok=True)
    (HERE / 'logs' / (label + '.log')).write_text(json.dumps(dict(command=args, cwd=str(root), exit=result.returncode)) + '\n' + result.stdout + result.stderr)
    print(label, result.returncode, flush=True)
    return result

def binaries(side):
    artifacts = [json.loads(line) for line in Path('/tmp/round017-' + side + '-build.json').read_text().splitlines() if line.startswith('{')]
    return [a for a in artifacts if a.get('reason') == 'compiler-artifact' and a.get('executable') and a['profile']['test']]

def probes(side, root=ROOT):
    binary = next(a['executable'] for a in binaries(side) if a['target']['name'] == 'shared_restrictions')
    for label, name in [('threshold', 'partition_table_threshold_matrix'), ('growth', 'growing_prefix_versions_preserve_order_cutoffs_and_release'), ('churn', 'growing_prefix_cache_churn_preserves_answers_and_release')]:
        run(side + '-' + label, ['timeout', '--kill-after=5s', '60s', binary, '--exact', name, '--nocapture', '--test-threads=1'], root)

def tests():
    for side, root in [('baseline', BASE), ('candidate', ROOT)]:
        for a in binaries(side):
            if side == 'candidate' or a['target']['name'] in ['shared_restrictions', 'store']:
                run(side + '-test-' + Path(a['executable']).name, ['timeout', '--kill-after=5s', '60s', a['executable']], root)
        probes(side, root)
    binary = next(a['executable'] for a in binaries('candidate') if a['target']['name'] == 'shared_restrictions')
    run('candidate-resource', ['timeout', '--kill-after=5s', '60s', binary, '--exact', 'tiny_partition_avoids_hash_allocation'], ROOT)

def isolated():
    for side, root in [('baseline', BASE), ('candidate', ROOT)]:
        binary = next(a['executable'] for a in binaries(side) if a['target']['name'] == 'shared_restrictions')
        for case in ['1/1','2/2','3/3','4/4','5/5','8/8','16/16','64/64','128/128','128/1','128/2','128/4']:
            run(side + '-isolated-' + case.replace('/', '-'), ['env', 'CHRIMP_PARTITION_CASE=' + case, 'timeout', '--kill-after=5s', '60s', binary, '--exact', 'partition_table_threshold_matrix', '--nocapture', '--test-threads=1'], root)

def perf():
    points = [
        *[(k, [f'notebook-behavior-{k}', '1']) for k in 'is'],
        *[(f'{kind}-{n}', [kind, str(n), '--rows', '8']) for kind in ['partial-join', 'partial-join-hit'] for n in [1, 2, 4, 8, 64, 256, 1024]],
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
    {'tests': tests, 'perf': perf, 'isolated': isolated, 'probes': lambda: probes(sys.argv[2])}[sys.argv[1]]()
