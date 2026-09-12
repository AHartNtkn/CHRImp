// Durable ownership and exact command replay for one local notebook.
import {OutputAssembler} from './answers.mjs';
const check = (ok, message) => { if (!ok) throw new Error(message); };
const positive = value => { check(Number.isSafeInteger(value) && value > 0, 'Invalid notebook identifier.'); return value; };
const decimal = value => { check(typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value) && BigInt(value) <= 18446744073709551615n, 'Invalid view identifier.'); return value; };
const controls = new Set(['start', 'inspect', 'step', 'resume', 'snapshot']);
const pause = () => new Promise(resolve => setTimeout(resolve, 0));

export class NotebookConnection {
  constructor(store, fetcher = (...args) => fetch(...args), locks = globalThis.navigator?.locks) {
    this.store = store; this.fetcher = fetcher; this.locks = locks;
    this.state = null; this.pendingDurable = false; this.durableOwner = null; this.initializing = null; this.attaching = null;
    this.tail = Promise.resolve(); this.locked = false; this.closed = false;
    this.request = this.request.bind(this);
    this.request.recover = options => this.serialize(() => this.recover(options));
    this.request.checkpoint = action => this.serialize(async () => { await this.initialize(); return action(); });
  }
  async initialize() {
    check(!this.closing && !this.closed, 'This notebook controller is closed.');
    this.initializing ??= this.open().catch(error => { this.initializing = null; throw error; });
    return this.initializing;
  }
  async open() {
    if (!this.locked) {
      check(this.locks?.request, 'This browser cannot coordinate notebook recovery.');
      await new Promise((resolve, reject) => {
        this.lockTask = this.locks.request('chr-notebook-controller', {ifAvailable:true}, async lock => {
          if (!lock) { reject(new Error('This notebook is controlled by another tab. Close that tab, then reload here.')); return; }
          this.locked = true;
          await new Promise(release => { this.releaseLock = release; resolve(); });
        }).catch(reject);
      });
    }
    const saved = await this.store.recovery('controller');
    if (saved) this.validateState(saved);
    const {boot} = await this.send('hello', '{}');
    check(typeof boot === 'string' && /^[0-9a-f]{32}$/.test(boot), 'Invalid server identity.');
    const restarted = saved !== null && saved.boot !== boot;
    if (restarted) await this.erase('live:');
    this.state = saved && !restarted ? saved : {boot, owner:null, attached:false, command:0, pending:null};
    this.durableOwner = saved && !restarted ? saved.owner : null;
    this.pendingDurable = saved && !restarted && saved.pending !== null;
    if (!saved || restarted) await this.save();
    return {restarted};
  }
  async close() {
    this.closing = true;
    await this.initializing?.catch(() => {});
    await this.attaching?.catch(() => {});
    await this.tail;
    this.closed = true; this.locked = false; this.releaseLock?.();
    await this.lockTask;
  }
  validateState(state) {
    check(typeof state.boot === 'string' && /^[0-9a-f]{32}$/.test(state.boot), 'Invalid saved server identity.');
    check(state.owner === null || (Number.isSafeInteger(state.owner) && state.owner > 0), 'Invalid saved notebook owner.');
    check(typeof state.attached === 'boolean' && Number.isSafeInteger(state.command) && state.command >= 0, 'Invalid saved command sequence.');
    if (state.pending !== null) {
      const pending = state.pending;
      check(pending && controls.has(pending.route) && pending.command === state.command + 1 && Number.isSafeInteger(pending.command), 'Invalid saved notebook command.');
      const input = JSON.parse(pending.body);
      check(state.attached && input.boot === state.boot && input.owner === state.owner && state.owner !== null
        && input.command === pending.command && pending.body === JSON.stringify(input), 'Saved command identity does not match its notebook.');
    }
  }
  async save() {
    await this.store.saveRecovery('controller', this.state);
    this.durableOwner = this.state.owner; this.pendingDurable = this.state.pending !== null;
  }
  serialize(action) {
    const operation = this.tail.then(action);
    this.tail = operation.catch(() => {});
    return operation;
  }
  async send(route, body) {
    const response = await this.fetcher(`/api/${route}`, {method:'POST', headers:{'Content-Type':'application/json'}, body});
    const text = await response.text();
    let data;
    try { data = JSON.parse(text); } catch { throw new Error(`${response.status}: ${text.slice(0,160) || 'Empty server response'}`); }
    if (!response.ok) {
      const error = new Error(typeof data.error === 'string' ? data.error : data.error?.message ?? data.message ?? `Request failed (${response.status})`);
      error.retry = data.retry === true; error.status = response.status; error.code = data.code; throw error;
    }
    return data;
  }
  async owner() {
    await this.initialize();
    if (this.state.attached) return {boot:this.state.boot, owner:this.state.owner};
    this.attaching ??= (async () => {
      if (this.state.owner === null) this.state.owner = positive((await this.send('reserve', JSON.stringify({boot:this.state.boot}))).owner);
      if (this.durableOwner !== this.state.owner) await this.save();
      try { await this.send('attach', JSON.stringify({boot:this.state.boot, owner:this.state.owner})); }
      catch (error) {
        if (error.code === 'unknown_owner' && !this.state.attached) { this.state.owner = null; await this.save(); }
        throw error;
      }
      const attached = {...this.state, attached:true};
      await this.store.saveRecovery('controller', attached); this.state = attached;
    })().catch(error => { this.attaching = null; throw error; });
    await this.attaching;
    return {boot:this.state.boot, owner:this.state.owner};
  }
  async request(route, payload) {
    check(!this.closing && !this.closed, 'This notebook controller is closed.');
    if (['hello', 'parse', 'format'].includes(route)) return this.send(route, JSON.stringify(payload));
    const identity = await this.owner();
    check(!this.closing && !this.closed, 'This notebook controller is closed.');
    return this.serialize(async () => {
      if (!controls.has(route)) return this.lifecycle(route, {...payload, ...identity});
      const command = this.state.pending?.command ?? positive(this.state.command + 1);
      const body = JSON.stringify({...payload, ...identity, command});
      if (this.state.pending) check(this.state.pending.route === route && this.state.pending.body === body, 'Recover the interrupted command before submitting another command.');
      else { this.state.pending = {route, command, body}; this.pendingDurable = false; }
      return this.replay();
    });
  }
  async replay({cancel = false} = {}) {
    if (!this.pendingDurable) await this.save(); // Never send a command whose exact bytes are not durable.
    const pending = this.state.pending;
    let response;
    try { response = await this.send(pending.route, pending.body); }
    catch (error) {
      if (error.status >= 400 && error.status < 500 && !error.retry) {
        const next = {...this.state, pending:null};
        await this.store.saveRecovery('controller', next); this.state = next; this.pendingDurable = false;
      }
      throw error;
    }
    const payload = JSON.parse(pending.body), run = pending.route === 'start' ? response.run : payload.run;
    if (cancel && pending.route === 'start') await this.send('cancel', JSON.stringify({boot:this.state.boot, owner:this.state.owner, run}));
    const adopted = await this.adopt(pending, response);
    if (cancel) {
      const key = `live:run:${run}`, row = await this.store.recovery(key);
      if (row) await this.store.saveRecovery(key, {...row, phase:'canceling'});
    }
    const next = {...this.state, command:pending.command, pending:null};
    await this.store.saveRecovery('controller', next); this.state = next; this.pendingDurable = false;
    return adopted;
  }
  async recover(options = {}) {
    check(!this.closing && !this.closed, 'This notebook controller is closed.');
    await this.initialize();
    if (!this.state.pending) return null;
    const {route, body} = this.state.pending;
    const {boot, owner, command, ...payload} = JSON.parse(body);
    await this.owner();
    if (options.cancel && route !== 'start') {
      await this.send('cancel', JSON.stringify({boot, owner, run:payload.run}));
      if (!this.pendingDurable) {
        // This control never passed its write-before-send boundary. Cancel the
        // existing run, then retire the unsent intent instead of executing it.
        const next = {...this.state, pending:null};
        await this.store.saveRecovery('controller', next); this.state = next;
        return null;
      }
    }
    for (;;) {
      try { return {route, payload, response:await this.replay(options)}; }
      catch (error) {
        if (!error.retry) throw error;
        await this.send('maintenance', JSON.stringify({boot, owner, run:payload.run, budget:2048}));
        await pause();
      }
    }
  }
  async adopt(pending, response) {
    const payload = JSON.parse(pending.body), runKey = `live:run:${payload.run}`;
    if (pending.route === 'start') {
      const run = positive(response.run);
      await this.store.saveRecovery(`live:source:${run}`, {submission:{program:payload.program, query:payload.query}, recordHistory:payload.record_history === true});
      const archive = await this.store.create(response, `Run ${run}`, {id:`live:run:${run}`, value:{run, phase:'paused', ack:null, index:0, sequence:null, applications:0, stepPending:false}});
      return {...response, archive};
    }
    if (pending.route === 'inspect') {
      const inspection = decimal(response.inspection), parent = await this.store.recovery(runKey);
      check(parent, 'The inspection has no saved execution.');
      const tables = await this.store.tables(parent.archive);
      const archive = await this.store.create(tables, `Inspection of run ${payload.run}`, {id:`live:inspection:${payload.run}:${inspection}`, value:{run:payload.run, inspection, phase:'active', ack:null, index:0, sequence:null}});
      return {...response, archive};
    }
    if (pending.route === 'snapshot') {
      const snapshot = decimal(response.snapshot);
      await this.store.saveRecovery(`live:snapshot:${payload.run}:${snapshot}`, {run:payload.run, snapshot, phase:'active'});
    } else {
      const run = await this.store.recovery(runKey);
      check(run, 'The command has no saved execution.');
      await this.store.saveRecovery(runKey, {...run, stepPending:pending.route === 'step'});
    }
    return response;
  }
  async lifecycle(route, payload) {
    let key, phase;
    if (['cancel', 'close'].includes(route)) { key = `live:run:${payload.run}`; phase = route === 'close' ? 'closing' : 'canceling'; }
    if (['inspect_cancel', 'inspect_release'].includes(route)) { key = `live:inspection:${payload.run}:${payload.inspection}`; phase = route === 'inspect_release' ? 'release' : 'canceling'; }
    if (route === 'snapshot_release') { key = `live:snapshot:${payload.run}:${payload.snapshot}`; phase = 'release'; }
    // Stopping a known execution is monotonic and keeps its identity/receipt.
    // Storage failure must not prevent that stop; reload reconciles server status.
    const stopped = route === 'cancel' ? await this.send(route, JSON.stringify(payload)) : null;
    if (key) {
      const row = await this.store.recovery(key);
      if (row) await this.store.saveRecovery(key, {...row, phase});
    }
    const response = route === 'cancel' ? stopped : await this.send(route, JSON.stringify(payload));
    if (phase === 'release') await this.forget(key, await this.store.recovery(key));
    if (route === 'close') {
      await this.erase(`live:snapshot:${payload.run}:`);
      await this.erase(`live:inspection:${payload.run}:`);
      await this.store.saveRecovery(`live:source:${payload.run}`, null);
      await this.forget(key, await this.store.recovery(key));
    }
    return response;
  }
  async forget(id, value) {
    if (value?.archive && value.assembler?.current) {
      const tables = await this.store.tables(value.archive);
      check(tables, 'Recovery archive tables are missing.');
      await this.store.discardPartial(value.archive, OutputAssembler.restore(tables, value.assembler));
    }
    await this.store.saveRecovery(id, null);
  }
  async erase(prefix) {
    let after = null;
    do {
      const page = await this.store.recoveryPage(prefix, after);
      for (const row of page.records) await this.forget(row.id, row.value);
      after = page.next;
      if (after !== null) await pause();
    } while (after !== null);
  }
}
