#!/usr/bin/env python3
"""Compare raw completed samples; distinguish measured changes from sparse evidence.

python3 examples/perf_compare.py /tmp/before /tmp/after --out /tmp/comparison.json
This is change evidence, not a universal performance acceptance threshold.
"""
import argparse
import itertools
import json
import math
from pathlib import Path
import random
import statistics
from collections import Counter
import signal
from perf import sample_values, classify, records, deadline_signal, CampaignDeadline, finite_float

DEFAULT_METRICS = ['workload.times_ms.source_delivery', 'workload.first_answer.ms',
                   'workload.times_ms.cleanup', 'process.user_cpu_seconds',
                   'process.system_cpu_seconds', 'workload.native_peak_rss_estimate_kib',
                   'diagnostics.source.0.allocation.process_peak_requested_bytes']


def permutation_probability(a, b, seed=0, draws=9999):
    """Two-sided permutation of the absolute mean difference.

    The null assumes exchangeable independent observations. Shared machine drift
    can violate that assumption; a detected change does not identify its cause.
    """
    joined=a+b; total=len(joined); count=len(a)
    observed=abs(statistics.fmean(a)-statistics.fmean(b))
    def extreme(indices):
        selected=set(indices)
        left=[v for i,v in enumerate(joined) if i in selected]
        right=[v for i,v in enumerate(joined) if i not in selected]
        return abs(statistics.fmean(left)-statistics.fmean(right)) >= observed
    combinations=math.comb(total,count)
    if combinations <= 10000:
        hits=sum(extreme(indices) for indices in itertools.combinations(range(total),count))
        return hits/combinations, 'exact', combinations
    rng=random.Random(seed)
    hits=sum(extreme(rng.sample(range(total),count)) for _ in range(draws))
    return (hits+1)/(draws+1), 'monte_carlo', draws


def compare_metric(a, b, draws=9999):
    if not a or not b:
        return {'status':'unavailable','before_n':len(a),'after_n':len(b)}
    old,new=statistics.fmean(a),statistics.fmean(b)
    result={'status':'insufficient_evidence','before_n':len(a),'after_n':len(b),
            'before_mean':old,'after_mean':new,'before_median':statistics.median(a),'after_median':statistics.median(b),'absolute_change':new-old,
            'ratio':new/old if old else None,'before_range':[min(a),max(a)],'after_range':[min(b),max(b)]}
    if min(len(a),len(b)) < 5: return result
    probability,method,draws=permutation_probability(a,b,draws=draws)
    result.update(p_value=probability,method=method,assignments=draws,status='uncertain')
    return result


def assess(before, after, metrics, alpha=.05):
    result={key:compare_metric([v[key] for v in before if v.get(key) is not None],
                               [v[key] for v in after if v.get(key) is not None]) for key in metrics}
    # Holm correction for the complete requested family, including unavailable
    # metrics; do not search for a favorable significance threshold per metric.
    candidates=sorted(((v['p_value'],k) for k,v in result.items() if 'p_value' in v))
    for index,(p,key) in enumerate(candidates):
        threshold=alpha/(len(metrics)-index)
        result[key]['holm_threshold']=threshold
        if p > threshold: break
        change=result[key]['absolute_change']
        result[key]['status']='increase' if change>0 else ('decrease' if change<0 else 'uncertain')
    return result


def read_json(path):
    def invalid(value):
        raise ValueError('nonfinite JSON number: '+value)
    return json.loads(path.read_text(), parse_constant=invalid, parse_float=finite_float)


def load(directory):
    metadata=read_json(directory/'campaign.json')
    samples=[read_json(directory/f'{index}.json') for index in range(metadata['samples_executed'])]
    configs=[]; completed=[]
    for sample in samples:
        if type(sample.get('warmup')) is not bool: raise ValueError('missing sample warmup flag')
        events=records('\n'.join('measurement='+json.dumps(e) for e in sample.get('records',[])))
        config=[e['data'] for e in events if e['kind']=='configuration']
        # Null optional dimensions mean the option does not apply to this case.
        configs.extend({k:v for k,v in c.items() if v is not None} for c in config)
        if sample['status']=='completed':
            if len(config)!=1 or classify(sample['process'],events,config[0]['case'])!='completed':
                raise ValueError('completed sample contradicts its measured outcome')
            if not sample['warmup']: completed.append(sample_values(sample))
    if configs and any(config!=configs[0] for config in configs):
        raise ValueError('changing workload configuration')
    return metadata,configs[0] if configs else None,samples,completed


def comparison(before, after, metrics):
    old,oc,os,ov=load(before);new,nc,ns,nv=load(after)
    if oc is not None and nc is not None and oc!=nc:
        raise ValueError('workload/observation configurations differ')
    findings=assess(ov,nv,metrics)
    failed=any(s['status'] not in ('completed','censored') for s in os+ns)
    censored=any(s['status']=='censored' for s in os+ns) or old['aggregate_censored'] or new['aggregate_censored']
    status='failed_comparison' if failed else ('censored_comparison' if censored else 'compared')
    if status != 'compared':
        for finding in findings.values():
            finding['completed_subset_status']=finding['status']
            finding['status']=status
    environment={key:{'before':old.get(key),'after':new.get(key)} for key in ('host','local_rustc','build_environment') if old.get(key)!=new.get(key)}
    return {'status':status,'environment_differences':environment,'before':str(before),'after':str(after),'configuration':oc or nc,
            'metrics':findings,
            'before_outcomes':dict(Counter(s['status'] for s in os if not s['warmup'])),
            'after_outcomes':dict(Counter(s['status'] for s in ns if not s['warmup'])),
            'interpretation':'Two-sided mean-difference permutation evidence with Holm family correction; increases need materiality/scaling assessment. Sparse, censored or missing observations cannot establish absence of regression. Machine drift and serial group order can confound differences. Native RSS is the whole harness address-space peak; wait4 RSS includes launcher overhead.'}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before',type=Path);parser.add_argument('after',type=Path)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--metric',action='append')
    parser.add_argument('--seconds',type=float,default=30,help='comparison processing budget')
    args=parser.parse_args()
    if not math.isfinite(args.seconds) or args.seconds<=0: parser.error('positive finite --seconds required')
    metrics=list(dict.fromkeys(args.metric or DEFAULT_METRICS))
    signal.signal(signal.SIGALRM,deadline_signal)
    try:
        signal.setitimer(signal.ITIMER_REAL,args.seconds)
        report=comparison(args.before,args.after,metrics)
    except CampaignDeadline:
        report={'status':'comparison_timeout','metrics':{},'seconds':args.seconds}
    except (ValueError,KeyError,TypeError,OSError) as error:
        report={'status':'invalid_comparison','metrics':{},'error':str(error)}
    finally:
        signal.setitimer(signal.ITIMER_REAL,0)
    with args.out.open('x') as out: json.dump(report,out,indent=2)
    print(json.dumps({'comparison':str(args.out),'status':report['status'],
                      'changes':{k:v['status'] for k,v in report['metrics'].items()}}))
    return 0 if report['status']=='compared' else (2 if report['status'] in ('censored_comparison','comparison_timeout') else 1)


if __name__=='__main__': raise SystemExit(main())
