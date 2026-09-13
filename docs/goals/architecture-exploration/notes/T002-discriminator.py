import sys,json,time,statistics
from pathlib import Path
sys.path.insert(0,'/home/ahart/Documents/CHRImp/examples')
from supervise import run
root=Path('/home/ahart/Documents/CHRImp')
out=Path('/tmp/chr-architecture-discriminator');out.mkdir(exist_ok=True)
program=out/'join.chr';program.write_text('r(X,Y),s(Y,Z),t(U),v(U) ==> hit(X,Z,U).\n')
control=out/'control.chr';control.write_text('p(X) <=> q(X).\n')
records=[];started=time.monotonic()
def sample(n,kind,rep):
    path=out/f'{kind}-{n}-{rep}'
    if kind=='control':
        src=control;query=','.join(f'p(A{i})' for i in range(32));expected={'q':32}
    else:
        src=program
        query=','.join(a for i in range(n) for a in (f'r(A{i},B)',f's(B,C{i})',f't(D{i})',f'v({"D0" if kind=="hit" and i==0 else "E"+str(i)})'))
        expected={'r':n,'s':n,'t':n,'v':n}
        if kind=='hit':expected['hit']=n*n
    with path.with_suffix('.stdout').open('w') as stdout,path.with_suffix('.stderr').open('w') as stderr:
        r=run([str(root/'target/release/chr'),str(src),'--query',query],str(root),stdout,stderr,seconds=3,memory_mib=1024,grace=.1)
    r.update(n=n,kind=kind,rep=rep)
    if r['status']=='completed':
        events=[json.loads(x) for x in path.with_suffix('.stdout').read_text().splitlines()]
        sigs=events[0]['signatures']; counts={}
        for e in events:
            if e['kind']=='fact':
                name=sigs[e['relation']]['name'];counts[name]=counts.get(name,0)+1
        assert counts==expected,(kind,n,counts,expected)
        assert sum(e['kind']=='begin' for e in events)==1
        assert sum(e['kind']=='end' for e in events)==1
        r['oracle']='one complete answer with expected relation multiplicities'
    records.append(r)
    (out/'results.json').write_text(json.dumps(records,indent=2))
    print(kind,n,rep,r['status'],round(1000*r['wall_seconds'],2),flush=True)
for rep in range(5):
    sample(32,'control',rep)
    for n in (8,16,32,64):
        if time.monotonic()-started>105:break
        sample(n,'empty',rep)
    for n in (8,16):
        if time.monotonic()-started>110:break
        sample(n,'hit',rep)
print('TOTAL',time.monotonic()-started,flush=True)
for kind,n in sorted(set((r['kind'],r['n']) for r in records)):
    rs=[r for r in records if r['kind']==kind and r['n']==n]
    print(kind,n,'completed',sum(r['status']=='completed' for r in rs),'/',len(rs),'median_ms',statistics.median(r['wall_seconds']*1000 for r in rs))
