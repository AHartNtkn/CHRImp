import assert from 'node:assert/strict';
import {test as runTest} from 'node:test';
import { applyEdit, at, validateNotebook, disposeGraph } from './graph.mjs';
import { RunSession, InspectionSelection, deliverCachedOutput } from './notebook.mjs';
import { OutputAssembler } from './answers.mjs';
import { NotebookConnection } from './connection.mjs';
import * as documents from './documents.mjs';

function memorySink() {
  let next = 0;
  const archives = new Map(), recovery = new Map();
  return {
    archives, journal:recovery,
    async recovery(id) { return structuredClone(recovery.get(id) ?? null); },
    async saveRecovery(id,value) { if (value === null) recovery.delete(id); else recovery.set(id,structuredClone(value)); },
    async recoveryPage(prefix,after=null,size=32) {
      const keys=[...recovery.keys()].filter(key=>key.startsWith(prefix) && (after === null || key > after)).sort();
      return {records:keys.slice(0,size).map(id=>({id,value:structuredClone(recovery.get(id))})),next:keys.length>size ? keys[size-1] : null};
    },
    async tables(archive) { return structuredClone(archives.get(archive)?.tables ?? null); },
    commits: [], beforeCommit: null, afterCommit: null,
    async create(tables, label, owned = null) {
      if (owned && recovery.has(owned.id)) return recovery.get(owned.id).archive;
      const archive = String(++next);
      archives.set(archive, { tables: structuredClone(tables), label, answers: new Map(), parts: new Map() });
      if (owned) recovery.set(owned.id,structuredClone({...owned.value,archive}));
      return archive;
    },
    async flush(archive, assembler, {discard = false, recovery:owned = null} = {}) {
      const discardNumber = discard ? assembler.current?.number : undefined;
      if (!assembler.writes.length && discardNumber === undefined && !owned) return;
      const batch = structuredClone(assembler.writes);
      assert.ok(batch.length <= assembler.capacity, 'each committed batch is bounded');
      const preserved = batch.filter(record => record.value.number !== discardNumber);
      if (preserved.length) await this.beforeCommit?.(preserved, archive);
      const saved = archives.get(archive);
      assert.ok(saved, 'archive must exist before storage');
      for (const { store, value } of preserved) {
        if (store === 'answers') saved.answers.set(value.number, value);
        else if (store === 'parts') saved.parts.set(JSON.stringify([value.number, value.kind, value.node, value.slot]), value);
        else {
          assert.equal(store, 'discard');
          for (const [key, part] of saved.parts) if (part.number === value.number) saved.parts.delete(key);
        }
      }
      if (discardNumber !== undefined) {
        for (const [key, part] of saved.parts) if (part.number === discardNumber) saved.parts.delete(key);
      }
      if (owned) recovery.set(owned.id,structuredClone({...owned.value,assembler:assembler.checkpoint({discard})}));
      if (preserved.length) this.commits.push(preserved);
      if (preserved.length) await this.afterCommit?.(preserved, archive);
      assembler.writes.splice(0, batch.length);
      assembler.answers.splice(0, batch.filter(record => record.store === 'answers').length);
      if (discardNumber !== undefined) assembler.discardPartial();
    },
    async discardPartial(archive, assembler, recovery = null) {
      await this.flush(archive, assembler, {discard:true,recovery});
    },
  };
}
function testConnection(store = memorySink(), fetcher = (...args) => fetch(...args)) {
  return new NotebookConnection(store, fetcher, {request:async (_name,_options,callback) => callback({})});
}

await runTest('transport pins execution requests to one server incarnation', async () => {
  const request = testConnection().request;
  const originalFetch = globalThis.fetch;
  const first = 'a'.repeat(32), second = 'b'.repeat(32);
  let boot = first, helloFails = true, starts = 0;
  const calls = [];
  globalThis.fetch = async (url, options) => {
    const body = JSON.parse(options.body);
    calls.push({url, body});
    if (url === '/api/hello') {
      if (helloFails) { helloFails = false; throw new Error('connection lost'); }
      return {ok:true, status:200, text:async () => JSON.stringify({boot})};
    }
    if (url === '/api/parse') {
      assert.equal(body.boot, undefined);
      return {ok:true, status:200, text:async () => '{}'};
    }
    if (body.boot !== boot) return {ok:false, status:409, text:async () => JSON.stringify({error:'Server restarted; execution is unavailable.'})};
    if (url === '/api/reserve') return {ok:true, status:200, text:async () => JSON.stringify({owner:1})};
    if (url === '/api/start') starts++;
    return {ok:true, status:200, text:async () => JSON.stringify({run:1})};
  };
  try {
    await request('parse', {program:'', query:'true'});
    await assert.rejects(request('start', {}), /connection lost/);
    assert.equal(starts, 0, 'no execution before handshake succeeds');
    await Promise.all([request('start', {}), request('status', {run:1})]);
    assert.equal(calls.filter(call => call.url === '/api/hello').length, 2, 'concurrent calls share the successful handshake');
    assert.equal(starts, 1);
    boot = second;
    for (const route of ['cancel', 'close', 'start', 'inspect', 'step']) {
      await assert.rejects(request(route, {run:1}), /Server restarted/);
      assert.equal(calls.at(-1).body.boot, first, 'never rebind stale handles to a new server');
    }
    assert.equal(starts, 1);
  } finally { globalThis.fetch = originalFetch; }
});

await runTest('control retries preserve exact commands and serialize accepted effects', async () => {
  const api = testConnection().request;
  const originalFetch = globalThis.fetch;
  const boot = 'd'.repeat(32);
  let reservations = 0, attachments = 0, loseAttach = true, loseReply = false, receipt = null;
  const applied = [], bodies = [];
  globalThis.fetch = async (url, options) => {
    const route = url.slice('/api/'.length), body = JSON.parse(options.body);
    const reply = (data, status = 200) => ({ok:status === 200, status, text:async () => JSON.stringify(data)});
    if (route === 'hello') return reply({boot});
    assert.equal(body.boot, boot);
    if (route === 'reserve') return reply({owner:++reservations});
    assert.equal(body.owner, 1);
    if (route === 'attach') {
      attachments++;
      if (loseAttach) { loseAttach = false; throw new Error('attach response lost'); }
      return reply({});
    }
    if (route === 'maintenance') return reply({});
    bodies.push(options.body);
    if (body.invalid) return reply({error:'invalid command'}, 400);
    if (receipt?.command === body.command) assert.equal(receipt.body, options.body, 'replay bytes are unchanged');
    else {
      assert.equal(body.command, (receipt?.command ?? 0) + 1);
      applied.push(route);
      receipt = {command:body.command, body:options.body, response:{run:1, inspection:'2', snapshot:'3', signatures:[], variables:[]}};
    }
    if (loseReply) { loseReply = false; throw new Error('command response lost'); }
    return reply(receipt.response);
  };
  try {
    await assert.rejects(api('start', {}), /attach response lost/);
    for (const route of ['start', 'inspect', 'step', 'resume', 'pause', 'snapshot']) {
      loseReply = true;
      await assert.rejects(api(route, {run:1}), /command response lost/);
      const accepted = applied.length, calls = bodies.length;
      await assert.rejects(api(route, {run:2}), /Recover the interrupted command/);
      assert.equal(bodies.length, calls, 'another command cannot overwrite unresolved delivery');
      await api('maintenance', {run:1});
      await api(route, {run:1});
      assert.equal(applied.length, accepted, 'retry cannot apply a second effect');
    }
    assert.equal(reservations, 1, 'lost attach keeps its reserved identity');
    assert.equal(attachments, 2);
    await assert.rejects(api('step', {invalid:true}), /invalid command/);
    await Promise.all([api('step', {run:1}), api('resume', {run:1})]);
    assert.deepEqual(applied, ['start', 'inspect', 'step', 'resume', 'pause', 'snapshot', 'step', 'resume']);
    assert.equal(receipt.command, 8);
  } finally { globalThis.fetch = originalFetch; }
});

// Exercise the production receipt authority with transport failures after effects.
async function controlSession(name, body) {
  const store = memorySink(), connection = testConnection(store), api = connection.request;
  const originalFetch = globalThis.fetch, calls = [], runs = new Map(), jobs = new Map(), snapshots = new Map();
  const losses = new Set(), effects = [], archives = [];
  let receipt = null, nextRun = 0, nextView = 0, applications = 0, failCreate = false;
  const model = {program:{rules:[]}, query:{kind:'true'}};
  globalThis.fetch = async (url, options) => {
    const route = url.slice('/api/'.length), payload = JSON.parse(options.body);
    calls.push({route, payload, body:options.body});
    let response = {};
    if (route === 'hello') response = {boot:'e'.repeat(32)};
    else if (route === 'reserve') response = {owner:1};
    else if (payload.command !== undefined) {
      if (receipt?.command === payload.command) {
        assert.equal(receipt.route, route); assert.equal(receipt.body, options.body);
        response = receipt.response;
      } else {
        assert.equal(payload.command, (receipt?.command ?? 0) + 1);
        effects.push(route);
        if (route === 'start') { runs.set(++nextRun, {canceled:false}); response = {run:nextRun, signatures:[], variables:[]}; }
        else if (route === 'inspect') { jobs.set(String(++nextView), payload.run); response = {inspection:String(nextView)}; }
        else if (route === 'snapshot') { snapshots.set(String(++nextView), payload.run); response = {snapshot:String(nextView)}; }
        else if (route === 'step') { applications++; response = {step:{done:false}}; }
        else assert.ok(['resume','pause'].includes(route));
        receipt = {route, command:payload.command, body:options.body, response};
      }
    } else if (route === 'cancel') { runs.get(payload.run).canceled = true; }
    else if (route === 'status') response = {canceled:runs.get(payload.run).canceled,applications};
    else if (route === 'close') runs.delete(payload.run);
    else if (route === 'inspect_cancel') { assert.equal(jobs.get(payload.inspection), payload.run); response = {done:true}; }
    else if (route === 'inspect_release') jobs.delete(payload.inspection);
    else if (route === 'snapshot_release') snapshots.delete(payload.snapshot);
    else if (route === 'views') response = {choices:[], snapshots:[...snapshots.keys()].map(id => ({id,label:id}))};
    else if (route === 'output') response = {events:[], applications, step:{done:true}, delivery_done:false};
    else assert.ok(['attach','maintenance'].includes(route), route);
    if (losses.delete(route)) throw new Error(`Lost ${route} response`);
    return {ok:true, status:200, text:async () => JSON.stringify(response)};
  };
  const create = store.create.bind(store);
  store.create = async (tables,label,owned) => {
    if (failCreate) { failCreate=false; throw new Error('Archive unavailable'); }
    const known = owned && await store.recovery(owned.id);
    const archive = await create(tables,label,owned);
    if (!known) archives.push({tables,label});
    return archive;
  };
  const session = new RunSession(api, () => {}, store);
  try { await body({api, session, model, calls, runs, jobs, snapshots, effects, archives, lose:route => losses.add(route), failArchive:() => { failCreate = true; }}); }
  finally { session.stopReading(); await connection.close(); globalThis.fetch = originalFetch; }
}

await runTest('lost step can close then start while ordinary retry commits once', async () => {
  await controlSession('step-close', async ({session, model, lose, effects, runs}) => {
    await session.start(model, false, false);
    lose('step'); await assert.rejects(session.step(), /Lost step/);
    await session.closeRun();
    await session.start(model, false, false);
    assert.equal(session.run, 2); assert.equal(runs.has(1), false);
    assert.equal(effects.filter(route => route === 'step').length, 1);
  });
  await controlSession('step-retry', async ({session, model, lose, effects}) => {
    await session.start(model, false, false);
    lose('step'); await assert.rejects(session.step(), /Lost step/);
    const result = await session.step();
    assert.equal(result.applications, 1);
    assert.equal(effects.filter(route => route === 'step').length, 1);
  });
});

await runTest('lost start is adopted through archive failure before cancellation and replacement', async () => {
  await controlSession('start-cancel', async ({session, model, lose, failArchive, effects, runs, archives}) => {
    lose('start'); await assert.rejects(session.start(model, true, false), /Lost start/);
    failArchive(); await assert.rejects(session.cancel(), /Archive unavailable/);
    assert.equal(runs.get(1).canceled, true, 'source stops before archive adoption can fail');
    assert.equal((await session.store.recovery('controller')).pending.route, 'start'); assert.equal(session.run, null);
    await session.cancel();
    assert.equal(runs.get(1).canceled, true); assert.equal(session.recordHistory, true);
    assert.deepEqual(session.submission, model); assert.equal(archives.length, 1);
    assert.equal(session.recoveredControl, null);
    await session.start({program:{rules:[]}, query:{kind:'fail'}}, false, false);
    assert.equal(session.run, 2); assert.equal(session.runs.get(1).archive, '1');
    assert.equal(effects.filter(route => route === 'start').length, 2);
  });
});

await runTest('cancellation-mode recovery stops an accepted Step before its quota-blocked adoption',async()=>{
  await controlSession('quota-step',async({session,model,lose,runs,effects})=>{
    await session.start(model,false,false);
    lose('step');await assert.rejects(session.step(),/Lost step/);
    const save=session.store.saveRecovery.bind(session.store);
    let blocked=true;
    session.store.saveRecovery=async(key,value)=>{
      if(blocked && key==='live:run:1')throw Error('Journal quota');
      return save(key,value);
    };
    await assert.rejects(session.cancel(),/Journal quota/);
    assert.equal(runs.get(1).canceled,true);
    assert.equal((await session.store.recovery('controller')).pending.route,'step');
    assert.equal(effects.filter(route=>route==='step').length,1);
    blocked=false;await session.cancel();
    assert.equal(session.status,'canceled');assert.equal(session.recoveredControl,null);
    assert.equal(effects.filter(route=>route==='step').length,1);
  });
});

await runTest('switch recovers an interrupted start without losing either execution', async () => {
  await controlSession('start-switch', async ({session, model, lose, runs, effects, archives}) => {
    await session.start(model, false, false);
    lose('start'); await assert.rejects(session.start(model, true, false), /Lost start/);
    // Recovery itself can lose the replay response; the original receipt stays authoritative.
    lose('start'); await assert.rejects(session.switchRun(1), /Lost start/);
    assert.equal(archives.length, 1);
    await session.switchRun(1);
    assert.equal(session.run, 1); assert.equal(session.status, 'canceled');
    assert.equal(session.runs.get(2).recordHistory, true);
    assert.equal(session.runs.get(2).archive, '2'); assert.equal(runs.size, 2);
    assert.equal(effects.filter(route => route === 'start').length, 2);
    await session.switchRun(2);
    lose('resume'); await assert.rejects(session.resume(), /Lost resume/);
    await session.switchRun(1);
    await session.closeRun(); await session.start(model, false, false);
    assert.equal(session.run, 3);
    assert.equal(effects.filter(route => route === 'resume').length, 1);
  });
});

await runTest('lost inspection creation retains cleanup phase across cancel and release losses', async () => {
  await controlSession('inspect-cleanup', async ({api, session, model, lose, jobs, effects, calls}) => {
    await session.start(model, false, false);
    lose('inspect'); await assert.rejects(api('inspect', {run:1,choices:{}}), /Lost inspect/);
    lose('inspect_cancel'); await assert.rejects(session.cancel(), /Lost inspect_cancel/);
    assert.equal(session.recoveredControl.route, 'inspect'); assert.equal(jobs.size, 1);
    lose('inspect_release'); await assert.rejects(session.cancel(), /Lost inspect_release/);
    assert.equal(jobs.size, 0); assert.equal(session.recoveredControl.route, 'inspect');
    const cancels = calls.filter(call => call.route === 'inspect_cancel').length;
    await session.closeRun();
    assert.equal(calls.filter(call => call.route === 'inspect_cancel').length, cancels);
    assert.equal(session.recoveredControl, null); assert.equal(session.run, null);
    await session.start(model, false, false);
    assert.equal(effects.filter(route => route === 'inspect').length, 1);
  });
});

await runTest('lost metadata capture retires only its recovered lease and survives release loss', async () => {
  await controlSession('snapshot-cleanup', async ({api, session, model, lose, snapshots, effects}) => {
    await session.start(model, false, false);
    await session.selection.page(api, 1);
    const previous = session.selection.lease;
    lose('snapshot'); await assert.rejects(session.selection.page(api, 1), /Lost snapshot/);
    assert.equal(snapshots.size, 2);
    lose('snapshot_release'); await assert.rejects(session.cancel(), /Lost snapshot_release/);
    assert.equal(session.recoveredControl.route, 'snapshot');
    assert.deepEqual(session.selection.lease, previous);
    assert.equal(session.selection.pendingCapture, null);
    await session.cancel();
    assert.deepEqual([...snapshots.keys()], [previous.snapshot]);
    assert.equal(session.selection.retiring, null); assert.equal(session.recoveredControl, null);
    await session.closeRun(); assert.equal(snapshots.size, 0);
    assert.equal(effects.filter(route => route === 'snapshot').length, 2);
  });
});

await runTest('notebook persistence and production UI', async t => {
const test = t.test.bind(t);

// Node tests explicitly own a small normalized-record sink. Production uses IndexedDB.
const testSession = (api, notify = () => {}, store = memorySink()) => {
  const registered = async (route,payload) => {
    if (route === 'status') return {canceled:false};
    const response = await api(route,payload);
    if (route !== 'start') return response;
    await store.saveRecovery(`live:source:${response.run}`,{submission:{program:payload.program,query:payload.query},recordHistory:payload.record_history});
    const archive = await store.create(response,`Run ${response.run}`,{id:`live:run:${response.run}`,value:{run:response.run,
      phase:'paused',ack:null,index:0,sequence:null,applications:0,stepPending:false}});
    return {...response,archive};
  };
  registered.checkpoint = action => action();
  return new RunSession(registered,notify,store);
};

const summaries = assembler => assembler.answers;
const parts = (records, number, kind, node) => records.filter(part => part.number === number && part.kind === kind && (node === undefined || part.node === node)).sort((a, b) => a.slot - b.slot);
const queuedParts = assembler => assembler.writes.filter(record => record.store === 'parts').map(record => record.value);

const atom = (relation, ...args) => ({ kind: 'atom', atom: { relation, args } });
const original = {
  program: { rules: [{ name: 'join', kept: [{ relation: 'p', args: ['X', 'Y'] }],
    removed: [{ relation: 'q', args: ['Y'] }], body: atom('r', 'X') }] },
  query: { kind: 'and', items: [atom('p', 'A', 'B'), { kind: 'or', items: [
    { kind: 'equal', left: 'A', right: 'B' }, { kind: 'true' }] }] },
};
const untouched = structuredClone(original);
let edited = applyEdit(original, { type: 'set-port', path: ['query', 'items', 0], index: 1, variable: 'A' });
assert.deepEqual(original, untouched);
assert.deepEqual(edited.query.items[0].atom.args, ['A', 'A']);
edited = applyEdit(edited, { type: 'insert-port', path: ['query', 'items', 0], variable: 'C' });
edited = applyEdit(edited, { type: 'move-port', path: ['query', 'items', 0], index: 2, to: 0 });
assert.deepEqual(edited.query.items[0].atom.args, ['C', 'A', 'A']);
edited = applyEdit(edited, { type: 'remove-port', path: ['query', 'items', 0], index: 1 });
edited = applyEdit(edited, { type: 'rename-relation', path: ['query', 'items', 0], relation: 'edge' });
assert.deepEqual(edited.query.items[0], atom('edge', 'C', 'A'));
edited = applyEdit(edited, { type: 'equal', path: ['query', 'items', 1, 'items', 0], left: 'C', right: 'A' });
edited = applyEdit(edited, { type: 'replace', path: ['query', 'items', 0], node:{kind:'or',items:[edited.query.items[0]]} });
assert.equal(edited.query.items[0].kind, 'or');
assert.equal(edited.query.items[0].items.length, 1);
assert.throws(() => documents.removeSelection(edited, [['query', 'items', 0, 'items', 0]]), /branch/);
edited = applyEdit(edited, { type: 'add-alternative', path: ['query', 'items', 0] });
edited = applyEdit(edited, { type: 'replace', path: ['query', 'items', 0,'items',1], node:{kind:'fail'} });
edited = documents.removeSelection(edited, [['query', 'items', 0, 'items', 0]]);
assert.deepEqual(edited.query.items[0], { kind: 'or', items: [{ kind: 'fail' }] });
assert.equal(at(edited, ['program', 'rules', 0, 'kept', 0]).relation, 'p');
edited = applyEdit(edited, { type: 'append', path: ['program', 'rules', 0, 'kept'], node: atom('s', 'Z') });
assert.deepEqual(edited.program.rules[0].kept[1], { relation: 's', args: ['Z'] });
assert.throws(() => applyEdit(edited, { type: 'append', path: ['program', 'rules', 0, 'kept'], node: { kind: 'true' } }));
assert.throws(() => applyEdit(edited, { type: 'rename-relation', path: ['program', 'rules', 0, 'kept', 0], relation: '<script>' }));
assert.throws(() => applyEdit(edited, { type: 'set-port', path: ['program', 'rules', 0, 'kept', 0], index: 0, variable: 'lower' }));
assert.throws(() => applyEdit(edited, { type: 'replace', path: ['__proto__'], node: {} }));
let soleHead = { program: { rules: [{ name: null, kept: [], removed: [{ relation: 'p', args: [] }], body: { kind: 'true' } }] }, query: { kind: 'true' } };
assert.throws(() => documents.removeSelection(soleHead, [['program', 'rules', 0, 'removed', 0]]), /head/);
assert.throws(() => validateNotebook({ ...original, query: { kind: 'or', items: [] } }));

// Deliberately differs from AST encounter order: response metadata is authoritative.
const tables = { signatures: [{ name: 'r', arity: 1 }, { name: 'p', arity: 2 }], variables: ['B', 'A'] };
const stream = new OutputAssembler(tables);
const answer = (id) => [
  { kind: 'begin', completion: id, alternative: 0 },
  { kind: 'variable', slot: 0, variable: 7 }, { kind: 'variable', slot: 1, variable: 7 },
  { kind: 'fact', occurrence: 11, relation: 1 },
  { kind: 'port', variable: 7 }, { kind: 'port', variable: 7 }, { kind: 'end_fact' },
  { kind: 'fact', occurrence: 12, relation: 1 },
  { kind: 'port', variable: 7 }, { kind: 'port', variable: 7 }, { kind: 'end_fact' }, { kind: 'end' },
];
for (let n = 0; n < 5; n++) for (const event of answer(n)) stream.push(event);
assert.equal(stream.total, 5);
assert.equal(summaries(stream).length, 5);
assert.equal(summaries(stream)[0].completion, '0');
const bounded = new OutputAssembler(tables, 4);
answer(0).slice(0,3).forEach(event => bounded.push(event));
assert.equal(bounded.needsFlush, true);
const heldWrites = structuredClone(bounded.writes);
assert.throws(() => bounded.push(answer(0)[3]), /[Ff]lush/);
assert.deepEqual(bounded.writes, heldWrites);
assert.equal(bounded.current.facts, 0);
assert.deepEqual(parts(queuedParts(stream), 2, 'node').map(node => node.occurrence), ['11', '12']);
assert.deepEqual(parts(queuedParts(stream), 2, 'port', 0).map(port => port.variable), ['7', '7']);
assert.deepEqual(summaries(stream)[1], {format:2, number:2, completion:'1', alternative:'0', variables:2, facts:2, pending:0, nodes:2, maxArity:2});
assert.equal(stream.current, null);
assert.throws(() => stream.push({ kind: 'port', variable: 1 }), /fact/);
assert.throws(() => stream.push({ kind: 'begin', completion: Number.MAX_SAFE_INTEGER + 1, alternative: 0 }), /integer/);
const broken = new OutputAssembler(tables);
broken.push(answer(0)[0]);
assert.throws(() => broken.push({ kind: 'end' }), /alternative/);
for (const event of answer(0).slice(1, 5)) broken.push(event);
assert.throws(() => broken.push({ kind: 'end_fact' }), /ports/);

const calls = [];
let resolveAdvance;
const api = async (route, payload) => {
  calls.push([route, structuredClone(payload)]);
  if (route === 'start') return { run: 8, ...tables };
  if (route === 'cancel') return {};
  if (route === 'output') return new Promise(resolve => { resolveAdvance = resolve; });
  throw new Error('unexpected endpoint');
};
const session = testSession(api);
await session.start(original, false, false);
original.query.items[0].atom.relation = 'changed_after_start';
assert.equal(session.submission.query.items[0].atom.relation, 'p');
assert.deepEqual(session.stream.tables, tables);
assert.equal(calls[0][1].record_history, false);
const advancing = session.readOutput();
assert.equal(calls[1][1].budget, 2048);
session.stopReading();
resolveAdvance({ events: answer(5), applications: 1, exhausted: true, delivery_done: true });
await advancing;
assert.equal(session.stream.total, 1);
assert.equal(session.status, 'done');
assert.equal(session.running, false);
await session.cancel();
assert.equal(session.run, 8);
assert.equal(session.status, 'canceled');
assert.equal(calls.at(-1)[0], 'cancel');
const disconnected = testSession(async () => { throw new Error('connection unavailable'); });
await assert.rejects(disconnected.start(untouched, false, false), /connection unavailable/);
assert.equal(disconnected.run, null);
console.log('AST editing, schema validation, server metadata, bounded stream assembly and run lifecycle checks passed.');

// Persistence is the ownership handoff: a failed save must not consume a new batch.
let storageBlocked = false, advanceCalls = 0, persistedRun = 8;
const store = memorySink();
const durable = store.archives;
store.beforeCommit = () => { if (storageBlocked) throw new Error('Storage full'); };
const persisted = testSession(async route => {
  if (route === 'start') return { run: ++persistedRun, ...tables };
  if (route === 'cancel') return {};
  if (route === 'output') { advanceCalls++; return { sequence: 7, events: [0, 1, 2].flatMap(answer), applications: 3, exhausted: true, delivery_done: true }; }
}, () => {}, store);
await persisted.start(untouched, false, false);
storageBlocked = true;
await assert.rejects(persisted.readOutput(), /Storage full/);
assert.equal(persisted.status, 'error');
assert.equal(persisted.running, false);
assert.equal(summaries(persisted.stream).length, 3);
assert.equal(summaries(persisted.stream)[0].completion, '0');
assert.equal(persisted.pendingDelivery.index, 3 * answer(0).length);
assert.equal(persisted.ack, null);
assert.equal(persisted.timer, null);
assert.equal(durable.get(persisted.archive).answers.size, 0);
const failedIndex = persisted.pendingDelivery.index;
await assert.rejects(persisted.readOutput(), /Storage full/);
assert.equal(persisted.pendingDelivery.index, failedIndex);
assert.equal(advanceCalls, 1);
assert.equal(persisted.stream.total, 3);
storageBlocked = false;
await persisted.readOutput();
assert.equal(advanceCalls, 1);
assert.equal(persisted.stream.writes.length, 0);
assert.equal(persisted.pendingDelivery, null);
assert.equal(persisted.ack, 7);
assert.equal(persisted.stream.total, 3);
assert.deepEqual([...durable.get(persisted.archive).answers.values()].map(a => a.completion), ['0', '1', '2']);
const savedParts = [...durable.get(persisted.archive).parts.values()];
assert.deepEqual(parts(savedParts, 1, 'node').map(node => node.relation), [1, 1]);
assert.equal(durable.get(persisted.archive).tables.signatures[1].name, 'p');
assert.deepEqual(parts(savedParts, 1, 'port', 0).map(port => port.variable), ['7', '7']);
const priorArchive = persisted.archive;
await persisted.start(untouched, false, false);
assert.notEqual(persisted.archive, priorArchive);
assert.equal(durable.get(priorArchive).answers.size, 3);
await persisted.cancel();
assert.throws(() => new OutputAssembler({ signatures: tables.signatures }), /tables/);
console.log('Durable handoff, storage failure backpressure, retry without duplicate delivery, and earlier-run retention checks passed.');

const selection = new InspectionSelection();
selection.update([{ id: 41, label: 'Query alternative' }, { id: 82, label: 'Join application' }], [{ id: 17, label: 'After joining two edges' }]);
assert.deepEqual(selection.payload(9), { run: 9, choices: {} });
selection.choose('41', 'first'); selection.choose('82', 'second'); selection.snapshot = '17';
assert.deepEqual(selection.payload(9), { run: 9, choices: { 41: true, 82: false }, snapshot: '17' });
selection.choose('41', 'either');
assert.deepEqual(selection.payload(9).choices, { 82: false });
selection.update([{ id: 82, label: 'Join application' }], []);
assert.equal(selection.snapshot, '');
assert.deepEqual(selection.payload(9), { run: 9, choices: { 82: false } });
assert.throws(() => selection.choose('41', 'first'), /available/);
assert.throws(() => selection.update([{ id: 1, label: 'a' }, { id: 1, label: 'b' }], []), /descriptor/);
console.log('Labeled tri-state choices, authoritative descriptors, and snapshot request checks passed.');

const stepCalls = [];
let stepAdvances = 0;
const stepped = testSession(async (route, payload) => {
  stepCalls.push([route, payload]);
  if (route === 'start') return {run: 12, ...tables};
  if (route === 'step') return {};
  if (route === 'output') {
    stepAdvances++;
    return {events: [], applications: stepAdvances > 1 ? 1 : 0, exhausted: false, delivery_done: false, step: {done: stepAdvances === 3}};
  }
  if (route === 'cancel') return {};
  throw new Error('Unexpected request');
});
await stepped.start(untouched, false, false);
await stepped.step({'41': true});
assert.equal(stepAdvances, 3);
assert.deepEqual(stepCalls.find(([route]) => route === 'step')[1].choices, {'41': true});
assert.equal(stepped.applications, 1);
assert.equal(stepped.status, 'paused');
const ownedRuns = testSession(async (route, payload) => {
  if (route === 'start') return {run: ownedRuns.runs.size + 1, ...tables};
  if (['cancel', 'close'].includes(route)) return {};
  throw new Error(`Unexpected ${route}`);
});
await ownedRuns.start(untouched, true, false);
await ownedRuns.start(untouched, false, false);
assert.equal(ownedRuns.run, 2);
await ownedRuns.switchRun(1);
assert.equal(ownedRuns.recordHistory, true);
assert.equal(ownedRuns.status, 'canceled');
await ownedRuns.closeRun();
assert.equal(ownedRuns.run, null);
assert.equal(ownedRuns.runs.has(1), false);
assert.equal(ownedRuns.runs.has(2), true);
const recoveryCalls = [];
let busyResume = true, loseBatch = true, fullStorage = false;
const recoveryStore = memorySink();
recoveryStore.beforeCommit = () => { if (fullStorage) throw new Error('Storage full'); };
const recoverableBatch = {sequence: 1, events: answer(1), applications: 1, exhausted: true, delivery_done: true};
const recovering = testSession(async (route, payload) => {
  recoveryCalls.push([route, payload]);
  if (route === 'start') return {run: 20, ...tables};
  if (route === 'resume') { if (busyResume) {busyResume = false; const error = new Error('Collecting'); error.retry = true; throw error;} return {}; }
  if (route === 'maintenance') return {};
  if (route === 'output') { if (loseBatch) {loseBatch = false;throw new Error('Response disconnected');} return recoverableBatch; }
  if (route === 'cancel') return {pending: recoverableBatch};
  throw new Error(`Unexpected ${route}`);
}, () => {}, recoveryStore);
await recovering.start(untouched, false, false);
await recovering.resume(); recovering.stopReading();
assert.deepEqual(recoveryCalls.slice(1,4).map(([route]) => route), ['resume', 'maintenance', 'resume']);
await assert.rejects(recovering.readOutput(), /disconnected/);
assert.equal(recovering.ack, null);
fullStorage = true;
await assert.rejects(recovering.cancel(), /Storage full/);
assert.equal(recoveryCalls.at(-1)[0], 'cancel');
assert.equal(recovering.status, 'canceled');
assert.ok(recovering.pendingDelivery);
assert.equal(recovering.ack, null);
fullStorage = false;
await recovering.deliverPending();
assert.equal(recovering.ack, 1);
assert.equal(recovering.stream.total, 1);

let recoveryRun = 0;
const switchedRecovery = testSession(async route => {
  if (route === 'start') return {run: ++recoveryRun, ...tables};
  if (route === 'output') throw new Error('Response disconnected');
  if (route === 'cancel') return {pending: recoverableBatch};
  throw new Error(`Unexpected ${route}`);
});
await switchedRecovery.start(untouched, false, false);
await assert.rejects(switchedRecovery.readOutput(), /disconnected/);
await switchedRecovery.start(untouched, false, false);
await switchedRecovery.switchRun(1);
assert.equal(switchedRecovery.ack, 1);
assert.equal(switchedRecovery.stream.total, 1);
await switchedRecovery.cancel();
assert.equal(switchedRecovery.stream.total, 1);

const pagedSelection = new InspectionSelection();
let pageRequests = 0;
const metadataApi = async (route, payload) => {
  if (route === 'snapshot') return {snapshot:'9000'};
  if (route === 'snapshot_release') return {};
  assert.equal(route, 'views'); pageRequests++;
  const page = (kind, noun) => {
    const start = payload[`before_${kind}`] ? Number(payload[`before_${kind}`]) - 64 : payload[`after_${kind}`] ? Number(payload[`after_${kind}`]) + 1 : 1;
    return Array.from({length:64}, (_, i) => ({id:String(start+i), label:`${noun} ${start+i}`}));
  };
  const choices = page('choice', 'Choice'), snapshots = page('snapshot', 'State');
  return {choices, snapshots, next_choice:choices.at(-1).id, next_snapshot:snapshots.at(-1).id,
    prev_choice:choices[0].id === '1' ? null : choices[0].id, prev_snapshot:snapshots[0].id === '1' ? null : snapshots[0].id};
};
await pagedSelection.page(metadataApi, 1);
pagedSelection.choose('1', 'first'); await pagedSelection.selectSnapshot(metadataApi, 1, '1'); pagedSelection.choose('1', 'first');
for (let i = 0; i < 20; i++) {
  await pagedSelection.page(metadataApi, 1, 'choice', 'next');
  await pagedSelection.page(metadataApi, 1, 'snapshot', 'next');
  assert.equal(pagedSelection.choices.length, 64); assert.equal(pagedSelection.snapshots.length, 64);
}
assert.equal(pageRequests, 42);
assert.deepEqual(pagedSelection.payload(1), {run:1, choices:{1:true}, snapshot:'1'});
assert.equal(pagedSelection.selectedSnapshot.label, 'State 1');
await pagedSelection.page(metadataApi, 1, 'choice', 'prev');
assert.equal(pagedSelection.choices[0].id, '1217');
assert.equal(pagedSelection.snapshots[0].id, '1281');
await pagedSelection.selectSnapshot(metadataApi, 1, '');
assert.deepEqual(pagedSelection.payload(1), {run:1, choices:{}});
const priorCursors = {...pagedSelection.cursors};
await assert.rejects(pagedSelection.page(async () => { throw new Error('Disconnected'); }, 1, 'snapshot', 'prev'), /Disconnected/);
assert.deepEqual(pagedSelection.cursors, priorCursors);
assert.equal(pagedSelection.loading, false);

// A failed or overlapping state change cannot install another state's choices.
pagedSelection.choose(pagedSelection.choices[0].id, 'second');
const coherentPayload = pagedSelection.payload(1), coherentChoices = pagedSelection.choices;
await assert.rejects(pagedSelection.selectSnapshot(async () => { throw new Error('Disconnected'); }, 1, pagedSelection.snapshots[0].id), /Disconnected/);
assert.deepEqual(pagedSelection.payload(1), coherentPayload);
assert.equal(pagedSelection.choices, coherentChoices);
let completePage;
const loadingPage = pagedSelection.page((route, payload) => route === 'views' ? new Promise(resolve => { completePage = async () => resolve(await metadataApi(route, payload)); }) : metadataApi(route, payload), 1, 'choice', 'next');
await new Promise(resolve => setTimeout(resolve, 0));
await assert.rejects(pagedSelection.selectSnapshot(metadataApi, 1, pagedSelection.snapshots[0].id), /already loading/);
assert.deepEqual(pagedSelection.payload(1), coherentPayload);
await completePage(); await loadingPage;
assert.equal(pagedSelection.snapshot, '');

const pendingStream = new OutputAssembler(tables);
const pendingEvents = [
  ...answer(9).slice(0,3),
  {kind:'pending_begin', event:'19'}, {kind:'expression', operator:'and'},
  {kind:'expression_relation', relation:1}, {kind:'expression_variable', variable:'7'}, {kind:'expression_variable', variable:'8'}, {kind:'expression_end'},
  {kind:'expression', operator:'or'},
  {kind:'expression', operator:'equal'}, {kind:'expression_variable', variable:'7'}, {kind:'expression_variable', variable:'8'}, {kind:'expression_end'},
  {kind:'expression', operator:'fail'}, {kind:'expression_end'},
  {kind:'expression_end'}, {kind:'expression_end'}, {kind:'pending_end'}, {kind:'end'},
];
pendingEvents.forEach(event => pendingStream.push(event)); pendingStream.finish();
assert.deepEqual(summaries(pendingStream), [{format:2, number:1, completion:'9', alternative:'0', variables:2, facts:0, pending:1, nodes:5, maxArity:0}]);
const pendingParts = queuedParts(pendingStream);
assert.deepEqual(parts(pendingParts, 1, 'pending'), [{number:1,kind:'pending',node:0,slot:0,event:'19',target:0}]);
assert.deepEqual(parts(pendingParts, 1, 'node').sort((a,b) => a.node-b.node).map(n => [n.node,n.kindValue,n.relation,n.arity,n.childcount]), [
  [0,'and',undefined,0,2], [1,'atom',1,2,0], [2,'or',undefined,0,2], [3,'equal',undefined,2,0], [4,'fail',undefined,0,0],
]);
assert.deepEqual(parts(pendingParts, 1, 'child', 0).map(n => n.target), [1,2]);
assert.deepEqual(parts(pendingParts, 1, 'child', 2).map(n => n.target), [3,4]);
assert.deepEqual(parts(pendingParts, 1, 'port', 1).map(n => n.variable), ['7','8']);
assert.deepEqual(parts(pendingParts, 1, 'port', 3).map(n => n.variable), ['7','8']);
const unfinishedPending = new OutputAssembler(tables);
pendingEvents.slice(0,6).forEach(event => unfinishedPending.push(event));
assert.throws(() => unfinishedPending.push({kind:'expression_end'}), /[Mm]issing expression ports/);
assert.throws(() => unfinishedPending.push({kind:'end'}), /alternative/);
assert.throws(() => unfinishedPending.discardPartial(), /through the store/);
const partialStore = memorySink();
const partialArchive = await partialStore.create(tables, 'Partial test');
await partialStore.discardPartial(partialArchive, unfinishedPending);
unfinishedPending.finish();
assert.equal(partialStore.archives.get(partialArchive).parts.size, 0);
assert.equal(unfinishedPending.pending, null);
assert.equal(unfinishedPending.expressions.length, 0);

// A metadata lease bridges refreshes without changing Current inspection targets.
const leaseCalls = [], heldLeases = new Map();
let leaseId = 10000, leaseRun = 20, failViews = false, failRelease = false, busyCapture = false, holdViews = null;
const leaseApi = async (route, payload) => {
  leaseCalls.push([route, {...payload}]);
  if (route === 'start') return {run: ++leaseRun, ...tables};
  if (route === 'cancel' || route === 'close' || route === 'maintenance') return {};
  if (route === 'snapshot') {
    if (busyCapture) { busyCapture = false; throw Object.assign(new Error('Collecting'), {retry:true}); }
    const snapshot = String(++leaseId); heldLeases.set(snapshot, payload.run); return {snapshot};
  }
  if (route === 'snapshot_release') {
    if (failRelease) throw new Error('Release unavailable');
    assert.equal(heldLeases.get(payload.snapshot), payload.run);
    heldLeases.delete(payload.snapshot); return {};
  }
  assert.equal(route, 'views');
  if (failViews) throw new Error('Metadata unavailable');
  assert.ok(payload.snapshot === '17' || heldLeases.get(payload.snapshot) === payload.run);
  if (holdViews) await new Promise(resolve => { holdViews = resolve; });
  return {
    choices:[{id:payload.after_choice ? '82' : '41', label:'Choice'}],
    snapshots:[{id:'17',label:'Recorded state'}, ...[...heldLeases.keys()].map(id => ({id,label:'Metadata lease'}))],
    next_choice:'41', prev_choice:null, next_snapshot:null, prev_snapshot:null,
  };
};
const leased = testSession(leaseApi);
await leased.start(untouched, false, false);
busyCapture = true;
await leased.selection.page(leaseApi, leased.run);
assert.ok(leaseCalls.some(([route]) => route === 'maintenance'));
assert.deepEqual(leased.selection.snapshots, [{id:'17',label:'Recorded state'}]);
leased.selection.choose('41', 'second');
const firstLease = leased.selection.lease.snapshot;
await leased.selection.page(leaseApi, leased.run, 'choice', 'next');
assert.equal(heldLeases.size, 1);
assert.ok(!heldLeases.has(firstLease));
assert.deepEqual(leased.selection.payload(leased.run), {run:leased.run, choices:{41:false}});
assert.deepEqual(leased.selection.snapshots, [{id:'17',label:'Recorded state'}]);
const preservedLease = leased.selection.lease.snapshot;
const preservedChoices = leased.selection.choices;
failViews = true;
await assert.rejects(leased.selection.page(leaseApi, leased.run), /Metadata unavailable/);
assert.equal(heldLeases.size, 1);
assert.equal(leased.selection.lease.snapshot, preservedLease);
assert.equal(leased.selection.choices, preservedChoices);
assert.equal(leased.selection.payload(leased.run).choices[41], false);
// A failed candidate cleanup remains owned and is retried before another capture.
failRelease = true;
await assert.rejects(leased.selection.page(leaseApi, leased.run), /Release unavailable/);
assert.equal(heldLeases.size, 2);
assert.ok(heldLeases.has(leased.selection.retiring.snapshot));
failViews = false; failRelease = false;
await leased.selection.page(leaseApi, leased.run);
assert.equal(heldLeases.size, 1);
assert.equal(leased.selection.retiring, null);
// Successful installation with failed old-lease release also keeps both owners.
failRelease = true;
await assert.rejects(leased.selection.page(leaseApi, leased.run), /Release unavailable/);
assert.equal(heldLeases.size, 2);
assert.deepEqual(leased.selection.snapshots, [{id:'17',label:'Recorded state'}]);
await assert.rejects(leased.selection.reset(leaseApi), /Release unavailable/);
assert.equal(heldLeases.size, 2);
assert.equal(leased.selection.payload(leased.run).choices[41], false);
failRelease = false;
await leased.selection.reset(leaseApi);
assert.equal(heldLeases.size, 0);
assert.deepEqual(leased.selection.payload(leased.run).choices, {});
// Reset waits for an admitted page, then releases its candidate before clearing.
holdViews = true;
const waitingPage = leased.selection.page(leaseApi, leased.run);
await new Promise(resolve => setTimeout(resolve, 0));
const resettingSelection = leased.selection.reset(leaseApi);
await assert.rejects(leased.selection.page(leaseApi, leased.run), /already loading/);
assert.equal(heldLeases.size, 1);
holdViews(); holdViews = null;
await waitingPage; await resettingSelection;
assert.equal(heldLeases.size, 0);
assert.deepEqual(leased.selection.choices, []);
// Historical selection uses its existing root and never acquires a metadata lease.
await leased.selection.page(leaseApi, leased.run);
const capturesBeforeHistory = leaseCalls.filter(([route]) => route === 'snapshot').length;
await leased.selection.selectSnapshot(leaseApi, leased.run, '17');
assert.equal(heldLeases.size, 0);
await leased.selection.page(leaseApi, leased.run);
assert.equal(leaseCalls.filter(([route]) => route === 'snapshot').length, capturesBeforeHistory);
assert.equal(leased.selection.payload(leased.run).snapshot, '17');
await leased.selection.selectSnapshot(leaseApi, leased.run, '');
assert.equal(heldLeases.size, 1);
assert.ok(!('snapshot' in leased.selection.payload(leased.run)));
// Real lifecycle methods release against the original run, including in-flight metadata.
const oldLeaseRun = leased.run;
leased.selection.choose('41', 'first');
const sharedSelection = leased.selection;
await leased.start(untouched, false, false);
assert.equal(leased.selection, sharedSelection);
assert.deepEqual(sharedSelection.choices, []);
assert.deepEqual(sharedSelection.snapshots, []);
assert.equal(sharedSelection.assignments.size, 0);
assert.equal(sharedSelection.snapshot, '');
assert.deepEqual(sharedSelection.navigation, {});
assert.equal(heldLeases.size, 0);
assert.ok(leaseCalls.some(([route,payload]) => route === 'snapshot_release' && payload.run === oldLeaseRun));
await leased.selection.page(leaseApi, leased.run);
await leased.switchRun(oldLeaseRun);
assert.equal(heldLeases.size, 0);
await leased.selection.page(leaseApi, leased.run);
await leased.closeRun();
assert.equal(heldLeases.size, 0);
assert.equal(leased.run, null);
console.log('Metadata snapshot ownership, cofactor-safe selections, page failure recovery, and lifecycle release checks passed.');

// Capture replay recovers ownership when the server mutates before transport fails.
const replaySelection = new InspectionSelection();
const replayHeld = new Map(), replayCalls = [];
let controlReceipt = null;
let replayId = 20000, loseCapture = false, loseRelease = false;
const replayFetch = globalThis.fetch;
const replayApi = testConnection().request;
globalThis.fetch = async (url, options) => {
  const route = url.slice('/api/'.length), payload = JSON.parse(options.body);
  const reply = data => ({ok:true, status:200, text:async () => JSON.stringify(data)});
  if (route === 'hello') return reply({boot:'c'.repeat(32)});
  if (route === 'reserve') return reply({owner:1});
  if (route === 'attach') return reply({});
  replayCalls.push([route, {...payload}]);
  if (route === 'snapshot') {
    assert.ok(Number.isSafeInteger(payload.command) && payload.command > 0);
    if (controlReceipt?.command === payload.command) assert.equal(controlReceipt.body, options.body);
    else {
      assert.equal(payload.command, (controlReceipt?.command ?? 0) + 1);
      controlReceipt = {command:payload.command, body:options.body, snapshot:String(++replayId)};
      replayHeld.set(controlReceipt.snapshot, payload.run);
    }
    if (loseCapture) { loseCapture = false; throw new Error('Capture response lost'); }
    return reply({snapshot:controlReceipt.snapshot});
  }
  if (route === 'snapshot_release') {
    if (replayHeld.has(payload.snapshot)) assert.equal(replayHeld.get(payload.snapshot), payload.run);
    replayHeld.delete(payload.snapshot);
    if (loseRelease) { loseRelease = false; throw new Error('Release response lost'); }
    return reply({});
  }
  assert.equal(route, 'views');
  assert.equal(replayHeld.get(payload.snapshot), payload.run);
  return reply({choices:[{id:'8',label:'Choice 8'}], snapshots:[...replayHeld.keys()].map(id => ({id,label:'Owned'}))});
};
try {
loseCapture = true;
await assert.rejects(replaySelection.page(replayApi, 1), /Capture response lost/);
assert.equal(replayHeld.size, 1);
assert.deepEqual(replaySelection.pendingCapture, {run:1});
await replaySelection.page(replayApi, 1);
assert.equal(replayHeld.size, 1);
assert.equal(replayId, 20001, 'Replay must not allocate another snapshot');
assert.equal(replaySelection.pendingCapture, null);
assert.deepEqual(replaySelection.snapshots, []);
replaySelection.choose('8', 'second');
const replayOld = replaySelection.lease.snapshot;
loseCapture = true;
await assert.rejects(replaySelection.page(replayApi, 1), /Capture response lost/);
assert.equal(replayHeld.size, 2);
assert.equal(replaySelection.lease.snapshot, replayOld);
assert.deepEqual(replaySelection.payload(1), {run:1,choices:{8:false}});
const uncertainToken = {...replaySelection.pendingCapture};
loseCapture = true;
await assert.rejects(replaySelection.reset(replayApi), /Capture response lost/);
assert.deepEqual(replaySelection.pendingCapture, uncertainToken);
assert.equal(replayHeld.size, 2);
// Reset learns the uncertain ID, then an uncertain release retains that owner.
loseRelease = true;
await assert.rejects(replaySelection.reset(replayApi), /Release response lost/);
assert.equal(replaySelection.pendingCapture, null);
assert.ok(replaySelection.retiring);
assert.equal(replayHeld.size, 1);
await replaySelection.reset(replayApi);
assert.equal(replayHeld.size, 0);
assert.equal(replaySelection.retiring, null);
assert.equal(replaySelection.lease, null);
await replaySelection.page(replayApi, 2);
assert.equal(controlReceipt.command, 3, 'Control sequence spans runs');
const heldSecondRun = replaySelection.lease.snapshot;
loseRelease = true;
await assert.rejects(replaySelection.reset(replayApi), /Release response lost/);
assert.equal(replaySelection.lease.snapshot, heldSecondRun);
assert.equal(replayHeld.size, 0);
await replaySelection.reset(replayApi);
assert.equal(replaySelection.lease, null);
await replaySelection.page(replayApi, 1);
assert.equal(controlReceipt.command, 4, 'Returning to a run uses a newer token');
await replaySelection.reset(replayApi);
assert.equal(replayHeld.size, 0);
assert.deepEqual(replayCalls.filter(([route]) => route === 'snapshot').map(([,p]) => [p.run,p.command]), [[1,1],[1,1],[1,2],[1,2],[1,2],[2,3],[1,4]]);
} finally { globalThis.fetch = replayFetch; }
console.log('Control replay preserves snapshot ownership across pages, resets and runs.');

// A single large answer crosses bounded commits; storage failure pauses midway.
const wideEvents = answer(70).slice(0, 3);
for (let i = 0; i < 160; i++) wideEvents.push(
  {kind:'fact', occurrence:String(1000+i), relation:1},
  {kind:'port', variable:'7'}, {kind:'port', variable:'8'}, {kind:'end_fact'},
);
wideEvents.push({kind:'end'});
const wideStore = memorySink();
let wideRequests = 0, blockWide = true;
const wideSession = testSession(async route => {
  if (route === 'start') return {run:30, ...tables};
  if (route === 'resume') return {};
  if (route === 'output') {
    wideRequests++;
    return {sequence:11, events:wideEvents, applications:1, exhausted:true, delivery_done:true};
  }
  throw new Error(`Unexpected ${route}`);
}, () => {}, wideStore);
await wideSession.start(untouched, false, false);
wideStore.beforeCommit = () => {
  assert.equal(wideSession.ack, null, 'ack waits for the entire response to persist');
  if (wideStore.commits.length === 1 && blockWide) throw new Error('Mid-answer storage failure');
};
const consumedWide = [], pushWide = wideSession.stream.push.bind(wideSession.stream);
wideSession.stream.push = event => { pushWide(event); consumedWide.push(event); };
await wideSession.resume();
assert.equal(wideSession.running, true);
await assert.rejects(wideSession.readOutput(), /Mid-answer storage failure/);
const stoppedAt = wideSession.pendingDelivery.index;
assert.ok(stoppedAt > 3 && stoppedAt < wideEvents.length);
assert.deepEqual(consumedWide, wideEvents.slice(0, stoppedAt));
assert.equal(wideSession.stream.total, 0);
assert.equal(wideSession.stream.answers.length, 0);
assert.ok(wideSession.stream.current);
assert.ok(Object.values(wideSession.stream.current).every(value => !Array.isArray(value)));
assert.ok(wideSession.stream.writes.length <= wideSession.stream.capacity);
assert.ok(wideStore.archives.get(wideSession.archive).parts.size > 0);
assert.equal(wideStore.archives.get(wideSession.archive).answers.size, 0, 'partial answer has no completed summary');
assert.equal(wideSession.running, false);
assert.equal(wideSession.timer, null);
assert.equal(wideSession.inFlight, null);
await assert.rejects(wideSession.readOutput(), /Mid-answer storage failure/);
assert.equal(wideRequests, 1);
assert.equal(wideSession.pendingDelivery.index, stoppedAt);
assert.equal(consumedWide.length, stoppedAt);
blockWide = false;
await wideSession.readOutput();
assert.equal(wideRequests, 1);
assert.deepEqual(consumedWide, wideEvents, 'each scalar event is consumed exactly once');
assert.equal(wideSession.ack, 11);
assert.equal(wideSession.pendingDelivery, null);
assert.equal(wideSession.stream.current, null);
assert.equal(wideSession.stream.writes.length, 0);
assert.equal(wideSession.stream.answers.length, 0);
assert.ok(wideStore.commits.length >= 3);
const wideSaved = wideStore.archives.get(wideSession.archive);
assert.deepEqual([...wideSaved.answers.values()], [{format:2, number:1, completion:'70', alternative:'0', variables:2, facts:160, pending:0, nodes:160, maxArity:2}]);
assert.equal(wideSaved.parts.size, 642);
assert.deepEqual(parts([...wideSaved.parts.values()], 1, 'port', 159).map(port => port.variable), ['7','8']);

// An uncertain flush may have committed; stable scalar keys make its retry idempotent.
const uncertainStore = memorySink();
let uncertainRequests = 0, loseCommit = true;
const uncertainEvents = [answer(80), answer(81)].flat();
const uncertainSession = testSession(async route => {
  if (route === 'start') return {run:31, ...tables};
  if (route === 'output') {
    uncertainRequests++;
    return {sequence:12, events:uncertainEvents, applications:2, exhausted:true, delivery_done:true};
  }
  throw new Error(`Unexpected ${route}`);
}, () => {}, uncertainStore);
await uncertainSession.start(untouched, false, false);
uncertainStore.afterCommit = () => { if (loseCommit) { loseCommit = false; throw new Error('Commit result lost'); } };
const consumedUncertain = [], pushUncertain = uncertainSession.stream.push.bind(uncertainSession.stream);
uncertainSession.stream.push = event => { pushUncertain(event); consumedUncertain.push(event); };
await assert.rejects(uncertainSession.readOutput(), /Commit result lost/);
assert.equal(uncertainSession.ack, null);
assert.equal(uncertainSession.stream.answers.length, 2);
assert.equal(uncertainStore.archives.get(uncertainSession.archive).answers.size, 2);
await uncertainSession.readOutput();
assert.equal(uncertainRequests, 1);
assert.deepEqual(consumedUncertain, uncertainEvents);
assert.equal(uncertainSession.stream.total, 2);
assert.equal(uncertainSession.ack, 12);
assert.equal(uncertainSession.stream.writes.length, 0);
assert.equal(uncertainSession.stream.answers.length, 0);
assert.equal(uncertainStore.archives.get(uncertainSession.archive).answers.size, 2);
assert.equal(uncertainStore.archives.get(uncertainSession.archive).parts.size, 20);

// Response-end flushes also persist unfinished output, whose cancellation is scoped.
const splitStore = memorySink();
let splitResponses = 0;
const splitEvents = answer(90);
const splitSession = testSession(async route => {
  if (route === 'start') return {run:32, ...tables};
  if (route === 'output') {
    splitResponses++;
    return splitResponses === 1
      ? {sequence:1, events:splitEvents.slice(0,5), applications:0, exhausted:false, delivery_done:false}
      : {sequence:2, events:splitEvents.slice(5), applications:0, exhausted:true, delivery_done:true};
  }
  throw new Error(`Unexpected ${route}`);
}, () => {}, splitStore);
await splitSession.start(untouched, false, false);
await splitSession.readOutput();
assert.equal(splitSession.ack, 1);
assert.ok(splitSession.stream.current);
assert.equal(splitSession.stream.writes.length, 0);
assert.equal(splitStore.archives.get(splitSession.archive).answers.size, 0);
assert.equal(splitStore.archives.get(splitSession.archive).parts.size, 3);
await splitSession.readOutput();
assert.equal(splitSession.ack, 2);
assert.equal(splitSession.stream.total, 1);
assert.equal(splitStore.archives.get(splitSession.archive).answers.size, 1);
assert.equal(splitStore.archives.get(splitSession.archive).parts.size, 10);

const cancelPartialStore = memorySink();
const cancelPartial = testSession(async route => {
  if (route === 'start') return {run:33, ...tables};
  if (route === 'output') return {sequence:1, events:[...answer(100), ...pendingEvents.slice(0,6)], applications:0, exhausted:false, delivery_done:false};
  if (route === 'cancel') return {};
  throw new Error(`Unexpected ${route}`);
}, () => {}, cancelPartialStore);
await cancelPartial.start(untouched, false, false);
await cancelPartial.readOutput();
const canceledArchive = cancelPartialStore.archives.get(cancelPartial.archive);
assert.equal(canceledArchive.answers.size, 1);
assert.ok([...canceledArchive.parts.values()].some(part => part.number === 2));
await cancelPartial.cancel();
assert.equal(cancelPartial.stream.current, null);
assert.equal(cancelPartial.stream.pending, null);
assert.equal(cancelPartial.stream.expressions.length, 0);
assert.equal(cancelPartial.stream.writes.length, 0);
assert.equal(cancelPartial.stream.answers.length, 0);
assert.equal(canceledArchive.answers.size, 1);
assert.equal(canceledArchive.parts.size, 10);
assert.ok([...canceledArchive.parts.values()].every(part => part.number === 1));
console.log('Bounded mid-answer writes, durable acknowledgements, quiescent retries, split responses, and partial cancellation checks passed.');

// Execute the UI's production closures with a small DOM and deferred storage.
// Keeping storage unresolved reproduces repeated clicks before a new page paints.
const {readFileSync} = await import('node:fs');
const {createContext:rawCreateContext, runInContext} = await import('node:vm');
function createContext(values) {
  const store = values.store ?? values.session?.store ?? memorySink();
  const pending = values.inspectionPending;
  if (pending) store.journal.set(`live:inspection:${pending.run}:${pending.inspection}`, {
    run:pending.run,inspection:pending.inspection,archive:pending.archive,phase:'active',ack:pending.ack,index:0,sequence:null});
  const api = values.liveRequest && (async (...args) => {
    const response = await values.liveRequest(...args);
    if (args[0] === 'inspect_release') await store.saveRecovery(`live:inspection:${args[1].run}:${args[1].inspection}`,null);
    return response;
  });
  const context = rawCreateContext({connected:true, restoring:false, connection:{state:null}, check:assert.ok, store,
    ...documents,clone:structuredClone,validateNotebook,at,disposeGraph,
    notebookDoc:documents.emptyNotebook(),model:documents.executionModel(documents.emptyNotebook()),
    fileHandle:null,fileName:'',fileDirty:false,fileBusy:false,needsQueryRun:false,
    runNotice:null, displayWriting:null,displayDirty:false,displayStamp:null,message(){},async saveDisplay(){},
    inspected:null,outputMode:'answers',savedSelection:'',answerNumber:null,answerPage:0,
    bindingPage:0,pendingNumber:0, session:values.session ?? {run:null},
    request:values.request ?? Object.assign(api ?? (()=>{}), {checkpoint:action => action()}), async saveEditor() {}, async restoreInspection() {}, async finishInspection() {}, ...values,
    ...(api ? {liveRequest:api} : {})});
  runInContext(productionSection('function attachBatch(', '// A cached response')
    + productionSection('  function currentDocument()', '  function fileState()'),context);
  return context;
}
const notebookSource = readFileSync(new URL('./notebook.mjs', import.meta.url), 'utf8');
function productionSection(start, end) {
  const first = notebookSource.indexOf(start), last = notebookSource.indexOf(end, first);
  assert.ok(first >= 0 && last > first, `Production section ${start} is available`);
  return notebookSource.slice(first, last);
}
function documentHarness(doc, values={}) {
  const controls=new Map(),calls=[];
  const $=name=>{if(!controls.has(name))controls.set(name,{value:'',checked:false,scrollIntoView(options){this.scrolled=options;},prepend(...children){this.children=children;}});return controls.get(name);};
  const context=createContext({$,notebookDoc:structuredClone(doc),model:documents.executionModel(doc),
    document:{title:'',querySelector:selector=>$(selector)},window:{},dirty:false,busy:false,revision:0,undo:[],redo:[],selected:null,selection:[],path:['query'],
    editorWriting:null,editorDirty:false,inspectionSelection:new InspectionSelection(),connection:{state:{boot:'a'.repeat(32)}},
    ruleCards:new Map(),editingControls:{},debounce:null,clearTimeout,Blob,
    renderWorkspace(){},renderRun(){},safe:action=>action(),
    request:async(route,model)=>{assert.equal(route,'format');calls.push(structuredClone(model));return {program:'formatted program',query:'formatted query'};},
    ...values});
  runInContext(productionSection('  function saveEditor()', '  function displayState()')
    +productionSection('  function currentDocument()', '  function chosenPaths()')
    +productionSection('  function remember()', '  const edit ='),context);
  return {context,$,calls};
}
function twoQueryDocument() {
  return {...documents.emptyNotebook(),title:'Two experiments',activeQuery:'second',
    queries:[{id:'first',name:'First',body:{kind:'atom',atom:{relation:'first',args:['A']}}},
      {id:'second',name:'Second',body:{kind:'atom',atom:{relation:'second',args:['B']}}}],
    layouts:{'query:first':[['["query"]',{x:10,y:20}]],'query:second':[['["query"]',{x:-30,y:40}]]}};
}
await test('document recovery preserves inactive queries, layout, file identity and unsynced source',async()=>{
  const doc=twoQueryDocument(),store=memorySink(),handle={name:'experiments.chrnb'};
  await store.saveRecovery('editor',{document:doc,fileHandle:handle,fileName:handle.name,fileDirty:true,needsQueryRun:true,
    program:'unsynced program',query:'unsynced query',dirty:true,history:true,run:null,boot:'a'.repeat(32)});
  const {context:c,$}=documentHarness(doc,{store,connected:false,restoring:true,
    connection:{state:{boot:'a'.repeat(32)},async initialize(){}},session:{run:null,async restore(){}},
    renderInspectionControls(){},async refreshSaved(){}});
  runInContext(productionSection('  function saveEditor()', '  function message('),c);
  await c.initialize();
  assert.deepEqual(structuredClone(c.currentDocument()),doc);
  assert.deepEqual(structuredClone(c.model.query),doc.queries[1].body);
  assert.equal($('query').value,'unsynced query');assert.equal(c.dirty,true);assert.equal(c.needsQueryRun,true);
  const saved=await store.recovery('editor');
  assert.deepEqual(saved.document,doc);assert.deepEqual(saved.fileHandle,handle);
  assert.equal(saved.fileDirty,true);assert.equal(saved.fileName,handle.name);assert.equal(saved.program,'unsynced program');
});
await test('query switching and undo preserve both bodies and their independent layouts',async()=>{
  const doc=twoQueryDocument(),{context:c,calls,$}=documentHarness(doc);
  await c.changeQuery('first');
  assert.equal(c.notebookDoc.activeQuery,'first');assert.equal(c.needsQueryRun,true);
  assert.deepEqual(calls,[{program:{rules:[]},query:doc.queries[0].body}]);
  assert.deepEqual(structuredClone(c.notebookDoc.queries),doc.queries);
  assert.deepEqual(structuredClone(c.notebookDoc.layouts),doc.layouts);
  assert.deepEqual(structuredClone(c.undo),[doc]);assert.equal($('query').value,'formatted query');
  await c.commitDocument(structuredClone(c.undo.at(-1)),'undo');
  assert.deepEqual(structuredClone(c.currentDocument()),doc);assert.equal(c.undo.length,0);
  assert.equal(c.redo.length,1);assert.equal(c.redo[0].activeQuery,'first');
  await c.commitDocument(structuredClone(c.redo.at(-1)),'redo');
  assert.equal(c.notebookDoc.activeQuery,'first');assert.equal(c.redo.length,0);
  assert.equal((await c.store.recovery('editor')).document.activeQuery,'first');
});
await test('completed layout persists once and undo restores the previous query positions',async()=>{
  const doc=twoQueryDocument(),{context:c}=documentHarness(doc),positions=[['["query"]',{x:70,y:-80}]];
  c.updateLayouts(['query'],positions);await c.editorWriting;
  assert.equal(c.undo.length,1);assert.deepEqual(structuredClone(c.undo[0]),doc);
  const saved=await c.store.recovery('editor');
  assert.deepEqual(saved.document.layouts['query:second'],positions);
  assert.deepEqual(saved.document.layouts['query:first'],doc.layouts['query:first']);assert.equal(saved.fileDirty,true);
  c.updateLayouts(['query'],structuredClone(positions));assert.equal(c.undo.length,1,'An echoed layout is not another edit');
  await c.commitDocument(structuredClone(c.undo.at(-1)),'undo');
  assert.deepEqual((await c.store.recovery('editor')).document.layouts,doc.layouts);
});
await test('file save serializes the whole document and a failed write retains dirty state',async()=>{
  const doc=twoQueryDocument(),written=[];let fail=true,aborted=0,closed=0;
  const handle={name:'experiments.chrnb',async queryPermission(){return 'granted';},async createWritable(){return {
    async write(text){written.push(text);if(fail)throw Error('Disk full');},async close(){closed++;},async abort(){aborted++;}};}};
  const {context:c}=documentHarness(doc,{fileHandle:handle,fileDirty:true});
  // File-system handles are structured-cloneable in browsers, unlike this Node test double.
  let persisted;
  c.store.saveRecovery=async(_key,value)=>{persisted={...value,fileHandle:value.fileHandle.name};};
  await assert.rejects(c.saveFile(),/Disk full/);
  assert.deepEqual(documents.parseDocument(written[0]),doc,'Even the failed write receives the full serialized document');
  assert.deepEqual(structuredClone(c.currentDocument()),doc,'A failed write preserves the current document');
  assert.equal(aborted,1);assert.equal(closed,0);assert.equal(c.fileDirty,true);assert.equal(c.fileBusy,false);
  assert.equal(persisted,undefined,'Failed file writes must not publish a saved checkpoint');
  fail=false;await c.saveFile();
  assert.deepEqual(documents.parseDocument(written.at(-1)),doc);assert.equal(closed,1);
  assert.equal(c.fileDirty,false);assert.equal(c.fileBusy,false);assert.equal(persisted.fileName,handle.name);
  assert.deepEqual(structuredClone(persisted.document),doc);
});
for(const phase of ['picker','write','close'])await test(`save ${phase} cancellation preserves the current document and file identity`,async()=>{
  const doc=twoQueryDocument(),oldHandle={name:'original.chrnb'},abort=Object.assign(Error('Canceled'),{name:'AbortError'});
  const written=[];let aborted=0,closed=0;
  const newHandle={name:'replacement.chrnb',async queryPermission(){return 'granted';},async createWritable(){return {
    async write(text){written.push(text);if(phase==='write')throw abort;},
    async close(){if(phase==='close')throw abort;closed++;},async abort(){aborted++;}};}};
  const {context:c,$}=documentHarness(doc,{fileHandle:oldHandle,fileName:oldHandle.name,fileDirty:true,dirty:phase==='picker',
    window:{async showSaveFilePicker(){if(phase==='picker')throw abort;return newHandle;}}});
  $('query').value='unsynced text';
  await c.saveFile(true);
  assert.deepEqual(structuredClone(c.currentDocument()),doc);assert.equal(c.fileHandle,oldHandle);
  assert.equal(c.fileName,'original.chrnb');assert.equal(c.fileDirty,true);assert.equal(c.dirty,phase==='picker');
  assert.equal($('query').value,'unsynced text');assert.equal(c.fileBusy,false);assert.equal(closed,0);
  assert.equal(await c.store.recovery('editor'),null,'Canceled saving does not publish a saved checkpoint');
  if(phase==='picker')assert.equal(written.length,0);
  else {assert.deepEqual(documents.parseDocument(written[0]),doc);assert.equal(aborted,1);}
});
await test('fallback download contains a reopenable complete notebook with the expected filename and MIME type',async()=>{
  const doc=twoQueryDocument(),links=[],timers=[];
  const {context:c}=documentHarness(doc,{fileDirty:true,URL,setTimeout:callback=>{timers.push(callback);},
    el:(tag,_text,attrs)=>{assert.equal(tag,'a');return {...attrs,click(){links.push(this);},remove(){}};}});
  c.document.body={append(){}};
  try {
    await c.saveFile();assert.equal(links.length,1);
    assert.equal(links[0].download,'Two experiments.chrnb');
    const response=await fetch(links[0].href);
    assert.equal(response.headers.get('content-type'),'application/json');
    const reopened=documents.parseDocument(await response.text());
    assert.deepEqual(reopened,doc);assert.equal(reopened.queries.length,2);
    assert.deepEqual(reopened.layouts['query:second'],[['["query"]',{x:-30,y:40}]]);
    assert.equal(c.fileHandle,null);assert.equal(c.fileName,'Two experiments.chrnb');assert.equal(c.fileDirty,false);
    assert.deepEqual((await c.store.recovery('editor')).document,doc);
  } finally {timers.forEach(callback=>callback());}
});
for(const outcome of ['invalid','declined','format failure'])await test(`open ${outcome} preserves unsaved source, document, layouts and file identity`,async()=>{
  const doc=twoQueryDocument(),handle={name:'original.chrnb'};let formats=0,pauses=0;
  const {context:c,$}=documentHarness(doc,{fileHandle:handle,fileName:handle.name,fileDirty:true,dirty:true,
    window:{confirm:()=>outcome!=='declined'},session:{run:7,stopReading(){pauses++;}},
    request:async()=>{formats++;throw Error('Format unavailable');}});
  $('program').value='unsynced program';$('query').value='unsynced query';
  c.undo=[structuredClone(doc)];c.selection=[{path:['query']}];
  const incoming=outcome==='invalid'?{...doc,queries:[]}:documents.emptyNotebook();
  if(outcome==='declined')await c.openDocument(incoming,null,'incoming.chrnb');
  else await assert.rejects(c.openDocument(incoming,null,'incoming.chrnb'),outcome==='invalid'?/at least one query/:/Format unavailable/);
  assert.deepEqual(structuredClone(c.currentDocument()),doc);assert.equal(c.fileHandle,handle);
  assert.equal(c.fileName,'original.chrnb');assert.equal(c.fileDirty,true);assert.equal(c.dirty,true);assert.equal(c.fileBusy,false);
  assert.equal($('program').value,'unsynced program');assert.equal($('query').value,'unsynced query');
  assert.deepEqual(structuredClone(c.undo),[doc]);assert.deepEqual(structuredClone(c.selection),[{path:['query']}]);
  assert.equal(pauses,0);assert.equal(formats,outcome==='format failure'?1:0);
  assert.equal(await c.store.recovery('editor'),null);
});
await test('open a serialized notebook restores both query bodies, active query and layouts into editor recovery',async()=>{
  const doc=twoQueryDocument(),contents=documents.serializeDocument(doc),handle={name:'roundtrip.chrnb'};let pauses=0;
  const {context:c,$,calls}=documentHarness(documents.emptyNotebook(),{session:{run:7,stopReading(){pauses++;}}});
  await c.openDocument(documents.parseDocument(contents),handle,handle.name);
  assert.deepEqual(structuredClone(c.currentDocument()),doc);
  assert.deepEqual(structuredClone(c.model),{program:{rules:[]},query:{kind:'atom',atom:{relation:'second',args:['B']}}});
  assert.deepEqual(calls,[{program:{rules:[]},query:doc.queries[1].body}]);
  assert.equal($('program').value,'formatted program');assert.equal($('query').value,'formatted query');
  assert.equal(c.fileHandle,handle);assert.equal(c.fileName,'roundtrip.chrnb');assert.equal(c.fileDirty,false);
  assert.equal(c.dirty,false);assert.equal(c.fileBusy,false);assert.equal(c.needsQueryRun,true);assert.equal(pauses,1);
  const saved=await c.store.recovery('editor');assert.deepEqual(saved.document,doc);assert.equal(saved.fileName,handle.name);
  assert.deepEqual(documents.parseDocument(documents.serializeDocument(c.currentDocument())),doc);
});
for(const example of ['arithmetic','type-synthesis','behavior-synthesis','lambda'])await test(`open bundled ${example} validates the document and Save chooses a file`,async()=>{
  const doc=twoQueryDocument(),urls=[],messages=[];let pauses=0,picks=0,written;
  const handle={name:'my-example.chrnb',async queryPermission(){return 'granted';},async createWritable(){return {async write(text){written=text;},async close(){}};}};
  const {context:c,$}=documentHarness(documents.emptyNotebook(),{
    fileHandle:{name:'original.chrnb'},fileName:'original.chrnb',fileDirty:true,
    fetch:async url=>{urls.push(url);return {ok:true,text:async()=>documents.serializeDocument(doc)};},
    session:{run:7,stopReading(){pauses++;}},message:text=>messages.push(text),
    window:{confirm:()=>true,async showSaveFilePicker(){picks++;return handle;}}});
  $('examples').value=example;
  await c.openExample();
  assert.deepEqual(urls,[`/examples/${example}.chrnb`]);
  assert.deepEqual(structuredClone(c.currentDocument()),doc);
  assert.equal(c.fileHandle,null);assert.equal(c.fileName,'');assert.equal(c.fileDirty,true);
  assert.equal(c.needsQueryRun,true);assert.equal(pauses,1);assert.equal($('query-section').scrolled.block,'start');
  assert.match(messages.at(-1),/Opened example/);
  // Native file handles are cloneable; this test double is not.
  let saved;
  c.store.saveRecovery=async(_key,value)=>{saved={...value,fileHandle:value.fileHandle.name};};
  await c.saveFile();
  assert.equal(picks,1);assert.deepEqual(documents.parseDocument(written),doc);
  assert.equal(saved.fileName,handle.name);assert.equal(saved.fileDirty,false);
});
await test('example loading releases busy execution controls after the document is installed',async()=>{
  const doc=twoQueryDocument(),states=[];
  const {context:c,$}=documentHarness(doc,{fetch:async()=>({ok:true,text:async()=>JSON.stringify(doc)}),session:{run:null,stopReading(){}}});
  c.renderRun=()=>states.push(c.fileBusy);
  $('examples').value='arithmetic';
  await c.openExample();
  assert.equal(states[0],true);assert.equal(states.at(-1),false);
  assert.equal(c.fileBusy,false);assert.deepEqual(structuredClone(c.currentDocument()),doc);
});
for(const outcome of ['network','http','invalid','declined','format failure'])await test(`bundled example ${outcome} preserves the user's notebook`,async()=>{
  const doc=twoQueryDocument(),handle={name:'original.chrnb'};let pauses=0,formats=0;
  const {context:c,$}=documentHarness(doc,{fileHandle:handle,fileName:handle.name,fileDirty:true,dirty:true,
    window:{confirm:()=>outcome!=='declined'},session:{run:7,stopReading(){pauses++;}},
    fetch:async()=>{if(outcome==='network')throw Error('Network unavailable');return {ok:outcome!=='http',status:404,
      text:async()=>JSON.stringify(outcome==='invalid'?{...doc,queries:[]}:documents.emptyNotebook())};},
    request:async()=>{formats++;throw Error('Format unavailable');}});
  $('examples').value='arithmetic';$('program').value='unsynced program';$('query').value='unsynced query';
  c.undo=[structuredClone(doc)];c.selection=[{path:['query']}];
  if(outcome==='declined')await c.openExample();
  else await assert.rejects(c.openExample(),/Network unavailable|404|at least one query|Format unavailable/);
  assert.deepEqual(structuredClone(c.currentDocument()),doc);assert.equal(c.fileHandle,handle);
  assert.equal(c.fileName,handle.name);assert.equal(c.fileDirty,true);assert.equal(c.dirty,true);assert.equal(c.fileBusy,false);
  assert.equal($('program').value,'unsynced program');assert.equal($('query').value,'unsynced query');
  assert.deepEqual(structuredClone(c.undo),[doc]);assert.deepEqual(structuredClone(c.selection),[{path:['query']}]);
  assert.equal(pauses,0);assert.equal(formats,outcome==='format failure'?1:0);
  assert.equal(await c.store.recovery('editor'),null);
});
await test('Cancel remains available to recover a lost first Start response', async () => {
  await controlSession('cancel-button', async ({session, model, lose, runs}) => {
    lose('start'); await assert.rejects(session.start(model, false, false), /Lost start/);
    const elements = new Map();
    const $ = name => { if (!elements.has(name)) elements.set(name, {replaceChildren() {}}); return elements.get(name); };
    const context = createContext({$, session, check:assert.ok, el:() => ({}), inspecting:false, launching:false, busy:false,
      inspectionPending:null, inspectionSelection:session.selection, inspectionCanceled:false,
      message() {}, renderResults() {}, refreshSaved() {}, safe:action => action(), inspect() {},
    });
    runInContext(productionSection('  function renderRun()', '  function refreshSaved()')
      + productionSection('  async function finishInspection()', '  function renderInspectionControls()')
      + productionSection("  $('pause').onclick", "  $('alternatives').onchange"), context);
    context.renderRun();
    assert.equal($('cancel').disabled, false);
    await $('cancel').onclick();
    assert.equal(runs.get(1).canceled, true); assert.equal(session.status, 'canceled');
  });
});
function renderingHarness() {
  const elements = new Map(), reads = [], paints = [], errors = [], tasks = [];
  const element = () => ({disabled:false, value:'', children:[],
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    toggleAttribute() {},
  });
  const $ = name => { if (!elements.has(name)) elements.set(name, element()); return elements.get(name); };
  const context = createContext({$, el:element, button:element, observedCollection:null,
    uint(value) { assert.ok(Number.isSafeInteger(value) && value >= 0); return value; },
    safe(action) { const task = Promise.resolve().then(action).catch(error => { errors.push(error); }); tasks.push(task); return task; },
    store:{scene(collection, number, options) { return new Promise(resolve => reads.push({collection,number,options,resolve})); }},
    renderScene(svg, scene) { paints.push(svg); return scene; },
    session:{stream:null, archive:'first', async switchRun(run) { assert.equal(run, 2); }},
    renderInspectionControls() {}, async refreshSaved() {}, async finishInspection() {},
    savedView:{id:'first',page:0,total:1,answers:[{number:1,completion:'1',alternative:'0'}]},
    savedSelection:'', inspected:null, outputMode:'answers', answerNumber:1, answerPage:0,
    page:0, portPage:0,   bindingPage:1,
    pendingNumber:1,
    sceneLoading:false, desiredScene:null, loadedSceneKey:null,
  });
  runInContext(productionSection('  function renderResults()', '  async function inspect()')
    + productionSection("  $('execution').onchange", "  $('release-run').onclick"), context);
  const scene = () => ({bindings:[],bindingPage:1,bindingPages:3,
    facts:{entries:[],page:1,pages:3,portPage:1,portPages:3,count:54},
    pending:{index:1,count:3,event:'1',path:[42,43],breadcrumbs:[],
      scene:{entries:[],page:1,pages:3,portPage:1,portPages:3,count:54}},
  });
  return {context,$,reads,paints,errors,tasks,scene,
    async start() { context.renderResults(); await Promise.resolve(); assert.equal(reads.length, 1); },
    async paint() { reads[0].resolve(scene()); await Promise.all(tasks); assert.equal(errors.length, 0); },
  };
}
for (const [control, field] of [
  ['binding','bindingPage'], ['pending-body','pendingNumber'],
]) for (const [direction, expected] of [['prev',0], ['next',2]]) {
  await test(`production ${control} ${direction}: rapid clicks target the displayed page`, async () => {
    const h = renderingHarness(); await h.start(); await h.paint();
    const controlElement = h.$(`${control}-${direction}`);
    assert.equal(controlElement.disabled, false);
    controlElement.onclick(); await Promise.resolve();
    controlElement.onclick(); await Promise.resolve();
    assert.equal(h.reads.length, 2, 'only one additional scene read is in flight');
    assert.equal(h.context.desiredScene.options[field], expected, 'both clicks request the same adjacent page');
    assert.ok(Object.entries(h.context.desiredScene.options).every(([key,value]) => value >= 0));
    h.reads[1].resolve(h.scene()); await Promise.all(h.tasks);
    assert.equal(h.errors.length, 0);
    assert.equal(h.reads.length, 2, 'rapid clicks do not schedule a further page');
  });
}
await test('production execution switch resets navigation and rejects an outstanding old scene', async () => {
  const h = renderingHarness(); await h.start();
  h.$('execution').value = '2';
  await h.$('execution').onchange();
  for (const field of ['answerPage','bindingPage','pendingNumber']) {
    assert.equal(h.context[field], 0, `${field} resets for the new execution`);
  }
  assert.equal(h.context.answerNumber, null);
  assert.equal(h.context.desiredScene, null, 'the old scene request is invalidated before loading new summaries');
  h.reads[0].resolve(h.scene()); await Promise.all(h.tasks);
  assert.equal(h.paints.length, 0, 'a late old-execution scene cannot paint');
  assert.equal(h.context.loadedSceneKey, null);
  assert.equal(h.errors.length, 0);
});


await test('quota cancellation discards an unfinished cached suffix without flushing it', async () => {
// Cancellation must not allocate storage for an unfinished cached suffix.
const quotaCancelStore = memorySink();
let quotaCancelBlocked = false;
quotaCancelStore.beforeCommit = () => { if (quotaCancelBlocked) throw new Error('Partial quota failure'); };
const partialResponse = {sequence:21, events:answer(200).slice(0,5), applications:0, exhausted:false, delivery_done:false};
const quotaCancelSession = testSession(async route => {
  if (route === 'start') return {run:40, ...tables};
  if (route === 'output') return partialResponse;
  if (route === 'cancel') return {pending:partialResponse};
  throw new Error(`Unexpected ${route}`);
}, () => {}, quotaCancelStore);
await quotaCancelSession.start(untouched, false, false);
quotaCancelBlocked = true;
await assert.rejects(quotaCancelSession.readOutput(), /Partial quota failure/);
assert.ok(quotaCancelSession.stream.current);
assert.equal(quotaCancelSession.pendingDelivery.index, partialResponse.events.length);
await quotaCancelSession.cancel();
assert.equal(quotaCancelSession.ack, 21);
assert.equal(quotaCancelSession.pendingDelivery, null);
assert.equal(quotaCancelSession.stream.current, null);
assert.equal(quotaCancelSession.stream.writes.length, 0);
assert.equal(quotaCancelStore.archives.get(quotaCancelSession.archive).parts.size, 0);
assert.equal(quotaCancelStore.archives.get(quotaCancelSession.archive).answers.size, 0);

});

await test('cancel retries complete cached alternatives without consuming the unfinished suffix', async () => {
  const store = memorySink();
  let blocked = true, advances = 0;
  store.beforeCommit = records => {
    if (blocked || records.some(record => record.value.number === 3)) throw Error('Quota');
  };
  const response = {sequence:31, events:[...answer(201), ...answer(202), ...answer(203).slice(0,5)]};
  const notifications = [];
  const session = testSession(async route => {
    if (route === 'start') return {run:41, ...tables};
    if (route === 'output') { advances++; return response; }
    if (route === 'cancel') return {pending:response};
    throw Error(route);
  }, () => notifications.push({error:session.error, ack:session.ack}), store);
  await session.start(untouched, false, false);
  session.stream = new OutputAssembler(tables, 8);
  let pushes = 0;
  const push = session.stream.push.bind(session.stream);
  session.stream.push = event => { push(event); pushes++; };
  await assert.rejects(session.readOutput(), /Quota/);
  const index = session.pendingDelivery.index;
  assert.ok(index < answer(201).length);
  await assert.rejects(session.cancel(), /Quota/);
  assert.equal(session.pendingDelivery.index, index);
  assert.equal(session.ack, null);
  assert.ok(session.error);
  blocked = false;
  await session.cancel();
  assert.equal(advances, 1);
  assert.equal(pushes, 2 * answer(201).length);
  assert.equal(session.ack, 31);
  assert.equal(session.error, null);
  assert.deepEqual(notifications.at(-1), {error:null, ack:31});
  const saved = store.archives.get(session.archive);
  assert.equal(saved.answers.size, 2);
  assert.equal(saved.parts.size, 20);
  assert.equal(session.stream.current, null);
  assert.equal(session.stream.writes.length, 0);
});

for (const complete of [false, true]) await test(`inspection cancellation preserves cached completions (${complete}) under quota`, async () => {
  const store = memorySink(), stream = new OutputAssembler(tables, 8);
  const archive = await store.create(tables, 'Inspection');
  const events = complete ? [...answer(211), ...answer(212).slice(0,5)] : answer(211).slice(0,5);
  const pending = {stream, archive, response:{sequence:42, events}, index:0, ack:null, run:42, inspection:7};
  for (; pending.index < 5; pending.index++) stream.push(events[pending.index]);
  let blocked = true;
  store.beforeCommit = records => {
    if (blocked || records.some(record => record.value.number === 2)) throw Error('Inspection quota');
  };
  const calls = [];
  const context = createContext({store, deliverCachedOutput, inspectionPending:pending, inspectionCanceled:true,
    liveRequest:async route => { calls.push(route); return {done:true, pending:pending.response}; },
    inspected:null, outputMode:'answers', answerNumber:1, answerPage:0,
    resetResultPages() {}, async refreshSaved() {}, message() {},
  });
  runInContext(productionSection('  async function inspectOnce()', '  function renderInspectionControls()'), context);
  if (complete) {
    await assert.rejects(context.inspectOnce(), /Inspection quota/);
    assert.equal(pending.index, 5);
    assert.equal(pending.ack, null);
    assert.deepEqual(calls, ['inspect_cancel']);
    calls.length = 0;
    blocked = false;
  }
  await context.inspectOnce();
  assert.deepEqual(calls, ['inspect_cancel', 'inspect_release']);
  assert.equal(context.inspectionPending, null);
  assert.equal(pending.ack, 42);
  assert.equal(stream.current, null);
  assert.equal(stream.writes.length, 0);
  assert.equal(store.archives.get(archive).answers.size, complete ? 1 : 0);
  assert.equal(store.archives.get(archive).parts.size, complete ? 10 : 0);
});

await test('inspection cancel recovers a lost cached batch and ignores its acknowledged replay', async () => {
  const store = memorySink(), stream = new OutputAssembler(tables, 8);
  const archive = await store.create(tables, 'Lost inspection');
  const batch = {sequence:51, events:[...answer(221), ...answer(222).slice(0,5)]};
  const pending = {stream, archive, response:null, index:0, ack:null, run:51, inspection:8};
  let cancels = 0;
  const context = createContext({store, deliverCachedOutput, inspectionPending:pending, inspectionCanceled:true,
    liveRequest:async route => {
      if (route === 'inspect_cancel') return {done:++cancels === 2, pending:batch};
      assert.equal(route, 'inspect_release'); return {};
    },
    inspected:null, outputMode:'answers', answerNumber:1, answerPage:0,
    resetResultPages() {}, async refreshSaved() {}, message() {},
  });
  runInContext(productionSection('  async function inspectOnce()', '  function renderInspectionControls()'), context);
  await context.inspectOnce();
  assert.equal(cancels, 2);
  assert.equal(pending.ack, 51);
  assert.equal(stream.total, 1);
  assert.equal(store.archives.get(archive).answers.size, 1);
  assert.equal(store.archives.get(archive).parts.size, 10);
  assert.equal(stream.current, null);
});

for (const active of [false, true]) await test(`Cancel resumes only a quiescent inspection (active=${active})`, async () => {
  const controls = new Map(), calls = [];
  const context = createContext({$:id => {
    if (!controls.has(id)) controls.set(id, {});
    return controls.get(id);
  }, safe:action => action(), check:assert.ok, inspectionCanceled:false, inspecting:active, inspectionPending:{},
  session:{run:null, async cancel() { calls.push('cancel'); }}, async inspect() { calls.push('inspect'); context.inspectionPending = null; }});
  runInContext(productionSection('  async function finishInspection()', '  function renderInspectionControls()')
    + productionSection("  $('cancel').onclick", "  $('alternatives').onchange"), context);
  await controls.get('cancel').onclick();
  assert.equal(context.inspectionCanceled, true);
  assert.deepEqual(calls, active ? ['cancel'] : ['cancel', 'inspect']);
});

for (const action of ['release', 'switch', 'start']) await test(`${action} preserves a pending inspection until persistence and release retry complete`, async () => {
  const store = memorySink(), stream = new OutputAssembler(tables);
  const archive = await store.create(tables, 'Unfinished inspection');
  const pending = {run:1, inspection:'90', stream, archive, response:{sequence:1, events:answer(300)}, index:0, ack:null};
  let quota = true, released = false, loseRelease = true, changes = 0, cancelCalls = 0;
  store.beforeCommit = () => { if (quota) throw Error('Inspection quota'); };
  const controls = new Map(), $ = name => { if (!controls.has(name)) controls.set(name, {}); return controls.get(name); };
  let context;
  const change = async target => {
    assert.equal(context.inspectionPending, null); assert.equal(released, true);
    assert.equal(store.archives.get(archive).answers.size, 1);
    if (action === 'switch') assert.equal(target, 2, 'cleanup repaint must not change the requested execution');
    changes++; context.session.run = action === 'release' ? null : 2;
  };
  context = createContext({$, safe:operation => operation(), check:assert.ok, store, deliverCachedOutput,
    session:{run:1, closeRun:change, switchRun:change, start:change}, inspectionPending:pending, inspecting:false, inspectionCanceled:false,
    async liveRequest(route) {
      if (route === 'inspect_cancel') { assert.equal(released, false); cancelCalls++; return {done:true}; }
      assert.equal(route, 'inspect_release'); released = true;
      if (loseRelease) { loseRelease = false; throw Error('Lost inspection release'); }
      return {};
    },
    model:untouched, launching:false, inspected:null, outputMode:'inspect', savedSelection:'', savedView:null,
    answerNumber:1, answerPage:0, resetResultPages() {}, async refreshSaved() {}, message() {},
    renderRun() { $('execution').value = '1'; }, renderResults() {}, renderInspectionControls() {}, async syncSource() {},
  });
  runInContext(productionSection('  async function inspect()', '  function renderInspectionControls()')
    + productionSection("  $('run').onclick", "  $('pause').onclick"), context);
  $('execution').value = '2';
  const invoke = () => {
    $('execution').value = '2';
    return action === 'release' ? $('release-run').onclick() : action === 'switch' ? $('execution').onchange() : $('run').onclick();
  };
  await assert.rejects(invoke(), /Inspection quota/);
  assert.equal(changes, 0); assert.equal(context.session.run, 1); assert.equal(context.inspectionPending, pending);
  quota = false;
  await assert.rejects(invoke(), /Lost inspection release/);
  assert.equal(changes, 0); assert.equal(context.session.run, 1); assert.equal(context.inspectionPending, pending);
  const cancels = cancelCalls;
  await invoke();
  assert.equal(cancelCalls, cancels, 'release retry never contacts the released job for cancellation');
  assert.equal(changes, 1); assert.equal(context.inspectionPending, null);
  assert.equal(store.archives.get(archive).answers.size, 1);
});

await test('clearing the last catalog entry resets the cursor only after successful storage', async () => {
  const controls = new Map(); let blocked = true;
  const context = createContext({$:id => {
    if (!controls.has(id)) controls.set(id, {});
    return controls.get(id);
  }, safe:action => action(), check:assert.ok, window:{confirm:() => true},
  savedView:{id:'last'}, session:{archive:'other',run:1}, inspectionPending:null, inspected:null,
  store:{async clear() { if (blocked) throw Error('Clear failed'); }},
  catalogDirty:false, catalogCursor:'last-page', catalogDirection:'prev',
  savedSelection:'last', answerNumber:1, answerPage:0, async refreshSaved() {}, message() {}});
  runInContext(productionSection("  $('clear-answers').onclick", '  renderWorkspace(); renderRun(); renderInspectionControls();'), context);
  await assert.rejects(controls.get('clear-answers').onclick(), /Clear failed/);
  assert.equal(context.catalogCursor, 'last-page');
  blocked = false;
  await controls.get('clear-answers').onclick();
  assert.equal(context.catalogCursor, null);
  assert.equal(context.catalogDirection, 'next');
  assert.equal(context.catalogDirty, true);
});

await test('cancel persists buffered summaries while omitting already consumed quota suffix writes', async () => {
  const store = memorySink(), stream = new OutputAssembler(tables);
  const archive = await store.create(tables, 'Buffered');
  const response = {sequence:61, events:[...answer(231), ...answer(232).slice(0,5)]};
  const pending = {response, index:0};
  store.beforeCommit = records => {
    if (records.some(record => record.value.number === 2)) throw Error('Suffix quota');
  };
  await assert.rejects(deliverCachedOutput(store, archive, stream, pending), /Suffix quota/);
  assert.equal(pending.index, response.events.length);
  await deliverCachedOutput(store, archive, stream, pending, true);
  assert.equal(store.archives.get(archive).answers.size, 1);
  assert.equal(store.archives.get(archive).parts.size, 10);
  assert.equal(stream.current, null);
  assert.equal(stream.writes.length, 0);
});

await test('refreshing answers respects a collapsed Observation panel',async()=>{
  const h=renderingHarness();await h.start();
  h.$('observations').open=false;
  h.context.renderResults();
  assert.equal(h.$('observations').open,false);
  h.reads[0].resolve(h.scene());await Promise.all(h.tasks);
});

await test('changing the selected archive cannot reload stale saved summaries', async () => {
  const h = renderingHarness(); await h.start();
  runInContext(productionSection("  $('saved-run').onchange", '  const changeSavedPage'), h.context);
  h.$('saved-run').value = 'second';
  await h.$('saved-run').onchange();
  h.context.renderResults();
  assert.equal(h.context.desiredScene, null);
  assert.equal(h.context.answerNumber, null);
  h.reads[0].resolve(h.scene()); await Promise.all(h.tasks);
  assert.equal(h.paints.length, 0);
  assert.equal(h.reads.length, 1);
  assert.equal(h.errors.length, 0);
});

await test('a previous answer page cannot overwrite the locator while its requested page loads', async () => {
  const h = renderingHarness(); await h.start();
  h.context.answerPage = 1; h.context.answerNumber = 13;
  h.context.renderResults();
  assert.equal(h.context.answerNumber, 13);
  assert.equal(h.context.desiredScene, null);
  h.reads[0].resolve(h.scene()); await Promise.all(h.tasks);
  assert.equal(h.paints.length, 0);
  h.context.savedView = {id:'first',page:1,total:14,answers:[{number:13,completion:'1',alternative:'12'},{number:14,completion:'1',alternative:'13'}]};
  h.context.renderResults(); await Promise.resolve();
  assert.equal(h.context.answerNumber, 13);
  assert.equal(h.reads[1].number, 13);
  h.reads[1].resolve(h.scene()); await Promise.all(h.tasks);
  assert.equal(h.errors.length, 0);
});

for (const action of ['close', 'start', 'switch', 'cancel', 'resume', 'step']) await test(`lost close response recovers through ${action} without contacting the reclaimed run`, async () => {
  const calls = [], live = new Set(); let next = 0, lost = false;
  const session = testSession(async (route, payload) => {
    calls.push([route, payload?.run]);
    if (route === 'start') { live.add(++next); return {run:next, ...tables}; }
    if (route === 'close') {
      live.delete(payload.run);
      if (!lost) { lost = true; throw Error('Lost close response'); }
      return {closed:true};
    }
    assert.ok(live.has(payload.run), `${route} must not contact a reclaimed run`);
    if (route === 'cancel') return {};
    throw Error(route);
  });
  await session.start(untouched, false, false);
  await session.start(untouched, false, false); // Keep run 1 available for switching.
  let resets = 0;
  session.selection.reset = async () => { resets++; };
  await assert.rejects(session.closeRun(), /Lost close response/);
  assert.equal(session.run, 2);
  const before = calls.length;
  if (action === 'close') await session.closeRun();
  if (action === 'start') await session.start(untouched, false, false);
  if (action === 'switch') await session.switchRun(1);
  if (action === 'cancel') await session.cancel();
  if (action === 'resume') await assert.rejects(session.resume(), /Start a run first/);
  if (action === 'step') await assert.rejects(session.step(), /Start a run first/);
  assert.deepEqual(calls[before], ['close', 2]);
  assert.equal(calls.slice(before).filter(([route]) => route === 'cancel').length, 0);
  assert.equal(session.runs.has(2), false);
  if (action === 'close' || action === 'cancel') assert.equal(resets, 1);
  assert.equal(session.run, action === 'switch' ? 1 : action === 'start' ? 3 : null);
  if (action !== 'start') await session.start(untouched, false, false);
  assert.equal(session.run, 3);
});

for (const outcome of ['cancel', 'done', 'error']) await test(`inspection ${outcome} retries only release after a lost release response`, async () => {
  const store = memorySink(), stream = new OutputAssembler(tables);
  const archive = await store.create(tables, 'Release retry');
  const response = {sequence:71, events:answer(241), done:true};
  if (outcome === 'error') response.error = 'Projection failed';
  const pending = {stream, archive, response, index:0, ack:null, run:71, inspection:9};
  const calls = []; let released = false;
  const context = createContext({store, deliverCachedOutput, inspectionPending:pending, inspectionCanceled:outcome === 'cancel',
    liveRequest:async route => {
      calls.push(route);
      if (route === 'inspect_release') {
        if (!released) { released = true; throw Error('Lost release response'); }
        return {};
      }
      assert.equal(released, false, 'no inspection control after release');
      assert.equal(route, 'inspect_cancel'); return {done:true, pending:response};
    },
    inspected:null, outputMode:'answers', answerNumber:1, answerPage:0,
    resetResultPages() {}, async refreshSaved() {}, message() {},
  });
  runInContext(productionSection('  async function inspectOnce()', '  function renderInspectionControls()'), context);
  await assert.rejects(context.inspectOnce(), /Lost release response/);
  assert.equal(store.archives.get(archive).answers.size, 1);
  const before = calls.length;
  // A release retry must not require further storage writes either.
  store.discardPartial = async () => { throw Error('Unexpected repeated persistence'); };
  if (outcome === 'error') await assert.rejects(context.inspectOnce(), /Inspection failed: Projection failed/);
  else await context.inspectOnce();
  assert.deepEqual(calls.slice(before), ['inspect_release']);
  assert.equal(context.inspectionPending, null);
  assert.equal(stream.total, 1);
});

function reloadServer(batch) {
  const calls = [], effects = [], views = new Map();
  let receipt = null, loseStep = false;
  const fetcher = async (url, options) => {
    const route = url.slice(5), payload = JSON.parse(options.body);
    calls.push({route,payload});
    let response = {};
    if (route === 'hello') response = {boot:'f'.repeat(32)};
    else if (route === 'reserve') response = {owner:1};
    else if (payload.command !== undefined) {
      if (receipt?.command === payload.command) {
        assert.equal(receipt.body,options.body); response = receipt.response;
      } else {
        effects.push(route);
        if (route === 'start') response = {run:1,...tables};
        if (route === 'inspect') { response = {inspection:'1'}; views.set('1',true); }
        if (route === 'step') response = {step:{done:false}};
        receipt = {command:payload.command,body:options.body,response};
      }
      if (route === 'step' && loseStep) { loseStep=false; throw Error('Lost accepted Step'); }
    } else if (route === 'status') response = {canceled:false};
    else if (route === 'output' || route === 'inspect_advance') response = batch;
    else if (route === 'cancel' || route === 'inspect_cancel') response = {done:true,pending:batch};
    else if (route === 'inspect_release') views.delete(payload.inspection);
    return {ok:true,status:200,text:async()=>JSON.stringify(response)};
  };
  return {fetcher,calls,effects,views,loseStep:()=>{loseStep=true;}};
}
for (const afterCommit of [false,true]) await test(`reload replays scalar checkpoint after ${afterCommit ? 'committed' : 'rejected'} mid-answer flush`,async()=>{
  const store=memorySink(), batch={sequence:81,events:[...answer(801),...answer(802)],applications:2,delivery_done:true,exhausted:true};
  const server=reloadServer(batch);
  const first=testConnection(store,server.fetcher), session=new RunSession(first.request,()=>{},store);
  await session.start(untouched,false,false);
  session.stream=new OutputAssembler(tables,8);
  let flushes=0;
  store[afterCommit ? 'afterCommit' : 'beforeCommit']=()=>{if (++flushes===2) throw Error('Page disappeared');};
  await assert.rejects(session.readOutput(),/Page disappeared/);
  const saved=await store.recovery('live:run:1');
  assert.equal(saved.sequence,81); assert.equal(saved.ack,null); assert.ok(saved.index>0);
  assert.equal(saved.response,undefined); assert.equal(JSON.stringify(saved).includes('"events"'),false);
  const index=saved.index, archive=session.archive;
  await first.close(); store.beforeCommit=store.afterCommit=null;
  const second=testConnection(store,server.fetcher), restored=new RunSession(second.request,()=>{},store);
  const before=server.calls.length;
  await restored.restore();
  assert.equal(restored.status,'done'); assert.equal(restored.running,false);
  assert.equal(server.calls.slice(before).some(call=>call.route==='output'||call.route==='resume'),false);
  assert.equal(restored.pendingDelivery.index,index);
  let pushes=0; const push=restored.stream.push.bind(restored.stream);
  restored.stream.push=event=>{push(event);pushes++;};
  await restored.readOutput();
  assert.equal(pushes,batch.events.length-index);
  assert.equal(server.calls.at(-1).payload.ack,null);
  assert.equal(restored.ack,81); assert.equal(restored.stream.total,2);
  assert.equal(store.archives.get(archive).answers.size,2);
  assert.equal(store.archives.get(archive).parts.size,20);
  const complete=await store.recovery('live:run:1');
  assert.equal(complete.sequence,null); assert.equal(complete.index,0); assert.equal(complete.ack,81);
  await second.close();
});
await test('reload of accepted Step polls its application without issuing a second Step',async()=>{
  const store=memorySink(),server=reloadServer({sequence:82,events:[],applications:1,delivery_done:false,exhausted:false,step:{done:true,event:'1',rule:0,shared:false}});
  const first=testConnection(store,server.fetcher),session=new RunSession(first.request,()=>{},store);
  await session.start(untouched,false,false);
  server.loseStep(); await assert.rejects(session.step(),/Lost accepted Step/);
  await first.close();
  const second=testConnection(store,server.fetcher),restored=new RunSession(second.request,()=>{},store);
  await restored.restore(); assert.equal(restored.stepPending,true);
  const result=await restored.step(); assert.equal(result.applications,1);
  assert.equal(server.effects.filter(route=>route==='step').length,1);
  assert.equal((await store.recovery('live:run:1')).stepPending,false);
  await second.close();
});
await test('reload closes a closing run before accessing its absent source or archive',async()=>{
  const store=memorySink(),server=reloadServer({events:[]});
  const first=testConnection(store,server.fetcher);
  await first.request('start',{...untouched,record_history:false}); await first.close();
  const saved=await store.recovery('live:run:1');
  await store.saveRecovery('live:run:1',{...saved,phase:'closing'});
  await store.saveRecovery('live:source:1',null);
  store.tables=async()=>{throw Error('Closing metadata must not be read');};
  const second=testConnection(store,server.fetcher),restored=new RunSession(second.request,()=>{},store);
  await restored.restore(); assert.equal(restored.run,null); assert.equal(restored.runs.size,0);
  assert.equal(await store.recovery('live:run:1'),null);
  assert.equal(server.calls.at(-1).route,'close'); await second.close();
});
for (const canceled of [false,true]) await test(`production inspection reload ${canceled ? 'cancels' : 'finishes'} the retained scalar batch`,async()=>{
  const store=memorySink(),batch={sequence:91,events:[...answer(901),...answer(902)],done:true};
  const server=reloadServer(batch),first=testConnection(store,server.fetcher);
  await first.request('start',{...untouched,record_history:false});
  const view=await first.request('inspect',{run:1,choices:{}});
  const key='live:inspection:1:1',stream=new OutputAssembler(tables,8),pending={response:batch,index:0};
  let writes=0;
  store.beforeCommit=()=>{if(++writes===2) throw Error('Reload inspection');};
  await assert.rejects(deliverCachedOutput(store,view.archive,stream,pending,false,{
    id:key,serialize:first.request.checkpoint,value:()=>({phase:'active'})}),/Reload inspection/);
  const saved=await store.recovery(key); assert.ok(saved.index>0); assert.equal(saved.response,undefined);
  if(canceled) await store.saveRecovery(key,{...saved,phase:'canceling'});
  store.beforeCommit=null; await first.close();
  const second=testConnection(store,server.fetcher); await second.initialize();
  const context=createContext({store,request:second.request,liveRequest:(route,payload)=>second.request(route,payload),
    session:{run:1},OutputAssembler,deliverCachedOutput,inspectionPending:null,inspectionCanceled:false,
    inspected:null,outputMode:'answers',answerNumber:null,answerPage:0,
    resetResultPages(){},async refreshSaved(){},message(){}});
  runInContext(productionSection('  async function inspectOnce()', '  function renderInspectionControls()'),context);
  await context.inspectOnce();
  assert.equal(server.effects.filter(route=>route==='inspect').length,1);
  assert.equal(store.archives.get(view.archive).answers.size,2);
  assert.equal(store.archives.get(view.archive).parts.size,20);
  assert.equal(await store.recovery(key),null);
  assert.equal(server.views.size,0);
  const replay=server.calls.findLast(call=>call.route===(canceled?'inspect_cancel':'inspect_advance'));
  assert.ok(replay); if(!canceled) assert.equal(replay.payload.ack,null);
  await second.close();
});
await test('metadata reload retains the selected lease and retires only other owned snapshots',async()=>{
  const store=memorySink(),selection=new InspectionSelection();
  selection.update([{id:'10',label:'Choice 10'}],[]); selection.choose('10','second');
  selection.lease={run:1,snapshot:'11'};
  await store.saveRecovery('live:snapshot:1:11',{run:1,snapshot:'11',phase:'active'});
  await store.saveRecovery('live:snapshot:1:12',{run:1,snapshot:'12',phase:'release'});
  const saved=selection.checkpoint(),restored=new InspectionSelection(),releases=[];
  await restored.restore(store,async(route,payload)=>{releases.push([route,payload]);await store.saveRecovery(`live:snapshot:1:${payload.snapshot}`,null);},1,saved);
  assert.deepEqual(restored.payload(1),{run:1,choices:{'10':false}});
  assert.deepEqual(restored.lease,{run:1,snapshot:'11'});
  assert.deepEqual(releases,[['snapshot_release',{run:1,snapshot:'12'}]]);
});
await test('reload completes source cancellation from its retained batch and keeps completed answers',async()=>{
  const store=memorySink(),batch={sequence:92,events:[...answer(921),...answer(922).slice(0,5)],applications:1,delivery_done:false,exhausted:false};
  const server=reloadServer(batch),first=testConnection(store,server.fetcher),session=new RunSession(first.request,()=>{},store);
  await session.start(untouched,false,false); session.stream=new OutputAssembler(tables,8);
  let writes=0;store.beforeCommit=()=>{if(++writes===2)throw Error('Reload cancellation');};
  await assert.rejects(session.readOutput(),/Reload cancellation/);
  const saved=await store.recovery('live:run:1');
  await store.saveRecovery('live:run:1',{...saved,phase:'canceling'});
  await first.close();store.beforeCommit=null;
  const second=testConnection(store,server.fetcher),restored=new RunSession(second.request,()=>{},store);
  const before=server.calls.length;
  await restored.restore();
  assert.equal(restored.status,'canceled'); assert.equal(restored.ack,92);
  assert.equal(restored.stream.current,null);assert.equal(restored.stream.total,1);
  assert.equal(store.archives.get(restored.archive).parts.size,10);
  assert.equal(server.calls.slice(before).some(call=>call.route==='output'),false);
  await restored.deliverPending();
  assert.equal((await store.recovery('live:run:1')).phase,'canceled','empty checkpoint preserves terminal phase');
  await second.close();
});
await test('reload rejects a different retained sequence before appending any scalar',async()=>{
  const store=memorySink(),batch={sequence:93,events:answer(931),applications:1,delivery_done:true};
  const server=reloadServer(batch),first=testConnection(store,server.fetcher),session=new RunSession(first.request,()=>{},store);
  await session.start(untouched,false,false);session.stream=new OutputAssembler(tables,8);
  store.beforeCommit=()=>{throw Error('Reload');};await assert.rejects(session.readOutput(),/Reload/);
  await first.close();store.beforeCommit=null;batch.sequence=94;
  const second=testConnection(store,server.fetcher),restored=new RunSession(second.request,()=>{},store);
  await restored.restore();const checkpoint=await store.recovery('live:run:1');
  await assert.rejects(restored.readOutput(),/Retained output sequence changed/);
  assert.deepEqual(await store.recovery('live:run:1'),checkpoint);
  assert.equal(store.archives.get(restored.archive).parts.size,0);await second.close();
});
for(const admitted of [false,true]) await test(`mount restores editor with controller admission ${admitted}`,async()=>{
  const store=memorySink(),boot='a'.repeat(32),selection=new InspectionSelection();
  await store.saveRecovery('editor',{model:untouched,program:'unsynced program',query:'unsynced query',dirty:true,history:true,boot,run:7,selection:selection.checkpoint()});
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{});return controls.get(name);};
  let restored=false,refreshed=false;
  const context=createContext({$,store,connected:false,restoring:true,editorWriting:null,editorDirty:false,
    model:{program:{rules:[]},query:{kind:'true'}},dirty:false,clone:structuredClone,validateNotebook,
    connection:{state:{boot},async initialize(){if(!admitted)throw Error('Other tab');}},
    session:{run:null,async restore(run){assert.equal(run,7);restored=true;}},inspectionSelection:selection,
    renderWorkspace(){},renderRun(){},renderInspectionControls(){},async refreshSaved(){refreshed=true;}});
  runInContext(productionSection('  function saveEditor()', '  function message('),context);
  if(admitted)await context.initialize();else await assert.rejects(context.initialize(),/Other tab/);
  assert.equal(context.connected,admitted);assert.equal(restored,admitted);assert.equal(refreshed,true);
  assert.equal($('program').value,'unsynced program');assert.equal($('query').value,'unsynced query');
  assert.equal(context.dirty,true);assert.equal($('history').checked,true);
  assert.deepEqual(structuredClone(context.model),untouched);
});
await test('a canceled advance preserves completions and discards the unfinished suffix',async()=>{
  const store=memorySink(),batch={sequence:95,events:[...answer(951),...answer(952).slice(0,5)],applications:1,delivery_done:false,canceled:true};
  const session=testSession(async route=>route==='start'?{run:95,...tables}:route==='output'?batch:{},()=>{},store);
  await session.start(untouched,false,false);
  session.running=true;
  await session.readOutput();
  assert.equal(session.status,'canceled');assert.equal(session.running,false);assert.equal(session.timer,null);
  assert.equal(session.stream.current,null);assert.equal(session.stream.total,1);
  const saved=await store.recovery('live:run:95');
  assert.equal(saved.phase,'canceled');assert.equal(saved.status.canceled,true);assert.equal(saved.ack,95);
  assert.equal(store.archives.get(session.archive).parts.size,10);
});
await test('reload honors server cancellation when the durable phase still says paused',async()=>{
  const store=memorySink(),batch={sequence:96,events:answer(961).slice(0,5),applications:0,delivery_done:false,canceled:false};
  const server=reloadServer(batch),first=testConnection(store,server.fetcher),session=new RunSession(first.request,()=>{},store);
  await session.start(untouched,false,false);await session.readOutput();await first.close();
  assert.equal((await store.recovery('live:run:1')).phase,'paused');
  const fetcher=(url,options)=>url==='/api/status'
    ?Promise.resolve({ok:true,status:200,text:async()=>JSON.stringify({canceled:true,applications:0})})
    :server.fetcher(url,options);
  const second=testConnection(store,fetcher),restored=new RunSession(second.request,()=>{},store);
  await restored.restore();
  assert.equal(restored.status,'canceled');assert.equal(restored.stream.current,null);
  assert.equal(store.archives.get(restored.archive).parts.size,0);
  assert.equal(server.calls.at(-1).route,'cancel');await second.close();
});

await test('inspection reload honors canceled delivery when its journal phase is active',async()=>{
  const store=memorySink(),batch={sequence:97,events:[...answer(971),...answer(972).slice(0,5)],done:true,canceled:true};
  const server=reloadServer(batch),first=testConnection(store,server.fetcher);
  await first.request('start',{...untouched,record_history:false});
  const view=await first.request('inspect',{run:1,choices:{}});await first.close();
  const second=testConnection(store,server.fetcher);await second.initialize();
  const context=createContext({store,request:second.request,liveRequest:(route,payload)=>second.request(route,payload),
    session:{run:1},OutputAssembler,deliverCachedOutput,inspectionPending:null,inspectionCanceled:false,
    inspected:null,outputMode:'answers',answerNumber:null,answerPage:0,
    resetResultPages(){},async refreshSaved(){},message(){}});
  runInContext(productionSection('  async function inspectOnce()', '  function renderInspectionControls()'),context);
  await context.inspectOnce();
  assert.equal(context.inspectionPending,null);assert.equal(context.inspectionCanceled,true);
  assert.equal(store.archives.get(view.archive).answers.size,1);
  assert.equal(store.archives.get(view.archive).parts.size,10);
  assert.equal(await store.recovery('live:inspection:1:1'),null);
  await second.close();
});
await test('reload exposes Cancel after quota blocks accepted Start adoption',async()=>{
  const store=memorySink(),server=reloadServer({events:[]}),first=testConnection(store,server.fetcher);
  const save=store.saveRecovery.bind(store);let blocked=true;
  store.saveRecovery=async(key,value)=>{if(blocked && key==='live:source:1')throw Error('Persistent quota');return save(key,value);};
  await assert.rejects(first.request('start',{...untouched,record_history:false}),/Persistent quota/);
  await first.close();
  const second=testConnection(store,server.fetcher),session=new RunSession(second.request,()=>{},store);
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{replaceChildren(){}});return controls.get(name);};
  const context=createContext({$,store,connection:second,request:second.request,session,
    connected:false,restoring:true,editorWriting:null,editorDirty:false,
    model:untouched,dirty:false,clone:structuredClone,validateNotebook,inspectionSelection:session.selection,
    inspecting:false,launching:false,busy:false,inspectionPending:null,inspectionCanceled:false,
    el:()=>({}),message(){},renderWorkspace(){},renderInspectionControls(){},renderResults(){},async refreshSaved(){},safe:action=>action(),async inspect(){}});
  runInContext(productionSection('  function saveEditor()', '  function message(')
    +productionSection('  function renderRun()', '  function refreshSaved()')
    +productionSection('  async function finishInspection()', '  function renderInspectionControls()')
    +productionSection("  $('cancel').onclick", "  $('alternatives').onchange"),context);
  await assert.rejects(context.initialize(),/Persistent quota/);
  assert.equal(session.run,null);assert.equal(session.status,'idle');
  assert.equal(context.connected,true);assert.equal(context.restoring,false);
  assert.equal($('cancel').disabled,false);
  const before=server.calls.length;
  await assert.rejects($('cancel').onclick(),/Persistent quota/);
  assert.ok(server.calls.slice(before).some(call=>call.route==='cancel'));
  assert.equal((await store.recovery('controller')).pending.route,'start');
  assert.equal(server.effects.filter(route=>route==='start').length,1);
  context.restoring=true;context.renderRun();
  for(const name of ['run','step','resume','cancel','inspect','execution'])assert.equal($(name).disabled,true);
  blocked=false;await second.close();
});
await test('cancel recovery waits for ordinary recovery then retries with cancellation mode',async()=>{
  let rejectOrdinary;const modes=[];
  const api=async()=>({});
  api.recover=options=>{
    modes.push(options?.cancel===true);
    if(!options?.cancel)return new Promise((_resolve,reject)=>{rejectOrdinary=reject;});
    throw Error('Cancellation reached storage');
  };
  const session=new RunSession(api,()=>{},memorySink());
  const ordinary=session.settleControl();
  const cancellation=session.cancel();
  rejectOrdinary(Error('Ordinary quota'));
  await assert.rejects(ordinary,/Ordinary quota/);
  await assert.rejects(cancellation,/Cancellation reached storage/);
  assert.deepEqual(modes,[false,true]);
});
await test('restore loads owned runs before reporting a definitively rejected pending Step',async()=>{
  const store=memorySink(),server=reloadServer({events:[]});
  let lost=true;
  const fetcher=async(url,options)=>{
    if(url==='/api/step') {
      if(lost){lost=false;throw Error('Lost rejected Step');}
      return {ok:false,status:409,text:async()=>JSON.stringify({error:'Source was canceled'})};
    }
    return server.fetcher(url,options);
  };
  const first=testConnection(store,fetcher),initial=new RunSession(first.request,()=>{},store);
  await initial.start(untouched,false,false);
  await assert.rejects(initial.step(),/Lost rejected Step/);await first.close();
  const second=testConnection(store,fetcher),restored=new RunSession(second.request,()=>{},store);
  await assert.rejects(restored.restore(1),/Source was canceled/);
  assert.equal((await store.recovery('controller')).pending,null);
  assert.equal(restored.run,1);assert.equal(restored.runs.has(1),true);
  assert.equal(restored.archive,initial.archive);assert.deepEqual(restored.submission,untouched);
  assert.equal(restored.status,'paused');assert.equal(restored.running,false);
  await restored.cancel();assert.equal(restored.status,'canceled');await second.close();
});
await test('mount restores selection context before surfacing a recovered command rejection',async()=>{
  const store=memorySink(),boot='a'.repeat(32),selection=new InspectionSelection();
  const savedSelection={lease:{run:7,snapshot:'8'},assignments:[['2','second']]};
  await store.saveRecovery('editor',{model:untouched,program:'p',query:'q',dirty:true,history:true,boot,run:7,selection:savedSelection});
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{});return controls.get(name);};
  let metadata=false,inspection=false;
  selection.restore=async(_store,_api,run,saved)=>{assert.equal(run,7);assert.deepEqual(saved,savedSelection);metadata=true;};
  const context=createContext({$,store,connected:false,restoring:true,editorWriting:null,editorDirty:false,
    model:untouched,dirty:false,clone:structuredClone,validateNotebook,request:()=>{},
    connection:{state:{boot},async initialize(){}},
    session:{run:null,async restore(){this.run=7;throw Error('Step rejected');}},inspectionSelection:selection,
    async restoreInspection(){inspection=true;},renderWorkspace(){},renderRun(){},renderInspectionControls(){},async refreshSaved(){}});
  runInContext(productionSection('  function saveEditor()', '  function message('),context);
  await assert.rejects(context.initialize(),/Step rejected/);
  assert.equal(metadata,true);assert.equal(inspection,true);assert.equal(context.connected,true);
});
await test('reload restores the inspected archive and pending navigation through bounded scene loading',async()=>{
  const h=renderingHarness(),c=h.context,store=memorySink(),messages=[];
  c.store=store;
  c.session={run:1,archive:'source',status:'paused',applications:1,stream:null,async restore(){}};
  Object.assign(c,{connection:{state:{boot:'a'.repeat(32)},async initialize(){}},request:()=>{},
    inspectionSelection:new InspectionSelection(),clone:structuredClone,validateNotebook,
    connected:false,restoring:true,model:untouched,dirty:false,editorWriting:null,editorDirty:false,
    catalog:{records:[],next:null,prev:null},catalogCursor:null,catalogDirection:'next',catalogDirty:true,catalogStamp:'',
    refreshing:null,refreshAgain:false,inspectionPending:null,renderWorkspace(){},renderRun(){},message:text=>messages.push(text)});
  const display={mode:'inspect',inspectionArchive:'inspection',savedArchive:'',sourceArchive:'source',
    answerNumber:1,answerPage:0,bindingPage:1,pendingNumber:1,};
  await store.saveRecovery('display',display);
  await store.saveRecovery('editor',{model:untouched,program:'p(X) <=> (q(X); r(X)).',query:'p(A)',dirty:false,history:false,run:1,boot:'a'.repeat(32)});
  store.list=async()=>({records:[],next:null,prev:null});
  const pageReads=[];
  store.page=async(archive,page)=>{pageReads.push({archive,page});return {id:archive,label:'Inspection',created:1,page:0,pages:1,total:1,answers:[{number:1,completion:'1',alternative:'0'}]};};
  store.scene=(...args)=>new Promise(resolve=>h.reads.push({collection:args[0],number:args[1],options:args[2],resolve}));
  runInContext(productionSection('  function saveEditor()', '  function message(')
    +productionSection('  function refreshSaved()', '  function renderResults()'),c);
  await c.initialize();
  assert.equal(c.outputMode,'inspect');assert.equal(c.inspected.archive,'inspection');
  assert.equal(c.answerNumber,1);
  assert.deepEqual(pageReads,[{archive:'inspection',page:0}]);
  assert.equal(h.reads.length,1);assert.equal(h.reads[0].collection,'inspection');
  assert.equal(h.reads[0].options.pendingNumber,1);
  h.reads[0].resolve(h.scene());await Promise.all(h.tasks);
  assert.equal(h.errors.length,0);assert.equal(h.$('pending-bodies').hidden,false);
  assert.match(messages.at(-1),/restored paused at 1 applications.*Showing saved inspection/);
  const persisted=await store.recovery('display');
  assert.equal(persisted.inspectionArchive,'inspection');assert.equal(persisted.pendingNumber,1);
  assert.equal('facts' in persisted,false);assert.equal('events' in persisted,false);
});
await test('inspection display ownership persists before release and retries without reprojection',async()=>{
  const store=memorySink(),batch={sequence:101,events:answer(1001),done:true},server=reloadServer(batch),connection=testConnection(store,server.fetcher);
  await connection.request('start',{...untouched,record_history:false});
  const view=await connection.request('inspect',{run:1,choices:{}});
  const c=createContext({store,request:connection.request,liveRequest:(route,payload)=>connection.request(route,payload),
    session:{run:1,archive:'source'},OutputAssembler,deliverCachedOutput,inspectionPending:null,inspectionCanceled:false,
    resetResultPages(){},async refreshSaved(){},message(){}});
  runInContext(productionSection('  function displayState()', '  async function initialize()')
    +productionSection('  async function inspectOnce()', '  function renderInspectionControls()'),c);
  const save=store.saveRecovery.bind(store);let blocked=true;
  store.saveRecovery=async(key,value)=>{if(key==='display' && blocked)throw Error('Display quota');return save(key,value);};
  await assert.rejects(c.inspectOnce(),/Display quota/);
  assert.equal(server.views.size,1);assert.equal(c.inspectionPending.cleanup,'release');
  assert.equal(store.archives.get(view.archive).answers.size,1);
  const advances=server.calls.filter(call=>call.route==='inspect_advance').length;
  blocked=false;await c.inspectOnce();
  assert.equal(server.views.size,0);assert.equal((await store.recovery('display')).inspectionArchive,view.archive);
  assert.equal(server.calls.filter(call=>call.route==='inspect_advance').length,advances);
  await connection.close();
});
await test('terminal run status replaces welcome text once without overwriting later UI messages',async()=>{
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{replaceChildren(){}});return controls.get(name);};
  const messages=[],session={run:1,status:'done',applications:2,runs:new Map(),error:null};
  const c=createContext({$,session,el:()=>({}),message:text=>messages.push(text),inspectionSelection:new InspectionSelection(),
    inspecting:false,launching:false,busy:false,inspectionPending:null,renderResults(){},async refreshSaved(){},safe:action=>action()});
  runInContext(productionSection('  function renderRun()', '  function refreshSaved()'),c);
  c.renderRun();assert.match(messages.at(-1),/Run 1 completed/);
  messages.push('Inspection saved.');c.renderRun();assert.equal(messages.at(-1),'Inspection saved.');
  session.status='canceled';c.renderRun();assert.match(messages.at(-1),/Run 1 canceled/);
});
await test('source terminal delivery refreshes the catalog in inspection mode without per-tick lists',async()=>{
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{replaceChildren(...children){this.children=children;}});return controls.get(name);};
  const session={archive:'source',status:'running',ack:1},lists=[];
  const c=createContext({$,session,inspected:{archive:'inspection'},outputMode:'inspect',
    catalog:{records:[],next:null,prev:null},catalogCursor:null,catalogDirection:'next',catalogDirty:true,catalogStamp:'',
    refreshing:null,refreshAgain:false,savedView:null,inspectionPending:null,
    el:(tag,text,attrs)=>({tag,text,attrs}),renderResults(){},resetResultPages(){},
    store:{list(options){assert.equal(options.size,32);return new Promise(resolve=>lists.push(resolve));},
      async page(id){assert.equal(id,'inspection');return {id,page:0,pages:1,total:1,answers:[]};}}});
  runInContext(productionSection('  function refreshSaved()', '  function renderResults()'),c);
  const initial=c.refreshSaved();
  const catalog=total=>({records:[{id:'source',label:'Run 1',total,created:1}],next:null,prev:null});
  await Promise.resolve(); lists[0](catalog(0));await initial;
  for(let i=0;i<5;i++)await c.refreshSaved();
  assert.equal(lists.length,1);
  session.status='done';session.ack=2;
  const completed=c.refreshSaved(); await Promise.resolve();
  assert.equal(lists.length,2,'terminal completion invalidates the source catalog entry');
  lists[1](catalog(2));await completed;
  assert.match($('saved-run').children.find(item=>item.attrs.value==='source').text,/Run 1 · 2 answers/);
  for(let i=0;i<5;i++)await c.refreshSaved();
  assert.equal(lists.length,2,'unchanged terminal state reuses the same bounded page');
  assert.equal(c.outputMode,'inspect');assert.equal(c.savedView.id,'inspection');
  session.status='canceled';const canceling=c.refreshSaved(); await Promise.resolve();
  session.ack=3;c.refreshSaved();lists[2](catalog(2));
  await new Promise(setImmediate);
  assert.equal(lists.length,4,'a committed ack change during a list read is not lost');
  lists[3](catalog(3));await canceling;
  assert.match($('saved-run').children.find(item=>item.attrs.value==='source').text,/Run 1 · 3 answers/);
});

for (const [inspection, mode, live] of [['present','inspect',false], ['missing','inspect',false], ['empty','inspect',false], ['read-error','inspect',false], ['present','answers',true]]) await test(`${live ? 'completed run' : 'idle'} reload reconciles ${inspection} ${mode} archive and selected answer`, async () => {
  const h = renderingHarness(), c = h.context, store = memorySink(), messages = [], pageReads = [];
  const oldBoot = 'a'.repeat(32), boot = live ? oldBoot : 'b'.repeat(32);
  c.store = store;
  c.session = {run:null, archive:null, status:'idle', applications:0, runs:new Map(), stream:null,
    async restore(preferred) {
      assert.equal(preferred, live ? 1 : null);
      if (live) { this.run = 1; this.archive = 'old-source'; this.status = 'done'; this.applications = 2; this.runs.set(1,{run:1}); }
    }};
  Object.assign(c, {connection:{state:{boot},async initialize(){}},request:()=>{},
    inspectionSelection:new InspectionSelection(),clone:structuredClone,validateNotebook,
    connected:false,restoring:true,model:untouched,dirty:false,editorWriting:null,editorDirty:false,
    catalog:{records:[],next:null,prev:null},catalogCursor:null,catalogDirection:'next',catalogDirty:true,catalogStamp:'',
    refreshing:null,refreshAgain:false,inspectionPending:null,inspecting:false,launching:false,busy:false,
    renderWorkspace(){},message:text=>messages.push(text)});
  const display = {mode,inspectionArchive:'old-inspection',savedArchive:'',sourceArchive:'old-source',
    answerNumber:1,answerPage:0,bindingPage:0,pendingNumber:0,};
  await store.saveRecovery('display',display);
  await store.saveRecovery('editor',{model:untouched,program:'p(X) <=> (q(X); r(X)).',query:'p(A)',dirty:false,history:false,run:1,boot:oldBoot});
  // A different archive can have the same label. It is not the saved locator.
  store.list = async () => ({records:[{id:'other-inspection',label:'Inspection of run 1',created:1,total:1}],next:null,prev:null});
  store.page = async (archive,page) => {
    pageReads.push({archive,page});
    if (inspection === 'read-error') throw Error('Archive read failed');
    if (archive !== (mode === 'inspect' ? 'old-inspection' : 'old-source') || inspection === 'missing') return null;
    return {id:archive,label:'Inspection of run 1',created:1,page:0,pages:1,total:inspection === 'empty' ? 0 : 2,
      answers:inspection === 'empty' ? [] : [{number:1,completion:'1',alternative:'0'}, {number:2,completion:'1',alternative:'1'}]};
  };
  store.scene = (...args) => new Promise(resolve => h.reads.push({collection:args[0],number:args[1],options:args[2],resolve}));
  runInContext(productionSection('  function saveEditor()', '  function message(')
    + productionSection('  function renderRun()', '  function renderResults()'),c);
  runInContext(productionSection('  async function safe(', '  function remember('), c);
  const productionSafe = c.safe;
  c.safe = action => { const pending = productionSafe(action); h.tasks.push(pending); return pending; };
  // Full mount starts a catalog refresh before initialize reads durable display state.
  c.renderRun(); c.renderInspectionControls();
  if (inspection === 'read-error') {
    await assert.rejects(c.initialize(), /Archive read failed/);
    await Promise.all(h.tasks);
    assert.equal((await store.recovery('display')).inspectionArchive, 'old-inspection');
    assert.equal(c.inspected.archive, 'old-inspection');
    assert.ok(!messages.some(text=>text.includes('Showing saved inspection')));
    return;
  }
  await c.initialize();
  assert.equal(h.$('run-status').textContent, live ? 'done' : 'idle');
  assert.equal(h.$('applications').textContent, `${live ? 2 : 0} applications`);
  assert.equal(h.$('program').value, 'p(X) <=> (q(X); r(X)).');
  if (inspection === 'present') {
    assert.equal(c.savedView.id, mode === 'inspect' ? 'old-inspection' : 'old-source');
    assert.equal(h.$('answer-count').textContent, '2 saved · 2 on this page');
    assert.equal(h.$('result-empty').hidden, true);
    assert.equal(h.reads.length, 1);
    assert.equal(c.answerNumber, 1, 'loading cannot replace the saved selection with the last answer');
    assert.equal(h.$('alternatives').value, 1);
    assert.equal(h.reads[0].number, 1);
    h.reads[0].resolve(h.scene()); await Promise.all(h.tasks);
    assert.equal(h.errors.length, 0);
    assert.match(messages.at(-1), live ? /Run 1 restored done at 2 applications/ : /Notebook restored\. Showing saved inspection\./);
    assert.equal((await store.recovery('display')).answerNumber, 1);
  } else if (inspection === 'empty') {
    assert.equal(h.reads.length, 0);
    assert.equal(h.$('answer-count').textContent, '0 saved · 0 on this page');
    assert.match(messages.at(-1), /Saved inspection has no completed answers/);
  } else {
    assert.equal(c.inspected, null);
    assert.equal(c.outputMode, 'answers');
    assert.equal(c.savedSelection, '');
    assert.equal(c.savedView, null);
    assert.equal(h.reads.length, 0);
    assert.match(messages.at(-1), /Saved inspection is unavailable/);
    assert.equal((await store.recovery('display')).inspectionArchive, null);
    assert.equal(h.$('saved-run').children.length, 2, 'other archives remain available');
  }
  assert.ok(pageReads.length <= 3, 'only bounded selected-archive lookups, no catalog search');
  assert.ok(!pageReads.some(read=>read.archive === 'other-inspection'));
});

for (const gap of [0,1,2,3,4,5,6,7,8]) await test(`restore refresh at drain completion loads selected archive (gap ${gap})`, async () => {
  const controls = new Map(), $ = name => { if (!controls.has(name)) controls.set(name,{replaceChildren(){}}); return controls.get(name); };
  const pages = [], paints = []; let injected = false, restored;
  const c = createContext({$,el:()=>({}),session:{run:null,archive:null,status:'idle'},
    catalog:{records:[],next:null,prev:null},catalogCursor:null,catalogDirection:'next',catalogDirty:true,catalogStamp:'',
    refreshing:null,refreshAgain:false,savedView:null,inspectionPending:null,
    resetResultPages(){},renderResults(){paints.push(c.savedView?.id ?? null);},
    store:{async list(){return {records:[{id:'available',label:'Inspection of run 2',created:1,total:1}],next:null,prev:null};},
      async page(archive){pages.push(archive);return {id:archive,total:1,page:0,pages:1,answers:[{number:1}]};}},
    async saveDisplay(){
      if (injected) return;
      injected = true;
      // initialize can resume after the drain loop ends, before an external
      // Promise.finally releases its cached promise.
      let remaining = gap;
      const admit = () => { if (remaining-- > 0) { queueMicrotask(admit); return; }
        c.outputMode = 'inspect'; c.inspected = {archive:'available'}; c.answerNumber = 1;
        restored = c.refreshSaved();
      };
      queueMicrotask(admit);
    }});
  runInContext(productionSection('  function refreshSaved()', '  function renderResults()'),c);
  await c.refreshSaved();
  while (!restored) await Promise.resolve();
  await restored;
  assert.deepEqual(pages,['available']);
  assert.equal(c.savedView.id,'available');
  assert.equal(paints.at(-1),'available');
  assert.equal(c.refreshAgain,false);
});

for (const kind of ['editor','display']) for (const gap of [0,1,2,3,4]) await test(`${kind} save retains a change arriving at drain completion (${gap})`, async () => {
  const controls = {program:{value:'first'},query:{value:'p(A)'},history:{checked:false}};
  const writes = []; let injected = false, next;
  const c = createContext({$:name=>controls[name],connection:{state:{boot:'a'.repeat(32)}},
    model:untouched,dirty:false,editorDirty:false,editorWriting:null,answerNumber:1,
    inspectionSelection:{checkpoint:()=>null},store:{async saveRecovery(key,value) {
      writes.push(structuredClone(value));
      if (injected) return;
      injected = true;
      let remaining = gap;
      const admit = () => {
        if (remaining-- > 0) { queueMicrotask(admit); return; }
        controls.program.value = 'second'; c.answerNumber = 2;
        next = kind === 'editor' ? c.saveEditor() : c.saveDisplay();
      };
      queueMicrotask(admit);
    }}});
  runInContext(productionSection('  function saveEditor()', '  function restoreDisplay('),c);
  await (kind === 'editor' ? c.saveEditor() : c.saveDisplay());
  while (!next) await Promise.resolve();
  await next;
  assert.equal(kind === 'editor' ? writes.at(-1).program : writes.at(-1).answerNumber,kind === 'editor' ? 'second' : 2);
  assert.equal(c[kind + 'Writing'],null);
  assert.equal(c[kind + 'Dirty'],false);
  c.store.saveRecovery = async()=>{throw Error('Storage unavailable');};
  controls.program.value = 'third'; c.answerNumber = 3;
  await assert.rejects(kind === 'editor' ? c.saveEditor() : c.saveDisplay(),/Storage unavailable/);
  c.store.saveRecovery = async(_key,value)=>writes.push(structuredClone(value));
  await (kind === 'editor' ? c.saveEditor() : c.saveDisplay());
  assert.equal(kind === 'editor' ? writes.at(-1).program : writes.at(-1).answerNumber,kind === 'editor' ? 'third' : 3);
});

await test('Step follows the current graph while preserving the selected alternative', async () => {
  const selection = new InspectionSelection(), calls = [], controls = new Map();
  selection.update([{id:'4',label:'Choice 4'}],[{id:'9',label:'Earlier state'}]);
  selection.snapshot = '9'; selection.selectedSnapshot = selection.snapshots[0]; selection.choose('4','second');
  const $ = name => {if (!controls.has(name)) controls.set(name,{}); return controls.get(name);};
  const c = createContext({$,launching:false,inspectionSelection:selection,
    session:{run:1,submission:{program:{rules:[{name:'rewrite'}]}},async finishClose(){},
      async step(choices){assert.equal(choices['4'],false);calls.push('step');return {step:{event:1,rule:0}};}},
    request:async(route,payload)=>{
      calls.push(route);
      if(route==='snapshot')return {snapshot:'10'};
      if(route==='views'){assert.equal(payload.snapshot,'10');return {choices:[{id:'4',label:'Choice 4'}],snapshots:[{id:'9',label:'Earlier state'}]};}
      throw Error(route);
    },renderRun(){},renderInspectionControls(){},message(){},safe:action=>action(),
    async inspect(){const payload=selection.payload(1);assert.equal(payload.snapshot,undefined);assert.equal(payload.choices['4'],false);calls.push('inspect');}});
  runInContext(productionSection("  async function runStep(", "  $('execution').onchange"),c);
  await $('step').onclick();
  assert.equal(calls.at(-1),'inspect');
  assert.equal(selection.snapshot,'');
});

await test('Pause stops unfinished Step polling and Resume completes the same application', async () => {
  const calls=[], waiting=[];
  const session=testSession(async(route,payload)=>{
    calls.push([route,payload]);
    if(route==='start')return {run:1,...tables};
    if(route==='step'||route==='resume')return {};
    if(route==='output')return new Promise(resolve=>waiting.push(resolve));
    throw Error(route);
  });
  await session.start(untouched,false,false);
  const pending=session.step({'41':true});
  while(!waiting.length)await new Promise(setImmediate);
  session.stopReading();
  waiting[0]({events:[],applications:0,delivery_done:false,exhausted:false,step:{done:false,event:null}});
  const settled=await Promise.race([pending.then(value=>({value})),new Promise(resolve=>setTimeout(()=>resolve(null),40))]);
  // Settle the old implementation's extra request before asserting the RED.
  if(!settled){waiting[1]({events:[],applications:1,delivery_done:false,exhausted:false,step:{done:true,event:1,rule:0}});await pending;}
  assert.ok(settled,'Pause must settle the driver without another source request');
  assert.equal(settled.value,undefined);assert.equal(waiting.length,1);assert.equal(session.stepPending,true);
  const resumed=session.resume();
  while(waiting.length<2)await new Promise(setImmediate);
  waiting[1]({events:[],applications:1,delivery_done:false,exhausted:false,step:{done:true,event:1,rule:0,shared:false}});
  const result=await resumed;
  assert.equal(result.step.event,1);assert.equal(session.running,false);assert.equal(session.status,'paused');
  assert.equal(calls.filter(([route])=>route==='step').length,1);
  assert.equal(calls.filter(([route])=>route==='resume').length,1);
  await session.resume();session.stopReading();assert.equal(calls.filter(([route])=>route==='resume').length,2);
});
await test('Resume during Step polling cannot release its logical boundary', async () => {
  const calls=[],waiting=[];
  const session=testSession(async(route)=>{calls.push(route);if(route==='start')return {run:1,...tables};
    if(route==='step'||route==='resume')return {};if(route==='output')return new Promise(r=>waiting.push(r));throw Error(route);});
  await session.start(untouched,false,false);const stepping=session.step({});
  while(!waiting.length)await new Promise(setImmediate);
  const resuming=session.resume();
  waiting[0]({events:[],applications:1,delivery_done:false,exhausted:false,step:{done:true,event:1,rule:0}});
  await stepping;await resuming;session.stopReading();
  assert.equal(calls.includes('resume'),false,'Resume joins an active Step rather than removing its engine gate');
});
await test('Step admission rejects an in-flight metadata selection', async () => {
  const session=testSession(async route=>{if(route==='start')return {run:1,...tables};throw Error('Unexpected '+route);});
  await session.start(untouched,false,false);session.selection.loading=true;
  await assert.rejects(session.step({'41':true}),/metadata/i);
});

await test('lost Step admission reloads its exact choices and Resume never admits a second Step', async () => {
  await controlSession('step-owner-reload', async ({session,api,model,lose,effects,calls}) => {
    await session.start(model,false,false);
    lose('step');await assert.rejects(session.step({'41':false}),/Lost step/);
    const saved=await session.store.recovery('live:run:1');assert.deepEqual(saved.stepChoices,{'41':false});
    const restored=new RunSession(api,()=>{},session.store);await restored.restore(1);
    assert.equal(restored.stepPending,true);assert.deepEqual(restored.stepOperation.choices,{'41':false});
    const result=await restored.resume();
    assert.deepEqual(result.stepChoices,{'41':false});assert.equal(restored.status,'paused');assert.equal(restored.running,false);
    assert.equal(effects.filter(route=>route==='step').length,1);assert.equal(effects.includes('resume'),true);
    const commands=calls.filter(call=>call.route==='step');assert.equal(commands.length,2);assert.equal(commands[0].body,commands[1].body);
    assert.equal((await session.store.recovery('live:run:1')).stepChoices,null);
    restored.stopReading();
  });
});
await test('cancel retires a polling Step without further advances or automatic completion', async () => {
  const calls=[];let release;
  const session=testSession(async route=>{calls.push(route);if(route==='start')return {run:1,...tables};
    if(route==='step'||route==='cancel')return {};if(route==='output')return new Promise(r=>release=r);throw Error(route);});
  await session.start(untouched,false,false);const stepping=session.step({'41':true});
  while(!release)await new Promise(setImmediate);
  const canceled=session.cancel();
  release({events:[],applications:0,delivery_done:false,exhausted:false,step:{done:false,event:null}});
  assert.equal(await stepping,undefined);await canceled;
  assert.equal(session.status,'canceled');assert.equal(session.stepOperation,null);assert.equal(session.stepPending,false);
  assert.equal(calls.filter(route=>route==='output').length,1);
  assert.equal((await session.store.recovery('live:run:1')).stepChoices,null);
});
await test('unfinished Step owns selection across Pause and UI Resume inspection uses admitted choices', async () => {
  const session=testSession(async route=>{if(route==='start')return {run:1,...tables};throw Error(route);});
  await session.start(untouched,false,false);
  session.selection.update([{id:'41',label:'Choice'}],[{id:'7',label:'State'}]);
  session.stepOperation={run:1,choices:{'41':true}};session.stepPending=true;
  assert.throws(()=>session.selection.choose('41','second'),/step/);
  assert.throws(()=>session.selection.selectSnapshot(session.api,1,'7'),/step/);
  assert.throws(()=>session.selection.page(session.api,1),/step/);
  const controls=new Map(),$=name=>{if(!controls.has(name))controls.set(name,{replaceChildren(){},append(){}});return controls.get(name);};
  const c=createContext({$,session,inspectionSelection:session.selection,launching:false,inspecting:false,busy:false,
    inspectionPending:null,el:()=>({append(){}}),safe:action=>action(),renderResults(){},refreshSaved(){},message(){}});
  runInContext(productionSection('  function renderRun()', '  function refreshSaved()')
    +productionSection('  function renderInspectionControls()', "  for (const name of ['program', 'query'])"),c);
  c.renderRun();c.renderInspectionControls();
  assert.equal($('resume').textContent,'Resume step');assert.equal($('resume').disabled,false);
  assert.equal($('snapshot').disabled,true);assert.equal($('inspect').disabled,true);
  runInContext(productionSection('  async function inspect()', '  async function inspectOnce()'),c);
  await assert.rejects(c.inspect(),/step/);
  await assert.rejects(c.metadataPage('choice','next'),/step/);
  await assert.rejects($('snapshot').onchange(),/step/);
  session.selection.checkEditable=()=>{};session.stepOperation=null;
  let inspections=0;session.resume=async()=>({step:{done:true,event:1,rule:0},stepChoices:{'41':true}});
  session.submission={program:{rules:[{name:'rewrite'}]}};
  c.inspect=async()=>{inspections++;assert.deepEqual(session.selection.payload(1).choices,{'41':true});};
  runInContext(productionSection('  async function runStep(', "  $('execution').onchange"),c);
  await c.runStep(true);assert.equal(inspections,1);
  session.step=async()=>undefined;
  await c.runStep();assert.equal(inspections,1,'Pause does not trigger inspection or claim step completion');
});

await test('Pause during Step checkpoint sends no control until Resume', async () => {
  const calls=[];
  const session=testSession(async route=>{calls.push(route);if(route==='start')return {run:1,...tables};
    if(route==='step')return {};if(route==='output')return {events:[],applications:1,delivery_done:false,exhausted:false,step:{done:true,event:1,rule:0}};throw Error(route);});
  await session.start(untouched,false,false);
  const save=session.store.saveRecovery.bind(session.store);let release;
  session.store.saveRecovery=async(key,value)=>{
    if(value?.stepChoices){await new Promise(r=>release=r);}await save(key,value);
  };
  const pending=session.step({'41':false});while(!release)await new Promise(setImmediate);
  session.stopReading();release();assert.equal(await pending,undefined);
  assert.equal(calls.includes('step'),false);assert.equal(session.stepPending,false);
  assert.deepEqual((await session.store.recovery('live:run:1')).stepChoices,{'41':false});
  session.store.saveRecovery=save;
  const response=await session.resume();assert.equal(response.step.event,1);
  assert.equal(calls.filter(route=>route==='step').length,1);assert.equal(calls.includes('resume'),false);
});
await test('switch waits for the Step driver before loading another run', async () => {
  const calls=[];let next=0,release;
  const session=testSession(async(route,payload)=>{calls.push([route,payload.run]);
    if(route==='start')return {run:++next,...tables};if(route==='cancel'||route==='step')return {};
    if(route==='output')return new Promise(r=>release=r);throw Error(route);});
  await session.start(untouched,false,false);await session.start(untouched,false,false);
  const pending=session.step({'41':true});while(!release)await new Promise(setImmediate);
  const switching=session.switchRun(1);await new Promise(setImmediate);
  assert.equal(session.run,2,'foreground run remains owned until its driver settles');
  release({events:[],applications:0,delivery_done:false,exhausted:false,step:{done:false,event:null}});
  assert.equal(await pending,undefined);await switching;
  assert.equal(session.run,1);assert.equal(session.stepDriving,null);
  assert.equal(calls.filter(([route])=>route==='output').length,1);
  assert.deepEqual((await session.store.recovery('live:run:2')).stepChoices,{'41':true});
});


await test('stopping output and switching preserve the running execution status', async () => {
  const session=testSession(async route=>{
    if(route==='start')return {run:1,...tables};
    if(route==='resume')return {};
    if(route==='output')return {sequence:1,events:[],running:true,applications:1,delivery_done:false};
    throw Error(route);
  });
  await session.start(untouched,false,false);
  await session.resume();
  await session.readOutput();
  session.stopReading();
  assert.equal((await session.store.recovery('live:run:1')).phase,'running');
  assert.equal(session.status,'running');
  assert.equal(session.running,false);
  session.runs.set(2,{run:2,archive:'other',status:'running'});
  session.loadRun=async run=>{session.run=run;session.status='running';};
  await session.switchRun(2);
  assert.equal(session.runs.get(1).status,'running');
});
await test('an older output read cannot overwrite an acknowledged runtime Pause', async () => {
  let output;
  const session=testSession(async route=>{
    if(route==='start')return {run:1,...tables};
    if(route==='pause')return {};
    if(route==='output')return new Promise(resolve=>output=resolve);
    throw Error(route);
  });
  await session.start(untouched,false,false);session.status='running';
  const reading=session.readOutput();
  await session.pause();
  output({sequence:1,events:[],running:true,applications:1,delivery_done:false});
  await reading;
  assert.equal(session.status,'paused');
  assert.equal((await session.store.recovery('live:run:1')).phase,'paused');
});
await test('restore preserves nonselected executions last-known running status', async () => {
  const session=testSession(async()=>{throw Error('No transport request expected');});
  await session.store.saveRecovery('live:run:1',{run:1,archive:'a',phase:'running'});
  await session.store.saveRecovery('live:run:2',{run:2,archive:'b',phase:'stepping'});
  session.loadRun=async()=>{};
  await session.restore(1);
  assert.equal(session.runs.get(1).status,'running');
  assert.equal(session.runs.get(2).status,'stepping');
});
await test('Pause changes status only on acknowledgement and preserves terminal states', async () => {
  let acknowledge;
  const session=testSession(async route=>{
    if(route==='start')return {run:1,...tables};
    if(route==='pause')return new Promise(resolve=>acknowledge=resolve);
    throw Error(route);
  });
  await session.start(untouched,false,false);
  session.status='running';
  const pending=session.pause();
  assert.equal(session.status,'running');
  acknowledge({});await pending;
  assert.equal(session.status,'paused');
  for(const terminal of ['done','canceled','error']) {
    session.status='running';const pending=session.pause();
    session.status=terminal;acknowledge({});await pending;
    assert.equal(session.status,terminal);
  }
});

await test('Cancel drains server-retained answers across batches before retiring a partial answer', async () => {
  const events=[...answer(80),...answer(81),...answer(82).slice(0,5)];
  let cursor=0,sequence=0,reads=0;
  const session=testSession(async(route,payload)=>{
    if(route==='start')return {run:1,...tables};
    if(route==='cancel')return {output_pending:cursor<events.length};
    assert.equal(route,'output');
    assert.equal(payload.ack,sequence || null);
    reads++;const batch=events.slice(cursor,cursor+5);cursor+=batch.length;
    return {events:batch,sequence:++sequence,output_drained:cursor===events.length,
      applications:2,canceled:true,delivery_done:false,exhausted:true};
  });
  await session.start(untouched,false,false);
  await session.cancel();
  assert.ok(reads>2);
  assert.equal(session.store.archives.get(session.archive).answers.size,2);
  assert.equal(session.stream.current,null);
  assert.equal(session.status,'canceled');
  assert.equal((await session.store.recovery('live:run:1')).phase,'canceled');
});

for (const fails of [false, true]) await test(`inspection ${fails ? 'failure' : 'completion'} unlocks alternative controls`, async () => {
  const controls = new Map();
  const element = () => ({children:[], append(...items){this.children.push(...items);}, replaceChildren(...items){this.children=items;}});
  const $ = name => {if (!controls.has(name)) controls.set(name,element()); return controls.get(name);};
  const selection = new InspectionSelection();
  selection.update([{id:'1',label:'Choice'}],[]);
  selection.navigation.next_choice='2';
  let finish;
  const c = createContext({$,el:element,session:{run:1,recordHistory:true},inspectionSelection:selection,
    inspecting:false,launching:false,renderRun(){},
    inspectOnce(){c.renderInspectionControls();return new Promise((resolve,reject)=>{finish=()=>fails?reject(Error('inspection failed')):resolve();});}});
  runInContext(productionSection('  function renderInspectionControls()', "  for (const name of ['program', 'query'])")
    +productionSection('  async function inspect()', '  async function inspectOnce()'),c);
  c.renderInspectionControls();
  const choice = () => $('choices').children[0].children[0];
  assert.equal(choice().disabled,false);
  const pending = c.inspect();
  assert.equal(choice().disabled,true);
  assert.equal($('choice-next').disabled,true);
  finish();
  if (fails) await assert.rejects(pending,/inspection failed/); else await pending;
  assert.equal(choice().disabled,false);
  assert.equal($('choice-next').disabled,false);
});

await test('normal Start begins reading an admitted execution without another control command', async () => {
  const calls=[];
  const session=testSession(async(route,payload)=>{
    calls.push(route);
    assert.equal(route,'start');
    assert.equal(payload.paused,false);
    return {run:1,...tables};
  });
  let scheduled=0;
  session.schedule=()=>{scheduled++;};
  await session.start(untouched);
  assert.deepEqual(calls,['start']);
  assert.equal(session.status,'running');
  assert.equal(session.running,true);
  assert.equal(scheduled,1);
});

});

await runTest('explicit Pause is a replayable server command, disconnected reading sends no Pause', async () => {
  await controlSession('runtime-pause', async ({session,model,lose,effects}) => {
    await session.start(model,false,false);
    lose('pause');
    await assert.rejects(session.pause(), /Lost pause/);
    assert.equal(session.status,'error');
    await session.settleControl();
    assert.equal(effects.filter(route=>route==='pause').length,1);
    session.fail(new Error('reader disconnected'));
    assert.equal(effects.filter(route=>route==='pause').length,1);
  });
});
