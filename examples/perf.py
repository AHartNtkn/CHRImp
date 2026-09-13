#!/usr/bin/env python3
"""Repeat a validated native measurement under declared external budgets.

python3 examples/perf.py --out /tmp/chr-rewrite --repeat 5 -- rewrite 128 5000000 5
Raw samples are retained; this command does not yet establish regression thresholds.
"""
import argparse
from collections import Counter
import json
import math
import os
from pathlib import Path
import platform
import statistics
import signal
import subprocess
import time
from supervise import run

ROOT = Path(__file__).resolve().parents[1]


def finite_float(value):
    number = float(value)
    if not math.isfinite(number): raise ValueError("nonfinite JSON number: "+value)
    return number


def records(text):
    result = []
    for line in text.splitlines():
        if not line.startswith('measurement='):
            continue
        record = json.loads(line[len('measurement='):], parse_float=finite_float, parse_constant=lambda value: (_ for _ in ()).throw(ValueError('nonfinite JSON number: '+value)))
        if not isinstance(record,dict) or record.get('schema') != 1 or not isinstance(record.get('kind'), str) or not isinstance(record.get('data'),dict):
            raise ValueError('invalid measurement record')
        result.append(record)
    return result


def classify(process, events, case):
    if not process['group_cleanup_complete']:
        return 'cleanup_failure'
    if process['status'] == 'censored':
        return 'censored'
    if process['descendants_signaled_after_exit']:
        return 'process_failure'
    results = [e['data'] for e in events if e['kind'] == 'result']
    errors = [e for e in events if e['kind'] == 'error']
    if process['returncode'] not in (0, 2):
        return 'workload_error' if errors else 'process_failure'
    configs = [e['data'] for e in events if e['kind'] == 'configuration']
    if len(results) != 1 or len(configs) != 1:
        return 'report_error'
    result, config = results[0], configs[0]
    if result.get('case') != case or config.get('case') != case or result.get('size') != config.get('size') or type(config.get('size')) is not int or config['size'] < (0 if case in ('prepare-reuse','prepare-independent') else 1):
        return 'report_error'
    if not isinstance(config.get('detailed'),bool) or not isinstance(config.get('diagnostics_feature'),bool):
        return 'report_error'
    if errors or 'error' not in result or result['error'] is not None:
        return 'report_error'
    status = result.get('status')
    if process['returncode'] == 2:
        return 'censored' if status == 'INCOMPLETE' else 'report_error'
    if status not in ('COMPLETE','PREFIX_COMPLETE'):
        return 'report_error'
    if case in ('prepare-reuse','prepare-independent'):
        options=config.get('preparation')
        if not isinstance(options,dict) or result.get('options')!=options: return 'report_error'
        uses=result.get('uses',[])
        if result.get('censored') is not False or result.get('prepared_released') is not True: return 'report_error'
        if len(uses)!=options.get('uses') or not uses: return 'report_error'
        if any(u.get('complete') is not True or u.get('answers')!=1 or u.get('applications')!=int(not options.get('empty')) or u.get('prepared_owners_after_engine_drop')!=1 for u in uses): return 'report_error'
        preparations=1 if case=='prepare-reuse' else len(uses)
        if len(result.get('prepare_ms',[]))!=preparations or len(result.get('prepared_drop_ms',[]))!=preparations: return 'report_error'
    elif case=='runtime-sessions':
        options=config.get('sessions')
        if not isinstance(options,dict) or result.get('options')!=options: return 'report_error'
        if result.get('censored') is not False or result.get('cleanup_complete') is not True: return 'report_error'
        expected={'history_completed':options['closed'],'retained_completed':options['retained'],'answers':config['size'],'tiny_answers':1,'finite_applications':options['rows'],'closed_verified':options['retained']+2+int(options['work']>0)}
        if any(result.get(k)!=v for k,v in expected.items()): return 'report_error'
        if result.get('background_applications',-1)<options['work']: return 'report_error'
        for key in ('after_close_spools','after_drop_spools'):
            if result.get(key) is not None and any(result[key].get(k)!=0 for k in ('files','descriptors','logical_bytes')): return 'report_error'
    elif case.startswith('life-') or case == 'runtime':
        if result.get('censored') is not False: return 'report_error'
    else:
        if any(result.get(key) is not True for key in ('source_goal_reached','cleanup_done','cleanup_in_time')):
            return 'report_error'
        if not isinstance(result.get('times_ms'),dict) or not isinstance(result.get('work'),dict) or not isinstance(result.get('memory_counts'),dict):
            return 'report_error'
        if not isinstance(result['times_ms'].get('source_delivery'),(int,float)):
            return 'report_error'
        if type(result['work'].get('answers')) is not int or result['work']['answers'] < 0: return 'report_error'
        if result['times_ms']['source_delivery'] < 0: return 'report_error'
        if config['detailed'] is False and result['times_ms'].get('validator') is not None: return 'report_error'
        if result.get('goal_count') is not None and result.get('goal') == 'answer_prefix' and result['work']['answers'] != result['goal_count']: return 'report_error'
        expected=result.get('expected',{})
        if result.get('goal') == 'complete':
            for key in ('answers','applications'):
                if expected.get(key) is not None and result['work'].get(key) != expected[key]: return 'report_error'
    return 'completed'


def numbers(value, prefix=''):
    if isinstance(value, dict):
        for key, item in value.items():
            yield from numbers(item, f'{prefix}.{key}' if prefix else key)
    elif isinstance(value, list):
        for index,item in enumerate(value):
            yield from numbers(item, f'{prefix}.{index}')
    elif value is None:
        yield prefix, None
    elif isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value):
        yield prefix, value


def distribution(values, completed):
    median = statistics.median(values) if values else None
    return {'n':len(values),'missing_completed':completed-len(values),'median':median,
            'min':min(values) if values else None,'max':max(values) if values else None,
            'mad':statistics.median(abs(value-median) for value in values) if values else None}

class CampaignDeadline(Exception):
    pass

def deadline_signal(*_):
    raise CampaignDeadline('aggregate deadline reached during campaign work')


def sample_values(sample):
    result = next(e['data'] for e in sample['records'] if e['kind'] == 'result')
    values = {'process': {k:v for k,v in sample['process'].items() if k not in ('limits', 'returncode', 'descendant_pids')}, 'workload':{k:result[k] for k in ('times_ms','work','memory_counts','first_event','first_answer','source_exhausted','native_peak_rss_estimate_kib','program_bytes','query_bytes','generation_ms','parse_program_ms','parse_query_ms','prepare_ms','uses','prepared_drop_ms','syntax_source_drop_ms','elapsed_ms','source_ms','tiny_answer_ms','tiny_answer_work','source','setup','cleanup','source_and_setup','runtime_drop_ms','before_close_memory','retained_spools','after_history_spools','before_close_spools','after_close_spools','after_drop_spools','background_applications','finite_applications') if k in result}}
    for kind in ('phase','diagnostics'):
        occurrences=Counter()
        grouped={}
        for event in sample['records']:
            if event['kind'] != kind: continue
            name=event['data'].get('phase','unnamed')
            index=occurrences[name]; occurrences[name]+=1
            grouped[f'{name}.{index}']=event['data']
        if grouped: values[kind]=grouped
    return dict(numbers(values))


def summary(samples):
    usable = [s for s in samples if not s['warmup'] and s['status'] == 'completed']
    metrics = {}
    for sample in usable:
        for key, value in sample_values(sample).items():
            metrics.setdefault(key, [])
            if value is not None: metrics[key].append(value)
    return {'counts':dict(Counter(s['status'] for s in samples if not s['warmup'])),
            'warmup_count':sum(s['warmup'] for s in samples),
            'metrics': {k:distribution(v,len(usable)) for k,v in metrics.items()},
            'regression_assessment':'not_performed'}


def capture(command):
    return subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=True, timeout=60).stdout.strip()


def main(argv=None, absolute_deadline=None, progress=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--repeat', type=int, default=5)
    parser.add_argument('--warmup', type=int, default=1)
    parser.add_argument('--seconds', type=float, default=10, help='per-sample wall deadline, all phases')
    parser.add_argument('--total-seconds', type=float, default=120, help='sample campaign budget, excluding build')
    parser.add_argument('--memory-mib', type=int, default=4096)
    parser.add_argument('--diagnostics', action='store_true')
    parser.add_argument('--binary', type=Path, help='use a specified prebuilt measure binary; build flags then unavailable')
    parser.add_argument('workload', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    workload = args.workload[1:] if args.workload[:1] == ['--'] else args.workload
    if len(workload) < 2 or args.repeat <= 0 or args.warmup < 0 or args.memory_mib <= 0 or any(not math.isfinite(v) or v <= 0 for v in [args.seconds,args.total_seconds]):
        parser.error('CASE SIZE and positive finite budgets/repeats required')
    if args.binary and args.diagnostics:
        parser.error('--diagnostics builds the feature; use the feature state of a supplied --binary')
    out = args.out.resolve(); out.mkdir(parents=True, exist_ok=False)
    build = None
    if args.binary:
        binary = args.binary.resolve()
    else:
        build = ['cargo','build','--offline','--release','--example','measure', *(['--features','diagnostics'] if args.diagnostics else [])]
        with (out/'build.log').open('w') as log:
            subprocess.run(build,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=300)
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT/'target'))
        if not target.is_absolute(): target = ROOT/target
        binary = target/'release/examples/measure'
    metadata = {'schema':1,'command':[str(binary),*workload], 'build':build,
                'local_revision':capture(['git','rev-parse','HEAD']), 'working_tree':capture(['git','status','--short']),
                'local_rustc':capture(['rustc','-Vv']), 'host':platform.platform(),
                'build_environment': {key:os.environ.get(key) for key in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_TARGET_DIR','RUSTUP_TOOLCHAIN')} if build else None,
                'limits':{'per_sample_seconds':args.seconds,'total_sample_seconds':args.total_seconds,'repeats':args.repeat,'warmups':args.warmup,'memory_mib':args.memory_mib},
                'status':'running'}
    if absolute_deadline is not None:
        args.total_seconds=max(0.,min(args.total_seconds,absolute_deadline-time.monotonic()))
        metadata['limits']['total_sample_seconds']=args.total_seconds
    path=out/'campaign.json'; path.write_text(json.dumps(metadata,indent=2)+'\n')
    samples=[]; inflight=None; started=time.monotonic()
    aggregate_deadline=False
    previous_handler=signal.signal(signal.SIGALRM,deadline_signal)
    try:
        signal.setitimer(signal.ITIMER_REAL,args.total_seconds)
        for index in range(args.warmup+args.repeat):
            remaining=args.total_seconds-(time.monotonic()-started)
            # Reserve the supervisor's interrupt and original-group cleanup grace.
            if remaining <= 2:
                break
            sample={'index':index,'warmup':index<args.warmup}
            inflight=sample
            try:
                with (out/f'{index}.stdout').open('w') as stdout, (out/f'{index}.stderr').open('w') as stderr:
                    process=run(metadata['command'],ROOT,stdout,stderr,min(args.seconds,remaining-2),args.memory_mib)
                sample['process']=process
                try:
                    events=records((out/f'{index}.stdout').read_text())
                    status=classify(process,events,workload[0])
                except (ValueError,TypeError) as error:
                    events=[]; status='cleanup_failure' if not process['group_cleanup_complete'] else ('censored' if process['status']=='censored' else 'report_error')
                    sample['report_error']=str(error)
                sample.update(status=status,records=events)
            except (OSError,ValueError,subprocess.SubprocessError) as error:
                sample.update(status='supervisor_error',error=str(error),records=[])
            temporary=out/f'{index}.json.tmp'
            temporary.write_text(json.dumps(sample)+'\n')
            temporary.replace(out/f'{index}.json')
            samples.append(sample)
            inflight=None
        statistics_report=summary(samples)
    except CampaignDeadline:
        aggregate_deadline=True
        if inflight is not None:
            inflight.update(status='censored',censor_scope='runner_processing')
            inflight.setdefault('records',[])
            temporary=out/f"{inflight['index']}.json.tmp"
            temporary.write_text(json.dumps(inflight)+'\n')
            temporary.replace(out/f"{inflight['index']}.json")
            if not samples or samples[-1] is not inflight: samples.append(inflight)
        statistics_report={'counts':dict(Counter(s['status'] for s in samples if not s['warmup'])),
                           'metrics':{},'unavailable_reason':'aggregate_deadline_during_processing','regression_assessment':'not_performed'}
    finally:
        signal.setitimer(signal.ITIMER_REAL,0)
        signal.signal(signal.SIGALRM,previous_handler)
        if absolute_deadline is not None:
            # The suite owns the remaining reporting budget after this campaign.
            signal.setitimer(signal.ITIMER_REAL,max(0.000001,absolute_deadline-time.monotonic()))
    elapsed=time.monotonic()-started
    metadata.update(status='finished',samples_executed=len(samples),requested_samples=args.warmup+args.repeat,
                    aggregate_censored=aggregate_deadline or elapsed >= args.total_seconds or len(samples)<args.warmup+args.repeat,
                    elapsed_seconds=elapsed,summary=statistics_report)
    if progress is not None:
        progress.update(outcomes=statistics_report['counts'],metrics=statistics_report['metrics'],
                        status='failed' if any(s['status'] not in ('completed','censored') for s in samples) else ('censored' if metadata['aggregate_censored'] or any(s['status']=='censored' for s in samples) else 'completed'))
    path.write_text(json.dumps(metadata,indent=2)+'\n')
    print(json.dumps({'campaign':str(path),'counts':metadata['summary']['counts'],'aggregate_censored':metadata['aggregate_censored'],'source_delivery_ms':metadata['summary']['metrics'].get('workload.times_ms.source_delivery'),'regression_assessment':'not_performed'}))
    if any(s['status'] not in ('completed','censored') for s in samples): return 1
    return 2 if metadata['aggregate_censored'] or any(s['status']=='censored' for s in samples) else 0


if __name__ == '__main__':
    raise SystemExit(main())
