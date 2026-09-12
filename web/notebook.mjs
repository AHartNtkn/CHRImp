import { clone, at, atomOf, applyEdit, validateNotebook, renderGraph } from './graph.mjs';

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

export class OutputAssembler {
  constructor(tables, limit = Infinity) {
    check(Array.isArray(tables?.signatures) && Array.isArray(tables?.variables), 'Run response needs signatures and variables.');
    for (const signature of tables.signatures) {
      check(typeof signature.name === 'string', 'Invalid relation name in run response.');
      uint(signature.arity);
    }
    check(tables.variables.every(v => typeof v === 'string'), 'Invalid query variable names in run response.');
    check(limit === Infinity || (Number.isSafeInteger(limit) && limit > 0), 'Answer queue capacity must be positive.');
    this.tables = clone({ signatures: tables.signatures, variables: tables.variables });
    this.limit = limit; this.answers = []; this.total = 0; this.current = null; this.fact = null;
  }
  push(event) {
    const current = this.current;
    switch (event.kind) {
      case 'begin': {
        check(!current, 'An alternative is already open.');
        check(this.answers.length < this.limit, 'Save or explicitly release queued answers before consuming more output.');
        const completion = id(event.completion), alternative = id(event.alternative);
        this.current = { completion, alternative, variables: [], facts: [] }; break;
      }
      case 'variable':
        check(current && !this.fact && !current.facts.length, 'Query variables must precede facts.');
        check(uint(event.slot) === current.variables.length && event.slot < this.tables.variables.length, 'Unexpected query variable slot.');
        current.variables.push(id(event.variable)); break;
      case 'fact': {
        check(current && !this.fact, 'Expected an alternative without an open fact.');
        check(current.variables.length === this.tables.variables.length, 'Missing query variables.');
        const relation = uint(event.relation), signature = this.tables.signatures[relation];
        check(signature, 'Unknown relation index in output.');
        this.fact = { occurrence: id(event.occurrence), relation, name: signature.name, args: [] }; break;
      }
      case 'port':
        check(this.fact, 'A port needs an open fact.');
        check(this.fact.args.length < this.tables.signatures[this.fact.relation].arity, 'Too many ports.');
        this.fact.args.push(id(event.variable)); break;
      case 'end_fact':
        check(this.fact && current, 'No open fact.');
        check(this.fact.args.length === this.tables.signatures[this.fact.relation].arity, 'Missing fact ports.');
        current.facts.push(this.fact); this.fact = null; break;
      case 'end':
        check(current && !this.fact, 'An alternative must finish its fact first.');
        check(current.variables.length === this.tables.variables.length, 'Missing query variables.');
        current.number = ++this.total; this.answers.push(current);
        this.current = null; break;
      default: throw new Error(`Unknown output event: ${event.kind}`);
    }
  }
  finish() { check(!this.current && !this.fact, 'Output delivery ended inside an alternative.'); }
}

// A completed answer leaves the delivery queue only after its IndexedDB transaction commits.
export class IndexedAnswerStore {
  constructor(indexed = globalThis.indexedDB) {
    this.ready = new Promise((resolve, reject) => {
      if (!indexed) { reject(new Error('This browser cannot open saved answer storage.')); return; }
      const request = indexed.open('chr-notebook-answers', 1);
      request.onupgradeneeded = () => {
        const db = request.result;
        db.createObjectStore('collections', { keyPath: 'id' });
        db.createObjectStore('answers', { keyPath: ['collection', 'number'] });
      };
      request.onsuccess = () => { request.result.onversionchange = () => request.result.close(); resolve(request.result); };
      request.onerror = () => reject(request.error);
      request.onblocked = () => reject(new Error('Close other notebook tabs to open saved answer storage.'));
    });
    this.ready.catch(() => {});
  }
  async transaction(names, mode, work) {
    const db = await this.ready;
    return new Promise((resolve, reject) => {
      const tx = db.transaction(names, mode);
      let result;
      tx.oncomplete = () => resolve(result);
      tx.onabort = () => reject(tx.error ?? new Error('Saved answer transaction was aborted.'));
      tx.onerror = () => {}; // Abort owns the rejection; pending output stays with its run.
      try { work(tx, value => { result = value; }); } catch (error) { tx.abort(); reject(error); }
    });
  }
  create(tables, label) {
    const record = { id: crypto.randomUUID(), tables: clone(tables), label, total: 0, created: Date.now() };
    return this.transaction(['collections'], 'readwrite', (tx, result) => { tx.objectStore('collections').add(record); result(record.id); });
  }
  append(collection, answer) {
    return this.transaction(['answers', 'collections'], 'readwrite', tx => {
      const records = tx.objectStore('collections'), read = records.get(collection);
      read.onsuccess = () => {
        if (!read.result) { tx.abort(); return; }
        tx.objectStore('answers').put({ ...clone(answer), collection });
        records.put({ ...read.result, total: Math.max(read.result.total, answer.number) });
      };
    });
  }
  list() {
    return this.transaction(['collections'], 'readonly', (tx, result) => {
      const request = tx.objectStore('collections').getAll();
      request.onsuccess = () => result(request.result.sort((a, b) => b.created - a.created));
    });
  }
  page(collection, page = 0, size = 12) {
    uint(page); uint(size); check(size > 0, 'Page size must be positive.');
    return this.transaction(['collections', 'answers'], 'readonly', (tx, result) => {
      const request = tx.objectStore('collections').get(collection);
      request.onsuccess = () => {
        const record = request.result;
        if (!record) { result(null); return; }
        const pages = Math.max(1, Math.ceil(record.total / size));
        const current = Math.min(page, pages - 1);
        const range = IDBKeyRange.bound([collection, current * size + 1], [collection, (current + 1) * size]);
        const answers = tx.objectStore('answers').getAll(range, size);
        answers.onsuccess = () => result({ ...record, answers: answers.result, page: current, pages });
      };
    });
  }
  clear(collection) {
    return this.transaction(['collections', 'answers'], 'readwrite', tx => {
      tx.objectStore('answers').delete(IDBKeyRange.bound([collection, 0], [collection, Number.MAX_SAFE_INTEGER]));
      tx.objectStore('collections').delete(collection);
    });
  }
}

export class InspectionSelection {
  constructor() { this.choices = []; this.snapshots = []; this.assignments = new Map(); this.snapshot = ''; }
  update(choices, snapshots) {
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
    this.assignments = new Map(nextChoices.filter(item => this.assignments.has(item.id)).map(item => [item.id, this.assignments.get(item.id)]));
    if (!nextSnapshots.some(item => item.id === this.snapshot)) this.snapshot = '';
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
      check(this.snapshots.some(item => item.id === this.snapshot), 'This snapshot is no longer available.');
      payload.snapshot = uint(Number(this.snapshot));
    }
    return payload;
  }
}

export async function request(route, payload) {
  const response = await fetch(`/api/${route}`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(payload),
  });
  const text = await response.text();
  let data;
  try { data = JSON.parse(text); } catch { throw new Error(`${response.status}: ${text.slice(0, 160) || 'Empty server response'}`); }
  if (!response.ok) throw new Error(typeof data.error === 'string' ? data.error : data.error?.message ?? data.message ?? `Request failed (${response.status})`);
  return data;
}

export class RunSession {
  constructor(api = request, notify = () => {}, store = null) {
    this.api = api; this.notify = notify; this.store = store; this.archive = null; this.pendingDelivery = null; this.run = null; this.status = 'idle';
    this.running = false; this.inFlight = null; this.timer = null; this.stream = null;
    this.applications = 0; this.error = null; this.starting = false;
  }
  async start(model, recordHistory = false, auto = true) {
    check(!this.starting, 'A run is already starting.');
    this.starting = true;
    try {
      await this.cancel();
      this.submission = clone(validateNotebook(model));
      this.status = 'starting'; this.error = null; this.notify();
      const response = await this.api('start', { ...clone(this.submission), record_history: recordHistory });
      this.run = uint(response.run);
      this.stream = new OutputAssembler(response, storeLimit(this.store));
      this.archive = null;
      await this.ensureArchive();
      this.recordHistory = recordHistory; this.applications = 0; this.status = 'paused';
      if (auto) this.resume();
    } catch (error) { this.fail(error); throw error; }
    finally { this.starting = false; this.notify(); }
  }
  async ensureArchive() {
    if (this.store && this.stream && this.archive === null) this.archive = await this.store.create(this.stream.tables, `Run ${this.run}`);
  }
  async savePending() {
    if (!this.store || !this.stream?.answers.length) return;
    await this.ensureArchive();
    while (this.stream?.answers.length) {
      await this.store.append(this.archive, this.stream.answers[0]);
      this.stream.answers.shift();
    }
  }
  async deliverPending() {
    await this.savePending();
    const pending = this.pendingDelivery;
    if (!pending) return;
    while (pending.index < pending.response.events.length) {
      this.stream.push(pending.response.events[pending.index]);
      pending.index++;
      await this.savePending();
    }
    const response = pending.response;
    if (response.delivery_done) this.stream.finish();
    this.pendingDelivery = null;
    return response;
  }
  fail(error) { this.pause(); this.error = error; this.status = 'error'; this.notify(); }
  pause() {
    this.running = false; clearTimeout(this.timer); this.timer = null;
    if (this.run !== null && !['done', 'error'].includes(this.status)) this.status = 'paused';
    this.notify();
  }
  resume() {
    check(this.run !== null && this.stream, 'Start a run first.');
    if (this.status === 'done') return;
    this.error = null; this.running = true; this.status = 'running'; this.schedule(); this.notify();
  }
  schedule() {
    if (!this.running || this.inFlight || this.timer !== null) return;
    this.timer = setTimeout(() => { this.timer = null; this.advance().catch(() => {}); }, 16);
  }
  advance(step = false) {
    if (this.inFlight) return this.inFlight;
    check(this.run !== null && this.stream, 'Start a run first.');
    const pending = this.pendingDelivery ? Promise.resolve(this.pendingDelivery.response) : this.api('advance', { run: this.run, budget: step ? 1 : 128, step });
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
  async step() { this.pause(); if (this.inFlight) await this.inFlight; if (this.status !== 'done') return this.advance(true); }
  async cancel() {
    this.pause();
    if (this.inFlight) await this.inFlight.catch(() => {});
    await this.deliverPending();
    if (this.run !== null) await this.api('cancel', { run: this.run });
    this.run = null; this.status = 'idle'; this.notify();
  }
}

const storeLimit = store => store ? 1 : Infinity;

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
  let savedView = null, savedSelection = '', answerPage = 0, loadRevision = 0, inspectionPending = null, inspecting = false, launching = false;
  let resultPage = 0, resultPortPage = 0, bindingPage = 0;
  let inspectionSelection = new InspectionSelection(), choicePage = 0;
  const session = new RunSession(request, renderRun, store);
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
      $(prefix + '-' + suffix).onclick = () => { if (prefix === 'graph') { if (ports) portPage += delta; else page += delta; } else { if (ports) resultPortPage += delta; else resultPage += delta; } render(); };
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
    $('run').disabled = session.starting || busy || launching || inspecting;
    $('pause').disabled = !session.running;
    $('resume').disabled = session.run === null || session.running || session.status === 'done' || session.starting;
    $('step').disabled = session.starting || !!session.inFlight || busy || launching || inspecting;
    $('cancel').disabled = session.run === null || session.starting;
    $('inspect').disabled = (session.run === null && !inspectionPending) || session.starting || inspecting || launching;
    $('snapshot').disabled = !session.recordHistory || inspecting;
    if (session.error) message(session.error.message, true);
    renderResults(); safe(refreshSaved);
  }
  async function refreshSaved() {
    const version = ++loadRevision;
    const records = await store.list();
    const collection = outputMode === 'inspect' ? inspected?.archive : savedSelection || session.archive;
    const view = collection ? await store.page(collection, answerPage) : null;
    if (version !== loadRevision) return;
    $('saved-run').replaceChildren(el('option', 'Current run', { value: '' }), ...records.map(record => el('option', `${record.label} · ${record.total} answers · ${new Date(record.created).toLocaleString()}`, { value: record.id })));
    $('saved-run').value = savedSelection;
    savedView = view; answerPage = view?.page ?? 0;
    $('saved-page').value = answerPage + 1;
    $('saved-page').max = view?.pages ?? 1;
    $('saved-pages').textContent = `/ ${view?.pages ?? 1}`;
    $('saved-prev').disabled = answerPage === 0;
    $('saved-next').disabled = !view || answerPage + 1 >= view.pages;
    $('clear-answers').disabled = !view || (view.id === session.archive && session.run !== null) || view.id === inspectionPending?.archive;
    renderResults();
  }
  function renderResults() {
    const stream = savedView;
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
    $('bindings').replaceChildren();
    const bindings = answer?.variables ?? [];
    const bindingPages = Math.max(1, Math.ceil(bindings.length / 24)); bindingPage = Math.min(bindingPage, bindingPages - 1);
    bindings.slice(bindingPage * 24, bindingPage * 24 + 24).forEach((variable, i) => $('bindings').append(el('span', `${stream.tables.variables[bindingPage * 24 + i]} = V${variable}`)));
    $('binding-page').textContent = `Bindings ${bindingPage + 1} / ${bindingPages}`;
    $('binding-prev').disabled = bindingPage === 0; $('binding-next').disabled = bindingPage === bindingPages - 1;
    $('binding-prev').onclick = () => { bindingPage--; renderResults(); }; $('binding-next').onclick = () => { bindingPage++; renderResults(); };
    $('result-empty').hidden = !!answer;
    $('result-graph').hidden = !answer;
    const facts = answer?.facts ?? [];
    const resultModel = { query: { kind: 'and', items: facts.map(fact => ({ kind: 'atom', atom: { relation: fact.name, args: fact.args.map(v => `V${v}`) } })) } };
    const info = renderGraph($('result-graph'), resultModel, ['query'], { readonly: true, page: resultPage, portPage: resultPortPage, occurrences: facts.map(f => f.occurrence), label: outputMode === 'inspect' ? 'Inspected graph' : 'Answer hypergraph' });
    resultPage = info.page; resultPortPage = info.portPage; pager('result', info, renderResults);
  }
  function resetResultPages() { resultPage = resultPortPage = bindingPage = 0; }
  async function inspect() {
    check(!inspecting, 'An inspection is already in progress.');
    inspecting = true; renderRun();
    try { await inspectOnce(); } finally { inspecting = false; renderRun(); }
  }
  async function inspectOnce() {
    if (!inspectionPending) {
      check(session.run !== null, 'Start a run first.');
      const run = session.run, tables = session.stream.tables;
      const payload = inspectionSelection.payload(run);
      const response = await request('inspect', payload);
      check(session.run === run, 'The run changed during inspection.');
      check(Array.isArray(response.events), 'Inspection response needs output events.');
      inspectionPending = { stream: new OutputAssembler(tables, 1), response, index: 0, archive: null, run };
    }
    const pending = inspectionPending;
    pending.archive ??= await store.create(pending.stream.tables, `Inspection of run ${pending.run}`);
    const save = async () => {
      while (pending.stream.answers.length) {
        await store.append(pending.archive, pending.stream.answers[0]); pending.stream.answers.shift();
      }
    };
    await save();
    while (pending.index < pending.response.events.length) {
      pending.stream.push(pending.response.events[pending.index]); pending.index++; await save();
    }
    pending.stream.finish();
    inspected = { archive: pending.archive }; inspectionPending = null;
    outputMode = 'inspect'; answerNumber = null; answerPage = 0; resetResultPages();
    inspectionSelection.update(pending.response.choices, pending.response.snapshots); renderInspectionControls();
    await refreshSaved(); message('Inspection saved.');
  }
  function renderInspectionControls() {
    const pages = Math.max(1, Math.ceil(inspectionSelection.choices.length / 12));
    choicePage = Math.min(choicePage, pages - 1);
    $('choices').replaceChildren();
    if (!inspectionSelection.choices.length) $('choices').append(el('p', session.run === null ? 'Start a run to inspect its choices.' : 'Inspect the graph to refresh available choices.'));
    inspectionSelection.choices.slice(choicePage * 12, choicePage * 12 + 12).forEach(item => {
      const label = el('label', item.label), select = el('select');
      select.append(el('option', 'Either', { value: 'either' }), el('option', 'First', { value: 'first' }), el('option', 'Second', { value: 'second' }));
      select.value = inspectionSelection.assignments.get(item.id) ?? 'either';
      select.onchange = () => safe(() => inspectionSelection.choose(item.id, select.value));
      label.append(select); $('choices').append(label);
    });
    $('choice-page').textContent = `Choices ${choicePage + 1} / ${pages}`;
    $('choice-prev').disabled = choicePage === 0; $('choice-next').disabled = choicePage + 1 >= pages;
    $('snapshot').replaceChildren(el('option', 'Current graph', { value: '' }), ...inspectionSelection.snapshots.map(item => el('option', item.label, { value: item.id })));
    $('snapshot').value = inspectionSelection.snapshot;
  }
  $('choice-prev').onclick = () => { choicePage--; renderInspectionControls(); };
  $('choice-next').onclick = () => { choicePage++; renderInspectionControls(); };
  $('snapshot').onchange = () => { inspectionSelection.snapshot = $('snapshot').value; inspectionSelection.assignments.clear(); renderInspectionControls(); };
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
    await syncSource(); inspected = null; outputMode = 'answers'; savedSelection = ''; savedView = null; answerPage = 0; answerNumber = null; resetResultPages();
    inspectionSelection = new InspectionSelection(); choicePage = 0; renderInspectionControls();
    await session.start(model, $('history').checked); message('Running the submitted notebook.');
    } finally { launching = false; renderRun(); }
  });
  $('step').onclick = () => safe(async () => {
    check(!launching, 'A step is already in progress.'); launching = true; renderRun();
    try {
    if (session.run === null) { await syncSource(); inspectionSelection = new InspectionSelection(); choicePage = 0; renderInspectionControls(); await session.start(model, $('history').checked, false); }
    await session.step(); await inspect();
    } finally { launching = false; renderRun(); }
  });
  $('pause').onclick = () => session.pause(); $('resume').onclick = () => safe(() => session.resume());
  $('cancel').onclick = () => safe(() => session.cancel()); $('inspect').onclick = () => safe(inspect);
  $('alternatives').onchange = () => { answerNumber = Number($('alternatives').value); resetResultPages(); renderResults(); };
  $('output-mode').onchange = () => safe(async () => { outputMode = $('output-mode').value; answerNumber = null; answerPage = 0; resetResultPages(); await refreshSaved(); });
  $('saved-run').onchange = () => safe(async () => { savedSelection = $('saved-run').value; outputMode = 'answers'; answerNumber = null; answerPage = 0; resetResultPages(); await refreshSaved(); });
  const changeSavedPage = next => safe(async () => { answerPage = uint(next); answerNumber = null; resetResultPages(); await refreshSaved(); });
  $('saved-prev').onclick = () => changeSavedPage(answerPage - 1);
  $('saved-next').onclick = () => changeSavedPage(answerPage + 1);
  $('saved-page').onchange = () => changeSavedPage(Number($('saved-page').value) - 1);
  $('clear-answers').onclick = () => safe(async () => {
    const collection = savedView?.id; check(collection, 'Select saved answers first.');
    check(collection !== session.archive || session.run === null, 'Cancel the run before clearing its saved answers.');
    check(collection !== inspectionPending?.archive, 'Finish saving the inspection before clearing it.');
    if (!window.confirm('Permanently clear all saved answers in this selection?')) return;
    await store.clear(collection);
    if (session.archive === collection) session.archive = null;
    if (inspected?.archive === collection) inspected = null;
    savedSelection = ''; savedView = null; answerNumber = null; answerPage = 0;
    await refreshSaved(); message('Saved answers cleared.');
  });
  renderWorkspace(); renderRun(); renderInspectionControls();
}
