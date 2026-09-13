#!/usr/bin/env python3
"""Native heaptrack allocation stacks, peak-live stacks and residency timeline.

Requires heaptrack's preload library, interpreter and print utility plus inferno.
Ubuntu package layout is selected with --prefix (normally /usr).
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET
from profile import ROOT
from perf import records, classify
from supervise import run


def stage_status(process):
    if not process['group_cleanup_complete']: return 'failed'
    if process['status']=='censored': return 'censored'
    return 'completed' if process['returncode']==0 and not process['descendants_signaled_after_exit'] else 'failed'


def allocation_count(text):
    matches=re.findall(r'^calls to allocation functions: (\d+) ',text,re.M)
    if len(matches)!=1: raise ValueError('missing or ambiguous heaptrack allocation total')
    return int(matches[0])


def heap_stacks(text):
    positive=[];total=0
    for line in text.splitlines():
        stack,weight=line.rsplit(' ',1);weight=int(weight)
        if not stack or weight<0: raise ValueError('invalid heap stack weight')
        total+=weight
        if weight: positive.append(line)
    if total==0: raise ValueError('no positive heap stack weights')
    return '\n'.join(positive)+'\n',total


def timeline(text):
    """Massif export timestamps are heaptrack elapsed seconds; observations sample live heap."""
    points=[]; current={}
    for line in text.splitlines():
        if line.startswith('snapshot='):
            if current:
                if set(current)!={'seconds','heap_bytes'}: raise ValueError('incomplete heap snapshot')
                points.append(current)
            current={}
        elif line.startswith('time='):
            current['seconds']=float(line[5:])
        elif line.startswith('mem_heap_B='):
            current['heap_bytes']=int(line[11:])
    if current:
        if set(current)!={'seconds','heap_bytes'}: raise ValueError('incomplete heap snapshot')
        points.append(current)
    if not points or any(not math.isfinite(p['seconds']) or p['seconds']<0 or p['heap_bytes']<0 for p in points):
        raise ValueError('invalid or empty heap timeline')
    if any(a['seconds']>b['seconds'] for a,b in zip(points,points[1:])): raise ValueError('unordered heap timeline')
    return points


def analyze(out,prefix,seconds):
    environment=os.environ.copy()
    interpreter=prefix/'lib/heaptrack/libexec/heaptrack_interpret'
    printer=prefix/'bin/heaptrack_print'
    result={}
    deadline=time.monotonic()+seconds
    def command_run(command,**kwargs):
        remaining=deadline-time.monotonic()
        if remaining<=0: raise TimeoutError('heap analysis budget exhausted')
        return subprocess.run(command,cwd=ROOT,env=environment,check=True,timeout=remaining,**kwargs)
    with (out/'allocations.raw').open('rb') as source,(out/'heaptrack.txt').open('wb') as target,(out/'interpret.log').open('wb') as log:
        command_run([str(interpreter)],stdin=source,stdout=target,stderr=log)
    for kind,unit in [('allocations','allocation calls'),('peak','requested bytes at peak')]:
        folded=out/(kind+'.folded')
        command=[str(printer),str(out/'heaptrack.txt'),'--merge-backtraces','0','--flamegraph-cost-type',kind,'--print-flamegraph',str(folded)]
        if kind=='allocations': command += ['--print-massif',str(out/'heap.massif')]
        with (out/(kind+'.txt')).open('wb') as report,(out/(kind+'.log')).open('wb') as log:
            command_run(command,stdout=report,stderr=log)
        readable=command_run(['c++filt','-s','rust','-i'],input=folded.read_text(),text=True,capture_output=True).stdout
        readable,weight=heap_stacks(readable)
        folded.write_text(readable)
        if kind=='allocations' and weight!=allocation_count((out/(kind+'.txt')).read_text()):
            raise ValueError('allocation stack weights do not match heaptrack total')
        svg=command_run(['inferno-flamegraph','--title','CHR heap: '+kind,'--countname',unit,'--colors','mem','--deterministic'],input=readable,text=True,capture_output=True).stdout
        if ET.fromstring(svg).tag!='{http://www.w3.org/2000/svg}svg': raise ValueError('invalid heap flame graph')
        (out/(kind+'.svg')).write_text(svg)
        result[kind+'_weight']=weight
    observations=timeline((out/'heap.massif').read_text())
    (out/'timeline.json').write_text(json.dumps(observations)+'\n')
    result.update(profile_validated=True,timeline_snapshots=len(observations),timeline_sampled_peak_bytes=max(p['heap_bytes'] for p in observations),
                    uncertainty='Timeline snapshots can miss short-lived peaks. Peak flame weights describe live allocations at the observed global peak, not a sum of independent per-site maxima. Censored recording may be incomplete; validation establishes readable accounting, not capture completeness.')
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--analyze',action='store_true',help=argparse.SUPPRESS)
    parser.add_argument('--prefix',type=Path,default=Path('/usr'),help='heaptrack installation prefix')
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--cli',action='store_true')
    parser.add_argument('--binary',type=Path,help='prebuilt native binary with readable symbols')
    parser.add_argument('--seconds',type=float,default=40)
    parser.add_argument('--analysis-seconds',type=float,default=120)
    parser.add_argument('--memory-mib',type=int,default=4096)
    parser.add_argument('args',nargs=argparse.REMAINDER)
    args=parser.parse_args();workload=args.args[1:] if args.args[:1]==['--'] else args.args
    if (not workload and not args.analyze) or any(not math.isfinite(v) or v<=0 for v in (args.seconds,args.analysis_seconds)) or args.memory_mib<=0:
        parser.error('workload and positive finite budgets required')
    prefix=args.prefix.resolve()
    library=prefix/'lib/heaptrack/libheaptrack_preload.so'
    interpreter=prefix/'lib/heaptrack/libexec/heaptrack_interpret'
    printer=prefix/'bin/heaptrack_print'
    for path in (library,interpreter,printer):
        if not path.is_file(): parser.error('required heaptrack component unavailable: '+str(path))
    for tool in ('inferno-flamegraph','c++filt'):
        if not shutil.which(tool): parser.error('required executable unavailable: '+tool)
    out=args.out.resolve()
    if args.analyze:
        print(json.dumps(analyze(out,prefix,args.analysis_seconds)));return 0
    out.mkdir(parents=True,exist_ok=False)
    environment=os.environ.copy()
    build=None
    if args.binary: binary=args.binary.resolve()
    else:
        environment['RUSTFLAGS']=(environment.get('RUSTFLAGS','')+' -C force-frame-pointers=yes').strip()
        build=['cargo','build','--offline','--profile','profiling',*(['--bin','chr'] if args.cli else ['--example','measure'])]
        with (out/'build.log').open('w') as log:
            subprocess.run(build,cwd=ROOT,env=environment,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=300)
        target=Path(environment.get('CARGO_TARGET_DIR',ROOT/'target'))
        if not target.is_absolute(): target=ROOT/target
        binary=target/'profiling'/('chr' if args.cli else 'examples/measure')
    command=['env','LD_PRELOAD='+str(library)+(':'+environment['LD_PRELOAD'] if environment.get('LD_PRELOAD') else ''),
             'DUMP_HEAPTRACK_OUTPUT='+str(out/'allocations.raw'),str(binary),*workload]
    metadata=dict(status='recording',profile_validated=False,command=command,build=build,
                  rustflags=environment.get('RUSTFLAGS'),prefix=str(prefix),seconds=args.seconds,
                  analysis_seconds=args.analysis_seconds,
                  scope='Heaptrack-intercepted native allocation calls and requested heap bytes, including harness/validation and loaded-library work. Not engine-exclusive ownership or RSS. Timings are instrumented, not baseline.')
    path=out/'heap.json';path.write_text(json.dumps(metadata,indent=2)+'\n')
    try:
        with (out/'workload.log').open('w') as stdout,(out/'recorder.log').open('w') as stderr:
            process=run(command,ROOT,stdout,stderr,args.seconds,args.memory_mib,grace=5)
        metadata.update(status=process['status'],process_resources=process)
        path.write_text(json.dumps(metadata,indent=2)+'\n')
        if args.cli:
            status=stage_status(process)
        else:
            try: status=classify(process,records((out/'workload.log').read_text()),workload[0])
            except (ValueError,TypeError) as error:
                metadata['report_error']=str(error)
                status='cleanup_failure' if not process['group_cleanup_complete'] else ('censored' if process['status']=='censored' else 'report_error')
        metadata.update(status=status,process_resources=process)
        if status not in ('completed','censored'): raise ValueError('native profiling workload failed; see raw logs')
        analysis_command=[sys.executable,str(Path(__file__).resolve()),'--analyze','--prefix',str(prefix),'--out',str(out),'--analysis-seconds',str(args.analysis_seconds)]
        with (out/'analysis.json').open('w') as stdout,(out/'analysis.log').open('w') as stderr:
            analysis_process=run(analysis_command,ROOT,stdout,stderr,args.analysis_seconds,args.memory_mib)
        metadata['analysis_resources']=analysis_process
        analysis_status=stage_status(analysis_process)
        if analysis_status=='censored':
            metadata['analysis_status']='censored'
            metadata['profile_error']='heap analysis reached an external resource/deadline limit'
        elif analysis_status=='failed':
            metadata['analysis_status']='failed'
            raise ValueError('heap analysis failed; see analysis.log')
        else:
            metadata['analysis_status']='completed'
            metadata.update(json.loads((out/'analysis.json').read_text()))
    except (OSError,ValueError,ET.ParseError,subprocess.SubprocessError) as error:
        metadata.update(profile_error=str(error));path.write_text(json.dumps(metadata,indent=2)+'\n');raise
    path.write_text(json.dumps(metadata,indent=2)+'\n')
    print(json.dumps(dict(metadata=str(path),status=metadata['status'],allocations=metadata.get('allocations_weight'),peak_bytes=metadata.get('peak_weight'))))
    return 2 if metadata['status']=='censored' or metadata.get('analysis_status')=='censored' else 0


if __name__=='__main__': raise SystemExit(main())
