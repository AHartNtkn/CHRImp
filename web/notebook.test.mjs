import assert from 'node:assert/strict';
import {test as runTest} from 'node:test';
import { applyEdit, at, sceneEntries, validateNotebook } from './graph.mjs';
import { RunSession, InspectionSelection, deliverCachedOutput, request } from './notebook.mjs';
import { OutputAssembler } from './answers.mjs';

await runTest('transport pins execution requests to one server incarnation', async () => {
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
  const api = (await import('./notebook.mjs?control-replay')).request;
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
      receipt = {command:body.command, body:options.body, response:{run:1, inspection:'2'}};
    }
    if (loseReply) { loseReply = false; throw new Error('command response lost'); }
    return reply(receipt.response);
  };
  try {
    await assert.rejects(api('start', {}), /attach response lost/);
    for (const route of ['start', 'inspect', 'step', 'resume', 'snapshot']) {
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
    assert.deepEqual(applied, ['start', 'inspect', 'step', 'resume', 'snapshot', 'step', 'resume']);
    assert.equal(receipt.command, 7);
  } finally { globalThis.fetch = originalFetch; }
});

// Exercise the production receipt authority with transport failures after effects.
async function controlSession(name, body) {
  const api = (await import(`./notebook.mjs?lifecycle-${name}`)).request;
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
        else assert.equal(route, 'resume');
        receipt = {route, command:payload.command, body:options.body, response};
      }
    } else if (route === 'cancel') { runs.get(payload.run).canceled = true; }
    else if (route === 'close') runs.delete(payload.run);
    else if (route === 'inspect_cancel') { assert.equal(jobs.get(payload.inspection), payload.run); response = {done:true}; }
    else if (route === 'inspect_release') jobs.delete(payload.inspection);
    else if (route === 'snapshot_release') snapshots.delete(payload.snapshot);
    else if (route === 'views') response = {choices:[], snapshots:[...snapshots.keys()].map(id => ({id,label:id}))};
    else if (route === 'advance') response = {events:[], applications, step:{done:true}, delivery_done:false};
    else assert.ok(['attach','maintenance'].includes(route), route);
    if (losses.delete(route)) throw new Error(`Lost ${route} response`);
    return {ok:true, status:200, text:async () => JSON.stringify(response)};
  };
  const store = {
    async create(tables, label) { if (failCreate) { failCreate = false; throw new Error('Archive unavailable'); } archives.push({tables, label}); return archives.length; },
    async flush() {}, async discardPartial() {},
  };
  const session = new RunSession(api, () => {}, store);
  try { await body({api, session, model, calls, runs, jobs, snapshots, effects, archives, lose:route => losses.add(route), failArchive:() => { failCreate = true; }}); }
  finally { session.pause(); globalThis.fetch = originalFetch; }
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
    assert.equal(session.recoveredControl.route, 'start'); assert.equal(session.run, 1);
    await session.cancel();
    assert.equal(runs.get(1).canceled, true); assert.equal(session.recordHistory, true);
    assert.deepEqual(session.submission, model); assert.equal(archives.length, 1);
    assert.equal(session.recoveredControl, null);
    await session.start({program:{rules:[]}, query:{kind:'fail'}}, false, false);
    assert.equal(session.run, 2); assert.equal(session.runs.get(1).archive, 1);
    assert.equal(effects.filter(route => route === 'start').length, 2);
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
    assert.equal(session.runs.get(2).archive, 2); assert.equal(runs.size, 2);
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
const testSession = (api, notify = () => {}, store = memorySink()) => new RunSession(api, notify, store);

function memorySink() {
  let next = 0;
  const archives = new Map();
  return {
    archives, commits: [], beforeCommit: null, afterCommit: null,
    async create(tables, label) {
      const archive = String(++next);
      archives.set(archive, { tables: structuredClone(tables), label, answers: new Map(), parts: new Map() });
      return archive;
    },
    async flush(archive, assembler, discard = false) {
      const discardNumber = discard ? assembler.current?.number : undefined;
      if (!assembler.writes.length && discardNumber === undefined) return;
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
      this.commits.push(preserved);
      if (preserved.length) await this.afterCommit?.(preserved, archive);
      assembler.writes.splice(0, batch.length);
      assembler.answers.splice(0, batch.filter(record => record.store === 'answers').length);
      if (discardNumber !== undefined) assembler.discardPartial();
    },
    async discardPartial(archive, assembler) {
      await this.flush(archive, assembler, true);
    },
  };
}
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
edited = applyEdit(edited, { type: 'wrap', path: ['query', 'items', 0], kind: 'or' });
assert.equal(edited.query.items[0].kind, 'or');
assert.equal(edited.query.items[0].items.length, 1);
assert.throws(() => applyEdit(edited, { type: 'remove', path: ['query', 'items', 0, 'items', 0] }), /branch/);
edited = applyEdit(edited, { type: 'append', path: ['query', 'items', 0], node: { kind: 'fail' } });
edited = applyEdit(edited, { type: 'remove', path: ['query', 'items', 0, 'items', 0] });
assert.deepEqual(edited.query.items[0], { kind: 'or', items: [{ kind: 'fail' }] });
assert.equal(sceneEntries(edited, ['query']).length, 2);
assert.equal(at(edited, ['program', 'rules', 0, 'kept', 0]).relation, 'p');
edited = applyEdit(edited, { type: 'append', path: ['program', 'rules', 0, 'kept'], node: atom('s', 'Z') });
assert.deepEqual(edited.program.rules[0].kept[1], { relation: 's', args: ['Z'] });
assert.throws(() => applyEdit(edited, { type: 'append', path: ['program', 'rules', 0, 'kept'], node: { kind: 'true' } }));
assert.throws(() => applyEdit(edited, { type: 'rename-relation', path: ['program', 'rules', 0, 'kept', 0], relation: '<script>' }));
assert.throws(() => applyEdit(edited, { type: 'set-port', path: ['program', 'rules', 0, 'kept', 0], index: 0, variable: 'lower' }));
assert.throws(() => applyEdit(edited, { type: 'replace', path: ['__proto__'], node: {} }));
let soleHead = { program: { rules: [{ name: null, kept: [], removed: [{ relation: 'p', args: [] }], body: { kind: 'true' } }] }, query: { kind: 'true' } };
assert.throws(() => applyEdit(soleHead, { type: 'remove', path: ['program', 'rules', 0, 'removed', 0] }), /head/);
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
  if (route === 'advance') return new Promise(resolve => { resolveAdvance = resolve; });
  throw new Error('unexpected endpoint');
};
const session = testSession(api);
await session.start(original, false, false);
original.query.items[0].atom.relation = 'changed_after_start';
assert.equal(session.submission.query.items[0].atom.relation, 'p');
assert.deepEqual(session.stream.tables, tables);
assert.equal(calls[0][1].record_history, false);
const advancing = session.advance();
assert.equal(calls[1][1].budget, 2048);
session.pause();
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
let storageBlocked = false, advanceCalls = 0;
const store = memorySink();
const durable = store.archives;
store.beforeCommit = () => { if (storageBlocked) throw new Error('Storage full'); };
const persisted = testSession(async route => {
  if (route === 'start') return { run: 9, ...tables };
  if (route === 'cancel') return {};
  if (route === 'advance') { advanceCalls++; return { sequence: 7, events: [0, 1, 2].flatMap(answer), applications: 3, exhausted: true, delivery_done: true }; }
}, () => {}, store);
await persisted.start(untouched, false, false);
storageBlocked = true;
await assert.rejects(persisted.advance(), /Storage full/);
assert.equal(persisted.status, 'error');
assert.equal(persisted.running, false);
assert.equal(summaries(persisted.stream).length, 3);
assert.equal(summaries(persisted.stream)[0].completion, '0');
assert.equal(persisted.pendingDelivery.index, 3 * answer(0).length);
assert.equal(persisted.ack, null);
assert.equal(persisted.timer, null);
assert.equal(durable.get(persisted.archive).answers.size, 0);
const failedIndex = persisted.pendingDelivery.index;
await assert.rejects(persisted.advance(), /Storage full/);
assert.equal(persisted.pendingDelivery.index, failedIndex);
assert.equal(advanceCalls, 1);
assert.equal(persisted.stream.total, 3);
storageBlocked = false;
await persisted.advance();
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
  if (route === 'advance') {
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
  if (route === 'advance') { if (loseBatch) {loseBatch = false;throw new Error('Response disconnected');} return recoverableBatch; }
  if (route === 'cancel') return {pending: recoverableBatch};
  throw new Error(`Unexpected ${route}`);
}, () => {}, recoveryStore);
await recovering.start(untouched, false, false);
await recovering.resume(); recovering.pause();
assert.deepEqual(recoveryCalls.slice(1,4).map(([route]) => route), ['resume', 'maintenance', 'resume']);
await assert.rejects(recovering.advance(), /disconnected/);
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
  if (route === 'advance') throw new Error('Response disconnected');
  if (route === 'cancel') return {pending: recoverableBatch};
  throw new Error(`Unexpected ${route}`);
});
await switchedRecovery.start(untouched, false, false);
await assert.rejects(switchedRecovery.advance(), /disconnected/);
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
const replayApi = (await import('./notebook.mjs?capture-replay')).request;
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
  if (route === 'advance') {
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
await assert.rejects(wideSession.advance(), /Mid-answer storage failure/);
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
await assert.rejects(wideSession.advance(), /Mid-answer storage failure/);
assert.equal(wideRequests, 1);
assert.equal(wideSession.pendingDelivery.index, stoppedAt);
assert.equal(consumedWide.length, stoppedAt);
blockWide = false;
await wideSession.advance();
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
  if (route === 'advance') {
    uncertainRequests++;
    return {sequence:12, events:uncertainEvents, applications:2, exhausted:true, delivery_done:true};
  }
  throw new Error(`Unexpected ${route}`);
}, () => {}, uncertainStore);
await uncertainSession.start(untouched, false, false);
uncertainStore.afterCommit = () => { if (loseCommit) { loseCommit = false; throw new Error('Commit result lost'); } };
const consumedUncertain = [], pushUncertain = uncertainSession.stream.push.bind(uncertainSession.stream);
uncertainSession.stream.push = event => { pushUncertain(event); consumedUncertain.push(event); };
await assert.rejects(uncertainSession.advance(), /Commit result lost/);
assert.equal(uncertainSession.ack, null);
assert.equal(uncertainSession.stream.answers.length, 2);
assert.equal(uncertainStore.archives.get(uncertainSession.archive).answers.size, 2);
await uncertainSession.advance();
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
  if (route === 'advance') {
    splitResponses++;
    return splitResponses === 1
      ? {sequence:1, events:splitEvents.slice(0,5), applications:0, exhausted:false, delivery_done:false}
      : {sequence:2, events:splitEvents.slice(5), applications:0, exhausted:true, delivery_done:true};
  }
  throw new Error(`Unexpected ${route}`);
}, () => {}, splitStore);
await splitSession.start(untouched, false, false);
await splitSession.advance();
assert.equal(splitSession.ack, 1);
assert.ok(splitSession.stream.current);
assert.equal(splitSession.stream.writes.length, 0);
assert.equal(splitStore.archives.get(splitSession.archive).answers.size, 0);
assert.equal(splitStore.archives.get(splitSession.archive).parts.size, 3);
await splitSession.advance();
assert.equal(splitSession.ack, 2);
assert.equal(splitSession.stream.total, 1);
assert.equal(splitStore.archives.get(splitSession.archive).answers.size, 1);
assert.equal(splitStore.archives.get(splitSession.archive).parts.size, 10);

const cancelPartialStore = memorySink();
const cancelPartial = testSession(async route => {
  if (route === 'start') return {run:33, ...tables};
  if (route === 'advance') return {sequence:1, events:[...answer(100), ...pendingEvents.slice(0,6)], applications:0, exhausted:false, delivery_done:false};
  if (route === 'cancel') return {};
  throw new Error(`Unexpected ${route}`);
}, () => {}, cancelPartialStore);
await cancelPartial.start(untouched, false, false);
await cancelPartial.advance();
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
const {createContext, runInContext} = await import('node:vm');
const notebookSource = readFileSync(new URL('./notebook.mjs', import.meta.url), 'utf8');
function productionSection(start, end) {
  const first = notebookSource.indexOf(start), last = notebookSource.indexOf(end, first);
  assert.ok(first >= 0 && last > first, `Production section ${start} is available`);
  return notebookSource.slice(first, last);
}
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
  const context = createContext({$, el:element, button:element,
    uint(value) { assert.ok(Number.isSafeInteger(value) && value >= 0); return value; },
    safe(action) { const task = Promise.resolve().then(action).catch(error => { errors.push(error); }); tasks.push(task); return task; },
    store:{scene(collection, number, options) { return new Promise(resolve => reads.push({collection,number,options,resolve})); }},
    renderScene(svg, scene) { paints.push(svg); return scene; },
    session:{stream:null, archive:'first', async switchRun(run) { assert.equal(run, 2); }},
    renderInspectionControls() {}, async refreshSaved() {}, async finishInspection() {},
    savedView:{id:'first',total:1,answers:[{number:1,completion:'1',alternative:'0'}]},
    savedSelection:'', inspected:null, outputMode:'answers', answerNumber:1, answerPage:0,
    page:0, portPage:0, resultPage:1, resultPortPage:1, bindingPage:1,
    pendingNumber:1, pendingPath:[42,43], pendingPage:1, pendingPortPage:1,
    sceneLoading:false, desiredScene:null, loadedSceneKey:null,
  });
  runInContext(productionSection('  function pager(', '  function renderInspector(')
    + productionSection('  function renderResults()', '  async function inspect()')
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
  ['result','page'], ['result-port','portPage'], ['binding','bindingPage'],
  ['pending','pendingPage'], ['pending-port','pendingPortPage'], ['pending-body','pendingNumber'],
]) for (const [direction, expected] of [['prev',0], ['next',2]]) {
  await test(`production ${control} ${direction}: rapid clicks target the displayed page`, async () => {
    const h = renderingHarness(); await h.start(); await h.paint();
    const controlElement = h.$(`${control}-${direction}`);
    assert.equal(controlElement.disabled, false);
    controlElement.onclick(); await Promise.resolve();
    controlElement.onclick(); await Promise.resolve();
    assert.equal(h.reads.length, 2, 'only one additional scene read is in flight');
    assert.equal(h.context.desiredScene.options[field], expected, 'both clicks request the same adjacent page');
    assert.ok(Object.entries(h.context.desiredScene.options).every(([key,value]) => key === 'pendingPath' || value >= 0));
    h.reads[1].resolve(h.scene()); await Promise.all(h.tasks);
    assert.equal(h.errors.length, 0);
    assert.equal(h.reads.length, 2, 'rapid clicks do not schedule a further page');
  });
}
await test('production execution switch resets navigation and rejects an outstanding old scene', async () => {
  const h = renderingHarness(); await h.start();
  h.$('execution').value = '2';
  await h.$('execution').onchange();
  for (const field of ['answerPage','resultPage','resultPortPage','bindingPage','pendingNumber','pendingPage','pendingPortPage']) {
    assert.equal(h.context[field], 0, `${field} resets for the new execution`);
  }
  assert.equal(h.context.answerNumber, null);
  assert.equal(h.context.pendingPath.length, 0);
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
  if (route === 'advance') return partialResponse;
  if (route === 'cancel') return {pending:partialResponse};
  throw new Error(`Unexpected ${route}`);
}, () => {}, quotaCancelStore);
await quotaCancelSession.start(untouched, false, false);
quotaCancelBlocked = true;
await assert.rejects(quotaCancelSession.advance(), /Partial quota failure/);
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
    if (route === 'advance') { advances++; return response; }
    if (route === 'cancel') return {pending:response};
    throw Error(route);
  }, () => notifications.push({error:session.error, ack:session.ack}), store);
  await session.start(untouched, false, false);
  session.stream = new OutputAssembler(tables, 8);
  let pushes = 0;
  const push = session.stream.push.bind(session.stream);
  session.stream.push = event => { push(event); pushes++; };
  await assert.rejects(session.advance(), /Quota/);
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
  session:{async cancel() { calls.push('cancel'); }}, async inspect() { calls.push('inspect'); }});
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

await test('changing the selected archive cannot reload stale saved summaries', async () => {
  const h = renderingHarness(); await h.start();
  h.context.session.archive = 'second';
  h.context.renderResults();
  assert.equal(h.context.desiredScene, null);
  assert.equal(h.context.answerNumber, null);
  h.reads[0].resolve(h.scene()); await Promise.all(h.tasks);
  assert.equal(h.paints.length, 0);
  assert.equal(h.reads.length, 1);
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

});
