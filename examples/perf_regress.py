#!/usr/bin/env python3
"""Compare suite costs and scaling against two unchanged control campaigns.

Robust control dispersion sets a descriptive resolution floor, not a speed target.
Mean and distribution permutation tests and independent scaling bootstrap tests
share one Holm family. Bootstrap qualification is approximate, especially at small n.
See scipy.stats.bootstrap (paired=False, basic intervals), scipy.stats.permutation_test,
and scipy.stats.median_abs_deviation for the statistical constructions.
"""
import argparse
import itertools
import json
import math
import random
import statistics
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
    # At most three tests per point/metric: mean, distribution, adjacent scaling.
    return max(9999,math.ceil(6*points*metrics/.05))


def ordered(sample):
    if any(isinstance(v,bool) or not isinstance(v,(int,float)) or not math.isfinite(v) for v in sample):
        raise ValueError('samples must contain finite numbers')
    return sorted(sample)


def mad(sample):
    center=statistics.median(sample)
    return statistics.median(abs(v-center) for v in sample)


def control_resolution(a,b):
    # A descriptive robust scale, NOT a confidence bound or a speed target.
    # 1.4826 is the conventional normal-consistent MAD scale conversion.
    return max(abs(statistics.median(a)-statistics.median(b)),1.4826*mad(a),1.4826*mad(b))


def distribution(a,b,ca,cb,draws):
    result=dict(status='unavailable',before_n=len(a),after_n=len(b),
                interpretation='Tie-aware two-sample KS permutation under exchangeability. '
                'A distribution change has no automatic adverse direction. Observed maxima '
                'and exceedance counts do not estimate a rare-event rate or population quantile.')
    if not a or not b: return result
    result.update(before_range=[min(a),max(a)],after_range=[min(b),max(b)],
                  after_above_before_max=sum(v>max(a) for v in b),
                  after_below_before_min=sum(v<min(a) for v in b))
    if min(len(a),len(b))<5:
        result['status']='insufficient_evidence';return result
    if min(len(ca),len(cb))<5:
        result['status']='uncalibrated';return result
    def setup(left,right):
        joined=sorted([(v,0) for v in left]+[(v,1) for v in right])
        ends=[i+1 for i in range(len(joined)) if i+1==len(joined) or joined[i][0]!=joined[i+1][0]]
        def score(indices):
            selected=set(indices); count=0; previous=0; maximum=0
            for end in ends:
                count+=sum(i in selected for i in range(previous,end))
                maximum=max(maximum,abs(count*len(right)-(end-count)*len(left)))
                previous=end
            return maximum
        observed=score(i for i,(_,label) in enumerate(joined) if label==0)
        return score,observed
    score,observed=setup(a,b)
    _,control=setup(ca,cb)
    total=len(a)+len(b); combinations=math.comb(total,len(a))
    if combinations<=10000:
        assignments=combinations;method='exact'
        hits=sum(score(indices)>=observed for indices in itertools.combinations(range(total),len(a)))
        probability=hits/combinations
    else:
        assignments=draws;method='monte_carlo';rng=random.Random(0)
        hits=sum(score(rng.sample(range(total),len(a)))>=observed for _ in range(draws))
        probability=(hits+1)/(draws+1)
    result.update(status='uncertain',p_value=probability,method=method,assignments=assignments,
                  statistic=observed/(len(a)*len(b)),noise_resolution=control/(len(ca)*len(cb)))
    return result


def evaluate(before,after,control_a,control_b,draws=9999):
    before,after,control_a,control_b=map(ordered,(before,after,control_a,control_b))
    result=compare_metric(before,after,draws=draws)
    result.update(control_a_n=len(control_a),control_b_n=len(control_b),
                  distribution=distribution(before,after,control_a,control_b,draws))
    if min(len(control_a),len(control_b))<5:
        result['status']='uncalibrated';return result
    control=control_a+control_b
    result.update(control_range=[min(control),max(control)],noise_span=max(control)-min(control),
                  control_medians=[statistics.median(control_a),statistics.median(control_b)],
                  control_mads=[mad(control_a),mad(control_b)],
                  noise_resolution=control_resolution(control_a,control_b),
                  control_interpretation='Maximum of between-control median drift and each '
                  'normal-scaled MAD; a robust descriptive resolution, not future-noise confidence. '
                  'All extremes remain in control_range/noise_span and the raw records.')
    return result


def hypotheses(findings):
    return [test for f in findings for test in ([f,f['distribution']] if 'distribution' in f else [f])]


def correct(findings,alpha=.05):
    tests=hypotheses(findings)
    eligible=sorted((f['p_value'],index) for index,f in enumerate(tests)
                    if f['status']=='uncertain' and 'noise_resolution' in f)
    for rank,(p,index) in enumerate(eligible):
        finding=tests[index];threshold=alpha/(len(tests)-rank)
        finding['holm_threshold']=threshold
        if p>threshold:
            for _,pending in eligible[rank:]:
                f=tests[pending]
                resolution=(2 if f['method']=='exact' and f['before_n']==f['after_n'] else 1)/(f['assignments']+(f['method']!='exact'))
                f['permutation_resolution_lower_bound']=resolution
                if resolution>threshold: f['status']='resolution_limited'
            break
        if 'statistic' in finding:
            finding['status']='distribution_change' if finding['statistic']>finding['noise_resolution'] else 'within_control_spread'
        else:
            delta=finding['absolute_change']
            if delta>finding['noise_resolution']: finding['status']='regression'
            elif -delta>finding['noise_resolution']: finding['status']='improvement'
            else: finding['status']='within_control_spread'
    for f in findings:
        if 'distribution' in f:
            f['mean_status']=f['status']
            if f['distribution']['status']=='distribution_change' and f['status'] not in ('regression','improvement'):
                f['status']='distribution_change'


def scaling(before,after,control_a,control_b,draws=9999):
    """Difference of high/low means, independent basic bootstrap of four samples.

    The centered bootstrap approximates the sampling error of this smooth contrast;
    it is not a label permutation or a test of exchangeability across sizes. Each
    group must be iid, with finite variance and low-size means away from zero.
    Sorting makes the seeded calculation invariant to input record order. Draws
    are Monte Carlo work, never additional observations. Small/heavy-tailed samples
    can give poor coverage; Holm using these approximate p-values is approximate.
    """
    operands={name:dict(low=ordered(pair[0]),high=ordered(pair[1])) for name,pair in
              zip(('before','after','control_a','control_b'),(before,after,control_a,control_b))}
    result=dict(status='unavailable',operands=operands,method='independent_bootstrap',
                interpretation=scaling.__doc__,before_n=[len(v) for v in before],after_n=[len(v) for v in after])
    groups=[operands[name][size] for name in operands for size in ('low','high')]
    if any(not group for group in groups): return result
    if any(v<0 for group in groups for v in group): raise ValueError('scaling costs must be nonnegative')
    def ratio(low,high,statistic=statistics.fmean):
        denominator=statistic(low)
        if denominator==0: raise ZeroDivisionError
        value=statistic(high)/denominator
        if not math.isfinite(value): raise ValueError('nonfinite scaling ratio')
        return value
    try:
        old=ratio(*groups[:2]);new=ratio(*groups[2:4])
    except ZeroDivisionError:
        result['undefined_reason']='zero low-size mean';return result
    # Treat roundoff-sized contrasts as ties (a numerical tolerance, not a cost target).
    delta=0. if math.isclose(new,old,rel_tol=1e-14,abs_tol=0.) else new-old
    result.update(before_ratio=old,after_ratio=new,absolute_change=delta)
    if min(map(len,groups[4:]))<5:
        result['status']='uncalibrated';return result
    if min(map(len,groups[:4]))<5:
        result['status']='insufficient_evidence';return result
    rng=random.Random(0);control_rng=random.Random(1)
    def resample(group,generator):
        return generator.choices(group,k=len(group))
    errors=[];control_differences=[]
    try:
        for _ in range(draws):
            low,high,new_low,new_high=[resample(g,rng) for g in groups[:4]]
            errors.append(ratio(new_low,new_high)-ratio(low,high)-delta)
            low,high,new_low,new_high=[resample(g,control_rng) for g in groups[4:]]
            # Control medians resist isolated excursions; these bootstrap values
            # calibrate resolution only and never enter an inference sample.
            control_differences.append(ratio(new_low,new_high,statistics.median)-ratio(low,high,statistics.median))
    except ZeroDivisionError:
        result['undefined_reason']='zero denominator in bootstrap; no draws discarded';return result
    probability=(1+sum(abs(e)>=abs(delta) for e in errors))/(draws+1)
    ordered_errors=sorted(abs(e) for e in errors)
    radius=ordered_errors[min(draws-1,math.ceil(.95*draws)-1)]
    result.update(status='uncertain',p_value=probability,assignments=draws,
                  approximate_95_percent_interval=[delta-radius,delta+radius],
                  interval_scope='Pointwise symmetric basic-bootstrap interval, not Holm simultaneous coverage.',
                  noise_resolution=abs(statistics.median(control_differences))+1.4826*mad(control_differences),
                  control_bootstrap_range=[min(control_differences),max(control_differences)],
                  control_interpretation='Absolute median plus normal-scaled MAD of independently '
                  'bootstrapped control median-ratio differences; descriptive resolution, not a confidence bound.')
    return result


def compare(control_a,control_b,before,after,metrics=None):
    loaded=[read_suite(p) for p in (control_a,control_b,before,after)]
    suites=[p for _,p in loaded];keys=set().union(*(s.keys() for s in suites))
    # Reserve at least two Monte Carlo resolution steps below the strictest
    # possible Holm threshold for this requested family (means, distributions, slopes).
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
                ca,cb,old,new=[(values(s.get(low),metric),values(s.get(high),metric)) for s in suites]
                finding=scaling(old,new,ca,cb,draws=draws)
                finding.update(kind='scaling',family=family,low=low,high=high,metric=metric)
                findings.append(finding)
    correct(findings)
    failed=any('failed' in status for status in outcomes.values()) or any(m['status']=='failed' for m,_ in loaded)
    incomplete=any(any(s!='completed' for s in status) for status in outcomes.values()) or any(m['status']!='completed' for m,_ in loaded) or any(f['status'] in ('uncalibrated','unavailable','insufficient_evidence','resolution_limited') for f in hypotheses(findings))
    regressions=[f for f in findings if f['status']=='regression']
    changes=sum(f.get('distribution',{}).get('status')=='distribution_change' for f in findings)
    return dict(status='failed_evidence' if failed else ('regression' if regressions else ('incomplete_evidence' if incomplete else ('distribution_change' if changes else 'no_regression_detected'))),
                incomplete_evidence=incomplete,controls=[str(control_a),str(control_b)],before=str(before),after=str(after),
                outcomes=outcomes,outcome_layers=outcome_layers,permutation_draws=draws,findings=findings,regression_count=len(regressions),distribution_change_count=changes,
                interpretation='Mean costs use exchangeability-based permutation tests; independent ratio-of-means scaling uses approximate centered bootstrap inference. Means, distributions and scaling share one Holm family, whose error control is approximate where bootstrap p-values are used. Robust control resolution retains extremes without making one excursion a veto. Distribution changes do not establish adverse direction; maxima and exceedance counts do not establish tail rates. No signal establishes equivalence or rare-event safety. IID finite-variance sampling and well-separated-from-zero denominators are required for bootstrap accuracy; serial drift and small or heavy-tailed samples can invalidate inference.')


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
