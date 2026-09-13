#!/usr/bin/env python3
"""Compare completed measurement intervals; retain incomplete workload outcomes.

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
from perf import sample_measurements, classify, records, deadline_signal, CampaignDeadline, finite_float

RESAMPLING_DRAWS = 9999

DEFAULT_METRICS = ['workload.times_ms.source_delivery', 'workload.first_answer.ms',
                   'workload.times_ms.cleanup', 'process.user_cpu_seconds',
                   'process.system_cpu_seconds', 'workload.native_peak_rss_estimate_kib',
                   'diagnostics.source.0.allocation.process_peak_requested_bytes']


def permutation_probability(a, b, seed=0, draws=RESAMPLING_DRAWS):
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
    if combinations <= RESAMPLING_DRAWS:
        hits=sum(extreme(indices) for indices in itertools.combinations(range(total),count))
        return hits/combinations, 'exact', combinations
    rng=random.Random(seed)
    hits=sum(extreme(rng.sample(range(total),count)) for _ in range(draws))
    return (hits+1)/(draws+1), 'monte_carlo', draws


def compare_metric(a, b, draws=RESAMPLING_DRAWS):
    if not a or not b or any(v is None for v in a+b):
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
    result={key:compare_metric([v.get(key) for v in before],
                               [v.get(key) for v in after]) for key in metrics}
    # Holm correction for the complete requested family, including unavailable
    # metrics; do not search for a favorable significance threshold per metric.
    candidates=sorted(((v['p_value'],k) for k,v in result.items() if 'p_value' in v))
    for index,(p,key) in enumerate(candidates):
        threshold=alpha/(len(metrics)-index)
        result[key]['holm_threshold']=threshold
        if p > threshold:
            for _,pending in candidates[index:]:
                finding=result[pending]
                resolution=(2 if finding['method']=='exact' and finding['before_n']==finding['after_n'] else 1)/(finding['assignments']+(finding['method']!='exact'))
                finding['permutation_resolution_lower_bound']=resolution
                if resolution>threshold: finding['status']='resolution_limited'
            break
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
    configs=[]; observations=[]
    for sample in samples:
        if type(sample.get('warmup')) is not bool: raise ValueError('missing sample warmup flag')
        events=records('\n'.join('measurement='+json.dumps(e) for e in sample.get('records',[])))
        config=[e['data'] for e in events if e['kind']=='configuration']
        # Null optional dimensions mean the option does not apply to this case.
        configs.extend({k:v for k,v in c.items() if v is not None} for c in config)
        if sample['status']=='completed':
            if len(config)!=1 or classify(sample['process'],events,config[0]['case'])!='completed':
                raise ValueError('completed sample contradicts its measured outcome')
        if not sample['warmup']:
            observations.append({key:item['value'] if item['complete'] else None
                                 for key,item in sample_measurements(sample).items()})
    if configs and any(config!=configs[0] for config in configs):
        raise ValueError('changing workload configuration')
    return metadata,configs[0] if configs else None,samples,observations


def sample_paths(directory, samples):
    return [str(directory/f'{index}.json') for index,sample in enumerate(samples) if not sample['warmup']]


def exposure(metadata, samples):
    """Declared sample limits plus actual supervisor limits; no progress copy."""
    limits=metadata.get('limits',{})
    actual=[s.get('process',{}).get('limits') for s in samples if not s['warmup']]
    return dict(campaign={k:limits.get(k) for k in ('per_sample_seconds','memory_mib')},
                process=actual)


def check_exposures(exposures):
    """Only censored comparisons require identical observation budgets."""
    normalized=[]
    for item in exposures:
        if item is None:
            normalized.append(None);continue
        actual=item['process']
        if actual and any(limit!=actual[0] for limit in actual):
            raise ValueError('sample external limits differ within censored campaign')
        normalized.append(dict(campaign=item['campaign'],process=actual[0] if actual else None))
    if normalized and any(item!=normalized[0] for item in normalized):
        raise ValueError('censored campaign external limits differ')


def comparison(before, after, metrics, observer_overhead=False):
    old,oc,os,ov=load(before);new,nc,ns,nv=load(after)
    observation_changes={}
    if oc is not None and nc is not None:
        changes={k:{'before':oc.get(k),'after':nc.get(k)} for k in set(oc)|set(nc) if oc.get(k)!=nc.get(k)}
        if changes and (not observer_overhead or set(changes)-{'detailed','diagnostics_feature'}):
            raise ValueError('workload/observation configurations differ')
        observation_changes=changes
    failed=any(s['status'] not in ('completed','censored') for s in os+ns)
    censored=any(s['status']=='censored' for s in os+ns) or old['aggregate_censored'] or new['aggregate_censored']
    status='failed_comparison' if failed else ('censored_comparison' if censored else 'compared')
    if censored: check_exposures([exposure(old,os),exposure(new,ns)])
    findings=assess(ov,nv,metrics)
    if failed:
        for finding in findings.values(): finding['status']='failed_comparison'
    environment={key:{'before':old.get(key),'after':new.get(key)} for key in ('host','local_rustc','build_environment') if old.get(key)!=new.get(key)}
    return {'status':status,'observer_overhead':observer_overhead,'observation_changes':observation_changes,'environment_differences':environment,'before':str(before),'after':str(after),'configuration':oc or nc,
            'measurement_scope':'Completed measurement intervals only. Censored resource observations describe the measured prefix, not whole-search cost or adverse amplification; inspect raw work and elapsed observations.',
            'metrics':findings,'sample_paths':{'before':sample_paths(before,os),'after':sample_paths(after,ns)},
            'before_outcomes':dict(Counter(s['status'] for s in os if not s['warmup'])),
            'after_outcomes':dict(Counter(s['status'] for s in ns if not s['warmup'])),
            'interpretation':'At most 9,999 resampling draws per hypothesis; processing deadline bounds the comparison. Two-sided mean-difference permutation evidence with Holm family correction; increases need materiality/scaling assessment. Sparse, censored or missing observations cannot establish absence of regression. Machine drift and serial group order can confound differences. Native RSS is the whole harness address-space peak; wait4 RSS includes launcher overhead.'}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before',type=Path);parser.add_argument('after',type=Path)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--metric',action='append')
    parser.add_argument('--observer-overhead',action='store_true',help='explicitly compare detailed/diagnostics modes; workload must remain identical')
    parser.add_argument('--seconds',type=float,default=30,help='comparison processing budget')
    args=parser.parse_args()
    if not math.isfinite(args.seconds) or args.seconds<=0: parser.error('positive finite --seconds required')
    metrics=list(dict.fromkeys(args.metric or DEFAULT_METRICS))
    signal.signal(signal.SIGALRM,deadline_signal)
    try:
        signal.setitimer(signal.ITIMER_REAL,args.seconds)
        report=comparison(args.before,args.after,metrics,args.observer_overhead)
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
