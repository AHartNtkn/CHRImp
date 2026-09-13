import assert from 'node:assert/strict';
import {diagramScene, layoutScene} from './graph.mjs';
const atom=(relation,...args)=>({kind:'atom',atom:{relation,args}});
const model={query:{kind:'and',items:[atom('p','X'),atom('q','X')]}};
const before=structuredClone(model),scene=diagramScene(model,['query']);
for(const positions of [new Map(),new Map([['["query","items",1]',{x:-140,y:180}]])]) {
  const layout=layoutScene(scene,positions),nodes=layout.items.filter(i=>i.type==='node'),ports=layout.items.filter(i=>i.type==='port');
  assert.equal(layout.routingError,null);
  for(const [i,n] of nodes.entries()) {
    assert.equal(n.width,n.height,'Circular node geometry');
    assert.ok(n.width<=64,'Short unary relations stay compact');
    const p=ports[i],dx=p.x+10-n.x-n.width/2,dy=p.y+10-n.y-n.height/2;
    assert.ok(Math.abs(Math.hypot(dx,dy)-n.width/2)<1e-6,'Port lies on the visible rim');
    const other=nodes[1-i],tx=other.x+other.width/2-n.x-n.width/2,ty=other.y+other.height/2-n.y-n.height/2;
    assert.ok((dx*tx+dy*ty)/Math.hypot(dx,dy)/Math.hypot(tx,ty)>.99,'Unary port faces its connection, including after movement');
  }
}
for(const arity of [0,1,2,3,10,30]) {
  const layout=layoutScene(diagramScene({query:atom('relation',...Array.from({length:arity},(_,i)=>`V${i}`))},['query']));
  const n=layout.items.find(i=>i.type==='node'),ports=layout.items.filter(i=>i.type==='port');
  assert.equal(ports.length,arity);
  for(const [i,p] of ports.entries()) {
    assert.equal(p.port,i);assert.equal(p.name,`V${i}`);
    assert.ok(Math.abs(Math.hypot(p.x+10-n.x-n.width/2,p.y+10-n.y-n.height/2)-n.width/2)<1e-6);
    if(arity>1) {
      const q=ports[(i+1)%arity];
      assert.ok(Math.hypot(p.x-q.x,p.y-q.y)>=20,'Port targets do not overlap');
      const angle=p=>Math.atan2(p.y+10-n.y-n.height/2,p.x+10-n.x-n.width/2);
      assert.ok(Math.abs((angle(q)-angle(p)+2*Math.PI)%(2*Math.PI)-2*Math.PI/arity)<1e-6,'Argument order runs clockwise');
    }
  }
}
assert.deepEqual(model,before,'Geometry never changes argument semantics');
console.log('Compact discs, perimeter ports, clockwise order and connection-facing rotation passed.');

// Real notebooks exercise cross-compartment ports and deeply nested alternatives.
const {readdirSync,readFileSync}=await import('node:fs');
function checkDiagram(scene,description) {
  const layout=layoutScene(scene);
  assert.equal(layout.routingError,null,description);
  const discs=layout.items.filter(i=>i.type==='node');
  for(const wire of layout.items.filter(i=>i.type==='wire'))for(let i=1;i<wire.points.length;i++) {
    const a=wire.points[i-1],b=wire.points[i],dx=b[0]-a[0],dy=b[1]-a[1];
    for(const n of discs) {
      const cx=n.x+n.radius,cy=n.y+n.radius,t=Math.max(0,Math.min(1,((cx-a[0])*dx+(cy-a[1])*dy)/(dx*dx+dy*dy||1)));
      assert.ok(Math.hypot(a[0]+t*dx-cx,a[1]+t*dy-cy)>=n.radius-1e-6,`${description}: wire clears ${n.label}`);
    }
  }
}
for(const file of readdirSync(new URL('../examples/',import.meta.url)).filter(n=>n.endsWith('.chrnb'))) {
  const notebook=JSON.parse(readFileSync(new URL(`../examples/${file}`,import.meta.url)));
  for(let i=0;i<notebook.program.rules.length;i++)checkDiagram(diagramScene(notebook,['program','rules',i]),`${file}, rule ${i}`);
  for(const query of notebook.queries)checkDiagram(diagramScene({query:query.body},['query']),`${file}, ${query.name}`);
}
