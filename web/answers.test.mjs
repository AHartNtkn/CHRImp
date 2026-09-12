import assert from 'node:assert/strict';
import {OutputAssembler, IndexedAnswerStore} from './answers.mjs';

const tables={signatures:[{name:'wide',arity:10000},{name:'empty',arity:0}],variables:['A']};
const assembler=new OutputAssembler(tables,16);
const persisted=new Map();
let fail=false, transactions=0, largestBatch=0;
// A minimal transaction sink exercises the production flush acknowledgement
// boundary. Native IDB queries/upgrade are tested by answers.browser.mjs.
const sink=Object.create(IndexedAnswerStore.prototype);
sink.flushing=new WeakSet();
sink.transaction=async (names, mode, work) => {
  const staged=new Map(persisted); let callbacks=[];
  const tx={abort(){throw new Error('abort');},objectStore(name){return {
    get(key){const request={};callbacks.push(()=>{request.result={id:key,total:0};request.onsuccess();});return request;},
    put(value){staged.set(JSON.stringify([name,value.collection,value.number,value.kind,value.node,value.slot]),structuredClone(value));},
  };}};
  work(tx); callbacks.forEach(run=>run());
  transactions++; if(fail) throw new Error('storage full');
  persisted.clear(); for(const [k,v] of staged) persisted.set(k,v);
};
async function push(event) {if(assembler.needsFlush)await sink.flush('c',assembler);assembler.push(event);largestBatch=Math.max(largestBatch,assembler.writes.length);}
await push({kind:'begin',completion:'18446744073709551615',alternative:'0'});
await push({kind:'variable',slot:0,variable:'7'});
await push({kind:'fact',relation:0,occurrence:'9'});
for(let i=0;i<10000;i++)await push({kind:'port',variable:String(i)});
assert.equal(assembler.fact.ports,10000);
assert.equal('args' in assembler.fact,false);
await push({kind:'end_fact'});
await push({kind:'pending_begin',event:'8'});
await push({kind:'expression',operator:'and'});
for(let i=0;i<10000;i++) {await push({kind:'expression_relation',relation:1});await push({kind:'expression_end'});}
assert.equal(assembler.expressions.length,1);
assert.equal(assembler.expressions[0].childcount,10000);
assert.equal('items' in assembler.expressions[0],false);
await push({kind:'expression_end'});await push({kind:'pending_end'});await push({kind:'end'});
assembler.finish();
assert.equal(assembler.total,1);assert.equal(assembler.answers.length,1);
assert.equal(assembler.answers[0].facts,1);assert.equal(assembler.answers[0].pending,1);
const before=structuredClone(assembler.writes), summaries=structuredClone(assembler.answers), count=persisted.size;
fail=true;await assert.rejects(sink.flush('c',assembler),/storage full/);
assert.deepEqual(assembler.writes,before);assert.deepEqual(assembler.answers,summaries);assert.equal(persisted.size,count);
fail=false;await sink.flush('c',assembler);assert.equal(assembler.writes.length,0);assert.equal(assembler.answers.length,0);
assert.ok(largestBatch<=16);assert.ok(transactions>1000);
const parts=[...persisted.values()].filter(row=>row.kind==='port');assert.equal(parts.length,10000);
assert.equal(parts.at(-1).variable,'9999');
// Backpressure is checked before mutation, and malformed scalar events do not
// require retaining ports or a completed syntax subtree to reject them.
const small=new OutputAssembler({signatures:[{name:'p',arity:1}],variables:[]},4);
small.push({kind:'begin',completion:1,alternative:0});small.push({kind:'fact',relation:0,occurrence:1});
assert.throws(()=>small.push({kind:'end_fact'}),/Missing/);
small.push({kind:'port',variable:7});assert.throws(()=>small.push({kind:'port',variable:8}),/Unexpected/);
small.push({kind:'end_fact'});const snapshot=JSON.stringify(small.current);
assert.throws(()=>small.push({kind:'end'}),/Flush/);assert.equal(JSON.stringify(small.current),snapshot);
assert.throws(()=>small.discardPartial(),/store first/);
// syntax.rs permits 128 containing groups and a leaf at depth 128.
for (const leaf of [{kind:'expression',operator:'true'}, {kind:'expression',operator:'fail'},
  {kind:'expression',operator:'equal'}, {kind:'expression_relation',relation:0}]) {
  const deep=new OutputAssembler({signatures:[{name:'p',arity:1}],variables:[]},512);
  deep.push({kind:'begin',completion:1,alternative:0});deep.push({kind:'pending_begin',event:1});
  for(let i=0;i<128;i++)deep.push({kind:'expression',operator:i%2 ? 'or' : 'and'});
  const before=JSON.stringify([deep.current,deep.pending,deep.expressions,deep.writes]);
  for(let i=0;i<10000;i++)assert.throws(()=>deep.push({kind:'expression',operator:i%2 ? 'or' : 'and'}),/nesting/);
  assert.equal(JSON.stringify([deep.current,deep.pending,deep.expressions,deep.writes]),before);
  deep.push(leaf);assert.equal(deep.expressions.length,129);
  const atLeaf=JSON.stringify([deep.current,deep.pending,deep.expressions,deep.writes]);
  assert.throws(()=>deep.push({kind:'expression',operator:'true'}),/nesting/);
  assert.throws(()=>deep.push({kind:'expression_relation',relation:0}),/nesting/);
  assert.equal(JSON.stringify([deep.current,deep.pending,deep.expressions,deep.writes]),atLeaf);
  const ports=leaf.kind==='expression_relation' ? 1 : leaf.operator==='equal' ? 2 : 0;
  for(let i=0;i<ports;i++)deep.push({kind:'expression_variable',variable:7+i});
  for(let i=0;i<129;i++)deep.push({kind:'expression_end'});
  deep.push({kind:'pending_end'});deep.push({kind:'end'});deep.finish();
  assert.equal(deep.answers[0].nodes,129);
}
console.log('Scalar assembly bounds, full language nesting boundary, flush failure/retry and protocol checks passed.');

const resumeTables={signatures:[{name:'p',arity:2}],variables:[]};
const events=[{kind:'begin',completion:1,alternative:0},{kind:'fact',relation:0,occurrence:2},
  {kind:'port',variable:3},{kind:'port',variable:4},{kind:'end_fact'},
  {kind:'pending_begin',event:5},{kind:'expression',operator:'and'},
  {kind:'expression',operator:'or'},{kind:'expression_relation',relation:0},
  {kind:'expression_variable',variable:6},{kind:'expression_variable',variable:7},
  {kind:'expression_end'},{kind:'expression_end'},{kind:'expression_end'},
  {kind:'pending_end'},{kind:'end'}];
const uninterrupted=new OutputAssembler(resumeTables);events.forEach(e=>uninterrupted.push(e));
for(let split=0;split<=events.length;split++) {
  const initial=new OutputAssembler(resumeTables);events.slice(0,split).forEach(e=>initial.push(e));
  const checkpoint=initial.checkpoint(), prefix=structuredClone(initial.writes);
  const resumed=OutputAssembler.restore(resumeTables,checkpoint);
  assert.deepEqual(resumed.writes,[]);assert.deepEqual(resumed.answers,[]);
  assert.deepEqual(resumed.checkpoint(),checkpoint);
  events.slice(split).forEach(e=>resumed.push(e));resumed.finish();
  assert.deepEqual([...prefix,...resumed.writes],uninterrupted.writes);
  assert.equal(resumed.total,1);
  const discarded=OutputAssembler.restore(resumeTables,initial.checkpoint({discard:true}));
  assert.equal(discarded.current,null);assert.equal(discarded.total,initial.total);
  if(checkpoint.current){checkpoint.current.nodes=999;assert.notEqual(initial.current.nodes,999);}
}
const structural=new OutputAssembler(resumeTables);
structural.push(events[0]);structural.push({kind:'pending_begin',event:1});
for(let i=0;i<128;i++)structural.push({kind:'expression',operator:'and'});
structural.push({kind:'expression',operator:'true'});
const full=structural.checkpoint();assert.equal(full.expressions.length,129);
assert.deepEqual(OutputAssembler.restore(resumeTables,full).checkpoint(),full);
for(const corrupt of [c=>c.expressions.push(c.expressions.at(-1)),c=>c.expressions[0].kind='atom',
  c=>c.expressions[1].parent=999,c=>c.current.nodes=1,c=>c.current.number=8,
  c=>c.expressions[0].ports=1,c=>{delete c.expressions[128]},c=>c.fact={},c=>c.writes=[{}]]) {
  const bad=structuredClone(full);corrupt(bad);assert.throws(()=>OutputAssembler.restore(resumeTables,bad));
}
console.log('Checkpoint split-point equivalence, discard, depth and validation checks passed.');
