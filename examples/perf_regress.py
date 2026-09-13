#!/usr/bin/env python3
"""Compare suite costs and scaling against two unchanged control campaigns.

Control spread sets a measured resolution floor, not a universal speed target.
All requested point/metric and scaling tests share one Holm-corrected family.
"""
import argparse
import itertools
import json
import math
from pathlib import Path
import signal
from perf import CampaignDeadline, deadline_signal
from perf_compare import load, read_json, compare_metric

def default_metrics(case):
    residency='workload.native_peak_rss_estimate_kib'
    if case in ('prepare-reuse','prepare-independent'):
        return ['workload.elapsed_ms','workload.parse_program_ms','workload.prepare_ms.0',residency,'process.wall_seconds']
    if case=='runtime-sessions':
        return ['workload.source_ms','workload.tiny_answer_ms','workload.cleanup.elapsed_ms',residency,'process.wall_seconds']
    if case.startswith('life-'):
        return ['phase.interaction_exit.0.elapsed_ms','phase.cancel.0.elapsed_ms','phase.release.0.elapsed_ms',residency,'process.wall_seconds']
    return ['workload.times_ms.source_delivery','workload.first_answer.ms','workload.times_ms.cleanup',residency,'process.wall_seconds']


def read_suite(directory):
    metadata=read_json(directory/'suite.json'); points={}
    for spec in metadata['points']:
        key=spec['id']
        if key in points: raise ValueError('duplicate suite point '+key)
        point=dict(spec=spec,configuration=None,values=[],suite_status=spec.get('status','unknown'),status=spec.get('status','unknown'))
        path=directory/key
        if (path/'campaign.json').exists():
            if read_json(path/'campaign.json')['status']!='finished':
                if point['status']!='failed': point['status']='censored'
                points[key]=point;continue
            campaign,config,samples,values=load(path)
            failure=any(s['status'] not in ('completed','censored') for s in samples)
            incomplete=campaign['status']!='finished' or campaign['aggregate_censored'] or any(s['status']=='censored' for s in samples)
            campaign_status='failed' if failure else ('censored' if incomplete else 'completed')
            status='failed' if failure or point['suite_status']=='failed' else ('censored' if incomplete or point['suite_status']!='completed' else 'completed')
            point.update(configuration=config,values=values,campaign_status=campaign_status,status=status)
        elif point['status']=='completed':
            raise ValueError('completed point has no raw campaign: '+key)
        points[key]=point
    if not points: raise ValueError("suite contains no points")
    return metadata,points


def values(point, metric):
    if point is None or point['status']!='completed': return []
    result=[row.get(metric) for row in point['values']]
    # Partial observations are not silently selected as a complete distribution.
    return result if result and all(v is not None for v in result) else []


def permutation_draws(points,metrics):
    return max(9999,math.ceil(4*points*metrics/.05))


def evaluate(before,after,control_a,control_b,draws=9999):
    result=compare_metric(before,after,draws=draws)
    result.update(control_a_n=len(control_a),control_b_n=len(control_b))
    if min(len(control_a),len(control_b))<5:
        result['status']='uncalibrated';return result
    control=control_a+control_b
    result['control_range']=[min(control),max(control)]
    result['noise_span']=max(control)-min(control)
    result['control_interpretation']='Observed unchanged-run span; not a confidence bound on future noise.'
    return result


def correct(findings,alpha=.05):
    eligible=sorted((f['p_value'],index) for index,f in enumerate(findings) if f['status']=='uncertain' and 'noise_span' in f)
    for rank,(p,index) in enumerate(eligible):
        finding=findings[index];threshold=alpha/(len(findings)-rank)
        finding['holm_threshold']=threshold
        if p>threshold:
            for _,pending in eligible[rank:]:
                f=findings[pending]
                resolution=(2 if f['method']=='exact' and f['before_n']==f['after_n'] else 1)/(f['assignments']+(f['method']=='monte_carlo'))
                f['permutation_resolution_lower_bound']=resolution
                if resolution>threshold: f['status']='resolution_limited'
            break
        delta=finding['absolute_change']
        if delta>finding['noise_span']: finding['status']='regression'
        elif -delta>finding['noise_span']: finding['status']='improvement'
        else: finding['status']='within_control_spread'


def ratio_values(low,high,metric):
    a=values(low,metric);b=values(high,metric)
    if not a or len(a)!=len(b) or any(v<=0 for v in a): return []
    # Independent size samples matched by repeat index, not a within-process pair.
    return [y/x for x,y in zip(a,b)]


def compare(control_a,control_b,before,after,metrics=None):
    loaded=[read_suite(p) for p in (control_a,control_b,before,after)]
    suites=[p for _,p in loaded];keys=set().union(*(s.keys() for s in suites))
    # Reserve at least two Monte Carlo resolution steps below the strictest
    # possible Holm threshold for this requested family (costs plus slopes).
    maximum_metrics=len(metrics) if metrics else 5
    draws=permutation_draws(len(keys),maximum_metrics)
    findings=[];outcomes={};specs={};outcome_layers={}
    for key in sorted(keys):
        points=[s.get(key) for s in suites]
        outcomes[key]=[p['status'] if p else 'missing' for p in points]
        outcome_layers[key]=[{'suite':p['suite_status'],'campaign':p.get('campaign_status')} if p else None for p in points]
        present=[p for p in points if p]
        configurations=[p['configuration'] for p in present if p['configuration'] is not None]
        if configurations and any(c!=configurations[0] for c in configurations):
            raise ValueError('point configurations differ: '+key)
        spec=present[0]['spec'];specs[key]=spec
        if any(any(p['spec'].get(k)!=spec.get(k) for k in ('family','axis','value','workload')) for p in present):
            raise ValueError('point axes or workloads differ: '+key)
        for metric in metrics or default_metrics(spec['workload'][0]):
            ca,cb,old,new=[values(p,metric) for p in points]
            finding=evaluate(old,new,ca,cb,draws=draws)
            finding.update(kind='cost',point=key,metric=metric)
            findings.append(finding)
    groups={}
    for key,spec in specs.items(): groups.setdefault(spec['family'],[]).append(key)
    for family,group in groups.items():
        group.sort(key=lambda k:specs[k]['value'])
        for low,high in itertools.pairwise(group):
            for metric in metrics or default_metrics(specs[low]['workload'][0]):
                ca,cb,old,new=[ratio_values(s.get(low),s.get(high),metric) for s in suites]
                finding=evaluate(old,new,ca,cb,draws=draws)
                finding.update(kind='scaling',family=family,low=low,high=high,metric=metric)
                findings.append(finding)
    correct(findings)
    failed=any('failed' in status for status in outcomes.values()) or any(m['status']=='failed' for m,_ in loaded)
    incomplete=any(any(s!='completed' for s in status) for status in outcomes.values()) or any(m['status']!='completed' for m,_ in loaded) or any(f['status'] in ('uncalibrated','unavailable','insufficient_evidence','resolution_limited') for f in findings)
    regressions=[f for f in findings if f['status']=='regression']
    return dict(status='failed_evidence' if failed else ('regression' if regressions else ('incomplete_evidence' if incomplete else 'no_regression_detected')),
                incomplete_evidence=incomplete,controls=[str(control_a),str(control_b)],before=str(before),after=str(after),
                outcomes=outcomes,outcome_layers=outcome_layers,permutation_draws=draws,findings=findings,regression_count=len(regressions),
                interpretation='A regression signal requires a mean cost or adjacent-size ratio increase beyond the unchanged-control span and a Holm-qualified permutation test. Missing calibration and sparse/noisy observations remain explicit; no signal does not establish equivalence or cover unmeasured risks. Serial execution drift can confound causes. Scaling ratios pair independent size samples by repeat index.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('control_a','control_b','before','after'): parser.add_argument(name,type=Path)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--metric',action='append')
    parser.add_argument('--seconds',type=float,default=300)
    args=parser.parse_args()
    if not math.isfinite(args.seconds) or args.seconds<=0: parser.error('positive finite seconds required')
    signal.signal(signal.SIGALRM,deadline_signal)
    try:
        signal.setitimer(signal.ITIMER_REAL,args.seconds)
        result=compare(args.control_a,args.control_b,args.before,args.after,list(dict.fromkeys(args.metric)) if args.metric else None)
    except CampaignDeadline: result=dict(status='comparison_timeout')
    except (ValueError,KeyError,TypeError,OSError) as error: result=dict(status='invalid_evidence',error=str(error))
    finally: signal.setitimer(signal.ITIMER_REAL,0)
    with args.out.open('x') as output: json.dump(result,output,indent=2)
    print(json.dumps(dict(result=str(args.out),status=result['status'],regression_count=result.get('regression_count'))))
    return 0 if result['status']=='no_regression_detected' else (2 if result['status'] in ('incomplete_evidence','comparison_timeout') else 1)


if __name__=='__main__': raise SystemExit(main())
