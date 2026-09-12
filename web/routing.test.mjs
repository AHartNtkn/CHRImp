import assert from 'node:assert/strict';
import {diagramScene,layoutScene} from './graph.mjs';
const atom=(relation,...args)=>({kind:'atom',atom:{relation,args}});
const scene=items=>diagramScene({query:{kind:'and',items}},['query']);
const key=p=>p.join(',');
function verify(layout) {
  const wires=layout.items.filter(i=>i.type==='wire'),nodes=layout.items.filter(i=>i.type==='node');
  const segments=wires.flatMap(w=>w.points.slice(1).map((p,i)=>({a:w.points[i],b:p,name:w.name})));
  for(const {a,b} of segments) {
    assert.ok(a[0]===b[0]||a[1]===b[1],'Orthogonal route');
    for(const n of nodes) {
      const crosses=a[0]===b[0]?a[0]>n.x&&a[0]<n.x+n.width&&Math.max(a[1],b[1])>n.y&&Math.min(a[1],b[1])<n.y+n.height:
        a[1]>n.y&&a[1]<n.y+n.height&&Math.max(a[0],b[0])>n.x&&Math.min(a[0],b[0])<n.x+n.width;
      assert.ok(!crosses,'Wire avoids relation interior');
    }
  }
  for(let i=0;i<segments.length;i++)for(const s of segments.slice(i+1)) {
    const t=segments[i];if(s.name===t.name)continue;
    for(const axis of [0,1])if(s.a[axis]===s.b[axis]&&t.a[axis]===t.b[axis]&&s.a[axis]===t.a[axis]) {
      const other=1-axis;
      assert.ok(Math.min(Math.max(s.a[other],s.b[other]),Math.max(t.a[other],t.b[other]))<=Math.max(Math.min(s.a[other],s.b[other]),Math.min(t.a[other],t.b[other])),`Different variables never share a segment: ${JSON.stringify([s,t])}`);
    }
  }
  const names=new Set(layout.items.filter(i=>i.type==='port').map(p=>p.name));
  for(const name of names) {
    const terminals=layout.items.filter(i=>i.type==='port'&&i.name===name),edges=wires.filter(w=>w.name===name);
    if(terminals.length===1){assert.equal(edges.length,0);continue;}
    const graph=new Map();
    for(const w of edges)for(const [a,b] of [[key(w.points[0]),key(w.points.at(-1))],[key(w.points.at(-1)),key(w.points[0])]]){if(!graph.has(a))graph.set(a,[]);graph.get(a).push(b);}
    const seen=new Set(),stack=[key([terminals[0].x+10,terminals[0].y+10])];
    while(stack.length){const p=stack.pop();if(seen.has(p))continue;seen.add(p);stack.push(...(graph.get(p)??[]));}
    for(const p of terminals)assert.ok(seen.has(key([p.x+10,p.y+10])),'Every terminal belongs to one connected tree');
    assert.equal(edges.length,graph.size-1,'Tree has no cycles');
    for(const j of layout.items.filter(i=>i.type==='junction'&&i.name===name))assert.ok(graph.get(key([j.x+24,j.y+12])).length>=3,'Dots only at branches');
  }
}
const pair=layoutScene(scene([atom('p','X'),atom('q','X')]));verify(pair);
assert.equal(pair.items.filter(i=>i.type==='wire').length,1);
assert.equal(pair.items.filter(i=>i.type==='junction').length,0);
const connected=scene(Array.from({length:12},(_,i)=>atom(`p${i}`,'X','Y','Z')));
const many=layoutScene(connected);verify(many);
assert.ok(many.items.filter(i=>i.type==='junction'&&i.name==='X').length>1);
assert.deepEqual(layoutScene(connected),many,'Identical inputs route deterministically');
verify(layoutScene(connected,new Map([[JSON.stringify(['query','items',5]),{x:17,y:33}]])));
// A relation between two terminals is an obstacle, even when its own port is unconnected.
const obstruction=scene([atom('a','X'),atom('b','X'),{kind:'equal',left:'U',right:'V'}]);
verify(layoutScene(obstruction,new Map([[JSON.stringify(['query','items',2]),{x:-300,y:75}]])));
verify(layoutScene(scene(Array.from({length:20},()=>atom('wide',...Array.from({length:10},(_,i)=>`V${i}`))))));
verify(layoutScene(scene([atom('repeated',...Array(30).fill('X'))])));
console.log('Connected acyclic trees, direct pairs, obstacle avoidance, distinct nets and moved nodes passed.');
