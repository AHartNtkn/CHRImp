#!/usr/bin/env python3
"""Matched final runs; frontier limit changes stopping only, never engine work."""
import os
import sys
from evaluate import run

baseline, candidate, mode = sys.argv[1:]
points = [
    *[(k, [f'notebook-behavior-{k}', '1']) for k in 'isbcwtm'],
    *[(f'archive-{n}', ['life-archive-rotate-conditional', str(n), '--rows', '4', '--work', '32', '--cadence', '4']) for n in (1, 4, 16)],
    ('inspect', ['life-inspections-conditional', '4', '--rows', '4', '--work', '32']),
    ('held', ['life-held-output', '2', '--rows', '3', '--work', '24']),
    ('history', ['life-history-choice', '64']),
    ('runtime', ['runtime-sessions', '4', '--rows', '4', '--batch', '4']),
    ('answers', ['answers', '256', '--rows', '0']),
    ('fair', ['fair-grow', '4', '--rows', '8']),
    ('rewrite', ['rewrite', '512']),
    *[(f'bits-{n}', ['bits-chain', str(n)]) for n in (16, 64, 128)],
]
if mode == 'ordinary':
    points = [p for p in points if p[0] in ('i', 's', 'archive-4', 'answers', 'rewrite')]
for name, args in points:
    if mode != 'ordinary' and len(name) == 1 and name in 'bcwtm':
        os.environ['CHRIMP_MEASURE_SOURCE_DISPATCHES'] = '500000'
    else:
        os.environ.pop('CHRIMP_MEASURE_SOURCE_DISPATCHES', None)
    args = [*args[:2], '50000000', '3', *args[2:]]
    if mode != 'ordinary':
        args.append('--detail')
    for side, binary in [('baseline', baseline), ('candidate', candidate)]:
        run(f'{mode}-{side}', name, binary, args)
