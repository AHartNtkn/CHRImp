import { clone, at, atomOf, applyEdit, validateNotebook, renderGraph, renderScene } from './graph.mjs';
import { OutputAssembler, IndexedAnswerStore } from './answers.mjs';

const check = (ok, message) => { if (!ok) throw new Error(message); };
const uint = value => {
  check(Number.isSafeInteger(value) && value >= 0, 'Expected a nonnegative safe integer.');
  return value;
};
const id = value => {
  if (typeof value === 'string') {
    check(/^(0|[1-9][0-9]*)$/.test(value) && BigInt(value) <= 18446744073709551615n, 'Expected an unsigned integer ID.');
    return value;
  }
  return String(uint(value));
};

export class InspectionSelection {
  constructor() { this.choices = []; this.snapshots = []; this.assignments = new Map(); this.snapshot = ''; this.selectedSnapshot = null; this.cursors = {}; this.navigation = {}; this.loading = false; this.lease = null; this.retiring = null; this.operation = null; this.resetting = false; this.pendingCapture = null; }
  update(choices, snapshots, preserve = false) {
    const descriptors = (items, name) => {
      check(Array.isArray(items), `Inspection response needs ${name} descriptors.`);
      const seen = new Set();
      return items.map(item => {
        const key = id(item.id);
        check(typeof item.label === 'string' && !seen.has(key), `Invalid ${name} descriptor.`);
        seen.add(key); return { id: key, label: item.label };
      });
    };
    const nextChoices = descriptors(choices, 'choice'), nextSnapshots = descriptors(snapshots, 'snapshot');
    this.choices = nextChoices; this.snapshots = nextSnapshots;
    if (!preserve) {
      this.assignments = new Map(nextChoices.filter(item => this.assignments.has(item.id)).map(item => [item.id, this.assignments.get(item.id)]));
      if (!nextSnapshots.some(item => item.id === this.snapshot)) { this.snapshot = ''; this.selectedSnapshot = null; }
    }
  }
  selectSnapshot(api, run, key) {
    const descriptor = this.snapshots.find(item => item.id === key) ?? this.selectedSnapshot;
    check(key === '' || descriptor?.id === key, 'This snapshot is no longer available.');
    return this.page(api, run, null, 'refresh', {key, descriptor:key === '' ? null : descriptor});
  }
  async capture(api, run) {
    if (!this.pendingCapture) {
      this.pendingCapture = {run};
    }
    check(this.pendingCapture.run === run, 'Recover the previous run metadata capture before changing runs.');
    const pending = this.pendingCapture;
    const response = await liveRequest('snapshot', pending, api);
    const lease = {run, snapshot: id(response.snapshot)};
    this.pendingCapture = null;
    return lease;
  }
  async releaseRetiring(api) {
    if (!this.retiring) return;
    const lease = this.retiring;
    await liveRequest('snapshot_release', lease, api);
    this.retiring = null;
  }
  async reset(api) {
    check(!this.resetting, 'Selection reset is already in progress.');
    this.resetting = true;
    try {
      if (this.operation) await this.operation.catch(() => {});
      await this.releaseRetiring(api);
      if (this.pendingCapture) {
        this.retiring = await this.capture(api, this.pendingCapture.run);
        await this.releaseRetiring(api);
      }
      if (this.lease) {
        await liveRequest('snapshot_release', this.lease, api);
        this.lease = null;
      }
      this.choices = []; this.snapshots = []; this.assignments.clear();
      this.snapshot = ''; this.selectedSnapshot = null; this.cursors = {}; this.navigation = {};
    } finally { this.resetting = false; }
  }
  page(api, run, kind = null, direction = 'refresh', selection = null) {
    if (this.loading || this.resetting) return Promise.reject(new Error('A metadata page is already loading.'));
    const operation = this.loadPage(api, run, kind, direction, selection);
    this.operation = operation;
    return operation.finally(() => { if (this.operation === operation) this.operation = null; });
  }
  async loadPage(api, run, kind = null, direction = 'refresh', selection = null) {
    check(!this.loading, 'A metadata page is already loading.');
    check(kind === null || ['choice', 'snapshot'].includes(kind), 'Unknown metadata page.');
    check(['refresh', 'next', 'prev'].includes(direction), 'Unknown page direction.');
    const cursors = {...this.cursors};
    if (selection) { delete cursors.after_choice; delete cursors.before_choice; }
    if (kind && direction !== 'refresh') {
      const cursor = this.navigation[`${direction}_${kind}`];
      check(cursor !== null && cursor !== undefined, 'No further metadata page.');
      delete cursors[`after_${kind}`]; delete cursors[`before_${kind}`];
      cursors[`${direction === 'next' ? 'after' : 'before'}_${kind}`] = cursor;
    }
    this.loading = true;
    try {
      await this.releaseRetiring(api);
      check(!this.lease || this.lease.run === run, 'Reset inspection selection before changing runs.');
      const snapshot = selection ? selection.key : this.snapshot;
      let candidate = null;
      if (snapshot && this.pendingCapture) {
        this.retiring = await this.capture(api, this.pendingCapture.run);
        await this.releaseRetiring(api);
      }
      if (!snapshot) candidate = await this.capture(api, run);
      let response;
      try {
        response = await liveRequest('views', {run, ...cursors, snapshot: snapshot || candidate.snapshot}, api);

        const navigation = {};
        for (const type of ['choice', 'snapshot']) for (const way of ['next', 'prev']) {
          const key = `${way}_${type}`;
          navigation[key] = response[key] == null ? null : id(response[key]);
        }
        // These snapshots own choice metadata; they are not recorded states.
        const owned = new Set([this.lease, this.retiring, candidate].filter(Boolean).map(lease => lease.snapshot));
        check(Array.isArray(response.snapshots), 'Inspection response needs snapshot descriptors.');
        this.update(response.choices, response.snapshots.filter(item => !owned.has(id(item.id))), true);
        this.cursors = cursors; this.navigation = navigation;
        if (selection) {
          if (this.snapshot !== selection.key) this.assignments.clear();
          this.snapshot = selection.key; this.selectedSnapshot = selection.descriptor;
        }
      } catch (error) {
        this.retiring = candidate;
        await this.releaseRetiring(api);
        throw error;
      }
      this.retiring = this.lease;
      this.lease = candidate;
      await this.releaseRetiring(api);
    } finally { this.loading = false; }
  }
  choose(key, value) {
    check(this.choices.some(item => item.id === key), 'This choice is no longer available.');
    check(['either', 'first', 'second'].includes(value), 'Choose Either, First or Second.');
    if (value === 'either') this.assignments.delete(key); else this.assignments.set(key, value);
  }
  payload(run) {
    // Engine's first arm uses the positive decision; its second uses the complement.
    const choices = Object.fromEntries([...this.assignments].map(([key, value]) => [key, value === 'first']));
    const payload = { run, choices };
    if (this.snapshot !== '') {
      check(this.snapshots.some(item => item.id === this.snapshot) || this.selectedSnapshot?.id === this.snapshot, 'This snapshot is no longer available.');
      payload.snapshot = this.snapshot;
    }
    return payload;
  }
}

let serverBoot = null, serverOwner = null, attaching = null;
let command = 0, pendingControl = null, controlTail = Promise.resolve();
const controlRoutes = new Set(['start', 'inspect', 'step', 'resume', 'snapshot']);
async function send(route, body) {
  const response = await fetch(`/api/${route}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body,
  });
  const text = await response.text();
  let data;
  try { data = JSON.parse(text); } catch { throw new Error(`${response.status}: ${text.slice(0, 160) || 'Empty server response'}`); }
  if (!response.ok) { const error = new Error(typeof data.error === 'string' ? data.error : data.error?.message ?? data.message ?? `Request failed (${response.status})`); error.retry = data.retry === true; error.status = response.status; error.code = data.code; throw error; }
  return data;
}
async function owner() {
  // Keep an accepted attach's identity when its response is lost.
  serverBoot ??= send('hello', '{}').then(response => {
    check(typeof response.boot === 'string' && /^[0-9a-f]{32}$/.test(response.boot), 'Invalid server identity.');
    return response.boot;
  }).catch(error => { serverBoot = null; throw error; });
  const boot = await serverBoot;
  attaching ??= (async () => {
    if (serverOwner === null) {
      const reserved = uint((await send('reserve', JSON.stringify({boot}))).owner);
      check(reserved > 0, 'Invalid notebook owner.'); serverOwner = reserved;
    }
    await send('attach', JSON.stringify({boot, owner:serverOwner}));
  })().catch(error => {
    attaching = null;
    // An overtaken reservation has never owned an execution. Its next attempt
    // may reserve again; commands under an attached owner never change owners.
    if (error.code === 'unknown_owner') serverOwner = null;
    throw error;
  });
  await attaching;
  return {boot, owner:serverOwner};
}
export async function request(route, payload) {
  if (['hello', 'parse', 'format'].includes(route)) return send(route, JSON.stringify(payload));
  const identity = await owner();
  if (!controlRoutes.has(route)) return send(route, JSON.stringify({...payload, ...identity}));
  return serializeControl(async () => {
    const input = JSON.stringify({...payload, ...identity});
    if (pendingControl) {
      check(pendingControl.route === route && pendingControl.input === input, 'Recover the interrupted command before submitting another command.');
    } else {
      check(Number.isSafeInteger(command + 1), 'Control command sequence exhausted.');
      pendingControl = {route, input, command:command + 1, body:JSON.stringify({...payload, ...identity, command:command + 1})};
    }
    return replayControl();
  });
}
function serializeControl(action) {
  const operation = controlTail.then(action);
  controlTail = operation.catch(() => {});
  return operation;
}
async function replayControl() {
  try {
    const response = await send(pendingControl.route, pendingControl.body);
    command = pendingControl.command; pendingControl = null;
    return response;
  } catch (error) {
    // Only a definite rejection permits a replacement. Ambiguous delivery
    // keeps the exact request for replay, including after a lost response.
    if (error.status >= 400 && error.status < 500 && !error.retry) pendingControl = null;
    throw error;
  }
}
request.recover = () => serializeControl(async () => {
  if (!pendingControl) return null;
  const route = pendingControl.route;
  const {boot, owner, ...payload} = JSON.parse(pendingControl.input);
  for (;;) {
    try { return {route, payload, response:await replayControl()}; }
    catch (error) {
      if (!error.retry) throw error;
      await send('maintenance', JSON.stringify({boot, owner, run:payload.run, budget:2048}));
      await new Promise(resolve => setTimeout(resolve, 0));
    }
  }
});

async function liveRequest(route, payload, api = request) {
    for (;;) {
      try { return await api(route, payload); }
      catch (error) {
        if (!error.retry) throw error;
        await api('maintenance', { run: payload.run, budget: 2048 });
        await new Promise(resolve => setTimeout(resolve, 0));
      }
    }
  }

// A cached response is bounded by the server's delivery budget. On cancel,
// preserve every complete alternative and retire only its unfinished suffix.
export async function deliverCachedOutput(store, archive, stream, pending, cancel = false) {
  const events = pending?.response?.events ?? [];
  let limit = events.length;
  if (cancel) {
    limit = pending?.index ?? 0;
    for (let index = events.length - 1; index >= limit; index--) {
      if (events[index].kind === 'end') { limit = index + 1; break; }
    }
  }
  if (!cancel || (pending && pending.index < limit)) await store.flush(archive, stream);
  while (pending && pending.index < limit) {
    stream.push(events[pending.index]);
    pending.index++;
    if (stream.needsFlush) await store.flush(archive, stream);
  }
  if (cancel) {
    await store.discardPartial(archive, stream);
    if (pending) pending.index = events.length;
  } else await store.flush(archive, stream);
}

export class RunSession {
  constructor(api = request, notify = () => {}, store = new IndexedAnswerStore()) {
    this.api = api; this.notify = notify; this.store = store; this.archive = null; this.pendingDelivery = null; this.run = null; this.status = 'idle';
    this.running = false; this.inFlight = null; this.timer = null; this.stream = null;
    this.applications = 0; this.error = null; this.starting = false; this.runs = new Map(); this.canceling = false; this.ack = null; this.selection = new InspectionSelection(); this.changingRun = false; this.pendingClose = null;
    this.recoveredControl = null; this.recovering = null;
  }
  async start(model, recordHistory = false, auto = true) {
    check(!this.starting && !this.changingRun, 'A run is already starting.');
    this.starting = true;
    try {
      await this.cancel();
      await this.selection.reset(this.api);
      this.submission = clone(validateNotebook(model));
      this.status = 'starting'; this.error = null; this.notify();
      const response = await this.api('start', { ...clone(this.submission), record_history: recordHistory });
      this.adoptStart({...this.submission, record_history:recordHistory}, response);
      await this.ensureArchive();
      if (auto) await this.resume();
    } catch (error) { this.fail(error); throw error; }
    finally { this.starting = false; this.notify(); }
  }
  async ensureArchive() {
    if (this.stream && this.archive === null) this.archive = await this.store.create(this.stream.tables, `Run ${this.run}`);
  }
  adoptStart(payload, response) {
    const run = uint(response.run), stream = new OutputAssembler(response);
    const submission = clone(validateNotebook({program:payload.program, query:payload.query}));
    this.run = run; this.stream = stream; this.submission = submission;
    this.ack = null; this.archive = null; this.pendingDelivery = null;
    this.recordHistory = payload.record_history === true;
    this.applications = 0; this.exhausted = false; this.status = 'paused';
  }
  settleControl() {
    this.recovering ??= this.recoverControl().finally(() => { this.recovering = null; });
    return this.recovering;
  }
  async recoverControl() {
    // Let the metadata consumer adopt a successful response before recovering
    // its interrupted capture. It shares the same single receipt authority.
    if (this.selection.operation) await this.selection.operation.catch(() => {});
    for (;;) {
      this.recoveredControl ??= await this.api.recover?.() ?? null;
      const recovered = this.recoveredControl;
      if (!recovered) return;
      const {route, payload, response} = recovered;
      if (route === 'start') {
        if (!recovered.adopted) {
          this.adoptStart(payload, response);
          recovered.adopted = true;
        }
        await this.ensureArchive();
      } else if (route === 'snapshot') {
        if (!recovered.adopted) {
          await this.selection.releaseRetiring(this.api);
          recovered.lease = {run:payload.run, snapshot:id(response.snapshot)};
          this.selection.retiring = recovered.lease;
          if (this.selection.pendingCapture?.run === payload.run) this.selection.pendingCapture = null;
          recovered.adopted = true;
        }
        await liveRequest('snapshot_release', recovered.lease, this.api);
        if (this.selection.retiring === recovered.lease) this.selection.retiring = null;
      } else if (route === 'inspect') {
        const target = {run:payload.run, inspection:id(response.inspection)};
        while (!recovered.release) {
          const canceled = await liveRequest('inspect_cancel', {...target, budget:2048}, this.api);
          recovered.release = canceled.done === true;
          if (!recovered.release) await new Promise(resolve => setTimeout(resolve, 0));
        }
        await liveRequest('inspect_release', target, this.api);
      } else check(route === 'step' || route === 'resume', 'Unknown recovered control command.');
      // The response belongs to this session until adoption/cleanup succeeds.
      this.recoveredControl = null;
    }
  }
  async deliverPending(cancel = false) {
    if (!this.stream) return;
    await this.ensureArchive();
    const pending = this.pendingDelivery;
    await deliverCachedOutput(this.store, this.archive, this.stream, pending, cancel);
    if (!pending) return;
    const response = pending.response;
    if (response.delivery_done) this.stream.finish();
    this.ack = response.sequence ?? this.ack;
    this.pendingDelivery = null;
    return response;
  }
  fail(error) { this.pause(); this.error = error; this.status = 'error'; this.notify(); }
  pause() {
    this.running = false; clearTimeout(this.timer); this.timer = null;
    if (this.run !== null && !['done', 'error', 'canceled'].includes(this.status)) this.status = 'paused';
    this.notify();
  }
  async resume() {
    await this.finishClose();
    check(this.run !== null && this.stream, 'Start a run first.');
    if (['done', 'canceled'].includes(this.status)) return;
    const run = this.run;
    this.error = null; this.running = true; this.status = 'running'; this.notify();
    try {
      await liveRequest('resume', { run }, this.api);
      if (this.run === run && this.running) this.schedule();
    } catch (error) { if (this.run === run) this.fail(error); throw error; }
  }

  schedule() {
    if (!this.running || this.inFlight || this.timer !== null) return;
    this.timer = setTimeout(() => { this.timer = null; this.advance().catch(() => {}); }, 16);
  }
  advance() {
    check(this.pendingClose === null, 'Finish closing the execution first.');
    if (this.inFlight) return this.inFlight;
    check(this.run !== null && this.stream, 'Start a run first.');
    const pending = this.pendingDelivery ? Promise.resolve(this.pendingDelivery.response) : this.api('advance', { run: this.run, budget: 2048, ack: this.ack });
    this.inFlight = (async () => {
      try {
        const response = await pending;
        check(Array.isArray(response.events), 'Advance response needs output events.');
        this.pendingDelivery ??= { response, index: 0 };
        await this.deliverPending();
        this.applications = uint(response.applications);
        this.exhausted = response.exhausted;
        if (response.delivery_done) { this.stream.finish(); this.running = false; this.status = 'done'; }
        else this.status = this.running ? 'running' : 'paused';
        this.notify(); return response;
      } catch (error) { this.fail(error); throw error; }
      finally { this.inFlight = null; this.schedule(); this.notify(); }
    })();
    return this.inFlight;
  }
  async step(choices = {}) {
    await this.finishClose();
    check(this.run !== null, 'Start a run first.');
    this.pause(); if (this.inFlight) await this.inFlight;
    if (['done', 'canceled'].includes(this.status)) return;
    await liveRequest('step', { run: this.run, choices }, this.api);
    let response;
    do {
      response = await this.advance();
      await new Promise(resolve => setTimeout(resolve, 0));
    } while (!response.step?.done && !response.delivery_done && this.run !== null && this.status !== 'canceled' && !this.canceling);
    return response;
  }
  rememberRun() {
    if (this.run !== null) this.runs.set(this.run, { run: this.run, stream: this.stream, archive: this.archive, recordHistory: this.recordHistory, applications: this.applications, submission: this.submission, status: this.status, exhausted: this.exhausted, ack: this.ack });
  }
  async switchRun(run) {
    check(!this.changingRun && !this.starting, 'An execution change is already in progress.');
    this.changingRun = true; this.notify();
    try {
      await this.finishClose();
      this.pause(); if (this.inFlight) await this.inFlight;
      await this.settleControl(); await this.deliverPending(); this.rememberRun();
      check(this.runs.has(run), 'Unknown execution.');
      await this.selection.reset(this.api);
      Object.assign(this, this.runs.get(run)); this.running = false; this.error = null;
    } finally { this.changingRun = false; this.notify(); }
  }
  async finishClose() {
    const run = this.pendingClose;
    if (run === null) return;
    await this.api('close', {run});
    // Another caller may already have completed this same close.
    if (this.pendingClose !== run) return;
    this.runs.delete(run); this.pendingClose = null;
    this.run = null; this.stream = null; this.archive = null; this.pendingDelivery = null;
    this.ack = null; this.status = 'idle'; this.recordHistory = false; this.error = null;
    this.notify();
  }
  async closeRun() {
    check(!this.changingRun && !this.starting, 'An execution change is already in progress.');
    this.changingRun = true; this.notify();
    try {
      if (this.pendingClose === null) {
        await this.cancel();
        await this.selection.reset(this.api);
        this.pendingClose = this.run;
      }
      await this.finishClose();
    } finally { this.changingRun = false; this.notify(); }
  }
  async cancel() {
    if (this.pendingClose !== null) { await this.finishClose(); return; }
    this.canceling = true;
    try {
      this.pause();
      if (this.inFlight) await this.inFlight.catch(() => {});
      await this.settleControl();
      if (this.run !== null) {
        const canceled = await this.api('cancel', { run: this.run });
        if (canceled.pending && canceled.pending.sequence !== this.ack) this.pendingDelivery ??= {response: canceled.pending, index: 0};
      }
      this.status = this.run === null ? 'idle' : 'canceled'; this.running = false;
      await this.deliverPending(true);
      this.error = null;
      this.rememberRun(); this.notify();
    } finally { this.canceling = false; }
  }

}

if (typeof document !== 'undefined' && document.getElementById('editor-graph')) mountNotebook();

function mountNotebook() {
  const $ = name => document.getElementById(name);
  const el = (tag, text, attrs = {}) => {
    const node = document.createElement(tag); if (text !== undefined) node.textContent = text;
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
    return node;
  };
  const button = (text, action, disabled = false) => {
    const node = el('button', text, { type: 'button' }); node.disabled = disabled;
    node.onclick = () => safe(action); return node;
  };
  const field = (label, value, action) => {
    const wrapper = el('label', label), input = el('input'); input.value = value;
    input.onchange = () => safe(() => action(input.value)); wrapper.append(input); return wrapper;
  };
  let model = { program: { rules: [] }, query: { kind: 'true' } };
  let path = ['query'], selected = null, page = 0, portPage = 0, dirty = false, busy = false, revision = 0;
  let undo = [], redo = [], debounce, inspected = null, outputMode = 'answers', answerNumber = null;
  const store = new IndexedAnswerStore();
  let savedView = null, savedSelection = '', answerPage = 0, inspectionPending = null, inspecting = false, launching = false;
  let resultPage = 0, resultPortPage = 0, bindingPage = 0;
  let pendingNumber = 0, pendingPath = [], pendingPage = 0, pendingPortPage = 0;
  let catalog = {records:[], next:null, prev:null}, catalogCursor = null, catalogDirection = 'next', catalogDirty = true, catalogStamp = '';
  let refreshing = null, refreshAgain = false, sceneLoading = false, desiredScene = null, loadedSceneKey = null;
  let inspectionCanceled = false;
  const session = new RunSession(request, renderRun, store);
  const inspectionSelection = session.selection;
  function message(text, error = false) { $('message').textContent = text; $('message').classList.toggle('error', error); }
  async function safe(action) { try { return await action(); } catch (error) { message(error.message, true); } }
  function remember() { undo.push(clone(model)); if (undo.length > 40) undo.shift(); redo = []; }
  async function syncSource() {
    clearTimeout(debounce);
    const version = revision;
    const response = await request('parse', { program: $('program').value, query: $('query').value });
    check(version === revision, 'Source changed while parsing. Sync the latest text.');
    validateNotebook(response);
    if (JSON.stringify(model) !== JSON.stringify(response)) remember();
    model = clone({ program: response.program, query: response.query }); revision++; dirty = false; selected = null;
    try { at(model, path); } catch { path = ['query']; }
    renderWorkspace(); message('Text and graph are in sync.');
  }
  async function commit(next, history = 'edit') {
    check(!dirty && !busy, 'Sync the source before editing the diagram.');
    const version = ++revision; busy = true; renderWorkspace();
    try {
      const formatted = await request('format', next);
      check(typeof formatted.program === 'string' && typeof formatted.query === 'string', 'Format response needs program and query source.');
      check(version === revision, 'Source changed while formatting. Sync the latest text.');
      if (history === 'undo') { redo.push(clone(model)); undo.pop(); }
      else if (history === 'redo') { undo.push(clone(model)); redo.pop(); }
      else remember();
      model = next; $('program').value = formatted.program; $('query').value = formatted.query;
      try { at(model, path); } catch { path = ['query']; }
      if (selected) { try { at(model, selected.path); } catch { selected = null; } }
      message(session.run === null ? 'Text and graph are in sync.' : 'Edits apply to the next run.');
    } finally { busy = false; renderWorkspace(); }
  }
  const edit = op => commit(applyEdit(model, op));
  const select = value => { selected = value; renderWorkspace(); };
  function navigate(next) { path = next; selected = null; page = portPage = 0; renderWorkspace(); }
  function renderWorkspace() {
    const disabled = dirty || busy;
    $('graph-tools').disabled = disabled; $('inspector').disabled = disabled;
    $('undo').disabled = disabled || !undo.length; $('redo').disabled = disabled || !redo.length;
    $('target').replaceChildren(el('option', 'Query', { value: 'query' }), ...model.program.rules.map((rule, i) => el('option', rule.name || `Rule ${i + 1}`, { value: String(i) })));
    const ruleIndex = path[0] === 'program' ? path[2] : null;
    $('target').value = ruleIndex === null ? 'query' : String(ruleIndex);
    $('rule-sides').hidden = ruleIndex === null;
    for (const control of $('rule-sides').querySelectorAll('[data-side]')) control.setAttribute('aria-pressed', String(path[3] === control.dataset.side));
    $('rule-name').hidden = ruleIndex === null;
    $('rule-name-input').value = ruleIndex === null ? '' : model.program.rules[ruleIndex].name ?? '';
    $('remove-rule').disabled = ruleIndex === null || disabled;
    $('breadcrumb').replaceChildren();
    const rootLength = ruleIndex === null ? 1 : 4;
    $('breadcrumb').append(button(ruleIndex === null ? 'Query' : `${model.program.rules[ruleIndex].name || `Rule ${ruleIndex + 1}`} / ${path[3]}`, () => navigate(path.slice(0, rootLength))));
    for (let i = rootLength; i < path.length; i += 2) {
      const prefix = path.slice(0, i + 2), node = at(model, prefix);
      $('breadcrumb').append(el('span', ' / '), button(`${node.kind} ${Number(path[i + 1]) + 1}`, () => navigate(prefix)));
    }
    const scope = at(model, path);
    if (scope.kind) $('breadcrumb').append(el('span', ` · ${scope.kind === 'or' ? 'Or alternatives' : scope.kind === 'and' ? 'And conjunction' : 'expression'} `), button('Select expression', () => select({ path: [...path] }), disabled));
    const info = renderGraph($('editor-graph'), model, path, {
      page, portPage, selected, readonly: disabled, label: 'Editable query or rule hypergraph',
      onSelect: select,
      onConnect: variable => safe(() => { check(selected?.port !== undefined, 'Select a numbered relation port first.'); return edit({ type: 'set-port', path: selected.path, index: selected.port, variable }); }),
    });
    page = info.page; portPage = info.portPage;
    pager('graph', info, () => renderWorkspace()); renderInspector();
  }
  function pager(prefix, info, render) {
    $(prefix + '-page').textContent = `Page ${info.page + 1} / ${info.pages} · ${info.count} items`;
    $(prefix + '-ports').textContent = `Port page ${info.portPage + 1} / ${info.portPages}`;
    for (const [suffix, delta, ports] of [['prev', -1, false], ['next', 1, false], ['port-prev', -1, true], ['port-next', 1, true]]) {
      const current = ports ? info.portPage : info.page, max = ports ? info.portPages : info.pages;
      $(prefix + '-' + suffix).disabled = current + delta < 0 || current + delta >= max;
      $(prefix + '-' + suffix).onclick = () => { if (prefix === 'graph') { if (ports) portPage = current + delta; else page = current + delta; } else if (prefix === 'pending') { if (ports) pendingPortPage = current + delta; else pendingPage = current + delta; } else { if (ports) resultPortPage = current + delta; else resultPage = current + delta; } render(); };
    }
  }
  function renderInspector() {
    const panel = $('selection'); panel.replaceChildren();
    if (!selected) { panel.append(el('p', 'Select a relation, group or numbered port.')); return; }
    let node;
    try { node = at(model, selected.path); } catch { selected = null; return; }
    const atom = atomOf(node), target = selected.path;
    panel.append(el('h3', atom ? 'Relation & ordered ports' : 'Expression'));
    if (atom) {
      panel.append(field('Relation', atom.relation, relation => edit({ type: 'rename-relation', path: target, relation })));
      const start = portPage * 8;
      atom.args.slice(start, start + 8).forEach((variable, offset) => {
        const index = start + offset, row = el('div', undefined, { class: 'port-row' });
        row.append(field(`Port ${index + 1}`, variable, variable => edit({ type: 'set-port', path: target, index, variable })),
          button('↑', () => edit({ type: 'move-port', path: target, index, to: index - 1 }), index === 0),
          button('↓', () => edit({ type: 'move-port', path: target, index, to: index + 1 }), index === atom.args.length - 1),
          button('×', () => edit({ type: 'remove-port', path: target, index })));
        row.children[1].setAttribute('aria-label', `Move port ${index + 1} earlier`);
        row.children[2].setAttribute('aria-label', `Move port ${index + 1} later`);
        row.children[3].setAttribute('aria-label', `Remove port ${index + 1}`); panel.append(row);
      });
      panel.append(button('Add port', () => edit({ type: 'insert-port', path: target, variable: 'X' })));
      panel.append(el('p', selected.port === undefined ? 'Select a port, then a variable junction to connect them.' : `Port ${selected.port + 1} selected. Choose a variable junction.`));
    } else if (node.kind === 'equal') {
      panel.append(field('Left variable', node.left, left => edit({ type: 'equal', path: target, left, right: node.right })), field('Right variable', node.right, right => edit({ type: 'equal', path: target, left: node.left, right })));
    } else if (node.items) {
      panel.append(button('Open group →', () => navigate(target)), button(node.kind === 'and' ? 'Change to Or' : 'Change to And', () => edit({ type: 'replace', path: target, node: { ...node, kind: node.kind === 'and' ? 'or' : 'and' } })));
    } else panel.append(button(node.kind === 'true' ? 'Change to fail' : 'Change to true', () => edit({ type: 'replace', path: target, node: { kind: node.kind === 'true' ? 'fail' : 'true' } })));
    if (node.kind) panel.append(button('Wrap in And', () => edit({ type: 'wrap', path: target, kind: 'and' })), button('Wrap in Or', () => edit({ type: 'wrap', path: target, kind: 'or' })));
    const parent = at(model, target.slice(0, -1));
    if (Array.isArray(parent)) {
      const index = target.at(-1);
      panel.append(button('Move earlier', () => edit({ type: 'move-item', path: target, to: index - 1 }), index === 0), button('Move later', () => edit({ type: 'move-item', path: target, to: index + 1 }), index === parent.length - 1));
    }
    panel.append(button('Remove item', async () => { await edit({ type: 'remove', path: target }); selected = null; renderWorkspace(); }));
  }
  function renderRun() {
    $('run-status').textContent = session.status;
    $('applications').textContent = `${session.applications} applications`;
    const runIds = [...new Set([...session.runs.keys(), ...(session.run === null ? [] : [session.run])])];
    $('execution').replaceChildren(el('option', 'Current notebook', {value: ''}), ...runIds.map(run => el('option', `Run ${run}`, {value: run})));
    $('execution').value = session.run ?? '';
    $('execution').disabled = inspecting || launching || session.starting;
    $('release-run').disabled = session.run === null || inspecting || launching || session.starting;

    $('run').disabled = session.starting || busy || launching || inspecting;
    $('pause').disabled = !session.running;
    $('resume').disabled = session.run === null || session.running || ['done', 'canceled'].includes(session.status) || session.starting;
    $('step').disabled = session.status === 'canceled' || session.starting || !!session.inFlight || busy || launching || inspecting;
    $('cancel').disabled = (session.run === null && session.status !== 'error') || session.starting;
    $('inspect').disabled = (session.run === null && !inspectionPending) || session.starting || inspecting || launching;
    $('snapshot').disabled = !session.recordHistory || inspecting || inspectionSelection.loading || session.starting || session.changingRun;
    if (session.error) message(session.error.message, true);
    renderResults(); safe(refreshSaved);
  }
  function refreshSaved() {
    refreshAgain = true;
    if (refreshing) return refreshing;
    refreshing = (async () => {
      while (refreshAgain) {
        refreshAgain = false;
        const stamp = `${session.archive}|${inspected?.archive}`;
        if (stamp !== catalogStamp) { catalogStamp = stamp; catalogDirty = true; }
        if (catalogDirty) {
          const cursor = catalogCursor, direction = catalogDirection;
          const page = await store.list({cursor, direction, size:32});
          if (cursor !== catalogCursor || direction !== catalogDirection) { refreshAgain = true; continue; }
          catalog = page; catalogDirty = false;
        }
        const collection = outputMode === 'inspect' ? inspected?.archive : savedSelection || session.archive;
        const requestedPage = answerPage;
        const view = collection ? await store.page(collection, requestedPage) : null;
        if (collection !== (outputMode === 'inspect' ? inspected?.archive : savedSelection || session.archive) || requestedPage !== answerPage) {
          refreshAgain = true; continue;
        }
        const records = catalog.records.map(record => record.id === view?.id ? {...record, total:view.total} : record);
        if (savedSelection && view && !records.some(record => record.id === savedSelection)) records.unshift(view);
        $('saved-run').replaceChildren(el('option', 'Current run', { value: '' }), ...records.map(record => el('option', `${record.label} · ${record.total} answers · ${new Date(record.created).toLocaleString()}`, { value: record.id })));
        $('saved-run').value = savedSelection;
        $('collections-prev').disabled = !catalog.prev;
        $('collections-next').disabled = !catalog.next;
        if (savedView?.id !== view?.id) resetResultPages();
        savedView = view; answerPage = view?.page ?? 0;
        $('saved-page').value = answerPage + 1;
        $('saved-page').max = view?.pages ?? 1;
        $('saved-pages').textContent = `/ ${view?.pages ?? 1}`;
        $('saved-prev').disabled = answerPage === 0;
        $('saved-next').disabled = !view || answerPage + 1 >= view.pages;
        $('clear-answers').disabled = !view || (view.id === session.archive && session.run !== null) || view.id === inspectionPending?.archive;
        renderResults();
      }
    })().finally(() => { refreshing = null; });
    return refreshing;
  }
  function renderResults() {
    const collection = outputMode === 'inspect' ? inspected?.archive : savedSelection || session.archive;
    const stream = savedView?.id === collection ? savedView : null;
    $('output-mode').value = outputMode;
    const answers = stream?.answers ?? [];
    let answer = answers.find(answer => answer.number === answerNumber) ?? answers.at(-1);
    answerNumber = answer?.number ?? null;
    $('alternatives').replaceChildren(...answers.map(answer => el('option', `Answer ${answer.number} · completion ${answer.completion} / alternative ${answer.alternative}`, { value: answer.number })));
    $('alternatives').value = answerNumber ?? '';
    $('answer-count').textContent = stream ? `${stream.total} saved · ${answers.length} on this page${session.stream?.current && outputMode === 'answers' && !savedSelection ? ' · receiving an alternative…' : ''}` : 'No answers yet';
    const index = answers.indexOf(answer);
    $('answer-prev').disabled = index <= 0; $('answer-next').disabled = index < 0 || index === answers.length - 1;
    $('answer-prev').onclick = () => { answerNumber = answers[index - 1].number; resetResultPages(); renderResults(); };
    $('answer-next').onclick = () => { answerNumber = answers[index + 1].number; resetResultPages(); renderResults(); };
    $('result-empty').hidden = !!answer;
    $('result-graph').toggleAttribute('hidden', !answer);
    if (!answer) {
      desiredScene = null; loadedSceneKey = null;
      $('bindings').replaceChildren(); $('result-graph').replaceChildren(); $('pending-bodies').hidden = true;
      return;
    }
    const options = {page:resultPage, portPage:resultPortPage, bindingPage, pendingNumber, pendingPage, pendingPortPage, pendingPath:[...pendingPath]};
    const key = JSON.stringify([stream.id, answer.number, options]);
    desiredScene = {key, collection:stream.id, number:answer.number, options};
    if (loadedSceneKey !== key) safe(loadScene);
  }
  async function loadScene() {
    if (sceneLoading) return;
    sceneLoading = true;
    try {
      while (desiredScene && desiredScene.key !== loadedSceneKey) {
        const request = desiredScene;
        let scene;
        try { scene = await store.scene(request.collection, request.number, request.options); }
        catch (error) { if (desiredScene?.key !== request.key) continue; throw error; }
        if (desiredScene?.key !== request.key) continue;
        if (!scene) { loadedSceneKey = request.key; return; }
        $('bindings').replaceChildren(...scene.bindings.map(binding => el('span', `${binding.name} = V${binding.variable}`)));
        bindingPage = scene.bindingPage;
        $('binding-page').textContent = `Bindings ${bindingPage + 1} / ${scene.bindingPages}`;
        $('binding-prev').disabled = bindingPage === 0;
        $('binding-next').disabled = bindingPage + 1 === scene.bindingPages;
        $('binding-prev').onclick = () => { bindingPage = scene.bindingPage - 1; renderResults(); };
        $('binding-next').onclick = () => { bindingPage = scene.bindingPage + 1; renderResults(); };
        const info = renderScene($('result-graph'), scene.facts, {readonly:true, label:outputMode === 'inspect' ? 'Inspected graph' : 'Answer hypergraph'});
        resultPage = info.page; resultPortPage = info.portPage; pager('result', info, renderResults);
        renderPending(scene.pending);
        loadedSceneKey = request.key;
      }
    } finally { sceneLoading = false; }
  }
  function renderPending(body) {
    $('pending-bodies').hidden = !body;
    if (!body) return;
    pendingNumber = body.index; pendingPath = body.path;
    $('pending-body-number').value = pendingNumber + 1; $('pending-body-number').max = body.count;
    $('pending-body-count').textContent = `/ ${body.count} · event ${body.event}`;
    const choose = number => { pendingNumber = Math.min(uint(number), body.count - 1); pendingPath = []; pendingPage = pendingPortPage = 0; renderResults(); };
    $('pending-body-prev').disabled = pendingNumber === 0; $('pending-body-next').disabled = pendingNumber + 1 === body.count;
    $('pending-body-prev').onclick = () => choose(body.index - 1); $('pending-body-next').onclick = () => choose(body.index + 1);
    $('pending-body-number').onchange = () => safe(() => choose(Number($('pending-body-number').value) - 1));
    const open = path => { pendingPath = path; pendingPage = pendingPortPage = 0; renderResults(); };
    $('pending-location').replaceChildren(button('Body', () => open([])));
    for (const crumb of body.breadcrumbs) $('pending-location').append(el('span', ' / '), button(crumb.label, () => open(crumb.path)));
    const info = renderScene($('pending-graph'), body.scene, {readonly:true, onOpen:open, label:'Pending body graph'});
    pendingPage = info.page; pendingPortPage = info.portPage; pager('pending', info, renderResults);
  }
  function resetResultPages() { resultPage = resultPortPage = bindingPage = pendingNumber = pendingPage = pendingPortPage = 0; pendingPath = []; desiredScene = null; loadedSceneKey = null; }
  async function inspect() {
    check(!inspecting, 'An inspection is already in progress.');
    inspecting = true; renderRun();
    try { await inspectOnce(); } finally { inspecting = false; renderRun(); }
  }
  async function inspectOnce() {
    if (!inspectionPending) {
      await session.finishClose();
      check(session.run !== null, 'Start a run first.');
      session.pause(); if (session.inFlight) await session.inFlight;
      const run = session.run, tables = session.stream.tables;
      const response = await liveRequest('inspect', inspectionSelection.payload(run));
      check(session.run === run, 'The run changed during inspection.');
      inspectionCanceled = false;
      inspectionPending = { stream: new OutputAssembler(tables), response: null, index: 0, archive: null, run, inspection: response.inspection, ack: null };
      await inspectionSelection.page(request, run); renderInspectionControls();
    }
    const pending = inspectionPending;
    pending.archive ??= await store.create(pending.stream.tables, `Inspection of run ${pending.run}`);
    while (!pending.cleanup) {
      if (inspectionCanceled) {
        let response;
        do {
          response = await liveRequest('inspect_cancel', { run: pending.run, inspection: pending.inspection, budget: 2048 });
          if (response.pending && response.pending.sequence !== pending.ack
              && response.pending.sequence !== pending.response?.sequence) {
            pending.response = response.pending; pending.index = 0;
          }
          await deliverCachedOutput(store, pending.archive, pending.stream, pending, true);
          pending.ack = pending.response?.sequence ?? pending.ack;
        } while (!response.done);
        break;
      }
      pending.response ??= await request('inspect_advance', { run: pending.run, inspection: pending.inspection, budget: 2048, ack: pending.ack });
      await deliverCachedOutput(store, pending.archive, pending.stream, pending);
      if (pending.response.error) {
        const failure = pending.response.error;
        let discarded;
        do { discarded = await liveRequest('inspect_cancel', {run: pending.run, inspection: pending.inspection, budget: 2048}); } while (!discarded.done);
        pending.failure = failure;
        break;
      }
      pending.ack = pending.response.sequence;
      if (pending.response.done) { pending.stream.finish(); break; }
      pending.response = null; pending.index = 0;
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    if (pending.cleanup !== 'release') {
      pending.cleanup = 'persist';
      await store.discardPartial(pending.archive, pending.stream);
      pending.cleanup = 'release';
    }
    await liveRequest('inspect_release', { run: pending.run, inspection: pending.inspection });
    inspectionPending = null;
    if (pending.failure) throw new Error(`Inspection failed: ${pending.failure}`);
    inspected = { archive: pending.archive };
    outputMode = 'inspect'; answerNumber = null; answerPage = 0; resetResultPages();
    await refreshSaved(); message(inspectionCanceled ? 'Inspection stopped; completed graphs are saved.' : 'Inspection saved.');
  }

  async function finishInspection() {
    check(!inspecting, 'Wait for the active inspection before changing executions.');
    inspectionCanceled = true;
    if (inspectionPending) await inspect();
  }

  function renderInspectionControls() {
    const unavailable = session.starting || session.changingRun || inspectionSelection.resetting;
    $('snapshot').disabled = !session.recordHistory || inspecting || inspectionSelection.loading || session.starting || session.changingRun;
    $('choices').replaceChildren();
    if (!inspectionSelection.choices.length) $('choices').append(el('p', session.run === null ? 'Start a run to inspect its choices.' : inspectionSelection.navigation.next_choice === undefined ? 'Inspect the graph to load available choices.' : 'No choices in this state.'));
    inspectionSelection.choices.forEach(item => {
      const label = el('label', item.label), select = el('select');
      select.append(el('option', 'Either', { value: 'either' }), el('option', 'First', { value: 'first' }), el('option', 'Second', { value: 'second' }));
      select.disabled = unavailable;
      select.value = inspectionSelection.assignments.get(item.id) ?? 'either';
      select.onchange = () => safe(() => inspectionSelection.choose(item.id, select.value));
      label.append(select); $('choices').append(label);
    });
    $('choice-page').textContent = `${inspectionSelection.choices.length} choices on this page`;
    for (const kind of ['choice', 'snapshot']) for (const direction of ['prev', 'next']) {
      $(`${kind}-${direction}`).disabled = unavailable || inspectionSelection.loading || !inspectionSelection.navigation[`${direction}_${kind}`];
    }
    const snapshots = [...inspectionSelection.snapshots];
    if (inspectionSelection.selectedSnapshot && !snapshots.some(item => item.id === inspectionSelection.snapshot)) snapshots.unshift(inspectionSelection.selectedSnapshot);
    $('snapshot').replaceChildren(el('option', 'Current graph', { value: '' }), ...snapshots.map(item => el('option', item.label, { value: item.id })));
    $('snapshot-page').textContent = `${inspectionSelection.snapshots.length} states on this page`;
    $('snapshot').value = inspectionSelection.snapshot;
  }
  async function metadataPage(kind, direction) {
    check(!session.starting && !session.changingRun, 'Wait for the execution change.');
    const pending = inspectionSelection.page(request, session.run, kind, direction);
    renderInspectionControls();
    try { await pending; } finally { renderInspectionControls(); }
  }
  for (const kind of ['choice', 'snapshot']) for (const direction of ['prev', 'next']) {
    $(`${kind}-${direction}`).onclick = () => safe(() => metadataPage(kind, direction));
  }
  $('snapshot').onchange = () => safe(async () => {
    check(!session.starting && !session.changingRun, 'Wait for the execution change.');
    const pending = inspectionSelection.selectSnapshot(request, session.run, $('snapshot').value);
    renderInspectionControls();
    try { await pending; } finally { renderInspectionControls(); }
  });
  for (const name of ['program', 'query']) $(name).addEventListener('input', () => {
    revision++; dirty = true; renderWorkspace(); clearTimeout(debounce);
    debounce = setTimeout(() => safe(syncSource), 650);
  });
  $('sync').onclick = () => safe(syncSource);
  $('target').onchange = () => navigate($('target').value === 'query' ? ['query'] : ['program', 'rules', Number($('target').value), 'body']);
  for (const control of $('rule-sides').querySelectorAll('[data-side]')) control.onclick = () => navigate(['program', 'rules', Number($('target').value), control.dataset.side]);
  $('rule-name-input').onchange = () => safe(() => edit({ type: 'rule-name', path: path.slice(0, 3), name: $('rule-name-input').value }));
  $('add-rule').onclick = () => safe(async () => { await edit({ type: 'add-rule' }); navigate(['program', 'rules', model.program.rules.length - 1, 'body']); });
  $('remove-rule').onclick = () => safe(async () => { await edit({ type: 'remove-rule', index: path[2] }); navigate(['query']); });
  $('add').onclick = () => safe(() => {
    const kind = $('add-kind').value;
    const node = kind === 'atom' ? { kind, atom: { relation: 'relation', args: ['X', 'Y'] } } : kind === 'equal' ? { kind, left: 'X', right: 'Y' } : kind === 'and' || kind === 'or' ? { kind, items: [{ kind: 'true' }] } : { kind };
    return edit({ type: 'append', path, node });
  });
  $('undo').onclick = () => safe(() => commit(clone(undo.at(-1)), 'undo'));
  $('redo').onclick = () => safe(() => commit(clone(redo.at(-1)), 'redo'));
  $('run').onclick = () => safe(async () => {
    check(!launching, 'A run is already starting.'); launching = true; renderRun();
    try {
    await syncSource(); await finishInspection(); inspected = null; outputMode = 'answers'; savedSelection = ''; savedView = null; answerPage = 0; answerNumber = null; resetResultPages();
    await session.start(model, $('history').checked); renderInspectionControls(); message('Running the submitted notebook.');
    } finally { launching = false; renderInspectionControls(); renderRun(); }
  });
  $('step').onclick = () => safe(async () => {
    check(!launching, 'A step is already in progress.'); launching = true; renderRun();
    try {
    await session.finishClose();
    if (session.run === null) { savedSelection = ''; savedView = null; answerPage = 0; answerNumber = null; await syncSource(); await session.start(model, $('history').checked, false); renderInspectionControls(); }
    const response = await session.step(inspectionSelection.payload(session.run).choices); await inspect();
    if (response?.step?.event !== null && response?.step?.rule !== undefined) {
      const name = session.submission.program.rules[response.step.rule]?.name ?? `Rule ${response.step.rule + 1}`;
      message(response.step.shared ? `${name} applied across the selection and other alternatives.` : `${name} applied.`);
    } else { message('No further rule applies in this selection.'); }
    } finally { launching = false; renderInspectionControls(); renderRun(); }
  });
  $('execution').onchange = () => safe(async () => {
    if (!$('execution').value) return;
    const run = Number($('execution').value);
    await finishInspection();
    await session.switchRun(run);
    inspected = null; outputMode = 'answers'; savedSelection = ''; savedView = null; answerPage = 0;
    answerNumber = null; resetResultPages(); renderResults();
    renderInspectionControls(); await refreshSaved();
  });
  $('release-run').onclick = () => safe(async () => {
    await finishInspection(); await session.closeRun(); renderInspectionControls(); message('Execution released. Saved answers remain available.');
  });
  $('pause').onclick = () => session.pause(); $('resume').onclick = () => safe(() => session.resume());
  $('cancel').onclick = () => safe(async () => { inspectionCanceled = true; await session.cancel(); if (!inspecting) await finishInspection(); }); $('inspect').onclick = () => safe(inspect);
  $('alternatives').onchange = () => { answerNumber = Number($('alternatives').value); resetResultPages(); renderResults(); };
  $('output-mode').onchange = () => safe(async () => { outputMode = $('output-mode').value; answerNumber = null; answerPage = 0; resetResultPages(); await refreshSaved(); });
  for (const [suffix, direction] of [['prev','prev'], ['next','next']]) $('collections-' + suffix).onclick = () => safe(async () => {
    catalogCursor = catalog[suffix]; catalogDirection = direction; catalogDirty = true;
    await refreshSaved();
  });
  $('saved-run').onchange = () => safe(async () => { savedSelection = $('saved-run').value; outputMode = 'answers'; answerNumber = null; answerPage = 0; resetResultPages(); await refreshSaved(); });
  const changeSavedPage = next => safe(async () => { answerPage = uint(next); answerNumber = null; resetResultPages(); await refreshSaved(); });
  $('saved-prev').onclick = () => changeSavedPage((savedView?.page ?? 0) - 1);
  $('saved-next').onclick = () => changeSavedPage((savedView?.page ?? 0) + 1);
  $('saved-page').onchange = () => changeSavedPage(Number($('saved-page').value) - 1);
  $('clear-answers').onclick = () => safe(async () => {
    const collection = savedView?.id; check(collection, 'Select saved answers first.');
    check(collection !== session.archive || session.run === null, 'Cancel the run before clearing its saved answers.');
    check(collection !== inspectionPending?.archive, 'Finish saving the inspection before clearing it.');
    if (!window.confirm('Permanently clear all saved answers in this selection?')) return;
    await store.clear(collection); catalogDirty = true; catalogCursor = null; catalogDirection = 'next';
    if (session.archive === collection) session.archive = null;
    if (inspected?.archive === collection) inspected = null;
    savedSelection = ''; savedView = null; answerNumber = null; answerPage = 0;
    await refreshSaved(); message('Saved answers cleared.');
  });
  renderWorkspace(); renderRun(); renderInspectionControls();
}
