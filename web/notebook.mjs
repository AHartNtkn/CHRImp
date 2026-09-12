import { clone, at, atomOf, applyEdit, validateNotebook, renderGraph, renderScene, diagramControl } from './graph.mjs';
import { OutputAssembler, IndexedAnswerStore } from './answers.mjs';
import { NotebookConnection } from './connection.mjs';

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
  constructor() { this.choices = []; this.snapshots = []; this.assignments = new Map(); this.snapshot = ''; this.selectedSnapshot = null; this.cursors = {}; this.navigation = {}; this.loading = false; this.lease = null; this.retiring = null; this.operation = null; this.resetting = false; this.pendingCapture = null; this.persist = async () => {}; }
  checkpoint() {
    return {choices:this.choices, snapshots:this.snapshots, assignments:[...this.assignments], snapshot:this.snapshot,
      selectedSnapshot:this.selectedSnapshot, cursors:this.cursors, navigation:this.navigation, lease:this.lease};
  }
  async restore(store, api, run, saved = null) {
    if (saved && (saved.snapshot || saved.lease?.run === run)) {
      const lease = saved.lease && await store.recovery(`live:snapshot:${run}:${saved.lease.snapshot}`);
      if (saved.snapshot || lease?.phase === 'active') {
        this.update(saved.choices, saved.snapshots, true); this.assignments = new Map(saved.assignments);
        this.snapshot = saved.snapshot; this.selectedSnapshot = saved.selectedSnapshot;
        this.cursors = saved.cursors; this.navigation = saved.navigation;
        this.lease = lease?.phase === 'active' ? saved.lease : null;
      }
    }
    let after = null;
    do {
      const page = await store.recoveryPage(`live:snapshot:${run}:`, after);
      for (const {value} of page.records) if (value.snapshot !== this.lease?.snapshot) {
        await liveRequest('snapshot_release', {run,snapshot:value.snapshot}, api);
      }
      after = page.next;
      if (after !== null) await new Promise(resolve => setTimeout(resolve, 0));
    } while (after !== null);
  }
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
  selectSnapshot(api, run, key, preserveChoices = false) {
    this.checkEditable?.();
    const descriptor = this.snapshots.find(item => item.id === key) ?? this.selectedSnapshot;
    check(key === '' || descriptor?.id === key, 'This snapshot is no longer available.');
    return this.page(api, run, null, 'refresh', {key, descriptor:key === '' ? null : descriptor, preserveChoices});
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
    await this.persist(); // Publish the replacement selection before releasing its predecessor.
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
      await this.persist();
    } finally { this.resetting = false; }
  }
  page(api, run, kind = null, direction = 'refresh', selection = null) {
    this.checkEditable?.();
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
          if (this.snapshot !== selection.key && !selection.preserveChoices) this.assignments.clear();
          this.snapshot = selection.key; this.selectedSnapshot = selection.descriptor;
        }
      } catch (error) {
        this.retiring = candidate;
        await this.releaseRetiring(api);
        throw error;
      }
      this.retiring = this.lease;
      this.lease = candidate;
      await this.persist();
      await this.releaseRetiring(api);
    } finally { this.loading = false; }
  }
  choose(key, value) {
    this.checkEditable?.();
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

async function liveRequest(route, payload, api) {
    for (;;) {
      try { return await api(route, payload); }
      catch (error) {
        if (!error.retry) throw error;
        await api('maintenance', { run: payload.run, budget: 2048 });
        await new Promise(resolve => setTimeout(resolve, 0));
      }
    }
  }

function attachBatch(pending, response) {
  check(Array.isArray(response.events), 'Output response needs events.');
  check(pending.sequence == null || pending.sequence === response.sequence, 'Retained output sequence changed.');
  check(Number.isSafeInteger(pending.index) && pending.index >= 0 && pending.index <= response.events.length, 'Retained output index is invalid.');
  pending.response = response;
}

// A cached response is bounded by the server's delivery budget. On cancel,
// preserve every complete alternative and retire only its unfinished suffix.
export async function deliverCachedOutput(store, archive, stream, pending, cancel = false, checkpoint = null) {
  const flush = async (discard = false, complete = false) => {
    if (!checkpoint) return discard ? store.discardPartial(archive, stream) : store.flush(archive, stream);
    return checkpoint.serialize(async () => {
      const previous = await store.recovery(checkpoint.id);
      check(previous, 'Output recovery ownership is missing.');
      const response = pending?.response;
      const value = {...previous, ...checkpoint.value(complete, cancel),
        ack:complete ? response?.sequence ?? previous.ack : previous.ack,
        status:response ? Object.fromEntries(['done','delivery_done','exhausted','applications','step','error','canceled'].filter(key => response[key] !== undefined).map(key => [key,response[key]])) : previous.status,
        sequence:complete ? null : response?.sequence ?? null,
        index:complete ? 0 : pending?.index ?? 0};
      if (previous.phase === 'closing' || (previous.phase === 'canceling' && !cancel)) value.phase = previous.phase;
      if (cancel) value.status = {...value.status,canceled:true};
      if (complete && (cancel || response?.step?.done)) { value.stepPending = false; value.stepChoices = null; }
      await store.flush(archive, stream, {discard, recovery:{id:checkpoint.id,value}});
    });
  };
  const events = pending?.response?.events ?? [];
  let limit = events.length;
  if (cancel) {
    limit = pending?.index ?? 0;
    for (let index = events.length - 1; index >= limit; index--) {
      if (events[index].kind === 'end') { limit = index + 1; break; }
    }
  }
  if (!cancel || (pending && pending.index < limit)) await flush();
  while (pending && pending.index < limit) {
    stream.push(events[pending.index]);
    pending.index++;
    if (stream.needsFlush) await flush();
  }
  if (cancel) {
    await flush(true, true);
    if (pending) pending.index = events.length;
  } else await flush(false, true);
}

export class RunSession {
  constructor(api, notify = () => {}, store = new IndexedAnswerStore()) {
    this.api = api; this.notify = notify; this.store = store; this.archive = null; this.pendingDelivery = null; this.run = null; this.status = 'idle';
    this.running = false; this.inFlight = null; this.timer = null; this.stream = null;
    this.applications = 0; this.error = null; this.starting = false; this.runs = new Map(); this.canceling = false; this.ack = null; this.selection = new InspectionSelection(); this.changingRun = false; this.pendingClose = null;
    this.recoveredControl = null; this.recovering = null;
    this.stepPending = false; this.stepOperation = null; this.stepDriving = null;
    this.selection.checkEditable = () => check(!this.stepOperation, 'Finish or cancel the selected step before changing metadata.');
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
      if (auto) await this.resume();
    } catch (error) { this.fail(error); throw error; }
    finally { this.starting = false; this.notify(); }
  }
  adoptStart(payload, response) {
    check(typeof response.archive === 'string', 'Start response needs its registered archive.');
    const run = uint(response.run), stream = new OutputAssembler(response);
    const submission = clone(validateNotebook({program:payload.program, query:payload.query}));
    this.run = run; this.stream = stream; this.submission = submission;
    this.ack = null; this.archive = response.archive; this.pendingDelivery = null;
    this.recordHistory = payload.record_history === true;
    this.applications = 0; this.exhausted = false; this.status = 'paused'; this.stepPending = false; this.stepOperation = null;
  }
  async restore(preferred = null) {
    let recoveryError;
    try { await this.api.recover?.(); } catch (error) { recoveryError = error; }
    let after = null;
    do {
      const page = await this.store.recoveryPage('live:run:', after);
      for (const {value} of page.records) {
        if (value.phase === 'closing') { await this.api('close', {run:value.run}); continue; }
        this.runs.set(value.run, {run:value.run, archive:value.archive,
          status:['done','canceled'].includes(value.phase) ? value.phase : 'paused', applications:value.applications ?? 0});
      }
      after = page.next;
      if (after !== null) await new Promise(resolve => setTimeout(resolve, 0));
    } while (after !== null);
    const run = this.runs.has(preferred) ? preferred : this.runs.keys().next().value;
    if (run !== undefined) await this.loadRun(run);
    this.notify();
    if (recoveryError) throw recoveryError;
  }
  async loadRun(run) {
    const saved = await this.store.recovery(`live:run:${run}`);
    check(saved, 'Unknown saved execution.');
    if (saved.phase === 'closing') { this.pendingClose = run; await this.finishClose(); return; }
    const source = await this.store.recovery(`live:source:${run}`);
    const tables = await this.store.tables(saved.archive);
    check(source && tables, 'Execution source or archive metadata is missing.');
    this.run = run; this.archive = saved.archive;
    this.stream = saved.assembler ? OutputAssembler.restore(tables, saved.assembler) : new OutputAssembler(tables);
    this.submission = source.submission;
    this.recordHistory = source.recordHistory === true;
    this.ack = saved.ack ?? null; this.applications = saved.applications ?? 0; this.exhausted = saved.exhausted ?? false;
    this.pendingDelivery = saved.sequence != null ? {response:null,sequence:saved.sequence,index:saved.index ?? 0} : null;
    this.stepPending = saved.stepPending === true;
    this.stepOperation = saved.stepChoices ? {run, choices:clone(saved.stepChoices)} : null;
    this.pendingClose = null;
    this.status = ['done','canceled'].includes(saved.phase) ? saved.phase : 'paused';
    this.running = false; this.error = null;
    if (saved.phase !== 'canceled') {
      const live = await this.api('status', {run});
      check(typeof live.canceled === 'boolean', 'Execution status needs cancellation state.');
      if (live.canceled || saved.phase === 'canceling') {
        this.status = 'canceled'; this.stepPending = false; this.stepOperation = null;
        await this.cancel();
      }
    }
  }
  settleControl(options = {}) {
    if (options.cancel && this.recovering) {
      return this.recovering.catch(() => {}).then(() => this.settleControl(options));
    }
    this.recovering ??= this.recoverControl(options).finally(() => { this.recovering = null; });
    return this.recovering;
  }
  async recoverControl(options) {
    // Let the metadata consumer adopt a successful response before recovering
    // its interrupted capture. It shares the same single receipt authority.
    if (this.selection.operation) await this.selection.operation.catch(() => {});
    for (;;) {
      const held = this.recoveredControl;
      this.recoveredControl ??= await this.api.recover?.(options) ?? null;
      const recovered = this.recoveredControl;
      if (!recovered) return;
      const {route, payload, response} = recovered;
      if (held && options.cancel) await this.api('cancel', {run:route === 'start' ? response.run : payload.run});
      if (route === 'start') {
        if (!recovered.adopted) {
          this.adoptStart(payload, response);
          recovered.adopted = true;
        }
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
    check(typeof this.archive === 'string', 'Output archive ownership is missing.');
    const pending = this.pendingDelivery;
    if (pending && !pending.response) {
      const response = await this.api('advance', {run:this.run,budget:2048,ack:this.ack});
      attachBatch(pending, response);
    }
    cancel ||= pending?.response.canceled === true;
    await deliverCachedOutput(this.store, this.archive, this.stream, pending, cancel, {
      id:`live:run:${this.run}`, serialize:action => this.api.checkpoint(action),
      value:complete => ({run:this.run, archive:this.archive,
        ...(cancel || pending ? {phase:cancel ? (complete ? 'canceled' : 'canceling') : pending.response.delivery_done && complete ? 'done' : 'paused'} : {}),
        applications:pending?.response.applications ?? this.applications, exhausted:pending?.response.exhausted ?? this.exhausted}),
    });
    if (cancel) { this.running = false; this.status = 'canceled'; this.stepPending = false; this.stepOperation = null; }
    if (!pending) return;
    const response = pending.response;
    if (response.delivery_done) this.stream.finish();
    this.ack = response.sequence ?? this.ack;
    if (cancel || response.step?.done) this.stepPending = false;
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
    if (this.stepDriving) return this.stepDriving;
    if (this.stepOperation || this.stepPending) return this.step();
    await this.finishClose();
    check(this.run !== null && this.stream, 'Start a run first.');
    if (['done', 'canceled'].includes(this.status)) return;
    const run = this.run;
    this.error = null; this.running = true; this.status = 'running'; this.notify();
    try {
      await liveRequest('resume', { run }, this.api);
      this.stepPending = false;
      if (this.run === run && this.running) this.schedule();
    } catch (error) { if (this.run === run) this.fail(error); throw error; }
  }

  schedule() {
    if (!this.running || this.stepDriving || this.stepOperation || this.inFlight || this.timer !== null) return;
    this.timer = setTimeout(() => { this.timer = null; this.advance().catch(() => {}); }, 16);
  }
  advance() {
    check(this.pendingClose === null, 'Finish closing the execution first.');
    if (this.inFlight) return this.inFlight;
    check(this.run !== null && this.stream, 'Start a run first.');
    const pending = this.pendingDelivery?.response ? Promise.resolve(this.pendingDelivery.response) : this.api('advance', { run: this.run, budget: 2048, ack: this.ack });
    this.inFlight = (async () => {
      try {
        const response = await pending;
        check(Array.isArray(response.events), 'Advance response needs output events.');
        if (this.pendingDelivery) attachBatch(this.pendingDelivery,response);
        else this.pendingDelivery = { response, index: 0 };
        await this.deliverPending();
        this.applications = uint(response.applications);
        this.exhausted = response.exhausted;
        if (response.canceled) { this.running = false; this.status = 'canceled'; }
        else if (response.delivery_done) { this.stream.finish(); this.running = false; this.status = 'done'; }
        else this.status = this.running ? (this.stepOperation ? 'stepping' : 'running') : 'paused';
        this.notify(); return response;
      } catch (error) { this.fail(error); throw error; }
      finally { this.inFlight = null; this.schedule(); this.notify(); }
    })();
    return this.inFlight;
  }
  async step(choices = {}) {
    if (this.pendingClose !== null) await this.finishClose();
    if (this.stepDriving) return this.stepDriving;
    check(!this.selection.loading && !this.selection.resetting, 'Wait for metadata selection before stepping.');
    check(this.run !== null, 'Start a run first.');
    if (['done', 'canceled'].includes(this.status)) return;
    this.pause();
    const operation = this.stepOperation ??= {run:this.run, choices:clone(choices)};
    this.running = true; this.status = 'stepping'; this.error = null;
    this.stepDriving = Promise.resolve().then(async () => {
      try {
        await this.finishClose();
        if (this.inFlight) await this.inFlight;
        if (this.status === 'done') { this.stepOperation = null; return; }
        if (!this.running || this.canceling || this.run !== operation.run) return;
        if (!this.stepPending) {
          // Persist the admitted selection before sending its exact control.
          await this.api.checkpoint(async () => {
            const key = `live:run:${operation.run}`, saved = await this.store.recovery(key);
            check(saved, 'Step recovery ownership is missing.');
            await this.store.saveRecovery(key, {...saved,stepChoices:operation.choices});
          });
          if (!this.running || this.canceling) return;
          await liveRequest('step', {run:operation.run, choices:operation.choices}, this.api);
          this.stepPending = true;
        }
        while (this.running && !this.canceling && this.run === operation.run) {
          const response = await this.advance();
          if (this.canceling || this.status === 'canceled' || this.run !== operation.run) return;
          if (response.step?.done || response.delivery_done) {
            this.stepOperation = null; this.pause();
            return {...response, stepChoices:operation.choices};
          }
          if (this.running) await new Promise(resolve => setTimeout(resolve, 0));
        }
      } catch (error) { this.fail(error); throw error; }
      finally { this.stepDriving = null; this.notify(); }
    });
    this.notify();
    return this.stepDriving;
  }
  rememberRun() {
    if (this.run !== null) this.runs.set(this.run, { run: this.run, archive: this.archive, recordHistory: this.recordHistory, applications: this.applications, status: this.status });
  }
  async switchRun(run) {
    check(!this.changingRun && !this.starting, 'An execution change is already in progress.');
    this.changingRun = true; this.notify();
    try {
      await this.finishClose();
      this.pause(); if (this.stepDriving) await this.stepDriving; if (this.inFlight) await this.inFlight;
      await this.settleControl(); await this.deliverPending(); this.rememberRun();
      check(this.runs.has(run), 'Unknown execution.');
      await this.selection.reset(this.api);
      await this.loadRun(run);
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
      if (this.stepDriving) await this.stepDriving.catch(() => {});
      if (this.inFlight) await this.inFlight.catch(() => {});
      await this.settleControl({cancel:true});
      if (this.run !== null) {
        const canceled = await this.api('cancel', { run: this.run });
        if (canceled.pending && canceled.pending.sequence !== this.ack) {
          if (this.pendingDelivery) attachBatch(this.pendingDelivery,canceled.pending);
          else this.pendingDelivery = {response:canceled.pending,index:0};
        }
        check(!this.pendingDelivery || this.pendingDelivery.response, 'Canceled output batch is missing.');
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
  let path = ['query'], selected = null, dirty = false, busy = false, revision = 0;
  let undo = [], redo = [], debounce, inspected = null, outputMode = 'answers', answerNumber = null;
  const store = new IndexedAnswerStore();
  const connection = new NotebookConnection(store), request = connection.request;
  let connected = false, restoring = true, editorWriting = null, editorDirty = false;
  let displayWriting = null, displayDirty = false, displayStamp = null;
  let savedView = null, savedSelection = '', answerPage = 0, inspectionPending = null, inspecting = false, launching = false;
  let bindingPage = 0;
  let pendingNumber = 0;
  let catalog = {records:[], next:null, prev:null}, catalogCursor = null, catalogDirection = 'next', catalogDirty = true, catalogStamp = '';
  let refreshing = null, refreshAgain = false, sceneLoading = false, desiredScene = null, loadedSceneKey = null;
  let inspectionCanceled = false, runNotice = null;
  const session = new RunSession(request, renderRun, store);
  const inspectionSelection = session.selection;
  inspectionSelection.persist = saveEditor;
  function saveEditor() {
    if (!connected || restoring) return Promise.resolve();
    editorDirty = true;
    editorWriting ??= Promise.resolve().then(async () => {
      try {
        while (editorDirty) {
          editorDirty = false;
          await store.saveRecovery('editor', {model,program:$('program').value,query:$('query').value,dirty,
            history:$('history').checked,run:session.run,boot:connection.state.boot,selection:inspectionSelection.checkpoint()});
        }
      } finally { editorWriting = null; }
    });
    return editorWriting;
  }
  function displayState() {
    return {mode:outputMode,inspectionArchive:inspected?.archive ?? null,savedArchive:savedSelection,sourceArchive:session.archive,
      answerNumber,answerPage,bindingPage,pendingNumber};
  }
  function saveDisplay() {
    if (!connected || restoring) return Promise.resolve();
    if (JSON.stringify(displayState()) === displayStamp && !displayWriting) return Promise.resolve();
    displayDirty = true;
    displayWriting ??= Promise.resolve().then(async () => {
      try {
        while (displayDirty) {
          displayDirty = false;
          const value = displayState(), stamp = JSON.stringify(value);
          if (stamp === displayStamp) continue;
          await store.saveRecovery('display',value);
          displayStamp = stamp;
        }
      } finally { displayWriting = null; }
    });
    return displayWriting;
  }
  function restoreDisplay(saved) {
    if (!saved) return;
    savedView = null; desiredScene = null; loadedSceneKey = null;
    outputMode = saved.mode;
    inspected = saved.inspectionArchive ? {archive:saved.inspectionArchive} : null;
    savedSelection = saved.savedArchive || (saved.sourceArchive !== session.archive ? saved.sourceArchive : '') || '';
    answerNumber = saved.answerNumber; answerPage = saved.answerPage;
    bindingPage = saved.bindingPage;
    pendingNumber = saved.pendingNumber;
  }
  async function initialize() {
    const editor = await store.recovery('editor'), display = await store.recovery('display');
    if (editor) {
      model = clone(validateNotebook(editor.model)); $('program').value = editor.program; $('query').value = editor.query;
      dirty = editor.dirty; $('history').checked = editor.history;
    }
    try {
      await connection.initialize();
      connected = true;
      let recoveryError;
      try { await session.restore(editor?.boot === connection.state.boot ? editor.run : null); }
      catch (error) { recoveryError = error; }
      if (session.run !== null) {
        await inspectionSelection.restore(store,request,session.run,
          editor?.boot === connection.state.boot && editor.run === session.run ? editor.selection : null);
        await restoreInspection();
      }
      if (recoveryError) throw recoveryError;
    } finally {
      restoreDisplay(display);
      restoring = false; renderWorkspace(); renderRun(); renderInspectionControls(); await refreshSaved();
    }
    await saveEditor();
    const inspectionNotice = display?.mode === 'inspect' && display.inspectionArchive
      ? outputMode === 'inspect' && savedView?.id === inspected?.archive
        ? savedView.total > 0 ? ' Showing saved inspection.' : ' Saved inspection has no completed answers.'
        : ' Saved inspection is unavailable. Choose a saved archive.'
      : '';
    if (session.run !== null) message(`Run ${session.run} restored ${session.status} at ${session.applications} applications.${inspectionNotice}${session.stepPending ? ' Step continues the unfinished application.' : ''}`);
    else if (editor || display) message(`Notebook restored.${inspectionNotice}`);
  }
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
    renderWorkspace(); await saveEditor(); message('Text and graph are in sync.');
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
    await saveEditor();
  }
  const edit = op => commit(applyEdit(model, op));
  const select = value => { selected = value; renderWorkspace(); };
  function navigate(next) { path = next; selected = null; renderWorkspace(); }
  function renderWorkspace() {
    const disabled = !connected || restoring || dirty || busy;
    for (const name of ['program','query','sync','history']) $(name).disabled = !connected || restoring;
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
    renderGraph($('editor-graph'), model, path, {
      selected, readonly: disabled, label: 'Editable query or whole rule diagram',
      onSelect: select,
      onConnect: variable => safe(() => { check(selected?.port !== undefined, 'Select a numbered relation port first.'); return edit({ type: 'set-port', path: selected.path, index: selected.port, variable }); }),
      onWire: (port, variable) => safe(() => edit({type:'set-port',path:port.path,index:port.port,variable})),
    });
    renderInspector();
  }
  function renderInspector() {
    $('selection-panel').hidden = !selected;
    const panel = $('selection'); panel.replaceChildren();
    if (!selected) { panel.append(el('p', 'Select a relation, group or numbered port.')); return; }
    let node;
    try { node = at(model, selected.path); } catch { selected = null; return; }
    if (node.kept) { panel.append(el('p','Select a head, body, relation or port to edit this rule.')); return; }
    const atom = atomOf(node), target = selected.path;
    panel.append(el('h3', atom ? 'Relation & ordered ports' : 'Expression'));
    if (atom) {
      panel.append(field('Relation', atom.relation, relation => edit({ type: 'rename-relation', path: target, relation })));
      const start = Math.floor((selected.port ?? 0) / 8) * 8;
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
      if (atom.args.length > 8) panel.append(button('Previous ports', () => select({path:target,port:start-8}),start===0),button('Next ports', () => select({path:target,port:start+8}),start+8>=atom.args.length));
      panel.append(button('Add port', () => edit({ type: 'insert-port', path: target, variable: 'X' })));
      panel.append(el('p', selected.port === undefined ? 'Select a port, then a wire or junction to connect it.' : `Port ${selected.port + 1} selected. Choose a variable junction.`));
    } else if (node.kind === 'equal') {
      panel.append(field('Left variable', node.left, left => edit({ type: 'equal', path: target, left, right: node.right })), field('Right variable', node.right, right => edit({ type: 'equal', path: target, left: node.left, right })));
    } else if (node.items) {
      panel.append(button('Add here', () => navigate(target)), button(node.kind === 'and' ? 'Change to Or' : 'Change to And', () => edit({ type: 'replace', path: target, node: { ...node, kind: node.kind === 'and' ? 'or' : 'and' } })));
    } else if (Array.isArray(node)) { panel.append(button('Add here', () => navigate(target))); return;
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
    $('execution').replaceChildren(el('option', 'Current notebook', {value: '', disabled: session.run !== null}), ...runIds.map(run => el('option', `Run ${run}`, {value: run})));
    $('execution').value = session.run ?? '';
    $('execution').disabled = inspecting || launching || session.starting;
    $('release-run').disabled = session.run === null || inspecting || launching || session.starting;

    $('run').disabled = session.starting || busy || launching || inspecting;
    $('pause').disabled = !session.running;
    $('resume').disabled = session.run === null || session.running || !!session.stepDriving || ['done', 'canceled'].includes(session.status) || session.starting || launching || inspecting;
    $('resume').textContent = session.stepOperation || session.stepPending ? 'Resume step' : 'Resume';
    $('step').disabled = session.status === 'canceled' || session.starting || !!session.inFlight || busy || launching || inspecting || inspectionSelection.loading;
    $('cancel').disabled = (session.run === null && session.status !== 'error' && !connection.state?.pending) || session.starting;
    $('inspect').disabled = (session.run === null && !inspectionPending) || session.starting || inspecting || launching || !!session.stepOperation;
    $('snapshot').disabled = !connected || restoring || !session.recordHistory || launching || !!session.stepOperation || inspecting || inspectionSelection.loading || session.starting || session.changingRun;
    if (session.error) message(session.error.message, true);
    else if (connected && !restoring && runNotice !== `${session.run}:${session.status}`) {
      runNotice = `${session.run}:${session.status}`;
      if (session.status === 'done') message(`Run ${session.run} completed. Answers are saved.`);
      else if (session.status === 'canceled') message(`Run ${session.run} canceled. Completed answers are saved.`);
    }
    if (!connected || restoring) for (const name of ['execution','release-run','run','pause','resume','step','cancel','inspect','snapshot']) $(name).disabled = true;
    renderResults(); safe(refreshSaved);
  }
  function refreshSaved() {
    refreshAgain = true;
    if (refreshing) return refreshing;
    refreshing = Promise.resolve().then(async () => {
      try {
        while (refreshAgain) {
          refreshAgain = false;
          const terminal = ['done','canceled'].includes(session.status) ? `${session.status}:${session.ack}` : '';
          const stamp = `${session.archive}|${inspected?.archive}|${terminal}`;
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
          // An archive locator is not a loaded view. Reconcile only a definitive
          // missing result; storage errors above retain the selection for retry.
          if (collection && !view && (outputMode === 'inspect' || savedSelection)) {
            if (outputMode === 'inspect') { inspected = null; outputMode = 'answers'; }
            else savedSelection = '';
            answerNumber = null; answerPage = 0; resetResultPages();
            refreshAgain = true; continue;
          }
          const records = catalog.records.map(record => record.id === view?.id ? {...record, total:view.total} : record);
          if (savedSelection && view && !records.some(record => record.id === savedSelection)) records.unshift(view);
          $('saved-run').replaceChildren(el('option', 'Current run', { value: '' }), ...records.map(record => el('option', `${record.label} · ${record.total} answers · ${new Date(record.created).toLocaleString()}`, { value: record.id })));
          $('saved-run').value = savedSelection;
          $('collections-prev').disabled = !catalog.prev;
          $('collections-next').disabled = !catalog.next;
          if (savedView && savedView.id !== view?.id) resetResultPages();
          savedView = view; answerPage = view?.page ?? 0;
          $('saved-page').value = answerPage + 1;
          $('saved-page').max = view?.pages ?? 1;
          $('saved-pages').textContent = `/ ${view?.pages ?? 1}`;
          $('saved-prev').disabled = answerPage === 0;
          $('saved-next').disabled = !view || answerPage + 1 >= view.pages;
          $('clear-answers').disabled = !view || (view.id === session.archive && session.run !== null) || view.id === inspectionPending?.archive;
          renderResults();
          await saveDisplay();
        }
      } finally { refreshing = null; }
    });
    return refreshing;
  }
  function renderResults() {
    const collection = outputMode === 'inspect' ? inspected?.archive : savedSelection || session.archive;
    const stream = savedView?.id === collection && savedView.page === answerPage ? savedView : null;
    $('output-mode').value = outputMode;
    const answers = stream?.answers ?? [];
    let answer = answers.find(answer => answer.number === answerNumber) ?? answers.at(-1);
    // A loading render has no authority to replace the restored answer locator.
    if (stream) answerNumber = answer?.number ?? null;
    if (answer) $('observations').open = true;
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
    const options = {bindingPage, pendingNumber};
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
        renderScene($('result-graph'), scene.facts, {readonly:true,key:`${request.collection}:${request.number}`, label:outputMode === 'inspect' ? 'Inspected graph' : 'Answer hypergraph'});
        renderPending(scene.pending);
        loadedSceneKey = request.key;
        await saveDisplay();
      }
    } finally { sceneLoading = false; }
  }
  function renderPending(body) {
    $('pending-bodies').hidden = !body;
    if (!body) return;
    pendingNumber = body.index;
    $('pending-body-number').value = pendingNumber + 1; $('pending-body-number').max = body.count;
    $('pending-body-count').textContent = `/ ${body.count} · event ${body.event}`;
    const choose = number => { pendingNumber = Math.min(uint(number), body.count - 1); renderResults(); };
    $('pending-body-prev').disabled = pendingNumber === 0; $('pending-body-next').disabled = pendingNumber + 1 === body.count;
    $('pending-body-prev').onclick = () => choose(body.index - 1); $('pending-body-next').onclick = () => choose(body.index + 1);
    $('pending-body-number').onchange = () => safe(() => choose(Number($('pending-body-number').value) - 1));
    renderScene($('pending-graph'), body.scene, {readonly:true,key:`${desiredScene.collection}:${desiredScene.number}:${body.index}`,label:'Pending body graph'});
  }
  function resetResultPages() { bindingPage = pendingNumber = 0; desiredScene = null; loadedSceneKey = null; }
  async function inspect() {
    check(!session.stepOperation, 'Finish or cancel the selected step before inspecting.');
    check(!inspecting, 'An inspection is already in progress.');
    inspecting = true; renderRun(); renderInspectionControls();
    try { await inspectOnce(); } finally { inspecting = false; renderRun(); renderInspectionControls(); }
  }
  async function inspectOnce() {
    await restoreInspection();
    if (!inspectionPending) {
      await session.finishClose();
      check(session.run !== null, 'Start a run first.');
      session.pause(); if (session.inFlight) await session.inFlight;
      const run = session.run, tables = session.stream.tables;
      const response = await liveRequest('inspect', inspectionSelection.payload(run), request);
      check(session.run === run, 'The run changed during inspection.');
      check(typeof response.archive === 'string', 'Inspection response needs its registered archive.');
      inspectionCanceled = false;
      inspectionPending = { stream: new OutputAssembler(tables), response: null, index: 0, archive: response.archive, run, inspection: response.inspection, ack: null };
      await inspectionSelection.page(request, run); renderInspectionControls();
    }
    const pending = inspectionPending;
    const key = `live:inspection:${pending.run}:${pending.inspection}`;
    check(typeof pending.archive === 'string', 'Inspection archive ownership is missing.');
    const checkpoint = {id:key, serialize:action => request.checkpoint(action),
      value:() => ({run:pending.run,inspection:pending.inspection,archive:pending.archive,
        ...(inspectionCanceled ? {phase:'canceling'} : {}),failure:pending.failure ?? null})};
    while (!pending.cleanup) {
      if (inspectionCanceled) {
        let response;
        do {
          response = await liveRequest('inspect_cancel', { run: pending.run, inspection: pending.inspection, budget: 2048 }, request);
          if (response.pending && response.pending.sequence !== pending.ack
              && response.pending.sequence !== pending.response?.sequence) {
            if (pending.sequence != null) attachBatch(pending,response.pending);
            else { pending.response = response.pending; pending.index = 0; }
          }
          check(pending.sequence == null || pending.response, 'Canceled inspection output batch is missing.');
          await deliverCachedOutput(store, pending.archive, pending.stream, pending, true, checkpoint);
          pending.ack = pending.response?.sequence ?? pending.ack;
        } while (!response.done);
        break;
      }
      if (!pending.response) attachBatch(pending, await request('inspect_advance', { run: pending.run, inspection: pending.inspection, budget: 2048, ack: pending.ack }));
      if (pending.response.canceled) { inspectionCanceled = true; continue; }
      await deliverCachedOutput(store, pending.archive, pending.stream, pending, false, checkpoint);
      if (pending.response.error) {
        const failure = pending.response.error;
        let discarded;
        do { discarded = await liveRequest('inspect_cancel', {run: pending.run, inspection: pending.inspection, budget: 2048}, request); } while (!discarded.done);
        pending.failure = failure;
        break;
      }
      pending.ack = pending.response.sequence;
      if (pending.response.done) { pending.stream.finish(); break; }
      pending.response = null; pending.sequence = null; pending.index = 0;
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    if (pending.cleanup !== 'release') {
      pending.cleanup = 'persist';
      await checkpoint.serialize(async () => {
        const previous = await store.recovery(key);
        await store.discardPartial(pending.archive, pending.stream, {id:key,value:{...previous,
          phase:'release',cleanup:'release',failure:pending.failure ?? null,index:0,sequence:null,ack:pending.ack}});
      });
      pending.cleanup = 'release';
    }
    if (!pending.failure && !pending.displaySaved) {
      inspected = {archive:pending.archive}; outputMode = 'inspect'; answerNumber = null; answerPage = 0; resetResultPages();
      await refreshSaved(); await saveDisplay(); pending.displaySaved = true;
    }
    await liveRequest('inspect_release', { run: pending.run, inspection: pending.inspection }, request);
    inspectionPending = null;
    if (pending.failure) throw new Error(`Inspection failed: ${pending.failure}`);
    message(inspectionCanceled ? 'Inspection stopped; completed graphs are saved.' : 'Inspection saved.');
  }

  async function finishInspection() {
    check(!inspecting, 'Wait for the active inspection before changing executions.');
    inspectionCanceled = true;
    do {
      if (inspectionPending) { inspectionCanceled = true; await inspect(); }
      await restoreInspection();
    } while (inspectionPending);
  }
  async function restoreInspection() {
    if (inspectionPending || session.run === null) return;
    for (;;) {
      const page = await store.recoveryPage(`live:inspection:${session.run}:`, null, 1);
      const saved = page.records[0]?.value;
      if (!saved) return;
      if (saved.phase === 'release') {
        await liveRequest('inspect_release', {run:saved.run,inspection:saved.inspection}, request);
        continue;
      }
      const tables = await store.tables(saved.archive);
      inspectionPending = {...saved, stream:saved.assembler ? OutputAssembler.restore(tables,saved.assembler) : new OutputAssembler(tables),
        response:saved.sequence == null && (saved.status?.done || saved.status?.error) ? {...saved.status,sequence:saved.ack,events:[]} : null,
        sequence:saved.sequence ?? null,index:saved.index ?? 0,ack:saved.ack ?? null};
      inspectionCanceled = saved.phase === 'canceling';
      return;
    }
  }

  function renderInspectionControls() {
    const unavailable = !connected || restoring || launching || !!session.stepOperation || inspecting || session.starting || session.changingRun || inspectionSelection.resetting;
    $('snapshot').disabled = !connected || restoring || !session.recordHistory || launching || !!session.stepOperation || inspecting || inspectionSelection.loading || session.starting || session.changingRun;
    $('choices').replaceChildren();
    if (!inspectionSelection.choices.length) $('choices').append(el('p', session.run === null ? 'Start a run to inspect its choices.' : inspectionSelection.navigation.next_choice === undefined ? 'Inspect the graph to load available choices.' : 'No choices in this state.'));
    inspectionSelection.choices.forEach(item => {
      const label = el('label', item.label), select = el('select');
      select.append(el('option', 'Either', { value: 'either' }), el('option', 'First', { value: 'first' }), el('option', 'Second', { value: 'second' }));
      select.disabled = unavailable;
      select.value = inspectionSelection.assignments.get(item.id) ?? 'either';
      select.onchange = () => safe(async () => { check(!launching && !session.stepOperation && !inspecting, 'Finish or cancel the selected step before changing metadata.'); inspectionSelection.choose(item.id, select.value); await saveEditor(); });
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
    check(!launching && !session.stepOperation && !inspecting, 'Finish or cancel the selected step before changing metadata.');
    check(!session.starting && !session.changingRun, 'Wait for the execution change.');
    const pending = inspectionSelection.page(request, session.run, kind, direction);
    renderInspectionControls();
    try { await pending; } finally { renderInspectionControls(); }
  }
  for (const kind of ['choice', 'snapshot']) for (const direction of ['prev', 'next']) {
    $(`${kind}-${direction}`).onclick = () => safe(() => metadataPage(kind, direction));
  }
  $('snapshot').onchange = () => safe(async () => {
    check(!launching && !session.stepOperation && !inspecting, 'Finish or cancel the selected step before changing metadata.');
    check(!session.starting && !session.changingRun, 'Wait for the execution change.');
    const pending = inspectionSelection.selectSnapshot(request, session.run, $('snapshot').value);
    renderInspectionControls();
    try { await pending; } finally { renderInspectionControls(); }
  });
  for (const name of ['program', 'query']) $(name).addEventListener('input', () => {
    revision++; dirty = true; renderWorkspace(); safe(saveEditor); clearTimeout(debounce);
    debounce = setTimeout(() => safe(syncSource), 650);
  });
  for (const control of document.querySelectorAll('[data-diagram]')) control.onclick = () => diagramControl($(control.dataset.diagram),control.dataset.action);
  $('close-selection').onclick = () => select(null);
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
    } finally { launching = false; renderInspectionControls(); renderRun(); await saveEditor(); }
  });
  async function runStep(resume = false) {
    check(!launching && !inspectionSelection.loading, 'Wait for the current step or metadata selection.'); launching = true; renderRun(); renderInspectionControls();
    try {
    if (!session.stepOperation && !session.stepPending) await finishInspection();
    await session.finishClose();
    if (session.run === null) { savedSelection = ''; savedView = null; answerPage = 0; answerNumber = null; await syncSource(); await session.start(model, $('history').checked, false); renderInspectionControls(); }
    const response = resume ? await session.resume() : await session.step(inspectionSelection.payload(session.run).choices);
    if (!response || session.status === 'canceled') return;
    if (response.stepChoices) inspectionSelection.assignments = new Map(Object.entries(response.stepChoices).map(([key,value]) => [key,value ? 'first' : 'second']));
    const run = session.run;
    if (inspectionSelection.snapshot) await inspectionSelection.selectSnapshot(request, run, '', true);
    if (session.run !== run || session.canceling || session.status === 'canceled') return;
    await inspect();
    if (response?.step?.event !== null && response?.step?.rule !== undefined) {
      const name = session.submission.program.rules[response.step.rule]?.name ?? `Rule ${response.step.rule + 1}`;
      message(response.step.shared ? `${name} applied across the selection and other alternatives.` : `${name} applied.`);
    } else { message('No further rule applies in this selection.'); }
    } finally { launching = false; renderInspectionControls(); renderRun(); await saveEditor(); }
  }
  $('step').onclick = () => safe(() => runStep());
  $('execution').onchange = () => safe(async () => {
    if (!$('execution').value) return;
    const run = Number($('execution').value);
    await finishInspection();
    await session.switchRun(run);
    inspected = null; outputMode = 'answers'; savedSelection = ''; savedView = null; answerPage = 0;
    answerNumber = null; resetResultPages(); renderResults();
    await restoreInspection(); renderInspectionControls(); await refreshSaved(); await saveEditor();
  });
  $('release-run').onclick = () => safe(async () => {
    await finishInspection(); await session.closeRun(); await saveEditor(); renderInspectionControls(); message('Execution released. Saved answers remain available.');
  });
  $('pause').onclick = () => session.pause(); $('resume').onclick = () => safe(() => session.stepOperation || session.stepPending ? runStep(true) : session.resume());
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
    await refreshSaved(); await saveDisplay(); message('Saved answers cleared.');
  });
  $('history').onchange = () => safe(saveEditor);
  renderWorkspace(); renderRun(); renderInspectionControls();
  safe(initialize);
}
