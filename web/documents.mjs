import {clone, validateNotebook, variablesIn, at, atomOf} from './graph.mjs';

const require = (condition, message) => { if (!condition) throw new Error(message); };
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const nonempty = value => typeof value === 'string' && value.trim().length > 0;

export function emptyNotebook() {
  return {format:'chr-notebook', version:1, title:'Untitled notebook', program:{rules:[]},
    queries:[{id:'query-1', name:'Query 1', body:{kind:'true'}}], activeQuery:'query-1', layouts:{}};
}

export function validateDocument(doc) {
  require(record(doc) && doc.format === 'chr-notebook' && doc.version === 1, 'Unsupported notebook format or version.');
  require(typeof doc.title === 'string', 'A notebook needs a title.');
  require(Array.isArray(doc.queries) && doc.queries.length > 0, 'A notebook needs at least one query.');
  const ids = new Set();
  for (const [i, query] of doc.queries.entries()) {
    require(record(query) && nonempty(query.id) && !ids.has(query.id), 'Query ids must be nonempty and unique.');
    require(nonempty(query.name), 'A query needs a nonempty name.');
    ids.add(query.id);
    validateNotebook({program:i === 0 ? doc.program : {rules:[]}, query:query.body});
  }
  require(ids.has(doc.activeQuery), 'Select an existing active query.');
  require(record(doc.layouts), 'Layouts must be a map of position lists.');
  for (const entries of Object.values(doc.layouts)) {
    require(Array.isArray(entries), 'A layout needs a position list.');
    const positions = new Set();
    for (const entry of entries) {
      require(Array.isArray(entry) && entry.length === 2 && typeof entry[0] === 'string' &&
        record(entry[1]) && Number.isFinite(entry[1].x) && Number.isFinite(entry[1].y),
      'Layout positions need ids and finite x/y coordinates.');
      const path = JSON.parse(entry[0]);
      require(Array.isArray(path) && path.length > 0 && path.every(key =>
        typeof key === 'string' ? key.length > 0 && !['__proto__','constructor','prototype'].includes(key) :
          Number.isSafeInteger(key) && key >= 0), 'Layout ids must encode safe source paths with nonnegative integer indices.');
      const id = JSON.stringify(path);
      require(!positions.has(id), 'Layout paths must be unique.');
      positions.add(id);
    }
  }
  return clone(doc);
}

export const serializeDocument = doc => JSON.stringify(validateDocument(doc), null, 2);
export function parseDocument(text) {
  require(typeof text === 'string', 'A notebook file must contain JSON text.');
  return validateDocument(JSON.parse(text));
}
export function executionModel(doc) {
  const next = validateDocument(doc);
  return {program:next.program, query:next.queries.find(query => query.id === next.activeQuery).body};
}
function remapRuleLayouts(layouts, before, after) {
  if (after.length >= before.length) return layouts;
  const originals = before.map(rule => JSON.stringify(rule)), indices = new Map();
  let from = 0;
  for (const [to, rule] of after.entries()) {
    const signature = JSON.stringify(rule);
    while (from < originals.length && originals[from] !== signature) from++;
    // Only an unchanged subsequence establishes structural removal.
    if (from === originals.length) return layouts;
    indices.set(from++, to);
  }
  return Object.fromEntries(Object.entries(layouts).flatMap(([key, entries]) => {
    if (!/^rule:(0|[1-9][0-9]*)$/.test(key)) return [[key, entries]];
    const from = Number(key.slice(5)), to = indices.get(from);
    if (to === undefined) return [];
    return [[`rule:${to}`, entries.map(([id, offset]) => {
      const path = JSON.parse(id);
      if (Array.isArray(path) && path[0] === 'program' && path[1] === 'rules' && path[2] === from) {
        path[2] = to;
        return [JSON.stringify(path), offset];
      }
      return [id, offset];
    })]];
  }));
}
export function replaceExecutionModel(doc, model) {
  const next = validateDocument(doc), replacement = clone(validateNotebook(model));
  next.layouts = remapRuleLayouts(next.layouts, next.program.rules, replacement.program.rules);
  next.program = replacement.program;
  next.queries.find(query => query.id === next.activeQuery).body = replacement.query;
  return validateDocument(next);
}

const prefix = (a, b) => a.length <= b.length && a.every((key, i) => key === b[i]);
const index = value => Number.isInteger(value) && value >= 0;
function selectionKind(model, path) {
  at(model, path);
  if (path.length === 3 && path[0] === 'program' && path[1] === 'rules' && index(path[2])) return 'rules';
  if (path[0] === 'program' && path[1] === 'rules' && index(path[2]) &&
      ['kept','removed'].includes(path[3]) && path.length === 5 && index(path[4])) return 'expressions';
  const start = path[0] === 'query' ? 1 :
    path[0] === 'program' && path[1] === 'rules' && index(path[2]) && path[3] === 'body' ? 4 : 0;
  require(start > 0 && (path.length - start) % 2 === 0, 'Select a rule or expression.');
  for (let i = start; i < path.length; i += 2)
    require(path[i] === 'items' && index(path[i + 1]), 'Select a rule or expression.');
  require(typeof at(model, path)?.kind === 'string', 'Select an expression.');
  return 'expressions';
}
function selections(model, paths) {
  require(Array.isArray(paths), 'Select graph items.');
  paths.forEach(path => selectionKind(model, path));
  return paths.filter((path, i) => !paths.some((other, j) => j !== i && prefix(other, path) &&
    (other.length < path.length || j < i))).map(path => [...path]);
}
function rename(node, names) {
  if (Array.isArray(node)) node.forEach(item => rename(item, names));
  else if (atomOf(node)) atomOf(node).args = atomOf(node).args.map(name => names.get(name));
  else if (node.kind === 'equal') { node.left = names.get(node.left); node.right = names.get(node.right); }
  else if (node.items) rename(node.items, names);
}
function freshMap(names, used) {
  const mapping = new Map();
  for (const name of names) {
    let candidate = name, suffix = 1;
    while (used.has(candidate)) candidate = `${name}_${suffix++}`;
    used.add(candidate); mapping.set(name, candidate);
  }
  return mapping;
}
function projectExpressions(model, entries) {
  const root = [...entries[0].path];
  while (!entries.every(({path}) => prefix(root, path))) root.pop();
  if (root.at(-1) === 'items') root.pop();
  function project(path, selected) {
    const whole = selected.find(entry => entry.path.length === path.length);
    if (whole) return whole.node;
    const node = at(model, path);
    return {kind:node.kind, items:node.items.flatMap((_, i) => {
      const child = [...path, 'items', i], descendants = selected.filter(entry => prefix(child, entry.path));
      return descendants.length ? [project(child, descendants)] : [];
    })};
  }
  const projected = project(root, entries);
  return projected.kind === 'and' ? projected.items : [projected];
}
export function fragmentFor(model, paths) {
  validateNotebook(model);
  const selected = selections(model, paths);
  require(selected.length > 0, 'Select something to copy.');
  const kind = selectionKind(model, selected[0]);
  require(selected.every(path => selectionKind(model, path) === kind), 'Copy rules or expressions together.');
  let items = selected.map(path => {
    const node = clone(at(model, path));
    return kind === 'expressions' && !node.kind ? {kind:'atom', atom:node} : node;
  });
  // Distinct source scopes must not acquire sharing when copied together.
  if (kind === 'expressions') {
    const scopes = new Map(), used = new Set();
    selected.forEach((path, i) => {
      const key = JSON.stringify(path[0] === 'query' ? ['query'] : path.slice(0, 3));
      if (!scopes.has(key)) scopes.set(key, []);
      scopes.get(key).push(items[i]);
    });
    for (const nodes of scopes.values()) rename(nodes, freshMap(variablesIn(nodes), used));
    // Project each body independently; head atoms have no expression ancestor.
    const bodies = new Map();
    const keys = selected.map((path, i) => {
      if (path[0] !== 'query' && path[3] !== 'body') return null;
      const key = JSON.stringify(path.slice(0, path[0] === 'query' ? 1 : 4));
      if (!bodies.has(key)) bodies.set(key, []);
      bodies.get(key).push({path, node:items[i]});
      return key;
    });
    items = items.flatMap((node, i) => {
      if (keys[i] === null) return [node];
      const entries = bodies.get(keys[i]);
      if (!entries) return [];
      bodies.delete(keys[i]);
      return projectExpressions(model, entries);
    });
  }
  return {format:'chr-fragment', version:1, kind, items};
}
function validateFragment(fragment) {
  require(record(fragment) && fragment.format === 'chr-fragment' && fragment.version === 1 &&
    ['rules','expressions'].includes(fragment.kind) && Array.isArray(fragment.items) && fragment.items.length > 0,
  'Unsupported or empty fragment.');
  validateNotebook(fragment.kind === 'rules' ? {program:{rules:fragment.items}, query:{kind:'true'}} :
    {program:{rules:[]}, query:{kind:'and', items:fragment.items}});
  return clone(fragment);
}
export function pasteFragment(model, destination, fragment) {
  const next = clone(validateNotebook(model)), copied = validateFragment(fragment), target = at(next, destination);
  const paths = [];
  if (copied.kind === 'rules') {
    require(destination.length === 2 && destination[0] === 'program' && destination[1] === 'rules', 'Paste rules into the program rule list.');
    const used = new Set([...next.program.rules, ...copied.items].map(rule => rule.name).filter(name => name !== null));
    for (const rule of copied.items) {
      if (rule.name !== null) rule.name = freshMap([rule.name], used).get(rule.name);
      paths.push([...destination, target.length]); target.push(rule);
    }
  } else {
    const head = destination.length === 4 && destination[0] === 'program' && destination[1] === 'rules' &&
      index(destination[2]) && ['kept','removed'].includes(destination[3]);
    if (!head) require(selectionKind(next, destination) === 'expressions' && !Array.isArray(target) && target.kind,
      'Paste expressions into a body or head list.');
    // All names change, even if the destination did not previously use them.
    const names = variablesIn(copied.items), scope = at(next, destination[0] === 'query' ? ['query'] : destination.slice(0, 3));
    rename(copied.items, freshMap(names, new Set([...variablesIn(scope), ...names])));
    if (head) require(copied.items.every(item => item.kind === 'atom'), 'Rule heads contain relations only.');
    const items = copied.items.flatMap(item => item.kind === 'and' ? item.items : [item]);
    if (head) {
      for (const item of items) { paths.push([...destination, target.length]); target.push(item.atom); }
    } else {
      let body;
      if (target.kind === 'and') body = target;
      else { body = {kind:'and', items:target.kind === 'true' ? [] : [target]}; at(next, destination.slice(0, -1))[destination.at(-1)] = body; }
      for (const item of items) { paths.push([...destination, 'items', body.items.length]); body.items.push(item); }
    }
  }
  return {model:validateNotebook(next), paths};
}
export function removeSelection(model, paths) {
  const next = clone(validateNotebook(model)), selected = selections(next, paths);
  // Descending sibling indices retain the meaning of every original path.
  selected.sort((a, b) => {
    for (let i = 0; i < Math.min(a.length, b.length); i++) {
      if (a[i] !== b[i]) return typeof a[i] === 'number' && typeof b[i] === 'number' ? b[i] - a[i] : String(a[i]).localeCompare(String(b[i]));
    }
    return b.length - a.length;
  });
  for (const path of selected) {
    const parent = at(next, path.slice(0, -1));
    if (Array.isArray(parent)) parent.splice(path.at(-1), 1);
    else parent[path.at(-1)] = {kind:'true'};
  }
  return validateNotebook(next);
}
export function searchNotebook(doc, text) {
  const next = validateDocument(doc);
  require(typeof text === 'string', 'Search needs text.');
  const needle = text.toLowerCase(), matches = [];
  if (!needle) return matches;
  const add = (label, path, queryId) => {
    if (label.toLowerCase().includes(needle)) matches.push({label, path, ...(queryId === undefined ? {} : {queryId})});
  };
  function visit(node, path, queryId) {
    const atom = atomOf(node);
    if (atom) add(atom.relation, path, queryId);
    else if (node.items) node.items.forEach((item, i) => visit(item, [...path, 'items', i], queryId));
  }
  next.program.rules.forEach((rule, i) => {
    const path = ['program','rules',i];
    if (rule.name !== null) add(rule.name, path);
    for (const key of ['kept','removed']) rule[key].forEach((atom, j) => visit(atom, [...path,key,j]));
    visit(rule.body, [...path,'body']);
  });
  next.queries.forEach(query => {
    add(query.name, ['query'], query.id);
    visit(query.body, ['query'], query.id);
  });
  return matches;
}
