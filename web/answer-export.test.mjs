import assert from 'node:assert/strict';
import {test} from 'node:test';
import {IndexedAnswerStore, OutputAssembler} from './answers.mjs';

// The existing memory sinks are private to their tests and do not implement reads.
// Emulate only asynchronous IDB reads; run production assembly and read queries.
function savedAnswers(tables) {
  const data = {collections:[{id:'c',total:0}], tables:[{collection:'c',tables}], answers:[], parts:[]};
  const reads = [];
  const key = (name, row) => name === 'parts' ? [row.collection,row.number,row.kind,row.node,row.slot]
    : name === 'answers' ? [row.collection,row.number] : name === 'tables' ? row.collection : row.id;
  const compare = (a,b) => {
    if (Array.isArray(a) && Array.isArray(b)) {
      for (let i=0;i<Math.min(a.length,b.length);i++) { const order=compare(a[i],b[i]); if(order)return order; }
      return a.length-b.length;
    }
    return a < b ? -1 : a > b ? 1 : 0;
  };
  const store = Object.create(IndexedAnswerStore.prototype);
  store.ranges = {bound:(lower,upper,lowerOpen=false)=>({lower,upper,lowerOpen})};
  store.transaction = (names, mode, work) => new Promise((resolve,reject) => {
    assert.equal(mode,'readonly');
    const request = fn => {
      const req={};
      queueMicrotask(()=>{try { req.result=structuredClone(fn()); req.onsuccess(); } catch(error) { reject(error); }});
      return req;
    };
    const tx={objectStore(name) {
      assert.ok(names.includes(name));
      return {
        get(k) { return request(()=>data[name].find(row=>compare(key(name,row),k)===0)); },
        getAll(range,limit) {
          assert.ok(limit > 0 && limit <= (name==='parts' ? 256 : 64), 'reads must be bounded');
          return request(()=>{
            const rows=data[name].filter(row=>{
              const k=key(name,row), lower=compare(k,range.lower);
              return (range.lowerOpen ? lower>0 : lower>=0) && compare(k,range.upper)<=0;
            }).sort((a,b)=>compare(key(name,a),key(name,b))).slice(0,limit);
            reads.push({name,range,rows:structuredClone(rows)});
            return rows;
          });
        },
      };
    }};
    try { work(tx,resolve,reject); } catch(error) { reject(error); }
  });
  function add(events, complete=true) {
    const assembler=new OutputAssembler(tables,16); assembler.total=data.collections[0].total;
    const flush=()=>{for(const {store:name,value} of assembler.writes)data[name].push({...value,collection:'c'});assembler.writes=[];};
    for(const event of events){if(assembler.needsFlush)flush();assembler.push(event);}
    if(complete)assembler.finish();
    flush(); data.collections[0].total=assembler.total;
  }
  return {store,data,reads,add};
}

const begin={kind:'begin',completion:'18446744073709551615',alternative:'17'};
const tables={variables:Array.from({length:300},(_,i)=>`Q${i}`),signatures:[{name:'wide',arity:600},{name:'zero',arity:0}]};
function* richAnswer() {
  yield begin;
  for(let slot=0;slot<300;slot++)yield {kind:'variable',slot,variable:slot===299 ? '18446744073709551615' : String(slot%7)};
  yield {kind:'fact',relation:0,occurrence:'18446744073709551615'};
  for(let i=0;i<600;i++)yield {kind:'port',variable:String(600-i)};
  yield {kind:'end_fact'};
  yield {kind:'fact',relation:1,occurrence:'2'};yield {kind:'end_fact'};
  yield {kind:'pending_begin',event:'18446744073709551615'};
  yield {kind:'expression',operator:'or'};
  yield {kind:'expression',operator:'and'};
  yield {kind:'expression',operator:'equal'};
  yield {kind:'expression_variable',variable:'4'};yield {kind:'expression_variable',variable:'9'};
  yield {kind:'expression_end'};
  yield {kind:'expression_relation',relation:1};yield {kind:'expression_end'};
  yield {kind:'expression_end'};
  yield {kind:'expression',operator:'fail'};yield {kind:'expression_end'};
  yield {kind:'expression_end'};yield {kind:'pending_end'};
  yield {kind:'pending_begin',event:'2'};
  yield {kind:'expression',operator:'true'};yield {kind:'expression_end'};yield {kind:'pending_end'};
  yield {kind:'end'};
}

test('exports complete bindings, ordered facts and every pending alternative in one part traversal', async()=>{
  const {store,reads,add}=savedAnswers(tables);add(richAnswer());
  assert.equal(typeof store.exportAnswer,'function');
  const answer=await store.exportAnswer('c',1);
  assert.deepEqual(JSON.parse(JSON.stringify(answer)),answer);
  assert.equal(answer.number,1);assert.equal(answer.completion,begin.completion);assert.equal(answer.alternative,'17');
  assert.equal(answer.bindings.length,300);
  assert.deepEqual(answer.bindings[299],{slot:299,name:'Q299',variable:'18446744073709551615'});
  assert.equal(answer.bindings[7].variable,'0');
  assert.equal(answer.facts.length,2);assert.equal(answer.facts[0].relation,'wide');
  assert.equal(answer.facts[0].occurrence,'18446744073709551615');
  assert.deepEqual(answer.facts[0].args,Array.from({length:600},(_,i)=>`V${600-i}`));
  assert.equal(answer.facts[1].relation,'zero');assert.deepEqual(answer.facts[1].args,[]);
  assert.equal(answer.pending.length,2);assert.equal(answer.pending[0].event,begin.completion);
  const alternatives=answer.pending[0].scene;
  assert.equal(alternatives.kind,'or');assert.deepEqual(alternatives.children.map(n=>n.kind),['and','fail']);
  assert.deepEqual(alternatives.children[0].children.map(n=>n.kind),['equal','atom']);
  assert.deepEqual(alternatives.children[0].children[0].args,['V4','V9']);
  assert.equal(alternatives.children[0].children[1].relation,'zero');
  assert.equal(answer.pending[1].event,'2');assert.equal(answer.pending[1].scene.kind,'true');
  const readParts=reads.filter(r=>r.name==='parts').flatMap(r=>r.rows);
  assert.equal(new Set(readParts.map(r=>JSON.stringify([r.kind,r.node,r.slot]))).size,readParts.length,'no part is read twice');
  const scene=await store.scene('c',1,{bindingPage:12,pendingNumber:1});
  assert.deepEqual(scene.bindings,answer.bindings.slice(288));assert.equal(scene.bindingPages,13);
  assert.deepEqual(scene.facts.children,answer.facts);assert.deepEqual(scene.pending.scene,answer.pending[1].scene);
  answer.facts[0].args[0]='changed';assert.equal((await store.exportAnswer('c',1)).facts[0].args[0],'V600');
});

test('missing, partial, invalid and damaged answers preserve read errors',async()=>{
  const {store,data,add}=savedAnswers({variables:[],signatures:[]});
  assert.equal(await store.exportAnswer('missing',1),null);
  add([begin],false);assert.equal(await store.exportAnswer('c',1),null);
  for(const number of [-1,1.5,'1',NaN,Number.MAX_SAFE_INTEGER+1])
    await assert.rejects(async()=>store.exportAnswer('c',number),/nonnegative safe integer/);
  add([begin,{kind:'end'}]);
  data.answers[0].format=1;await assert.rejects(store.exportAnswer('c',1),/migration/);
  data.answers[0].format=2;
  data.parts.push({collection:'c',number:1,kind:'fact',node:0,slot:0,target:999});
  await assert.rejects(store.exportAnswer('c',1),/Missing saved fact/);
  store.transaction=async()=>{throw new Error('storage unavailable');};
  await assert.rejects(store.exportAnswer('c',1),/storage unavailable/);
});

test('iteration reads one full answer at a time and fixes the total on first next',async()=>{
  const {store,data,reads,add}=savedAnswers({variables:[],signatures:[]});
  for(let i=0;i<65;i++)add([begin,{kind:'end'}]);
  const iterator=store.iterateAnswers('c');assert.equal(reads.length,0);
  const first=await iterator.next();assert.equal(first.value.number,1);
  assert.deepEqual(first.value.bindings,[]);assert.deepEqual(first.value.facts,[]);assert.deepEqual(first.value.pending,[]);
  assert.ok(reads.filter(r=>r.name==='parts').every(r=>r.range.lower[1]===1));
  add([begin,{kind:'end'}]);
  const numbers=[first.value.number];for await(const answer of iterator)numbers.push(answer.number);
  assert.deepEqual(numbers,Array.from({length:65},(_,i)=>i+1));
  const missing=[];for await(const answer of store.iterateAnswers('absent'))missing.push(answer);assert.deepEqual(missing,[]);
  const interrupted=store.iterateAnswers('c');await interrupted.next();data.answers=data.answers.filter(row=>row.number!==2);
  await assert.rejects(interrupted.next(),/missing/i);
  data.collections[0].total=0;
  for await(const answer of store.iterateAnswers('c'))assert.fail('empty collection yielded an answer');
});

test('exports exact part batches and expression children across batch boundaries',async()=>{
  const {store,add}=savedAnswers({variables:[],signatures:[{name:'p',arity:0}]});
  function* events() {
    yield begin;
    for(let i=0;i<256;i++){yield {kind:'fact',relation:0,occurrence:String(i)};yield {kind:'end_fact'};}
    for(let i=0;i<257;i++){
      yield {kind:'pending_begin',event:String(i)};
      yield {kind:'expression',operator:'and'};
      if(i===0)for(let j=0;j<257;j++){
        yield {kind:'expression',operator:j%2 ? 'fail' : 'true'};yield {kind:'expression_end'};
      }
      yield {kind:'expression_end'};yield {kind:'pending_end'};
    }
    yield {kind:'end'};
  }
  add(events());
  const answer=await store.exportAnswer('c',1);
  assert.deepEqual(answer.facts.map(f=>f.occurrence),Array.from({length:256},(_,i)=>String(i)));
  assert.deepEqual(answer.pending.map(p=>p.event),Array.from({length:257},(_,i)=>String(i)));
  assert.deepEqual(answer.pending[0].scene.children.map(c=>c.kind),Array.from({length:257},(_,i)=>i%2 ? 'fail' : 'true'));
  assert.deepEqual(answer.pending[256].scene.children,[]);
});
