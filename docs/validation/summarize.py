from pathlib import Path
import csv,json,statistics as st,random,hashlib
import argparse
parser=argparse.ArgumentParser(description="Reconstruct the frozen comparison tables from compact CSV evidence.")
parser.add_argument('--input', type=Path, default=Path(__file__).resolve().parent/'results')
parser.add_argument('--output', type=Path, required=True)
args=parser.parse_args();p=args.input;output=args.output
output.mkdir(parents=True,exist_ok=True)
F=['init_us','runtime_output_us','engine_drop_us','output_drop_us']
P=['parse_us','prepare_us','translation_us','ast_drop_us','prepared_drop_us']
def read(f):return list(csv.DictReader((p/f).open()))
def group(rows):
 d={}
 for r in rows:d.setdefault((r['case'],int(r['n']),r['runner']),[]).append(r)
 return d
def total(r,fs):return sum(float(r[k]) for k in fs)
def ci(a):
 rng=random.Random(5321); v=sorted(st.median(rng.choices(a,k=len(a))) for _ in range(10000));return [v[250],v[9749]]
sections=[]; result={}
for name,file,prep,reps,count in [('Ordinary primary','ordinary/active.csv','ordinary/preparation.csv',21,630),('Ordinary secondary','ordinary/global.csv','ordinary/preparation.csv',7,315),('Search default','search/paired-default.csv','search/preparation-default.csv',21,756),('Search COW','search/paired-cow.csv','search/preparation-cow.csv',21,756)]:
 rows=read(file);assert len(rows)==count
 g=group(rows); pg=group(read(prep)); out={}
 for key,rs in g.items():
  assert len(rs)==reps and {int(r['rep']) for r in rs}==set(range(reps))
  case,n,runner=key
  if 'trace_enabled' in rs[0]:assert all(r['trace_enabled']=='false' for r in rs)
  else:
   answers=n if case=='common-wide' else 1 if case=='failure' else 2
   for r in rs:
    assert int(r['answers'])==answers
    assert all(int(r[f])==n*answers for f in ['facts','ports','query_bindings'])
    assert int(r['qualified_physical_source_apps'])==(2*n if runner=='current' or case in ['distinct','failure'] else 2*n*answers)
    assert int(r['admission_apps'])==(0 if runner=='current' else 1)
  pr=pg[(case,n,'current' if runner=='current' else 'older')];assert len(pr)==21
  byrep={int(r['rep']):r for r in rs}; pb={int(r['rep']):r for r in pr}
  reused=[total(byrep[i],F) for i in range(reps)]
  cold=[reused[i]+total(pb[i],P) for i in range(reps)]
  out['|'.join(map(str,key))]={'reuse_us':st.median(reused),'cold_charged_us':st.median(cold),'preparation_charge_us':st.median([total(r,P) for r in pr]),'phases_us':{f:st.median(float(r[f]) for r in rs) for f in F},'preparation_phases_us':{f:st.median(float(r[f]) for r in pr) for f in P},'ticks':sorted({int(r['ticks']) for r in rs}),'qualified_apps':sorted({int(r.get('qualified_apps',r.get('qualified_physical_source_apps'))) for r in rs})}
 lines=[f'## {name}', '',f'{reps} measured repetitions per cell, two warmups. Times in milliseconds; ratios current/older, below 1 favors current. Cold is the additive preparation-charge model described in REPORT.md.','', '| Regime | n | Control | Current reused ms | Older reused ms | Paired ratio [bootstrap 95%] | Current cold ms | Older cold ms |', '|---|---:|---|---:|---:|---:|---:|---:|']
 for case,n in dict.fromkeys((k[0],k[1]) for k in g):
  cur={int(r['rep']):r for r in g[(case,n,'current')]};c=out[f'{case}|{n}|current']
  for runner in ['old-active','old-global']:
   if (case,n,runner) not in g:continue
   old={int(r['rep']):r for r in g[(case,n,runner)]}; o=out[f'{case}|{n}|{runner}']; ratios=[total(cur[i],F)/total(old[i],F) for i in range(reps)]; band=ci(ratios)
   o['paired_current_ratio']=st.median(ratios);o['ratio_bootstrap95']=band
   lines.append(f"| {case} | {n} | {runner} | {c['reuse_us']/1000:.3f} | {o['reuse_us']/1000:.3f} | {st.median(ratios):.3f} [{band[0]:.3f}, {band[1]:.3f}] | {c['cold_charged_us']/1000:.3f} | {o['cold_charged_us']/1000:.3f} |")
 sections.append('\n'.join(lines));result[name]=out
work=read('ordinary/workchecks.csv');assert len(work)==45
for r in work:
 assert r['trace_enabled']==('false' if r['runner']=='current' else 'true')
 for file in ['ordinary/active.csv','ordinary/global.csv']:
  matching=[x for x in read(file) if all(x[k]==r[k] for k in ['case','n','runner'])]
  assert all(x['qualified_apps']==r['qualified_apps'] for x in matching)
(output/'TABLES.md').write_text('\n\n'.join(sections)+'\n')
(output/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
print('PASS: 2457 measured timing rows, 45 traced-reference rows, 1638 fresh preparation rows; counts, repetition coverage, tracing boundaries and search semantics validated.')
for name,out in result.items():
 print(name)
 for k,v in out.items():
  if '|128|' in k:print(k,round(v['reuse_us']/1000,3),v.get('paired_current_ratio'))
