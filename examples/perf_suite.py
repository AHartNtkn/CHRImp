#!/usr/bin/env python3
"""Bounded routine and deeper size/shape/lifetime exploration on native workloads.

Each point uses perf.py's existing validated samples and external supervision.
Scaling is descriptive evidence until calibrated growth/regression policy is supplied.
"""
import argparse
import json
import math
import os
from pathlib import Path
import random
import signal
import subprocess
import time
import perf
from perf_compare import read_json


def notebook_inventory():
    """Saved queries, with exact unrestricted synthesis cases distinguished from other coverage."""
    targets={'identity':'i','constant':'k','second argument':'ki','substitution':'s',
             'composition':'b','swap arguments':'c','duplicate argument':'w',
             'apply to a function':'t','self-application':'m'}
    result=[]
    for path in sorted((perf.ROOT/'examples').glob('*.chrnb')):
        document=read_json(path)
        for query in document['queries']:
            name=query['name'];case=None
            kind='saved_query'
            if path.stem in ('type-synthesis','behavior-synthesis') and name.startswith('Synthesize '):
                target=targets[name.removeprefix('Synthesize ')]
                case=f"notebook-{path.stem.split('-')[0]}-{target}"
                kind='unrestricted_synthesis'
            result.append(dict(notebook=str(path.relative_to(perf.ROOT)),id=query['id'],name=name,
                               kind=kind,case=case))
    return result


def plan(mode, seed):
    points=[]
    def sweep(name, case, axis, values, size=None, options=(), risk=''):
        for value in values:
            args=[case,str(value if axis=='size' else size),'50000000','3',*map(str,options)]
            if axis!='size': args += ['--'+axis,str(value)]
            points.append(dict(id=f'{name}-{value}',family=name,axis=axis,value=value,workload=args,risk=risk))
    sizes=[32,128] if mode=='routine' else [16,32,64,128]
    sweep('rewrite','rewrite','size',sizes,risk='linear productive rewriting and release')
    sweep('partial-miss','partial-join','size',sizes,options=('--rows',8),risk='partially bound correlation scans with fixed probes')
    sweep('partial-hit','partial-join-hit','size',sizes,options=('--rows',8),risk='successful correlation control')
    sweep('duplicates','duplicate-heads','size',[3,6] if mode=='routine' else [2,4,8,16],options=('--rows',2),risk='occurrence multiplicity and fresh witnesses')
    sweep('fresh-contract','fresh-contract','depth',[1,4] if mode=='routine' else [0,1,2,4,8],size=4,options=('--rows',4),risk='fresh production coupled to equality and consumption')
    sweep('fresh-control','fresh-unmerged','depth',[1,4],size=4,options=('--rows',4),risk='fresh identity and contraction control')
    sweep('proof-diamond','proof-dag','size',[4,7] if mode=='routine' else [4,7,10,13],options=('--shape','diamond'),risk='reconvergent derivation amplification')
    sweep('fairness','fair-grow','size',[1,4] if mode=='routine' else [1,2,4,8],options=('--rows',8),risk='finite answer latency competing with continuing growth')
    sweep('archive','life-archive-rotate','size',[1,4],options=('--rows',4,'--work',32,'--cadence',4),risk='retained ownership and turnover')
    sweep('sessions-history','runtime-sessions','closed',[0,8] if mode=='routine' else [0,4,16,64],size=4,options=('--rows',4,'--batch',4),risk='historical retired sessions versus fresh request cost')
    sweep('prepare-shape','prepare-reuse','heads',[1,4] if mode=='routine' else [1,2,4,8],size=32,options=('--arity',4,'--width',8,'--uses',4),risk='preparation shape independent of result size')
    for case in ('arithmetic-decompose','lambda','type-i','behavior-i','behavior-w'):
        sweep('notebook-'+case,'notebook-'+case,'size',[1],risk='end-to-end validated query or honest bounded prefix')
    if mode=='deep':
        for shape in ('chain','ring','star','random'):
            for variant in (seed, seed+1):
                sweep(f'bits-{shape}-seed{variant}','graph-bits','size',[4,8],options=('--shape',shape,'--seed',variant),risk='conditional graph topology and contradiction correlations')
                sweep(f'walks-{shape}-seed{variant}','graph-walks','size',[4,8,16],options=('--shape',shape,'--seed',variant,'--rows',2),risk='graph degree, duplicate edges and ordered matching')
        sweep('sessions-retained','runtime-sessions','retained',[0,4,16],size=4,options=('--rows',4,'--batch',4),risk='live completed sessions versus new execution latency')
        sweep('sessions-batch','runtime-sessions','batch',[1,4,64],size=8,options=('--rows',8,'--work',32,'--replay-every',2),risk='tiny scalar reads and replay while source progresses')
        sweep('sessions-replay','runtime-sessions','replay-every',[0,1,4],size=4,options=('--rows',4,'--work',32,'--retained',4),risk='replay amplification with continuing work and retained owners')
        for case in ('life-held-output','life-archive-fixed','life-inspections'):
            sweep(case,case,'size',[1,4,8],options=('--rows',4,'--work',64),risk='continuing source with independent concurrent owners')
        for case in ('life-held-output-conditional','life-archive-fixed-conditional','life-archive-rotate-conditional','life-inspections-conditional'):
            sweep(case+'-owners',case,'size',[1,4],options=('--rows',4,'--work',64),risk='retained readers during continued explicit choice turnover')
            sweep(case+'-duration',case,'work',[16,64,256],size=4,options=('--rows',4),risk='conditional turnover duration independent of retained owner count; pins survive and release')
        sweep('archive-cadence','life-archive-rotate','cadence',[1,4,16],size=4,options=('--rows',4,'--work',64),risk='fixed retained population versus turnover frequency')
        for order in ('grouped','interleaved','reverse'):
            sweep('fresh-copies-'+order,'fresh-contract','rows',[1,2,4,8],size=4,options=('--depth',4,'--order',order),risk='copy contraction under alternate query admission orders')
        for case in ('prepare-reuse','prepare-independent'):
            sweep(case+'-uses',case,'uses',[1,4,16],size=32,options=('--heads',4,'--arity',4,'--depth',8,'--width',8),risk='preparation reuse and session-local destruction')
        for case in ('type-k','type-ki','type-s','type-b','type-c','type-w','type-t','behavior-k','behavior-ki','behavior-s','behavior-b','behavior-c','behavior-t','behavior-m','arithmetic-forward','arithmetic-reverse'):
            sweep('notebook-'+case,'notebook-'+case,'size',[1],risk='end-to-end synthesis/evaluation query coverage')
    return points


def scaling_metric(key):
    """Only cost/resource observations, never configuration, IDs or oracle payloads."""
    if key in {'process.wall_seconds', 'process.user_cpu_seconds', 'process.system_cpu_seconds',
               'process.max_rss_kib', 'workload.native_peak_rss_estimate_kib',
               'workload.source_ms', 'workload.tiny_answer_ms', 'workload.runtime_drop_ms',
               'workload.elapsed_ms', 'workload.generation_ms', 'workload.parse_program_ms',
               'workload.parse_query_ms', 'workload.syntax_source_drop_ms',
               'workload.program_bytes', 'workload.query_bytes'}:
        return True
    if key.startswith(('workload.times_ms.', 'workload.work.', 'workload.memory_counts.',
                       'allocations.')):
        return True
    parts=key.split('.')
    if len(parts)==3 and parts[0]=='workload' and parts[1] in {'prepare_ms','prepared_drop_ms'} and parts[2].isdigit():
        return True
    if len(parts)==4 and parts[:2]==['workload','uses'] and parts[2].isdigit():
        return parts[3] in {'engine_init_ms','use_ms','engine_drop_ms','ticks','applications'}
    return (key.startswith(('phase.', 'workload.cleanup.')) and
            (parts[-1] in {'elapsed_ms', 'request_ms', 'ticks', 'applications'} or
             'memory' in parts))


def run_points(points, args, binary, out, deadline):
    """One measured sample per point per pass; suspended campaigns share the deadline."""
    active=[]
    admitted=[]
    expired=False
    errors=(ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError)
    try:
        for point in points:
            active.append((point, None))
        while active:
            next_pass=[]
            for point, generator in active:
                remaining=deadline-time.monotonic()
                if remaining<=3:
                    expired=True
                    break
                signal.setitimer(signal.ITIMER_REAL,remaining)
                try:
                    if generator is None:
                        location=out/point['id']
                        point.update(status='running',campaign=str(location/'campaign.json'))
                        generator=perf.campaign(
                            ['--binary',str(binary),'--out',str(location),
                             '--repeat',str(args.repeat),'--warmup','0',
                             '--seconds',str(args.sample_seconds),'--total-seconds',str(remaining),
                             '--memory-mib',str(args.memory_mib),'--',*point['workload']],
                            absolute_deadline=deadline,progress=point)
                        admitted.append((point,generator))
                    next(generator)
                    next_pass.append((point,generator))
                except StopIteration as done:
                    point['status']='completed' if done.value==0 else ('censored' if done.value==2 else 'failed')
                except errors as error:
                    point.update(status='failed',error=str(error))
            if expired: break
            active=next_pass
    except perf.CampaignDeadline:
        expired=True
        if point.get('status')!='failed': point['status']='censored'
        point['censor_scope']='suite_processing'
    finally:
        # Closing finalizes raw reports even when the sampling deadline expired.
        signal.setitimer(signal.ITIMER_REAL,0)
        for point,generator in admitted:
            failed=point.get('status')=='failed'
            try:
                generator.close()
                raw=read_json(Path(point['campaign']))
                counts=raw['summary']['counts']
                point.update(outcomes=counts,metrics=raw['summary']['metrics'],
                             requested_samples=args.repeat,samples_executed=sum(counts.values()))
                failed=failed or any(v for k,v in counts.items() if k not in ('completed','censored'))
                partial=(raw.get('status')!='finished' or raw.get('aggregate_censored',False)
                         or counts.get('completed',0)!=args.repeat)
                point['status']='failed' if failed else ('censored' if partial else 'completed')
            except perf.CampaignDeadline:
                point['status']='failed' if failed else 'censored'
                point['censor_scope']='suite_reporting'
                expired=True
            except errors as error:
                point.update(status='failed',error=str(error))
            finally:
                signal.setitimer(signal.ITIMER_REAL,0)
        for point in points: point.setdefault('status','not_run_budget')
    return expired


def scaling(points):
    """Adjacent ratios retain both operands; censored points never bridge a slope."""
    groups={}
    for point in points: groups.setdefault(point['family'],[]).append(point)
    result=[]
    for family, members in groups.items():
        members.sort(key=lambda p:p['value'])
        for small,large in zip(members,members[1:]):
            pair=dict(family=family,axis=small['axis'],small=small['value'],large=large['value'],metrics={})
            if any(p.get('status')!='completed' or 'metrics' not in p for p in (small,large)):
                pair['status']='incomplete_scaling';result.append(pair);continue
            pair['status']='observed'
            for key in sorted(k for k in set(small['metrics']) | set(large['metrics']) if scaling_metric(k)):
                left=small['metrics'].get(key,{});right=large['metrics'].get(key,{})
                a=left.get('median');b=right.get('median')
                complete=not left.get('incomplete_or_missing',0) and not right.get('incomplete_or_missing',0)
                finite=all(type(v) in (int,float) and math.isfinite(v) for v in (a,b))
                ratio=b/a if complete and finite and a>0 else None
                if ratio is not None and not math.isfinite(ratio): ratio=None
                exponent=math.log(ratio)/math.log(large['value']/small['value']) if ratio is not None and ratio>0 and small['value']>0 and large['value']>small['value'] else None
                pair['metrics'][key]=dict(small=a,large=b,ratio=ratio,exponent=exponent)
            result.append(pair)
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode',choices=['routine','deep'])
    parser.add_argument('--out',type=Path)
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--diagnostics',action='store_true')
    parser.add_argument('--seconds',type=float,help='aggregate execution/analysis budget; default routine120, deep600')
    parser.add_argument('--sample-seconds',type=float,default=5)
    parser.add_argument('--repeat',type=int,default=12)
    parser.add_argument('--memory-mib',type=int,default=4096)
    parser.add_argument('--seed',type=int,default=0)
    parser.add_argument('--only',action='append',help='run selected family, repeat flag for several')
    parser.add_argument('--list',action='store_true')
    parser.add_argument('--notebook-inventory',action='store_true',help='list saved queries and exact unrestricted synthesis cases')
    args=parser.parse_args(); budget=args.seconds if args.seconds is not None else (120 if args.mode=='routine' else 600)
    if any(not math.isfinite(v) or v<=0 for v in (budget,args.sample_seconds)) or args.repeat<=0 or args.memory_mib<=0:
        parser.error('positive finite budgets/repeats required')
    if args.notebook_inventory:
        print(json.dumps(notebook_inventory(),indent=2));return 0
    points=plan(args.mode,args.seed)
    if args.only:
        unknown=set(args.only)-{p['family'] for p in points}
        if unknown: parser.error('unknown families: '+', '.join(sorted(unknown)))
        points=[p for p in points if p['family'] in args.only]
    random.Random(args.seed).shuffle(points)
    if args.list:
        print(json.dumps(dict(mode=args.mode,seconds=budget,sample_seconds=args.sample_seconds,repeats=args.repeat,points=points),indent=2));return 0
    if not args.out: parser.error('--out required')
    if args.binary and args.diagnostics: parser.error('--diagnostics builds; supplied binary determines its own feature state')
    out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
    build=None
    if args.binary: binary=args.binary.resolve()
    else:
        build=['cargo','build','--offline','--release','--example','measure',*(['--features','diagnostics'] if args.diagnostics else [])]
        with (out/'build.log').open('w') as log:
            subprocess.run(build,cwd=perf.ROOT,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=300)
        target=Path(os.environ.get('CARGO_TARGET_DIR',perf.ROOT/'target'))
        if not target.is_absolute(): target=perf.ROOT/target
        binary=target/'release/examples/measure'
    report=dict(mode=args.mode,seed=args.seed,seconds=budget,sample_seconds=args.sample_seconds,repeat=args.repeat,
                warmup=0,scheduling='round_robin_samples',build=build,binary=str(binary),status='running',points=points,regression_assessment='not_performed')
    (out/'suite.json').write_text(json.dumps(report,indent=2)+'\n')
    started=time.monotonic()
    handler=signal.signal(signal.SIGALRM,perf.deadline_signal)
    try:
        if run_points(points,args,binary,out,started+budget):
            report['execution_censored']=True
    finally:
        signal.setitimer(signal.ITIMER_REAL,0);signal.signal(signal.SIGALRM,handler)
    # Summarization also has a deadline. Preserve raw point observations on expiry.
    handler=signal.signal(signal.SIGALRM,perf.deadline_signal)
    try:
        remaining=budget-(time.monotonic()-started)
        if remaining<=0: raise perf.CampaignDeadline()
        signal.setitimer(signal.ITIMER_REAL,remaining)
        report['scaling']=scaling(points)
    except perf.CampaignDeadline:
        report['analysis_censored']=True
    finally:
        signal.setitimer(signal.ITIMER_REAL,0);signal.signal(signal.SIGALRM,handler)
    report['elapsed_seconds']=time.monotonic()-started
    report['status']='failed' if any(p['status']=='failed' for p in points) else ('censored' if report.get('execution_censored') or report.get('analysis_censored') or any(p['status']!='completed' for p in points) else 'completed')
    (out/'suite.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(dict(suite=str(out/'suite.json'),status=report['status'],points=len(points))))
    return 0 if report['status']=='completed' else (2 if report['status']=='censored' else 1)


if __name__=='__main__': raise SystemExit(main())
