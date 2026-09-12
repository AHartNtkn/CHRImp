import assert from 'node:assert/strict';
import {test} from 'node:test';
import {NotebookConnection} from './connection.mjs';
import {OutputAssembler} from './answers.mjs';

// Protocol tests inject the storage boundary; native IndexedDB checks cover
// transaction atomicity separately. Each fresh connection represents a reload.
function journal() {
  const rows = new Map(), archives = new Map();
  return {
    rows, archives, failSave:null, discarded:[], rejectWrites:false,
    async recovery(key) { return structuredClone(rows.get(key) ?? null); },
    async saveRecovery(key, value) {
      if (this.rejectWrites) throw Error('storage unavailable');
      if (this.failSave?.(key, value)) { this.failSave = null; throw Error('storage unavailable'); }
      if (value === null) rows.delete(key); else rows.set(key, structuredClone(value));
    },
    async recoveryPage(prefix, after = null, size = 32) {
      const matching = [...rows.keys()].filter(key => key.startsWith(prefix) && (after === null || key > after)).sort();
      const keys = matching.slice(0,size);
      return {records:keys.map(id=>({id,value:structuredClone(rows.get(id))})),next:matching.length > size ? keys.at(-1) : null};
    },
    async create(tables, label, recovery) {
      const old = rows.get(recovery.id);
      if (old) { assert.ok(archives.has(old.archive)); return old.archive; }
      const archive = `archive-${archives.size + 1}`;
      archives.set(archive, structuredClone(tables));
      rows.set(recovery.id, structuredClone({...recovery.value, archive}));
      return archive;
    },
    async discardPartial(archive, assembler) { this.discarded.push(archive); assembler.discardPartial(); },
    async tables(archive) { return structuredClone(archives.get(archive)); },
  };
}
function locks() {
  let held = false;
  return {async request(name, options, action) {
    if (held) return action(null);
    held = true;
    try { return await action({name}); } finally { held = false; }
  }};
}
function server(store) {
  return {
    boot:'a'.repeat(32), canceled:new Set(), issued:0, owners:new Map(), nextRun:0, applications:[], drop:null, calls:[],
    async fetch(url, options) {
      const route = url.slice(5), input = JSON.parse(options.body);
      this.calls.push({route, input});
      const reply = (data, status = 200) => ({ok:status === 200,status,text:async()=>JSON.stringify(data)});
      if (route === 'hello') return reply({boot:this.boot});
      if (input.boot !== this.boot) return reply({code:'stale_boot',error:'server restarted'},409);
      if (route === 'reserve') return reply({owner:++this.issued});
      if (route === 'attach') { this.owners.set(input.owner,this.owners.get(input.owner) ?? {receipt:null}); return reply({}); }
      const owner = this.owners.get(input.owner);
      if (!owner) return reply({code:'unknown_owner',error:'unknown owner'},409);
      let response = {};
      if (input.command !== undefined) {
        const pending = store.rows.get('controller').pending;
        assert.equal(pending.body, options.body, 'intent is committed before the effect');
        if (owner.receipt?.command === input.command) {
          assert.equal(owner.receipt.body, options.body); response = owner.receipt.response;
        } else {
          assert.equal(input.command, (owner.receipt?.command ?? 0) + 1);
          this.applications.push(route);
          if (route === 'start') response = {run:++this.nextRun,signatures:[{name:'p',arity:1}],variables:['A']};
          if (route === 'inspect') response = {inspection:'1'};
          if (route === 'snapshot') response = {snapshot:'2'};
          if (route === 'step') response = {step:{done:false}};
          owner.receipt = {command:input.command,body:options.body,response};
        }
      }
      if (route === 'cancel') this.canceled.add(input.run);
      if (route === 'status') response = {canceled:this.canceled.has(input.run)};
      if (route === 'close') response = {closed:true};
      if (this.drop === route) { this.drop = null; throw Error('response lost'); }
      return reply(response);
    },
  };
}
const model = {program:{rules:[]},query:{kind:'atom',atom:{relation:'p',args:['A']}}};
function connection(store, backend, mutex) { return new NotebookConnection(store,backend.fetch.bind(backend),mutex); }

await test('reload recovers accepted creation without duplicating resources', async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  backend.drop = 'start';
  await assert.rejects(client.request('start',model),/response lost/);
  const exact = store.rows.get('controller').pending.body;
  await client.close();
  client = connection(store,backend,mutex);
  const recovered = await client.request.recover();
  assert.equal(recovered.route,'start'); assert.equal(recovered.response.run,1);
  assert.equal(backend.owners.get(1).receipt.body,exact);
  assert.equal(backend.issued,1); assert.equal(backend.nextRun,1);
  assert.equal(store.archives.size,1);
  assert.equal(store.rows.get('live:run:1').archive,recovered.response.archive);
  assert.deepEqual(store.rows.get('live:source:1').submission,model);
  assert.equal(store.rows.get('controller').pending,null);
  await client.close();
});

await test('adoption remains idempotent when the final command checkpoint fails', async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  store.failSave = (key,value)=>key === 'controller' && value.command === 1 && value.pending === null;
  await assert.rejects(client.request('start',model),/storage unavailable/);
  const archive = store.rows.get('live:run:1').archive;
  assert.equal(store.rows.get('controller').pending.command,1);
  await client.close();
  client = connection(store,backend,mutex);
  assert.equal((await client.request.recover()).response.archive,archive);
  assert.equal(store.archives.size,1); assert.equal(backend.nextRun,1);
  await client.close();
});

await test('failed intent storage sends no command and can safely retry', async () => {
  const store = journal(), backend = server(store), client = connection(store,backend,locks());
  store.failSave = (key,value)=>key === 'controller' && value.pending !== null;
  await assert.rejects(client.request('start',model),/storage unavailable/);
  assert.equal(backend.nextRun,0);
  assert.equal((await client.request('start',model)).run,1);
  await client.close();
});

await test('view ownership is scoped by run and release survives a lost response', async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  await client.request('start',model); await client.request('start',model);
  await client.request('inspect',{run:1}); await client.request('inspect',{run:2});
  assert.equal(store.rows.get('live:inspection:1:1').run,1);
  assert.equal(store.rows.get('live:inspection:2:1').run,2);
  backend.drop = 'inspect_release';
  await assert.rejects(client.request('inspect_release',{run:1,inspection:'1'}),/response lost/);
  assert.equal(store.rows.get('live:inspection:1:1').phase,'release');
  await client.close(); client = connection(store,backend,mutex);
  await client.request('inspect_release',{run:1,inspection:'1'});
  assert.equal(store.rows.has('live:inspection:1:1'),false);
  assert.equal(store.rows.has('live:inspection:2:1'),true);
  await client.request('close',{run:1});
  assert.equal(store.rows.has('live:source:1'),false);
  assert.equal(store.rows.has('live:run:1'),false);
  assert.equal(store.archives.size,4);
  await client.close();
});

await test('a restart keeps editor and saved graphs without replaying old commands', async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  await client.request('start',model);
  const row = store.rows.get('live:run:1');
  const partial = new OutputAssembler(await store.tables(row.archive));
  partial.push({kind:'begin',completion:'1',alternative:'0'});
  row.assembler = partial.checkpoint();
  await store.saveRecovery('editor',{program:'',query:'p(A)'});
  backend.drop = 'step'; await assert.rejects(client.request('step',{run:1}),/response lost/);
  await client.close();
  backend.boot = 'b'.repeat(32); backend.owners.clear(); backend.issued = 0;
  const before = backend.applications.length;
  client = connection(store,backend,mutex);
  assert.deepEqual(await client.initialize(),{restarted:true});
  assert.equal(backend.applications.length,before);
  assert.equal([...store.rows.keys()].some(key=>key.startsWith('live:')),false);
  assert.equal(store.archives.size,1);
  assert.deepEqual(store.discarded,[row.archive]);
  assert.deepEqual(store.rows.get('editor'),{program:'',query:'p(A)'});
  assert.equal(await client.request.recover(),null);
  await client.close();
});

await test('one tab holds the controller through recovery and hands off on close', async () => {
  const store = journal(), backend = server(store), mutex = locks();
  const first = connection(store,backend,mutex), second = connection(store,backend,mutex);
  await first.initialize();
  const calls = backend.calls.length;
  await assert.rejects(second.initialize(),/another tab/);
  assert.equal(backend.calls.length,calls);
  await first.close();
  await second.initialize(); await second.request('start',model);
  assert.equal(backend.nextRun,1);
  await second.close();
});

for (const paused of [false,true]) await test(`quota preserves last-known ${paused ? 'paused' : 'running'} phase while server cancellation survives reload`, async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  await client.request('start',{...model,paused});
  store.rejectWrites = true;
  await assert.rejects(client.request('cancel',{run:1}),/storage unavailable/);
  assert.equal(backend.canceled.has(1),true);
  assert.equal(store.rows.get('live:run:1').phase,paused ? 'paused' : 'running');
  await client.close(); client = connection(store,backend,mutex);
  assert.equal((await client.request('status',{run:1})).canceled,true);
  await client.close();
});

for (const route of ['start','step']) await test(`cancellation recovers a lost ${route} response before quota-blocked adoption`, async () => {
  const store = journal(), backend = server(store), mutex = locks();
  let client = connection(store,backend,mutex);
  if (route !== 'start') await client.request('start',model);
  backend.drop = route;
  await assert.rejects(client.request(route,route === 'start' ? model : {run:1}),/response lost/);
  const pending = structuredClone(store.rows.get('controller').pending);
  await client.close(); store.rejectWrites = true;
  client = connection(store,backend,mutex);
  await assert.rejects(client.request.recover({cancel:true}),/storage unavailable/);
  assert.equal(backend.canceled.has(1),true);
  assert.equal(backend.nextRun,1);
  assert.deepEqual(store.rows.get('controller').pending,pending);
  store.rejectWrites = false;
  await client.request.recover({cancel:true});
  assert.equal(store.rows.get('controller').pending,null);
  assert.equal(store.rows.get('live:run:1').phase,'canceling');
  await client.close();
});

await test('an unsent step cannot block cancellation of its existing run', async () => {
  const store = journal(), backend = server(store), client = connection(store,backend,locks());
  await client.request('start',model);
  store.rejectWrites = true;
  await assert.rejects(client.request('step',{run:1}),/storage unavailable/);
  await assert.rejects(client.request.recover({cancel:true}),/storage unavailable/);
  assert.equal(backend.canceled.has(1),true);
  assert.deepEqual(backend.applications,['start']);
  store.rejectWrites = false;
  assert.equal(await client.request.recover({cancel:true}),null);
  assert.deepEqual(backend.applications,['start']);
  assert.equal(store.rows.get('controller').pending,null);
  await client.close();
});
