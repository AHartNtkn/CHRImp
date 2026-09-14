"""Audit retained per-binary test results, including corrected environment runs."""
import json
from pathlib import Path
import re
from evaluate import HERE, binaries

summary={}
for side in ['baseline','candidate','ordinary']:
    results=[]
    for a in binaries(side):
        name=a['target']['name']
        if side=='baseline': label='baseline-test-'+name
        elif side=='ordinary': label='ordinary-loopback-'+name if name in ['cli','notebook'] else 'ordinary-'+Path(a['executable']).name
        elif name=='chr': label='candidate-unit-'+Path(a['executable']).name
        elif name=='shared_restrictions': label='candidate-focused-final'
        elif name=='cli': label='candidate-loopback-restored-cli'
        elif name=='notebook': label='candidate-loopback-notebook'
        else: label='candidate-test-'+name
        path=HERE/'logs'/(label+'.log')
        lines=path.read_text().splitlines()
        meta=json.loads(lines[0])
        assert meta['exit']==0,(side,name)
        assert meta['command'][:3]==['timeout','--kill-after=5s','60s']
        match=re.search(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored', '\n'.join(lines))
        assert match,(side,name)
        results.append(dict(binary=a['executable'],log=path.name,passed=int(match[1]),ignored=int(match[2])))
    summary[side]=dict(passed=sum(r['passed'] for r in results),ignored=sum(r['ignored'] for r in results),binaries=results)
    print(side,summary[side]['passed'],'passed;',summary[side]['ignored'],'ignored')
(HERE/'verification.json').write_text(json.dumps(summary,indent=2)+'\n')
