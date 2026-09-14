#!/usr/bin/env python3
"""Audit endpoints; this supplements the unchanged workload-specific oracles."""
import collections
import json
from evaluate import HERE

audits = json.loads((HERE / 'audit.json').read_text())
frontiers = collections.Counter()
for a in audits:
    for phase, p in a['phases'].items():
        assert p['rules_equal'] and p['dispatch_equal'], (a['case'], a['sample'], phase)
    if a['case'] in [f'matched/{k}' for k in 'bcwtm']:
        for r, checkpoint in zip(a['results'], a['checkpoints']):
            assert r['source_dispatch_limit'] == 500000
            assert r['censor_reason'] == 'source_dispatches'
            assert r['work']['answers'] == r['work']['scalars'] == 0
            assert r['cleanup_done'] and r['cleanup_in_time'] and r['prepared_released']
            d = checkpoint['source']['work']['dispatch']
            assert sum(v for k, v in d.items() if k != 'collection') == 500000
        frontiers[a['case']] += 1
    for checkpoints in a['checkpoints']:
        if 'after_cancel' in checkpoints:
            d = checkpoints['after_cancel']
            if not d['memory_counts']['snapshots']:
                assert d['memory_counts']['conditions'] == 0
                if 'condition_slab' in d:
                    assert d['condition_slab'][:5] == [0, 0, 0, 0, 0]
                    assert d['condition_slab'][8:10] == [0, 0]
            else:
                assert a['case'] == 'matched/history'
                assert d['memory_counts']['conditions'] == 1
    for phases in a['lifecycle_phases']:
        for p in phases:
            if p['phase'] in ('cancel', 'release'):
                assert p['complete']
            if p['phase'] == 'release':
                assert p['memory']['conditions'] == p['memory']['snapshots'] == 0
            if p['phase'] == 'interaction_resumed' and p['data'].get('projection_only'):
                assert p['data']['source_applications_during_projection'] == 0
assert dict(frontiers) == {f'matched/{k}': 3 for k in 'bcwtm'}
counts = collections.Counter()
for f in sorted((HERE / 'raw').glob('*/*/[0-9]*.json')):
    s = json.loads(f.read_text())
    counts[s['status']] += 1
    assert s['status'] in ('completed', 'censored'), f
    assert s['process']['group_cleanup_complete'], f
print('PASS: all paired rule/dispatch vectors, five aligned synthesis frontiers,')
print('projection-only progress, history release, and zero unowned post-cancel slab backing.')
print('Native observations:', dict(counts))
