#!/usr/bin/env python3
"""Preserve paired measurements, semantic frontiers and phase-specific costs."""
import json
import statistics
from evaluate import HERE, load

def metrics(s):
    a = s['allocations']
    result = {
        'allocations': sum(p['allocations'] for p in a.get('phases', [])),
        'allocated_bytes': sum(p['allocated_bytes'] for p in a.get('phases', [])),
        'peak_bytes': a.get('process_peak_requested_bytes'),
        'after_drop_bytes': a.get('process_live_requested_bytes'),
    }
    for p in a.get('phases', []):
        for k in ('allocations', 'allocated_bytes', 'deallocations', 'freed_bytes'):
            result[f'{p["phase"]}.{k}'] = p[k]
    for phase, d in s['checkpoints'].items():
        result[f'{phase}.live_bytes'] = d['allocation']['process_live_requested_bytes']
        result[f'{phase}.peak_bytes'] = d['allocation']['process_peak_requested_bytes']
        if 'condition_slab' in d:
            for k, v in zip(('live', 'vacant', 'length', 'capacity', 'backing_bytes', 'accesses', 'moves', 'constructions', 'directory_bytes', 'occupied_blocks', 'reclaimed_blocks', 'lookup_steps'), d['condition_slab']):
                result[f'{phase}.nodes.{k}'] = v
        if 'condition_tree' in d:
            result[f'{phase}.nodes.accesses'], result[f'{phase}.nodes.constructions'] = d['condition_tree']
        for k, v in d.get('memory_counts', {}).items():
            result[f'{phase}.memory.{k}'] = v
        result[f'{phase}.collection_dispatch'] = d['work']['dispatch']['collection']
    for k, v in s['result'].get('times_ms', {}).items():
        result['ms.' + k] = v
    for k in ('max_rss_kib', 'user_cpu_seconds', 'system_cpu_seconds', 'wall_seconds'):
        result['process.' + k] = s['process'].get(k)
    for p in s['phases']:
        for k in ('elapsed_ms', 'applications', 'ticks'):
            result[f'lifecycle.{p["phase"]}.{k}'] = p.get(k)
    return result

summary, audit = {}, []
for mode in ('matched', 'ordinary'):
    base = HERE / 'raw' / (mode + '-baseline')
    if not base.exists(): continue
    for case in sorted(base.iterdir()):
        cand = HERE / 'raw' / (mode + '-candidate') / case.name
        if not cand.exists(): continue
        groups = [[load(d / f'{i}.json') for i in range(3)] for d in (case, cand)]
        measured = [[metrics(s) for s in g] for g in groups]
        keys = set.intersection(*(set(m) for group in measured for m in group))
        m = {}
        for k in sorted(keys):
            values = [[s[k] for s in g] for g in measured]
            if any(v is None or not isinstance(v, (int, float)) for vs in values for v in vs): continue
            b, c = [statistics.median(vs) for vs in values]
            m[k] = {'baseline': b, 'candidate': c, 'percent': 100*(c/b-1) if b else None, 'samples': values}
        summary[f'{mode}/{case.name}'] = m
        for i, (b, c) in enumerate(zip(*groups)):
            phases = {}
            for p in set(b['checkpoints']) & set(c['checkpoints']):
                w = [s['checkpoints'][p]['work'] for s in (b,c)]
                d = [{k:v for k,v in x['dispatch'].items() if k != 'collection'} for x in w]
                phases[p] = {'rules_equal': w[0]['rules'] == w[1]['rules'], 'dispatch_equal': d[0] == d[1], 'dispatch': d}
            audit.append({'case': f'{mode}/{case.name}', 'sample': i, 'phases': phases,
                          'statuses': [s['status'] for s in (b,c)],
                          'results': [s['result'] for s in (b,c)],
                          'lifecycle_phases': [s['phases'] for s in (b,c)],
                          'checkpoints': [s['checkpoints'] for s in (b,c)]})
        print(mode, case.name, ' | '.join(f'{k}: {m[k]["baseline"]} -> {m[k]["candidate"]} ({m[k]["percent"]:.2f}%)' for k in ('allocations','allocated_bytes','peak_bytes','source.live_bytes') if k in m and m[k]['percent'] is not None))
(HERE / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
(HERE / 'audit.json').write_text(json.dumps(audit, indent=2) + '\n')
