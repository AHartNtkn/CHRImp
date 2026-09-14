"""Check raw equivalent progress and cleanup; keep timings out of acceptance."""
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
audits = []
for point in sorted((HERE / 'raw/baseline').iterdir()):
    for i in range(3):
        sides = []
        for side in ('baseline','candidate'):
            raw = json.loads((HERE / 'raw' / side / point.name / f'{i}.json').read_text())
            assert raw['status'] == 'completed'
            assert raw['process']['group_cleanup_complete']
            result = next(r['data'] for r in raw['records'] if r['kind']=='result')
            assert result['error'] is None
            cps = {r['data']['phase']: r['data'] for r in raw['records'] if r['kind']=='diagnostics'}
            for phase, cp in cps.items():
                if phase == 'after_cancel':
                    assert cp['shared_restrictions']['retained_rows'] == 0
                    assert cp['shared_restrictions']['retained_partitions'] == 0
                    assert cp['shared_restrictions'].get('cached_prefix_plans', 0) == 0
            sides.append((result,cps))
        b, c = sides
        bwork, cwork = b[0].get('work', {}), c[0].get('work', {})
        if not isinstance(bwork, dict):
            assert bwork == cwork
            bwork = cwork = {}
        for key in ['answers','applications','scalars']:
            assert bwork.get(key) == cwork.get(key), (point.name,key)
        for key in ['first_complete_answer','expected','prepared_released','source_goal_reached','cleanup_done','cleanup_in_time']:
            assert b[0].get(key) == c[0].get(key), (point.name,key)
        bw, cw = (s[1].get('source', {}).get('work', {}) for s in sides)
        # Matching dispatches include the new split/subscription transitions;
        # compare completed per-rule obligations, and retain those extra costs.
        obligations = lambda w: [{k:v for k,v in rule.items() if k != 'matching_dispatches'} for rule in w.get('rules', [])]
        assert obligations(bw) == obligations(cw), point.name
        audits.append({'point': point.name, 'sample': i,
            'same_rule_obligations': True,
            'matching_dispatch_delta': sum(r['matching_dispatches'] for r in cw.get('rules', [])) - sum(r['matching_dispatches'] for r in bw.get('rules', [])),
            'baseline_dispatch': bw.get('dispatch'),
            'candidate_dispatch': cw.get('dispatch')})
(HERE / 'audit.json').write_text(json.dumps(audits, indent=2)+'\n')
print(f'{len(audits)} paired samples: native outcomes, rule vectors and restriction reclamation checked')
