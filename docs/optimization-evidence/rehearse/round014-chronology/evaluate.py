#!/usr/bin/env python3
"""Round 014 orchestration of maintained perf.py; all raw probes are retained."""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]

def run(label, name, binary, args):
    out = HERE / 'raw' / label / name
    command = [sys.executable, str(ROOT / 'examples/perf.py'), '--binary', binary,
               '--repeat', '3', '--warmup', '0', '--out', str(out), '--', *args]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    out.mkdir(parents=True, exist_ok=True)
    (out / 'runner.log').write_text(result.stdout + result.stderr)
    print(label, name, result.returncode, flush=True)
    return out

def load(path):
    raw = json.loads(path.read_text())
    records = raw['records']
    return {
        'status': raw['status'],
        'process': raw.get('process', {}),
        'phases': [r['data'] for r in records if r['kind'] == 'phase'],
        'result': next((r['data'] for r in records if r['kind'] == 'result'), {}),
        'allocations': next((r['data'] for r in records if r['kind'] == 'allocations'), {}),
        'checkpoints': {r['data']['phase']: r['data'] for r in records if r['kind'] == 'diagnostics'},
    }

def frontier(sample):
    w = sample['checkpoints']['source']['work']
    return ({k: v for k, v in w['dispatch'].items() if k != 'collection'}, w['rules'])

if __name__ == '__main__':
    baseline, candidate = sys.argv[1:]
    points = [
        *[(k, [f'notebook-behavior-{k}', '1', '50000000', '3', '--detail']) for k in 'is'],
        *[(k, [f'notebook-behavior-{k}', '1', '1000000', '3', '--detail']) for k in 'bcwtm'],
        ('archive', ['life-archive-rotate-conditional', '4', '50000000', '3', '--rows', '4', '--work', '32', '--cadence', '4', '--detail']),
        ('inspect', ['life-inspections-conditional', '4', '50000000', '3', '--rows', '4', '--work', '32', '--detail']),
        ('held', ['life-held-output', '2', '50000000', '3', '--rows', '3', '--work', '24', '--detail']),
        ('history', ['life-history-choice', '64', '50000000', '3', '--detail']),
        ('runtime', ['runtime-sessions', '4', '50000000', '3', '--rows', '4', '--batch', '4', '--detail']),
        ('answers', ['answers', '256', '50000000', '3', '--rows', '0', '--detail']),
        ('fair', ['fair-grow', '4', '50000000', '3', '--rows', '8', '--detail']),
        ('rewrite', ['rewrite', '512', '50000000', '3', '--detail']),
        *[(f'bits-{n}', ['bits-chain', str(n), '50000000', '3', '--detail']) for n in (16, 64, 128)],
    ]
    for name, args in points:
        bdir = run('baseline', name, baseline, args)
        cdir = run('candidate', name, candidate, args)
        if name not in 'bcwtm' or len(name) != 1:
            continue
        b = load(bdir / '0.json')
        c = load(cdir / '0.json')
        budget = int(args[2])
        attempts = []
        for attempt in range(16):
            bf, cf = frontier(b), frontier(c)
            if bf == cf:
                break
            budget += sum(bf[0].values()) - sum(cf[0].values())
            aligned = args.copy()
            aligned[2] = str(budget)
            cdir = run('alignment', f'{name}-{attempt}', candidate, aligned)
            samples = [load(cdir / f'{i}.json') for i in range(3)]
            c = samples[0]
            attempts.append({'budget': budget, 'path': str(cdir.relative_to(HERE)),
                             'matches': [frontier(s) == bf for s in samples]})
        (HERE / f'alignment-{name}.json').write_text(json.dumps(attempts, indent=2) + '\n')
