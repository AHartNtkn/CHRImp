import json
import pathlib
import re
import statistics

HERE = pathlib.Path(__file__).resolve().parent

def sample(path):
    raw = json.loads(path.read_text())
    records = raw['records']
    get = lambda kind: next(r['data'] for r in records if r['kind'] == kind)
    checkpoints = {r['data']['phase']: r['data'] for r in records if r['kind'] == 'diagnostics'}
    allocations = get('allocations')
    return dict(status=raw['status'], process=raw['process'], result=get('result'),
                allocations=allocations, checkpoints=checkpoints)

summary = {}
for path in sorted((HERE / 'raw/baseline').iterdir()):
    pair = {}
    for side in ['baseline', 'candidate']:
        samples = [sample(HERE / 'raw' / side / path.name / f'{i}.json') for i in range(3)]
        assert all(s['status'] == 'completed' and s['process']['group_cleanup_complete'] for s in samples)
        med = lambda f: statistics.median(f(s) for s in samples)
        pair[side] = {
            'peak': med(lambda s: s['allocations']['process_peak_requested_bytes']),
            'final_live': med(lambda s: s['allocations']['process_live_requested_bytes']),
            'allocated': med(lambda s: sum(p['allocated_bytes'] for p in s['allocations']['phases'])),
            'allocation_calls': med(lambda s: sum(p['allocations'] for p in s['allocations']['phases'])),
            'phases': {p['phase']: {key: med(lambda s, phase=p['phase'], key=key: next(q[key] for q in s['allocations']['phases'] if q['phase']==phase)) for key in ('allocations', 'allocated_bytes', 'freed_bytes')} for p in samples[0]['allocations']['phases']},
            'result': samples[0]['result'],
            'restrictions': {phase: cp.get('shared_restrictions') for phase, cp in samples[0]['checkpoints'].items()},
            'source_live': med(lambda s: s['checkpoints'].get('source', {}).get('allocation', {}).get('process_live_requested_bytes', 0)),
        }
        pair[side]['work_samples'] = [s['checkpoints'].get('source', {}).get('work') for s in samples]
    b, c = pair['baseline'], pair['candidate']
    pair['change_percent'] = {k: (c[k]/b[k]-1)*100 if b[k] else None for k in ['peak','allocated','allocation_calls','source_live']}
    summary[path.name] = pair
    print(path.name, {k: round(v, 2) if v is not None else None for k,v in pair['change_percent'].items()},
          'prefixes', (c['restrictions'].get('source') or {}).get('rebuilt_prefixes'))

growth = {}
for side in ['baseline','candidate']:
    rows = {}
    for line in (HERE / 'logs' / f'{side}-growth.log').read_text().splitlines():
        m = re.search(r'prefix_allocation n=(\d+) phase=(\w+) data=(.*)', line)
        if m:
            n, phase, data = m.groups()
            rows.setdefault(n, {})[phase] = json.loads(data)
    growth[side] = rows
for n in growth['baseline']:
    values = {}
    for side in growth:
        rows = growth[side][n]
        values[side] = {
            'updated_live': rows['updated']['process_live_requested_bytes'],
            'peak': rows['dropped']['process_peak_requested_bytes'],
            'final_live': rows['dropped']['process_live_requested_bytes'],
            'interval_allocated': sum(p['allocated_bytes'] for p in rows['dropped']['phases']) - sum(p['allocated_bytes'] for p in rows['start']['phases']),
            'interval_calls': sum(p['allocations'] for p in rows['dropped']['phases']) - sum(p['allocations'] for p in rows['start']['phases']),
        }
    print('growth', n, values)
(HERE / 'summary.json').write_text(json.dumps({'perf': summary, 'growth': growth}, indent=2)+'\n')
