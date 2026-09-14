"""Audit the maintained routine matrix; censored frontiers stay incomparable."""
import json
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent
reports={s:json.loads((HERE/'suite'/s/'suite.json').read_text()) for s in ['baseline','candidate']}
summary={}
completed=0
for point in reports['baseline']['points']:
    name=point['id']
    pairs=[]
    for i in range(3):
        pair=[]
        for side in ['baseline','candidate']:
            raw=json.loads((HERE/'suite'/side/name/f'{i}.json').read_text())
            assert raw['process']['group_cleanup_complete']
            get=lambda kind:next(r['data'] for r in raw['records'] if r['kind']==kind)
            pair.append(dict(status=raw['status'],result=get('result'),allocation=get('allocations')))
        b,c=pair
        assert b['status']==c['status'],name
        if b['status']=='completed':
            completed+=1
            for key in ['expected','first_complete_answer','cleanup_done','cleanup_in_time','source_goal_reached']:
                assert b['result'].get(key)==c['result'].get(key),(name,key)
            bw,cw=b['result'].get('work',{}),c['result'].get('work',{})
            if isinstance(bw,dict):
                for key in ['answers','applications','facts','ports','scalars']:
                    assert bw.get(key)==cw.get(key),(name,key)
            else: assert bw==cw,name
        else: assert b['status']=='censored',name
        pairs.append(pair)
    values={}
    for index,side in enumerate(['baseline','candidate']):
        rows=[pair[index] for pair in pairs]
        values[side]=dict(status=rows[0]['status'],peak=median(r['allocation']['process_peak_requested_bytes'] for r in rows),
            allocated=median(sum(p['allocated_bytes'] for p in r['allocation']['phases']) for r in rows),
            calls=median(sum(p['allocations'] for p in r['allocation']['phases']) for r in rows),
            final_live=median(r['allocation']['process_live_requested_bytes'] for r in rows),
            results=[r['result'] for r in rows])
    if values['baseline']['status']=='completed':
        values['percent']={k:100*(values['candidate'][k]/values['baseline'][k]-1) for k in ['peak','allocated','calls']}
    summary[name]=values
    print(name,values.get('percent','censored: no equivalent-work cost claim'))
(HERE/'suite-summary.json').write_text(json.dumps(dict(completed_pairs=completed,points=summary),indent=2)+'\n')
print(completed,'completed pairs audited; all six W frontiers retained without equivalence claim.')
