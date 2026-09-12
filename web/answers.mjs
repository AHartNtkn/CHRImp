// Scalar answer storage. Only completed summaries are visible in answer pages.
const check = (ok, message) => { if (!ok) throw new Error(message); };
const uint = value => { check(Number.isSafeInteger(value) && value >= 0, 'Expected a nonnegative safe integer.'); return value; };
const id = value => {
  if (typeof value === 'number') return String(uint(value));
  check(typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value) && BigInt(value) <= 18446744073709551615n, 'Expected an unsigned integer ID.');
  return value;
};
// Match syntax.rs MAX_NESTING: 128 containers, then a leaf at depth 128.
const MAX_NESTING = 128;
const yieldTask = () => new Promise(resolve => setTimeout(resolve, 0));

export class OutputAssembler {
  constructor(tables, capacity = 256) {
    check(Array.isArray(tables?.signatures) && Array.isArray(tables?.variables), 'Missing output tables.');
    for (const signature of tables.signatures) { check(typeof signature.name === 'string', 'Invalid relation name.'); uint(signature.arity); }
    check(tables.variables.every(name => typeof name === 'string'), 'Invalid variable names.');
    check(Number.isSafeInteger(capacity) && capacity >= 4, 'Write capacity must be at least four.');
    this.tables = structuredClone({signatures:tables.signatures, variables:tables.variables});
    this.capacity = capacity; this.writes = []; this.answers = []; this.current = null; this.total = 0;
    this.fact = null; this.pending = null; this.expressions = [];
  }
  checkpoint({discard = false} = {}) {
    return continuation(this.tables, {total:this.total, current:discard ? null : this.current,
      fact:discard ? null : this.fact, pending:discard ? null : this.pending,
      expressions:discard ? [] : this.expressions});
  }
  static restore(tables, checkpoint, capacity = 256) {
    const assembler = new OutputAssembler(tables, capacity);
    Object.assign(assembler, continuation(assembler.tables, checkpoint));
    return assembler;
  }
  get needsFlush() { return this.writes.length > this.capacity - 3; }
  part(kind, node, slot, value) { this.writes.push({store:'parts', value:{number:this.current.number, kind, node, slot, ...value}}); }
  node(frame) {
    const {node, kind, relation, occurrence, arity, childcount, parent, maxChildArity = 0} = frame;
    this.part('node', node, 0, {kindValue:kind, ...(relation === undefined ? {} : {relation}), ...(occurrence === undefined ? {} : {occurrence}), arity, childcount, parent:parent ?? null, maxChildArity});
  }
  signature(relation) { const value = this.tables.signatures[uint(relation)]; check(value, 'Unknown relation index.'); return value; }
  push(event) {
    check(!this.needsFlush, 'Flush pending writes before consuming more output.');
    const current = this.current;
    switch (event.kind) {
      case 'begin': {
        check(!current, 'An alternative is already open.');
        const completion = id(event.completion), alternative = id(event.alternative);
        this.current = {format:2, number:uint(this.total + 1), completion, alternative, variables:0, facts:0, pending:0, nodes:0, maxArity:0};
        break;
      }
      case 'variable': {
        check(current && !this.fact && !this.pending && !current.facts && !current.pending, 'Bindings must precede facts and bodies.');
        check(uint(event.slot) === current.variables && event.slot < this.tables.variables.length, 'Unexpected binding slot.');
        const variable = id(event.variable); this.part('binding', 0, current.variables++, {variable}); break;
      }
      case 'fact': {
        check(current && !this.fact && !this.pending && !current.pending, 'Expected an alternative without an open fact or body.');
        check(current.variables === this.tables.variables.length, 'Missing query variables.');
        const signature = this.signature(event.relation), occurrence = id(event.occurrence);
        this.fact = {node:current.nodes++, kind:'atom', relation:event.relation, occurrence, arity:signature.arity, ports:0, childcount:0}; break;
      }
      case 'port': {
        check(this.fact && this.fact.ports < this.fact.arity, 'Unexpected fact port.');
        const variable = id(event.variable); this.part('port', this.fact.node, this.fact.ports++, {variable}); break;
      }
      case 'end_fact':
        check(this.fact && this.fact.ports === this.fact.arity, 'Missing fact ports.');
        current.maxArity = Math.max(current.maxArity, this.fact.arity);
        this.node(this.fact); this.part('fact', 0, current.facts++, {target:this.fact.node}); this.fact = null; break;
      case 'pending_begin':
        check(current && !this.fact && !this.pending && current.variables === this.tables.variables.length, 'Expected an alternative with complete bindings.');
        this.pending = {event:id(event.event), root:null}; break;
      case 'expression':
        check(['and','or','equal','true','fail'].includes(event.operator), 'Unknown expression.');
        this.expression(event.operator, undefined, event.operator === 'equal' ? 2 : 0); break;
      case 'expression_relation':
        this.expression('atom', event.relation, this.signature(event.relation).arity); break;
      case 'expression_variable': {
        const frame = this.expressions.at(-1);
        check(frame && ['atom','equal'].includes(frame.kind) && frame.ports < frame.arity, 'Unexpected expression port.');
        const variable = id(event.variable); this.part('port', frame.node, frame.ports++, {variable}); break;
      }
      case 'expression_end': {
        const frame = this.expressions.at(-1);
        check(frame && frame.ports === frame.arity, 'Missing expression ports.');
        this.node(frame); this.expressions.pop(); break;
      }
      case 'pending_end':
        check(this.pending?.root !== null && this.pending && !this.expressions.length, 'A pending body must finish exactly one root.');
        this.part('pending', 0, current.pending++, {event:this.pending.event, target:this.pending.root}); this.pending = null; break;
      case 'end':
        check(current && !this.fact && !this.pending && current.variables === this.tables.variables.length, 'Incomplete alternative.');
        this.answers.push({...current}); this.writes.push({store:'answers', value:this.answers.at(-1)}); this.total = current.number; this.current = null; break;
      default: throw new Error(`Unknown output event: ${event.kind}`);
    }
  }
  expression(kind, relation, arity) {
    check(this.pending, 'An expression needs a pending body.');
    const depth = this.expressions.length;
    check(['and','or'].includes(kind) ? depth < MAX_NESTING : depth <= MAX_NESTING, 'Body nesting exceeds 128 containers.');
    const parent = this.expressions.at(-1);
    check(parent ? ['and','or'].includes(parent.kind) : this.pending.root === null, 'Unexpected expression child.');
    const node = this.current.nodes++;
    if (parent) { this.part('child', parent.node, parent.childcount++, {target:node}); parent.maxChildArity = Math.max(parent.maxChildArity ?? 0, arity); } else this.pending.root = node;
    this.expressions.push({node, kind, relation, arity, ports:0, childcount:0, parent:parent?.node});
  }
  discardPartial() {
    if (!this.current) return;
    check(!this.writes.length, 'Discard persisted partial output through the store first.');
    this.current = null; this.fact = null; this.pending = null; this.expressions = [];
  }
  finish() { check(!this.current, 'Output delivery ended inside an alternative.'); }
}

// Validate and copy only the fixed scalar continuation; never traverse saved facts.
function continuation(tables, state) {
  const shape = (value, keys) => {
    check(value && typeof value === 'object' && !Array.isArray(value), 'Invalid checkpoint object.');
    check(Object.keys(value).every(key => keys.includes(key)), 'Unexpected checkpoint field.');
  };
  shape(state, ['total','current','fact','pending','expressions']);
  const total = uint(state.total), current = state.current, fact = state.fact, pending = state.pending;
  check(Array.isArray(state.expressions) && state.expressions.length <= MAX_NESTING + 1, 'Invalid checkpoint nesting.');
  const frames = state.expressions;
  if (current === null) {
    check(fact === null && pending === null && !frames.length, 'Checkpoint has no open alternative.');
    return {total,current:null,fact:null,pending:null,expressions:[]};
  }
  shape(current, ['format','number','completion','alternative','variables','facts','pending','nodes','maxArity']);
  check(current.format === 2 && uint(current.number) === uint(total + 1), 'Invalid checkpoint alternative.');
  id(current.completion); id(current.alternative);
  for (const key of ['variables','facts','pending','nodes','maxArity']) uint(current[key]);
  check(current.variables <= tables.variables.length && current.facts + current.pending <= current.nodes, 'Invalid checkpoint counts.');
  check(!(fact !== null && pending !== null), 'Overlapping checkpoint bodies.');
  if (fact !== null || pending !== null || current.nodes) check(current.variables === tables.variables.length, 'Missing checkpoint bindings.');
  const frame = (value, depth, parent, isFact = false) => {
    shape(value, ['node','kind','relation','occurrence','arity','ports','childcount','parent','maxChildArity']);
    check(uint(value.node) < current.nodes, 'Invalid checkpoint node.');
    const group = ['and','or'].includes(value.kind);
    check(group ? depth < MAX_NESTING : depth <= MAX_NESTING, 'Invalid checkpoint nesting.');
    check(['atom','equal','and','or','true','fail'].includes(value.kind), 'Invalid checkpoint expression.');
    const signature = value.kind === 'atom' ? tables.signatures[uint(value.relation)] : null;
    if (value.kind === 'atom') check(signature, 'Unknown checkpoint relation.');
    else check(value.relation === undefined, 'Unexpected checkpoint relation.');
    check(uint(value.arity) === (signature ? signature.arity : value.kind === 'equal' ? 2 : 0), 'Invalid checkpoint arity.');
    check(uint(value.ports) <= value.arity, 'Invalid checkpoint ports.');
    check(uint(value.childcount) <= current.nodes - value.node - 1 && (group || value.childcount === 0), 'Invalid checkpoint children.');
    if (value.maxChildArity !== undefined) uint(value.maxChildArity);
    check(value.parent === parent, 'Invalid checkpoint parent.');
    if (isFact) { check(value.kind === 'atom', 'Invalid checkpoint fact.'); id(value.occurrence); }
    else check(value.occurrence === undefined, 'Unexpected checkpoint occurrence.');
    return {...value};
  };
  let factCopy = null, pendingCopy = null;
  if (fact !== null) {
    factCopy = frame(fact, 0, undefined, true);
    check(!current.pending && fact.node === current.nodes - 1, 'Invalid open checkpoint fact.');
  }
  if (pending !== null) {
    shape(pending, ['event','root']); id(pending.event);
    if (pending.root !== null) check(uint(pending.root) < current.nodes, 'Invalid checkpoint root.');
    else check(!frames.length, 'Checkpoint stack without root.');
    pendingCopy = {...pending};
  } else check(!frames.length, 'Checkpoint stack without body.');
  const expressions = Array.from(frames, (value, depth) => {
    check(value && typeof value === 'object', 'Invalid checkpoint frame.');
    const parent = frames[depth - 1];
    if (parent) check(['and','or'].includes(parent.kind) && parent.node < value.node && parent.childcount > 0, 'Invalid checkpoint stack.');
    else check(value.node === pending.root, 'Invalid checkpoint stack root.');
    return frame(value, depth, parent?.node);
  });
  check(current.facts + current.pending + (fact ? 1 : 0) + (pending?.root !== null && pending ? 1 : 0) <= current.nodes, 'Invalid checkpoint node counts.');
  return {total,current:{...current},fact:factCopy,pending:pendingCopy,expressions};
}

export class IndexedAnswerStore {
  constructor(indexed = globalThis.indexedDB, ranges = globalThis.IDBKeyRange) {
    this.ranges = ranges; this.flushing = new WeakSet();
    this.opened = new Promise((resolve, reject) => {
      if (!indexed) { reject(new Error('This browser cannot open saved answer storage.')); return; }
      const request = indexed.open('chr-notebook-answers', 3);
      request.onupgradeneeded = event => {
        const db = request.result;
        if (!db.objectStoreNames.contains('collections')) db.createObjectStore('collections', {keyPath:'id'});
        if (!db.objectStoreNames.contains('answers')) db.createObjectStore('answers', {keyPath:['collection','number']});
        if (!db.objectStoreNames.contains('parts')) db.createObjectStore('parts', {keyPath:['collection','number','kind','node','slot']});
        if (!db.objectStoreNames.contains('tables')) db.createObjectStore('tables', {keyPath:'collection'});
        if (!db.objectStoreNames.contains('recovery')) db.createObjectStore('recovery', {keyPath:'id'});
        const recovery = request.transaction.objectStore('recovery');
        if (!recovery.indexNames.contains('archive')) recovery.createIndex('archive', 'value.archive');
        if (!db.objectStoreNames.contains('migration')) db.createObjectStore('migration', {keyPath:'id'});
        if (event.oldVersion === 1) request.transaction.objectStore('migration').put({id:'v1',phase:'collections',after:null});
        const catalog = request.transaction.objectStore('collections');
        if (!catalog.indexNames.contains('created')) catalog.createIndex('created', ['created','id']);
      };
      request.onsuccess = () => { request.result.onversionchange = () => request.result.close(); resolve(request.result); };
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error('Close other notebook tabs to upgrade saved answer storage.'));
    });
    this.ready = this.opened.then(async db => { await this.migrate(); return db; });
    this.ready.catch(() => {});
  }
  async transaction(names, mode, work, migrating = false) {
    const db = await (migrating ? this.opened : this.ready);
    return new Promise((resolve, reject) => {
      const tx = db.transaction(names, mode); let value;
      tx.oncomplete = () => resolve(value);
      tx.onabort = () => reject(tx.error ?? new Error('Saved answer transaction aborted.'));
      tx.onerror = () => {};
      try { work(tx, result => { value = result; }, error => { tx.abort(); reject(error); }); } catch (error) { tx.abort(); reject(error); }
    });
  }
  recovery(id) {
    check(typeof id === 'string', 'Invalid recovery ID.');
    return this.transaction(['recovery'], 'readonly', (tx, result) => {
      const request = tx.objectStore('recovery').get(id);
      request.onsuccess = () => result(request.result?.value ?? null);
    });
  }
  saveRecovery(id, value) {
    check(typeof id === 'string', 'Invalid recovery ID.');
    return this.transaction(['recovery'], 'readwrite', tx => {
      const store = tx.objectStore('recovery');
      if (value === null) store.delete(id); else store.put({id,value});
    });
  }
  recoveryPage(prefix, after = null, size = 32) {
    check(typeof prefix === 'string' && (after === null || typeof after === 'string' && after.startsWith(prefix)), 'Invalid recovery cursor.');
    uint(size); check(size > 0 && size <= 64, 'Recovery page size must be 1..64.');
    return this.transaction(['recovery'], 'readonly', (tx, result) => {
      const records = [];
      const request = tx.objectStore('recovery').openCursor(this.ranges.lowerBound(after ?? prefix, after !== null));
      request.onsuccess = () => {
        const item = request.result, more = item && typeof item.key === 'string' && item.key.startsWith(prefix);
        if (!more || records.length === size) { result({records,next:more ? records.at(-1).id : null}); return; }
        records.push(item.value); item.continue();
      };
    });
  }
  async create(tables, label, recovery = null) {
    if (recovery) check(typeof recovery.id === 'string' && recovery.value && typeof recovery.value === 'object', 'Invalid recovery record.');
    const collection = crypto.randomUUID();
    return this.transaction(['collections','tables', ...(recovery ? ['recovery'] : [])], 'readwrite', (tx, result, fail) => {
      const create = () => {
        tx.objectStore('collections').add({id:collection, label, total:0, created:Date.now()});
        tx.objectStore('tables').add({collection, tables:structuredClone(tables)});
        if (recovery) tx.objectStore('recovery').put({id:recovery.id,value:{...recovery.value,archive:collection}});
        result(collection);
      };
      if (!recovery) { create(); return; }
      const request = tx.objectStore('recovery').get(recovery.id);
      request.onsuccess = () => {
        if (!request.result) { create(); return; }
        const archive = request.result.value?.archive;
        if (typeof archive !== 'string') { fail(new Error('Recovery record has no archive.')); return; }
        const catalog = tx.objectStore('collections').get(archive), tables = tx.objectStore('tables').get(archive);
        catalog.onsuccess = () => { if (!catalog.result) fail(new Error('Recovery archive is missing.')); };
        tables.onsuccess = () => { if (!tables.result) fail(new Error('Recovery archive tables are missing.')); else result(archive); };
      };
    });
  }
  async flush(collection, assembler, {migrating = false, discard = false, recovery = null} = {}) {
    check(!this.flushing.has(assembler), 'Output flush already in progress.');
    const discardNumber = discard ? assembler.current?.number : undefined;
    if (!assembler.writes.length && discardNumber === undefined && !recovery) return;
    if (recovery) check(typeof recovery.id === 'string' && recovery.value && typeof recovery.value === 'object', 'Invalid recovery record.');
    const savedRecovery = recovery ? {id:recovery.id,value:structuredClone({...recovery.value,assembler:assembler.checkpoint({discard})})} : null;
    this.flushing.add(assembler);
    const batch = assembler.writes.slice();
    try {
      await this.transaction(['collections','answers','parts', ...(recovery ? ['recovery'] : [])], 'readwrite', tx => {
        if (savedRecovery) tx.objectStore('recovery').put(savedRecovery);
        const catalog = tx.objectStore('collections'), request = catalog.get(collection);
        request.onsuccess = () => {
          if (!request.result) { tx.abort(); return; }
          let total = request.result.total;
          for (const record of batch) {
            if (record.value.number === discardNumber) continue;
            const value = {...record.value, collection};
            tx.objectStore(record.store).put(value);
            if (record.store === 'answers') total = Math.max(total, value.number);
          }
          if (discardNumber !== undefined) tx.objectStore('parts').delete(this.answerRange(collection, discardNumber));
          if (total !== request.result.total) catalog.put({...request.result, total});
        };
      }, migrating);
      assembler.writes.splice(0, batch.length);
      assembler.answers.splice(0, batch.filter(record => record.store === 'answers').length);
      if (discardNumber !== undefined) assembler.discardPartial();
    } finally { this.flushing.delete(assembler); }
  }
  answerRange(collection, number) { return this.ranges.bound([collection,number], [collection,number,[]]); }
  collectionRange(collection) { return this.ranges.bound([collection], [collection,[]]); }
  tables(collection) {
    return this.transaction(['tables'], 'readonly', (tx, result) => {
      const request = tx.objectStore('tables').get(collection); request.onsuccess = () => result(request.result?.tables ?? null);
    });
  }
  list({cursor = null, direction = 'next', size = 32} = {}) {
    uint(size); check(size > 0 && size <= 64, 'Catalog page size must be 1..64.');
    check(['next','prev'].includes(direction), 'Invalid catalog direction.');
    if (cursor) { uint(cursor.created); check(typeof cursor.id === 'string', 'Invalid catalog cursor.'); }
    return this.transaction(['collections'], 'readonly', (tx, result) => {
      const rows = [], backwards = direction === 'prev';
      const range = cursor ? (backwards ? this.ranges.lowerBound([cursor.created,cursor.id], true) : this.ranges.upperBound([cursor.created,cursor.id], true)) : null;
      const request = tx.objectStore('collections').index('created').openCursor(range, backwards ? 'next' : 'prev');
      request.onsuccess = () => {
        const item = request.result;
        if (!item || rows.length === size) {
          if (backwards) rows.reverse();
          const key = row => row ? {created:row.created,id:row.id} : null;
          result({records:rows, next:(backwards ? cursor : item) ? key(rows.at(-1)) : null,
            prev:(backwards ? item : cursor) ? key(rows[0]) : null}); return;
        }
        rows.push(item.value); item.continue();
      };
    });
  }
  page(collection, page = 0, size = 12) {
    uint(page); uint(size); check(size > 0 && size <= 64, 'Answer page size must be 1..64.');
    return this.transaction(['collections','answers'], 'readonly', (tx, result) => {
      const request = tx.objectStore('collections').get(collection);
      request.onsuccess = () => {
        const record = request.result;
        if (!record) { result(null); return; }
        const pages = Math.max(1, Math.ceil(record.total / size)), current = Math.min(page, pages - 1);
        const answers = tx.objectStore('answers').getAll(this.ranges.bound([collection,current*size+1], [collection,(current+1)*size]), size);
        answers.onsuccess = () => result({...record, answers:answers.result, page:current, pages});
      };
    });
  }
  discardPartial(collection, assembler, recovery = null) {
    // Skip unpublished writes and remove their persisted parts in the same
    // transaction that saves any queued completed answers. Quota need not grow.
    return this.flush(collection, assembler, {discard:true, recovery});
  }
  scene(collection, number, options = {}) {
    uint(number);
    const {bindingPage=0,pendingNumber=0}=options;
    [bindingPage,pendingNumber].forEach(uint);
    return this.transaction(['answers','parts','tables'], 'readonly', (tx, result, fail) => {
      const read = request => new Promise((resolve,reject)=>{request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
      const parts=tx.objectStore('parts');
      const rows=async(kind,visit)=>{
        let start=[collection,number,kind,0,0],open=false;
        for(;;){
          const batch=await read(parts.getAll(this.ranges.bound(start,[collection,number,kind,Number.MAX_SAFE_INTEGER,Number.MAX_SAFE_INTEGER],open),256));
          batch.forEach(visit);
          if(batch.length<256)return;
          const last=batch.at(-1);start=[collection,number,kind,last.node,last.slot];open=true;
        }
      };
      const work=async()=>{
        const [summary,record]=await Promise.all([read(tx.objectStore('answers').get([collection,number])),read(tx.objectStore('tables').get(collection))]);
        if(!summary||!record){result(null);return;}
        check(summary.format===2,'Answer migration is incomplete.');
        const nodes=new Map(),facts={kind:'and',path:['facts'],children:[]},bodies=[];
        await rows('node',row=>nodes.set(row.node,{kind:row.kindValue,path:[row.node],args:[],
          ...(row.relation===undefined?{}:{relation:record.tables.signatures[row.relation].name}),
          ...(row.occurrence===undefined?{}:{occurrence:row.occurrence}),
          ...(['and','or'].includes(row.kindValue)?{children:[]}:{}),arity:row.arity,count:row.childcount}));
        await rows('port',row=>{const node=nodes.get(row.node);check(node,'Missing saved node.');node.args[row.slot]=`V${row.variable}`;});
        await rows('child',row=>{const parent=nodes.get(row.node),child=nodes.get(row.target);check(parent?.children&&child,'Missing saved expression.');parent.children[row.slot]=child;});
        await rows('fact',row=>{const node=nodes.get(row.target);check(node,'Missing saved fact.');facts.children[row.slot]=node;});
        await rows('pending',row=>{const scene=nodes.get(row.target);check(scene,'Missing saved body.');bodies[row.slot]={scene,event:row.event};});
        for(const node of nodes.values())check(node.args.length===node.arity&&(!node.children||node.children.length===node.count),'Incomplete saved diagram.');
        const bindingPages=Math.max(1,Math.ceil(summary.variables/24)),page=Math.min(bindingPage,bindingPages-1);
        const bindings=await read(parts.getAll(this.ranges.bound([collection,number,'binding',0,page*24],[collection,number,'binding',0,page*24+23]),24));
        const index=Math.min(pendingNumber,Math.max(0,bodies.length-1));
        result({facts,bindings:bindings.map(row=>({slot:row.slot,name:record.tables.variables[row.slot],variable:row.variable})),bindingPage:page,bindingPages,
          pending:bodies.length?{...bodies[index],index,count:bodies.length}:null});
      };
      work().catch(fail);
    });
  }
  clear(collection) {
    return this.transaction(['collections','tables','answers','parts','recovery'], 'readwrite', (tx, result, fail) => {
      const owner = tx.objectStore('recovery').index('archive').getKey(collection);
      owner.onsuccess = () => {
        if (owner.result !== undefined) { fail(new Error('Release the execution or inspection before clearing its saved answers.')); return; }
        tx.objectStore('collections').delete(collection); tx.objectStore('tables').delete(collection);
        tx.objectStore('answers').delete(this.collectionRange(collection)); tx.objectStore('parts').delete(this.collectionRange(collection));
      };
    });
  }
  async migrate() {
    let checkpoint = await this.transaction(['migration'], 'readonly', (tx, result) => {
      const request=tx.objectStore('migration').get('v1'); request.onsuccess=()=>result(request.result);
    }, true);
    if (!checkpoint) return;
    while (checkpoint.phase === 'collections') {
      const record = await this.nextRecord('collections', checkpoint.after ?? undefined);
      checkpoint = record ? {id:'v1',phase:'collections',after:record.id} : {id:'v1',phase:'answers',after:null};
      await this.transaction(['collections','tables','migration'], 'readwrite', tx => {
        if (record?.tables) {
          const {tables,...catalog}=record;
          tx.objectStore('tables').put({collection:record.id,tables}); tx.objectStore('collections').put(catalog);
        }
        tx.objectStore('migration').put(checkpoint);
      }, true);
      await yieldTask();
    }
    for (;;) {
      const record = await this.nextRecord('answers', checkpoint.after ?? undefined);
      if (!record) break;
      if (record.format !== 2) {
        const tables = await this.transaction(['tables'], 'readonly', (tx, result) => {
          const request = tx.objectStore('tables').get(record.collection); request.onsuccess = () => result(request.result?.tables);
        }, true);
        const assembler = new OutputAssembler(tables); assembler.total = record.number - 1;
        for (const event of legacyEvents(record, tables)) {
          if (assembler.needsFlush) { await this.flush(record.collection, assembler, {migrating:true}); await yieldTask(); }
          assembler.push(event);
        }
        assembler.finish(); await this.flush(record.collection, assembler, {migrating:true});
      }
      checkpoint={id:'v1',phase:'answers',after:[record.collection,record.number]};
      await this.transaction(['migration'], 'readwrite', tx => tx.objectStore('migration').put(checkpoint), true);
      await yieldTask();
    }
    await this.transaction(['migration'], 'readwrite', tx => tx.objectStore('migration').delete('v1'), true);
  }
  nextRecord(store, after) {
    return this.transaction([store], 'readonly', (tx, result) => {
      const request = tx.objectStore(store).openCursor(after === undefined ? null : this.ranges.lowerBound(after, true));
      request.onsuccess = () => result(request.result?.value ?? null);
    }, true);
  }
}

// A generator traverses the already-atomic v1 record without constructing a
// second answer or a complete event array. Pending traversal is depth-only.
function* legacyEvents(answer, tables) {
  yield {kind:'begin', completion:answer.completion, alternative:answer.alternative};
  for (let slot=0; slot<answer.variables.length; slot++) yield {kind:'variable',slot,variable:answer.variables[slot]};
  for (const fact of answer.facts) {
    yield {kind:'fact',occurrence:fact.occurrence,relation:fact.relation};
    for (const variable of fact.args) yield {kind:'port',variable};
    yield {kind:'end_fact'};
  }
  for (const pending of answer.pending ?? []) {
    yield {kind:'pending_begin',event:pending.event};
    const stack = [{body:pending.body, index:0, opened:false}];
    while (stack.length) {
      const frame=stack.at(-1), body=frame.body;
      if (!frame.opened) {
        frame.opened=true;
        if (body.kind === 'atom') {
          const relation=tables.signatures.findIndex(s => s.name === body.atom.relation && s.arity === body.atom.args.length);
          check(relation >= 0, 'Legacy pending relation has no signature.');
          yield {kind:'expression_relation',relation};
        } else yield {kind:'expression',operator:body.kind};
      } else if ((body.kind === 'and' || body.kind === 'or') && frame.index < body.items.length) {
        stack.push({body:body.items[frame.index++],index:0,opened:false});
      } else if ((body.kind === 'atom' || body.kind === 'equal') && frame.index < (body.kind === 'equal' ? 2 : body.atom.args.length)) {
        const variable = body.kind === 'equal' ? (frame.index++ === 0 ? body.left : body.right) : body.atom.args[frame.index++];
        check(typeof variable === 'string' && variable.startsWith('V'), 'Invalid legacy pending variable.');
        yield {kind:'expression_variable',variable:variable.slice(1)};
      } else { yield {kind:'expression_end'}; stack.pop(); }
    }
    yield {kind:'pending_end'};
  }
  yield {kind:'end'};
}
