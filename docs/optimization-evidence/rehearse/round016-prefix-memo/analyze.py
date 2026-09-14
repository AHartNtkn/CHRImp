"""Audit equal obligations and summarize requested allocation and lookup work."""
import json
from pathlib import Path
import re
from statistics import median

HERE = Path(__file__).resolve().parent
summary = {}
for point in sorted((HERE / 'raw/baseline').iterdir()):
    paired = []
    for side in ['baseline', 'candidate']:
        samples = []
        for i in range(3):
            raw = json.loads((HERE / 'raw' / side / point.name / f'{i}.json').read_text())
            assert raw['status'] == 'completed' and raw['process']['group_cleanup_complete']
            get = lambda kind: next(r['data'] for r in raw['records'] if r['kind'] == kind)
            result = get('result')
            assert result['error'] is None
            cp = {r['data']['phase']: r['data'] for r in raw['records'] if r['kind'] == 'diagnostics'}
            for phase, data in cp.items():
                if phase == 'after_cancel':
                    for key in ['retained_rows', 'retained_partitions', 'cached_prefix_plans']:
                        assert data['shared_restrictions'][key] == 0
                    assert data['prefix_memo'][3:] == [0, 0]
            samples.append(dict(result=result, checkpoints=cp, allocation=get('allocations')))
        paired.append(samples)
    for b, c in zip(*paired):
        bw, cw = b['result'].get('work', {}), c['result'].get('work', {})
        if isinstance(bw, dict):
            for key in ['answers', 'applications', 'scalars']:
                assert bw.get(key) == cw.get(key), (point.name, key)
        else:
            assert bw == cw
        for key in ['first_complete_answer', 'expected', 'prepared_released', 'source_goal_reached', 'cleanup_done', 'cleanup_in_time']:
            assert b['result'].get(key) == c['result'].get(key), (point.name, key)
        for phase in b['checkpoints']:
            assert b['checkpoints'][phase].get('work', {}).get('rules') == c['checkpoints'][phase].get('work', {}).get('rules'), (point.name, phase, 'rules')
            for key in ['shared_restrictions', 'normalization', 'field_updates']:
                assert b['checkpoints'][phase].get(key) == c['checkpoints'][phase].get(key), (point.name, phase, key)
    values = {}
    for side, samples in zip(['baseline', 'candidate'], paired):
        med = lambda f: median(f(s) for s in samples)
        values[side] = dict(
            peak=med(lambda s: s['allocation']['process_peak_requested_bytes']),
            final_live=med(lambda s: s['allocation']['process_live_requested_bytes']),
            allocated=med(lambda s: sum(p['allocated_bytes'] for p in s['allocation']['phases'])),
            calls=med(lambda s: sum(p['allocations'] for p in s['allocation']['phases'])),
            checkpoints=samples[0]['checkpoints'], result=samples[0]['result'],
            phases=samples[0]['allocation']['phases'])
    b, c = values.values()
    values['percent'] = {key: 100 * (c[key] / b[key] - 1) for key in ['peak', 'allocated', 'calls']}
    summary[point.name] = values
    print(point.name, {k: round(v, 3) for k, v in values['percent'].items()},
          'lookup', b['checkpoints'].get('source', {}).get('prefix_lookup'), c['checkpoints'].get('source', {}).get('prefix_lookup'),
          'memo', c['checkpoints'].get('source', {}).get('prefix_memo'))

growth = {}
for probe in ['growth', 'churn']:
    growth[probe] = {}
    for side in ['baseline', 'candidate']:
        rows = {}
        versions = 9
        for line in (HERE / 'logs' / f'{side}-{probe}.log').read_text().splitlines():
            m = re.search(r'prefix_churn versions=(\d+)', line)
            if m: versions = int(m[1])
            m = re.search(r'prefix_allocation n=(\d+) phase=(\w+) data=(.*)', line)
            if m:
                n, phase, data = m.groups()
                rows.setdefault(f'{n}/{versions}', {})[phase] = json.loads(data)
        growth[probe][side] = rows
    for name in growth[probe]['baseline']:
        values = {}
        for side in ['baseline', 'candidate']:
            rows = growth[probe][side][name]
            values[side] = dict(peak=rows['dropped']['process_peak_requested_bytes'],
                retained=rows['readback']['process_live_requested_bytes'],
                final=rows['dropped']['process_live_requested_bytes'],
                allocated=sum(p['allocated_bytes'] for p in rows['dropped']['phases']) - sum(p['allocated_bytes'] for p in rows['start']['phases']),
                calls=sum(p['allocations'] for p in rows['dropped']['phases']) - sum(p['allocations'] for p in rows['start']['phases']))
        print(probe, name, values)
(HERE / 'summary.json').write_text(json.dumps(dict(perf=summary, growth=growth), indent=2) + '\n')
print('42 paired samples pass equal-result, exact-rule-work and final restriction/memo release audit; variable collection work retained.')
