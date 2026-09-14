"""Summarize whole intervals; peaks are cumulative within each probe process."""
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent

def read(side):
    cases = {}
    for line in (HERE / 'logs' / f'{side}-threshold.log').read_text().splitlines():
        m = re.search(r'partition_case rows=(\d+) keys=(\d+)', line)
        if m:
            case = '/'.join(m.groups())
            cases[case] = {}
        m = re.search(r'prefix_allocation n=\d+ phase=(\w+) data=(.*)', line)
        if m: cases[case][m[1]] = json.loads(m[2])
        m = re.search(r'partition_diagnostics data=(.*)', line)
        if m: cases[case]['diagnostics'] = json.loads(m[1])
        m = re.search(r'partition_cleanup ticks=(\d+)', line)
        if m: cases[case]['cleanup'] = int(m[1])
    return cases

def metrics(case):
    start, end = case['start'], case['dropped']
    return dict(live=case['readback']['process_live_requested_bytes'] - start['process_live_requested_bytes'],
        calls=sum(p['allocations'] for p in end['phases']) - sum(p['allocations'] for p in start['phases']),
        allocated=sum(p['allocated_bytes'] for p in end['phases']) - sum(p['allocated_bytes'] for p in start['phases']),
        table=case['diagnostics']['partition_table_bytes'],
        hashes=case['diagnostics']['partition_hash_requests'],
        comparisons=case['diagnostics']['inline_comparisons'])

if __name__ == '__main__':
    sides = {s: read(s) for s in ['baseline','threshold1','threshold2','threshold4']}
    for case in sides['baseline']:
        print(case, {s: metrics(cases[case]) for s,cases in sides.items()})
    (HERE / 'thresholds.json').write_text(json.dumps(sides, indent=2) + '\n')
