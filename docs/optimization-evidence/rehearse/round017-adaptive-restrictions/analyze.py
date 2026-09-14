"""Check equal semantic obligations before comparing independent cost measures."""
import json
from pathlib import Path
import re
from statistics import median

HERE = Path(__file__).resolve().parent
NEW = {'inline_comparisons','partition_hash_requests','partition_promotions','promoted_entries',
       'shifted_inline_entries','partition_table_bytes','peak_partition_table_bytes','inline_partition_limit'}
def original(d): return {k:v for k,v in d.items() if k not in NEW}
def total(d, field): return sum(p[field] for p in d['phases'])
summary = {}
paired_count = 0
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
                    for key in ['retained_rows','retained_partitions','cached_prefix_plans','partition_table_bytes']:
                        assert data['shared_restrictions'][key] == 0
                    assert data['prefix_memo'][3:] == [0,0]
            samples.append(dict(result=result, checkpoints=cp, allocation=get('allocations')))
        paired.append(samples)
    for b,c in zip(*paired):
        paired_count += 1
        bw,cw = b['result'].get('work',{}),c['result'].get('work',{})
        if isinstance(bw,dict):
            for key in ['answers','applications','scalars']: assert bw.get(key)==cw.get(key),(point.name,key)
        else: assert bw==cw
        for key in ['first_complete_answer','expected','prepared_released','source_goal_reached','cleanup_done','cleanup_in_time']:
            assert b['result'].get(key)==c['result'].get(key),(point.name,key)
        for phase in b['checkpoints']:
            bp,cp = b['checkpoints'][phase],c['checkpoints'][phase]
            assert bp.get('work',{}).get('rules')==cp.get('work',{}).get('rules'),(point.name,phase,'rules')
            assert original(bp['shared_restrictions'])==original(cp['shared_restrictions']),(point.name,phase,'restriction')
            for key in ['normalization','field_updates','prefix_lookup','prefix_memo','graph_allocations','store_mutations']:
                assert bp.get(key)==cp.get(key),(point.name,phase,key)
    values={}
    for side,samples in zip(['baseline','candidate'],paired):
        med=lambda f: median(f(s) for s in samples)
        values[side]=dict(peak=med(lambda s:s['allocation']['process_peak_requested_bytes']),
            final_live=med(lambda s:s['allocation']['process_live_requested_bytes']),
            allocated=med(lambda s:total(s['allocation'],'allocated_bytes')),
            calls=med(lambda s:total(s['allocation'],'allocations')),
            checkpoints=samples[0]['checkpoints'],result=samples[0]['result'],
            phases=samples[0]['allocation']['phases'],
            ranges={key:[min(f(s) for s in samples),max(f(s) for s in samples)] for key,f in [
                ('peak',lambda s:s['allocation']['process_peak_requested_bytes']),
                ('allocated',lambda s:total(s['allocation'],'allocated_bytes')),
                ('calls',lambda s:total(s['allocation'],'allocations'))]})
    b,c=values.values()
    values['percent']={key:100*(c[key]/b[key]-1) for key in ['peak','allocated','calls']}
    summary[point.name]=values
    print(point.name,{k:round(v,3) for k,v in values['percent'].items()})

def checkpoints(path):
    cases={}
    versions=9
    case=None
    for line in path.read_text().splitlines():
        m=re.search(r'prefix_churn versions=(\d+)',line)
        if m: versions=int(m[1])
        m=re.search(r'partition_case rows=(\d+) keys=(\d+)',line)
        if m: case='/'.join(m.groups())
        m=re.search(r'prefix_allocation n=(\d+) phase=(\w+) data=(.*)',line)
        if m:
            n,phase,data=m.groups()
            key=case or f'{n}/{versions}'
            cases.setdefault(key,{})[phase]=json.loads(data)
        m=re.search(r'(?:prefix_growth n=\d+ phase=(\w+) diagnostics=|partition_diagnostics data=)(\{.*?\})(?: graph_nodes=(\d+) graph_allocations=(\d+))?$',line)
        if m:
            cases[key][(m[1] or 'readback')+'_diagnostics']=json.loads(m[2])
            if m[3]: cases[key]['graph']=[int(m[3]),int(m[4])]
        m=re.search(r'(?:cleanup_ticks|partition_cleanup ticks)=(\d+)',line)
        if m: cases[key]['cleanup']=int(m[1])
        m=re.search(r'prefix_(lookup|memo) n=\d+ counts=(.*)',line)
        if m: cases[key][m[1]]=json.loads(m[2])
    return cases

probes={}
names=['growth','churn']+['isolated-'+c for c in ['1-1','2-2','3-3','4-4','5-5','8-8','16-16','64-64','128-128','128-1','128-2','128-4']]
for name in names:
    pair={s:checkpoints(HERE/'logs'/f'{s}-{name}.log') for s in ['baseline','candidate']}
    assert pair['baseline'].keys()==pair['candidate'].keys()
    for case in pair['baseline']:
        b,c=(pair[s][case] for s in ['baseline','candidate'])
        for key in ['cleanup','graph','lookup','memo']: assert b.get(key)==c.get(key),(name,case,key)
        for key in ['updated_diagnostics','readback_diagnostics']:
            if key in b: assert original(b[key])==original(c[key]),(name,case,key)
        values={}
        for side,rows in [('baseline',b),('candidate',c)]:
            start,end=rows['start'],rows['dropped']
            values[side]=dict(peak=end['process_peak_requested_bytes'],
                live=rows['readback']['process_live_requested_bytes']-start['process_live_requested_bytes'],
                final=end['process_live_requested_bytes']-start['process_live_requested_bytes'],
                calls=total(end,'allocations')-total(start,'allocations'),
                allocated=total(end,'allocated_bytes')-total(start,'allocated_bytes'))
        print(name,case,values)
        pair.setdefault('metrics',{})[case]=values
    probes[name]=pair
(HERE/'summary.json').write_text(json.dumps(dict(perf=summary,probes=probes,paired_samples=paired_count),indent=2)+'\n')
print(f'{paired_count} paired native samples and all isolated/growth/churn probes pass semantic/progress/release audit.')
