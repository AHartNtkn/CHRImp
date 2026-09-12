import {OutputAssembler, IndexedAnswerStore} from '../web/answers.mjs';
const assert=(ok,message)=>{if(!ok)throw new Error(message);};
const equal=(actual,expected,message)=>assert(JSON.stringify(actual)===JSON.stringify(expected),`${message}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
const request = req => new Promise((resolve,reject)=>{req.onsuccess=()=>resolve(req.result);req.onerror=()=>reject(req.error);});
const complete=tx=>new Promise((resolve,reject)=>{tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error??new Error('Aborted'));});
const tables={signatures:[{name:'wide',arity:513},{name:'p',arity:1}],variables:Array.from({length:49},(_,i)=>`X${i}`)};
const factory=name=>({open:(_ignored,version)=>indexedDB.open(name,version)});
async function write(store,collection,stream,event) {if(stream.needsFlush)await store.flush(collection,stream);stream.push(event);}
async function answer(store,collection,stream,facts=40) {
  const push=event=>write(store,collection,stream,event);
  await push({kind:'begin',completion:'19',alternative:'2'});
  for(let i=0;i<49;i++)await push({kind:'variable',slot:i,variable:String(i)});
  for(let f=0;f<facts;f++) {
    await push({kind:'fact',relation:0,occurrence:String(f)});
    for(let p=0;p<513;p++)await push({kind:'port',variable:String(p)});
    await push({kind:'end_fact'});
  }
  await push({kind:'pending_begin',event:'99'});await push({kind:'expression',operator:'and'});
  for(let c=0;c<40;c++) {
    await push({kind:'expression',operator:'or'});
    await push({kind:'expression_relation',relation:0});
    for(let p=0;p<513;p++)await push({kind:'expression_variable',variable:String(p)});
    await push({kind:'expression_end'});await push({kind:'expression_end'});
  }
  await push({kind:'expression_end'});await push({kind:'pending_end'});await push({kind:'end'});
  await store.flush(collection,stream);
}
class FaultStore extends IndexedAnswerStore {
  async transaction(names,mode,work,migrating=false) {
    if(mode==='readwrite' && names.includes('parts') && this.abortNext) {
      this.abortNext=false;
      return super.transaction(names,mode,(tx,result,fail)=>{work(tx,result,fail);tx.abort();},migrating);
    }
    const value=await super.transaction(names,mode,work,migrating);
    if(mode==='readwrite' && names.includes('parts') && this.loseNext) {this.loseNext=false;throw new Error('Lost commit acknowledgement');}
    return value;
  }
}
export async function runChecks(log=()=>{}) {
  const names=[], dbs=[];
  try {
    const name=`chr-answer-check-${crypto.randomUUID()}`;names.push(name);
    const store=new FaultStore(factory(name)); dbs.push(await store.ready);
    const collection=await store.create(tables,'Large scalar answer'), stream=new OutputAssembler(tables,32);
    await answer(store,collection,stream);
    const page=await store.page(collection);
    equal(page.total,1,'Completed answer count');assert(!('tables'in page),'Page cannot include tables');
    equal(page.answers[0].facts,40,'Summary facts');equal(page.answers[0].variables,49,'Summary bindings');
    equal(page.answers[0].pending,1,'Summary pending');assert(!Array.isArray(page.answers[0].facts),'No whole answer');
    const scene=await store.scene(collection,1,{page:1,portPage:63,bindingPage:1});
    equal(scene.facts.entries.length,18,'Bounded fact page');equal(scene.bindings.length,24,'Bounded bindings');equal(scene.bindings[0],{slot:24,name:'X24',variable:'24'},'Raw binding');
    equal(scene.facts.entries[0].args,Array.from({length:8},(_,i)=>`V${504+i}`),'Ordered port window');
    equal(scene.pending.path,[],'Root presentation');equal(scene.pending.scene.entries.length,1,'Single root');
    equal(scene.pending.scene.entries[0].kind,'and','Root syntax');
    const rootPath=scene.pending.scene.entries[0].path;
    const children=await store.scene(collection,1,{pendingPath:rootPath,pendingPage:1});
    equal(children.pending.scene.entries.length,18,'Bounded children');equal(children.pending.scene.count,40,'Full child count');
    const childPath=children.pending.scene.entries[0].path;
    const nested=await store.scene(collection,1,{pendingPath:childPath,pendingPortPage:64});
    equal(nested.pending.scene.entries[0].args,['V512'],'Nested final port page');
    equal(nested.pending.breadcrumbs.at(-1).path,childPath,'Breadcrumb ancestry');
    let invalid=false;try{await store.scene(collection,1,{pendingPath:[999999]});}catch{invalid=true;}assert(invalid,'Invalid ancestry rejected');
    log('Large scalar answer, bounded scenes and ancestry passed');
    // Flush abort and uncertain commit keep identical retryable write batches.
    stream.push({kind:'begin',completion:'20',alternative:'0'});
    stream.push({kind:'variable',slot:0,variable:'7'});
    const queued=JSON.stringify(stream.writes);store.abortNext=true;
    let failed=false;try{await store.flush(collection,stream);}catch{failed=true;}assert(failed,'Injected transaction abort');equal(JSON.stringify(stream.writes),queued,'Abort retains writes');
    store.loseNext=true;failed=false;try{await store.flush(collection,stream);}catch{failed=true;}assert(failed,'Lost commit acknowledgement');equal(JSON.stringify(stream.writes),queued,'Uncertain commit retains writes');
    await store.flush(collection,stream);equal(stream.writes.length,0,'Replay acknowledged');
    await store.discardPartial(collection,stream);equal(stream.current,null,'Discard resets partial');equal((await store.page(collection)).total,1,'Discard preserves completed answers');
    const tx=dbs[0].transaction('parts','readonly');const count=request(tx.objectStore('parts').count(IDBKeyRange.bound([collection,2],[collection,2,[]])));equal(await count,0,'Persisted partial parts gone');
    stream.push({kind:'begin',completion:'21',alternative:'0'});
    stream.push({kind:'variable',slot:0,variable:'8'});
    const nativePut=IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put=function(value,...args){
      if(this.name==='parts' && value.collection===collection && value.number===2)throw new DOMException('No quota for partial writes','QuotaExceededError');
      return nativePut.call(this,value,...args);
    };
    try{await store.discardPartial(collection,stream);}finally{IDBObjectStore.prototype.put=nativePut;}
    equal(stream.current,null,'Quota cannot require saving a discarded partial');
    for(let i=0;i<70;i++)await store.create(tables,`Catalog ${i}`);
    const catalog=await store.list({size:32});equal(catalog.records.length,32,'Bounded catalog');assert(catalog.records.every(row=>!('tables'in row)),'Catalog excludes tables');
    const second=await store.list({cursor:catalog.next,size:32});assert(!second.records.some(row=>catalog.records.some(old=>old.id===row.id)),'Catalog pages disjoint');
    const back=await store.list({cursor:second.prev,direction:'prev',size:32});equal(back.records.map(r=>r.id),catalog.records.map(r=>r.id),'Reverse keyset page');
    log('Native abort/replay, partial discard, catalog paging passed');
    // Build a real version-one database, interrupt its first parts transaction,
    // and reopen. The original row survives; scalar publication is atomic.
    const legacyName=`chr-answer-legacy-${crypto.randomUUID()}`;names.push(legacyName);
    const open=indexedDB.open(legacyName,1);
    open.onupgradeneeded=()=>{open.result.createObjectStore('collections',{keyPath:'id'});open.result.createObjectStore('answers',{keyPath:['collection','number']});};
    const legacy=await request(open);
    const old={collection:'legacy',number:1,completion:'7',alternative:'3',variables:tables.variables.map((_,i)=>String(i)),facts:[{occurrence:'6',relation:0,name:'wide',args:Array.from({length:513},(_,i)=>String(i))}],pending:[{event:'12',body:{kind:'and',items:[{kind:'equal',left:'V7',right:'V8'},{kind:'atom',atom:{relation:'p',args:['V9']}}]}}]};
    const put=legacy.transaction(['collections','answers'],'readwrite');put.objectStore('collections').put({id:'legacy',label:'Old archive',created:1,total:1,tables});put.objectStore('answers').put(old);await complete(put);legacy.close();
    const interrupted=new FaultStore(factory(legacyName));interrupted.abortNext=true;
    failed=false;try{await interrupted.ready;}catch{failed=true;}assert(failed,'Migration interruption');
    const interruptedDb=await interrupted.opened;
    const inspect=interruptedDb.transaction('answers','readonly');equal(await request(inspect.objectStore('answers').get(['legacy',1])),old,'Original remains intact');interruptedDb.close();
    const migrated=new IndexedAnswerStore(factory(legacyName));const migratedDb=await migrated.ready;dbs.push(migratedDb);
    const saved=await migrated.page('legacy');equal(saved.answers[0].format,2,'Summary replaces legacy row');equal(saved.total,1,'Legacy count');
    const legacyScene=await migrated.scene('legacy',1,{portPage:64});equal(legacyScene.facts.entries[0].args,['V512'],'Migrated fact ports');
    const oldChildren=await migrated.scene('legacy',1,{pendingPath:legacyScene.pending.scene.entries[0].path});
    equal(oldChildren.pending.scene.entries.map(e=>[e.kind,e.args]),[['equal',['V7','V8']],['atom',['V9']]],'Migrated pending syntax');
    equal(await migrated.tables('legacy'),tables,'Migrated tables');
    migratedDb.close();
    let cursors=0;const nativeCursor=IDBObjectStore.prototype.openCursor;
    IDBObjectStore.prototype.openCursor=function(...args){cursors++;return nativeCursor.apply(this,args);};
    try {const reopened=new IndexedAnswerStore(factory(legacyName));dbs.push(await reopened.ready);equal(cursors,0,'Completed migration startup has no archive scan');}
    finally{IDBObjectStore.prototype.openCursor=nativeCursor;}
    log('Interrupted v1 migration recovery and constant-work v2 reopen passed');
  } finally {
    dbs.forEach(db=>db.close());
    for(const name of names)await request(indexedDB.deleteDatabase(name));
  }
}
const button=document.getElementById('run'), output=document.getElementById('result');
button.onclick=async()=>{button.disabled=true;output.textContent='Running…';try{await runChecks(message=>{output.textContent+=`\n${message}`;});output.textContent+='\nPASS';}catch(error){output.textContent+=`\nFAIL: ${error.stack??error}`;}finally{button.disabled=false;}};
