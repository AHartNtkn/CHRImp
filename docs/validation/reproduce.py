#!/usr/bin/env python3
"""Extract exact local git revisions and run the offline comparison harness."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

CURRENT = '294c010ebab311cf652d282398a3a2e38f85d4a5'
OLDER = '4ed9c045dc4eccfca58fb25e39a4f13f46a1a223'
HERE = Path(__file__).resolve().parent


def archive(repo, revision, destination):
    actual = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', revision + '^{commit}'], text=True).strip()
    if actual != revision:
        raise RuntimeError(f'Unexpected revision: {actual}')
    destination.mkdir()
    producer = subprocess.Popen(['git', '-C', str(repo), 'archive', revision], stdout=subprocess.PIPE)
    try:
        subprocess.run(['tar', '-x', '-C', str(destination)], stdin=producer.stdout, check=True)
    finally:
        producer.stdout.close()
        status = producer.wait()
    if status:
        raise RuntimeError(f'git archive failed: {status}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--older-repo', type=Path, required=True, help='Local CHRLang repository containing the frozen older commit')
    parser.add_argument('--current-repo', type=Path, default=HERE.parents[1])
    parser.add_argument('--output', type=Path, help='New directory; defaults to a system temporary directory (honors TMPDIR)')
    parser.add_argument('--cpu', type=int, help='Optional Linux CPU affinity; historical measurement used CPU10')
    parser.add_argument('--full', action='store_true', help='Run the full historical matrix and fresh preparation samples; default is a small semantic smoke run')
    args = parser.parse_args()
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    if args.output:
        out = args.output.resolve()
        out.mkdir(parents=True, exist_ok=False)
    else:
        out = Path(tempfile.mkdtemp(prefix='chr-validation-'))
    print(f'Output: {out}', flush=True)
    archive(args.current_repo, CURRENT, out/'current')
    archive(args.older_repo, OLDER, out/'older')
    for name in ['ordinary', 'search']:
        shutil.copytree(HERE/'harness'/name, out/name)
    env = dict(os.environ)
    for key in ['TRACE_WORKCHECK', 'SKIP_GLOBAL', 'WIDE_DIAGNOSTIC', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR']:
        env.pop(key, None)
    (out/'metadata.json').write_text(json.dumps({'current':CURRENT, 'older':OLDER,
        'cpu_affinity':sorted(os.sched_getaffinity(0)) if hasattr(os,'sched_getaffinity') else None,
        'rustc':subprocess.check_output(['rustc','-Vv'],text=True), 'full':args.full}, indent=2))

    def run(command, stdout, stderr=None, extra=None):
        with (out/stdout).open('w') as output:
            if stderr:
                with (out/stderr).open('w') as errors:
                    subprocess.run(list(map(str,command)), env=env | (extra or {}), stdout=output, stderr=errors, check=True)
            else:
                subprocess.run(list(map(str,command)), env=env | (extra or {}), stdout=output, stderr=subprocess.STDOUT, check=True)

    for name in ['ordinary','search']:
        run(['cargo','build','--offline','--locked','--release','--bins','--manifest-path',out/name/'Cargo.toml'], name+'/build.log')
    ordinary=out/'ordinary/target/release'
    search=out/'search/target/release'
    for binary, saved in [('chr-search-qualification','default-runner'),('preparation','default-preparation')]:
        shutil.copy2(search/binary,out/'search'/saved)
    run(['cargo','build','--offline','--locked','--release','--bins','--features','cow','--manifest-path',out/'search/Cargo.toml'], 'search/build-cow.log')
    for binary, saved in [('chr-search-qualification','cow-runner'),('preparation','cow-preparation')]:
        shutil.copy2(search/binary,out/'search'/saved)
    run([ordinary/'qualify'],'ordinary/qualification.log')
    if not args.full:
        run([ordinary/'regimes','sparse','8','1'],'ordinary/smoke.csv','ordinary/smoke.log',{'SKIP_GLOBAL':'1'})
        for mode in ['default','cow']:
            run([out/'search'/f'{mode}-runner','--wide','--n=8','--reps=1'],f'search/smoke-{mode}.csv',f'search/smoke-{mode}.log')
        print('PASS: ordinary qualification, sparse8, and common-wide8 default/COW semantic runs.', flush=True)
        return
    for mode,reps in [('workchecks',1),('active',21),('global',7)]:
        extra={'TRACE_WORKCHECK':'1'} if mode=='workchecks' else {'SKIP_GLOBAL':'1'} if mode=='active' else {}
        with (out/f'ordinary/{mode}.csv').open('w') as output, (out/f'ordinary/{mode}.log').open('w') as errors:
            first=True
            for case in ['alias','degree','sparse','dense','cyclic']:
                for n in [8,32,128]:
                    result=subprocess.run([str(ordinary/'regimes'),case,str(n),str(reps)],env=env|extra,text=True,stdout=subprocess.PIPE,stderr=errors,check=True)
                    rows=result.stdout.splitlines();output.write('\n'.join(rows if first else rows[1:])+'\n');output.flush();first=False
    for mode in ['default','cow']:
        run([out/'search'/f'{mode}-runner','--all','--reps=21'],f'search/paired-{mode}.csv',f'search/paired-{mode}.log')
    run([ordinary/'preparation'],'ordinary/preparation.csv','ordinary/preparation.log')
    for mode in ['default','cow']:
        run([out/'search'/f'{mode}-preparation'],f'search/preparation-{mode}.csv',f'search/preparation-{mode}.log')
    run(['python3',HERE/'summarize.py','--input',out,'--output',out],'summary-check.log')
    print('PASS: full matrix and table reconstruction.', flush=True)


if __name__ == '__main__':
    main()
