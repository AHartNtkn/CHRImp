import assert from 'node:assert/strict';
import { applyEdit, at, sceneEntries, validateNotebook } from './graph.mjs';
import { OutputAssembler, RunSession, InspectionSelection } from './notebook.mjs';

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
assert.equal(stream.answers.length, 5);
assert.equal(stream.answers[0].completion, '0');
const bounded = new OutputAssembler(tables, 1);
answer(0).forEach(event => bounded.push(event));
assert.throws(() => bounded.push(answer(1)[0]), /release/);
assert.equal(bounded.answers[0].completion, '0');
assert.deepEqual(stream.answers[1].facts.map(f => f.occurrence), ['11', '12']);
assert.deepEqual(stream.answers[1].facts[0].args, ['7', '7']);
assert.equal(stream.current, null);
assert.throws(() => stream.push({ kind: 'port', variable: 1 }), /fact/);
assert.throws(() => stream.push({ kind: 'begin', completion: Number.MAX_SAFE_INTEGER + 1, alternative: 0 }), /integer/);
const broken = new OutputAssembler(tables);
broken.push(answer(0)[0]);
assert.throws(() => broken.push({ kind: 'end' }), /variables/);
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
const session = new RunSession(api);
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
const disconnected = new RunSession(async () => { throw new Error('connection unavailable'); });
await assert.rejects(disconnected.start(untouched, false, false), /connection unavailable/);
assert.equal(disconnected.run, null);
console.log('AST editing, schema validation, server metadata, bounded stream assembly and run lifecycle checks passed.');

// Persistence is the ownership handoff: a failed save must not consume a new batch.
const durable = new Map();
let storageBlocked = false, nextCollection = 0, advanceCalls = 0;
const store = {
  async create(metadata) { const key = String(++nextCollection); durable.set(key, { metadata: structuredClone(metadata), answers: new Map() }); return key; },
  async append(key, answer) { if (storageBlocked) throw new Error('Storage full'); durable.get(key).answers.set(answer.number, structuredClone(answer)); },
};
const persisted = new RunSession(async route => {
  if (route === 'start') return { run: 9, ...tables };
  if (route === 'cancel') return {};
  if (route === 'advance') { advanceCalls++; return { events: [0, 1, 2].flatMap(answer), applications: 3, exhausted: true, delivery_done: true }; }
}, () => {}, store);
await persisted.start(untouched, false, false);
storageBlocked = true;
await assert.rejects(persisted.advance(), /Storage full/);
assert.equal(persisted.status, 'error');
assert.equal(persisted.running, false);
assert.equal(persisted.stream.answers.length, 1);
assert.equal(persisted.stream.answers[0].completion, '0');
assert.ok(persisted.pendingDelivery.index > 0);
assert.equal(durable.get(persisted.archive).answers.size, 0);
storageBlocked = false;
await persisted.advance();
assert.equal(advanceCalls, 1);
assert.equal(persisted.stream.answers.length, 0);
assert.equal(persisted.pendingDelivery, null);
assert.equal(persisted.stream.total, 3);
assert.deepEqual([...durable.get(persisted.archive).answers.values()].map(a => a.completion), ['0', '1', '2']);
assert.equal(durable.get(persisted.archive).answers.get(1).facts[0].name, 'p');
const priorArchive = persisted.archive;
await persisted.start(untouched, false, false);
assert.notEqual(persisted.archive, priorArchive);
assert.equal(durable.get(priorArchive).answers.size, 3);
await persisted.cancel();
assert.throws(() => new OutputAssembler({ signatures: tables.signatures }), /variables/);
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
const stepped = new RunSession(async (route, payload) => {
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
const ownedRuns = new RunSession(async (route, payload) => {
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
const recoveryStore = { async create() { return 'recovery'; }, async append() { if (fullStorage) throw new Error('Storage full'); } };
const recoverableBatch = {sequence: 1, events: answer(1), applications: 1, exhausted: true, delivery_done: true};
const recovering = new RunSession(async (route, payload) => {
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
const switchedRecovery = new RunSession(async route => {
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
