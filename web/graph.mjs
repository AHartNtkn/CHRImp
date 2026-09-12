// The editor uses the syntax AST directly. Paths identify syntax, never SVG nodes.
const relationName = /^[a-z][A-Za-z0-9_]*$/;
const variableName = /^[A-Z_][A-Za-z0-9_]*$/;
const ruleName = /^[A-Za-z_][A-Za-z0-9_]*$/;
const require = (condition, message) => { if (!condition) throw new Error(message); };
export const clone = value => structuredClone(value);

export function at(model, path) {
  require(Array.isArray(path), 'Select a graph item.');
  let node = model;
  for (const key of path) {
    require(key !== '__proto__' && key !== 'constructor' && key !== 'prototype' &&
      node !== null && typeof node === 'object' && Object.hasOwn(node, key), 'This selection no longer exists.');
    node = node[key];
  }
  return node;
}
export const atomOf = node => node?.kind === 'atom' ? node.atom : node?.relation !== undefined ? node : null;

function validateAtom(atom) {
  require(atom && typeof atom.relation === 'string' && relationName.test(atom.relation), 'A relation starts with a lowercase letter and uses letters, digits or underscores.');
  require(Array.isArray(atom.args) && atom.args.every(arg => typeof arg === 'string' && variableName.test(arg)),
    'Variables start with an uppercase letter or underscore.');
}
function validateBody(body) {
  const pending = [[body, 0]];
  while (pending.length) {
    const [node, depth] = pending.pop();
    require(node && typeof node === 'object', 'Choose a body expression.');
    if (node.kind === 'atom') validateAtom(node.atom);
    else if (node.kind === 'equal') require(typeof node.left === 'string' && typeof node.right === 'string' && variableName.test(node.left) && variableName.test(node.right), 'Equality needs two variable names.');
    else if (node.kind === 'and' || node.kind === 'or') {
      require(depth < 128, 'Groups may nest at most 128 levels.');
      require(Array.isArray(node.items), 'A group needs an item list.');
      require(node.kind !== 'or' || node.items.length > 0, 'An alternative needs at least one branch.');
      for (const item of node.items) pending.push([item, depth + 1]);
    } else require(node.kind === 'true' || node.kind === 'fail', 'Unknown body expression.');
  }
}
export function validateNotebook(model) {
  require(model?.program && Array.isArray(model.program.rules), 'A program needs a rule list.');
  const names = new Set();
  for (const rule of model.program.rules) {
    require(rule.name === null || (typeof rule.name === 'string' && ruleName.test(rule.name)), 'Choose a valid rule name, or leave it blank.');
    if (rule.name !== null) { require(!names.has(rule.name), 'Rule names must be unique.'); names.add(rule.name); }
    require(Array.isArray(rule.kept) && Array.isArray(rule.removed) && rule.kept.length + rule.removed.length > 0, 'A rule needs at least one head relation.');
    [...rule.kept, ...rule.removed].forEach(validateAtom);
    validateBody(rule.body);
  }
  validateBody(model.query);
  return model;
}

export function applyEdit(model, op) {
  const next = clone(model);
  const path = op.path ?? [];
  const node = at(next, path);
  const atom = atomOf(node);
  const replace = value => {
    require(path.length > 0, 'Select an expression.');
    const parent = at(next, path.slice(0, -1));
    parent[path.at(-1)] = value;
  };
  const port = () => require(atom && Number.isInteger(op.index) && op.index >= 0 && op.index < atom.args.length, 'Select a numbered port.');
  switch (op.type) {
    case 'rename-relation': require(atom, 'Select a relation.'); atom.relation = op.relation; break;
    case 'set-port': port(); atom.args[op.index] = op.variable; break;
    case 'insert-port': require(atom, 'Select a relation.'); atom.args.push(op.variable); break;
    case 'remove-port': port(); atom.args.splice(op.index, 1); break;
    case 'move-port': {
      port(); require(Number.isInteger(op.to) && op.to >= 0 && op.to < atom.args.length, 'Choose a port position.');
      atom.args.splice(op.to, 0, atom.args.splice(op.index, 1)[0]); break;
    }
    case 'equal': require(node.kind === 'equal', 'Select an equality.'); node.left = op.left; node.right = op.right; break;
    case 'replace': replace(clone(op.node)); break;
    case 'wrap':
      require(node.kind && ['and', 'or'].includes(op.kind), 'Only body expressions can be grouped.');
      replace({ kind: op.kind, items: [node] }); break;
    case 'append': {
      if (Array.isArray(node)) {
        require(['kept', 'removed'].includes(path.at(-1)) && op.node.kind === 'atom', 'Rule heads contain relations only.');
        node.push(clone(op.node.atom));
      } else if (node.kind === 'and' || node.kind === 'or') node.items.push(clone(op.node));
      else { require(node.kind, 'Select a body or head list.'); replace({ kind: 'and', items: [node, clone(op.node)] }); }
      break;
    }
    case 'remove': {
      const parent = at(next, path.slice(0, -1));
      if (Array.isArray(parent)) {
        const container = at(next, path.slice(0, -2));
        require(container.kind !== 'or' || parent.length > 1, 'An alternative needs at least one branch.');
        parent.splice(path.at(-1), 1);
      } else { require(node.kind, 'Select a body expression.'); replace({ kind: 'true' }); }
      break;
    }
    case 'move-item': {
      const parent = at(next, path.slice(0, -1));
      require(Array.isArray(parent) && Number.isInteger(op.to) && op.to >= 0 && op.to < parent.length, 'Choose an item position.');
      parent.splice(op.to, 0, parent.splice(path.at(-1), 1)[0]); break;
    }
    case 'add-rule': next.program.rules.push(clone(op.rule ?? { name: null, kept: [], removed: [{ relation: 'p', args: ['X'] }], body: { kind: 'true' } })); break;
    case 'remove-rule': require(Number.isInteger(op.index) && next.program.rules[op.index], 'Select a rule.'); next.program.rules.splice(op.index, 1); break;
    case 'rule-name': require(node.kept && node.removed, 'Select a rule.'); node.name = op.name || null; break;
    default: throw new Error('Unknown graph edit.');
  }
  return validateNotebook(next);
}

export function variablesIn(node) {
  const names = new Set();
  const pending = [node];
  while (pending.length) {
    const item = pending.pop();
    if (Array.isArray(item)) pending.push(...item.slice().reverse());
    else if (atomOf(item)) atomOf(item).args.forEach(arg => names.add(arg));
    else if (item?.kind === 'equal') { names.add(item.left); names.add(item.right); }
    else if (item?.items) pending.push(...item.items.slice().reverse());
    else if (item?.kept) pending.push(item.body, item.removed, item.kept);
  }
  return [...names];
}

export function sceneEntries(model, path) {
  const node = at(model, path);
  if (Array.isArray(node)) return node.map((node, i) => ({ node, path: [...path, i] }));
  if (node.kind === 'and' || node.kind === 'or') return node.items.map((node, i) => ({ node, path: [...path, 'items', i] }));
  return [{ node, path }];
}
export function relationColor(name) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return ['cyan', 'ochre', 'violet'][(hash >>> 0) % 3];
}

const NS = 'http://www.w3.org/2000/svg';
function svgNode(tag, attrs = {}, text) {
  const node = document.createElementNS(NS, tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  if (text !== undefined) node.textContent = text;
  return node;
}
function interactive(node, label, action) {
  node.setAttribute('tabindex', '0'); node.setAttribute('role', 'button'); node.setAttribute('aria-label', label);
  node.addEventListener('click', event => { event.stopPropagation(); action(); });
  node.addEventListener('keydown', event => {
    if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); event.stopPropagation(); action(); }
  });
}

export function renderGraph(svg, model, path, options = {}) {
  const entries = sceneEntries(model, path);
  const pageSize = 18, portsPerPage = 8;
  const pages = Math.max(1, Math.ceil(entries.length / pageSize));
  const page = Math.min(options.page ?? 0, pages - 1);
  const maxPorts = entries.reduce((n, e) => Math.max(n, atomOf(e.node)?.args.length ?? 0), 0);
  const portPages = Math.max(1, Math.ceil(maxPorts / portsPerPage));
  const portPage = Math.min(options.portPage ?? 0, portPages - 1);
  const visible = entries.slice(page * pageSize, (page + 1) * pageSize);
  const names = new Set();
  for (const { node } of visible) {
    const args = atomOf(node)?.args.slice(portPage * portsPerPage, (portPage + 1) * portsPerPage) ??
      (node.kind === 'equal' ? [node.left, node.right] : []);
    args.forEach(v => names.add(v));
  }
  const variables = [...names];
  const columns = 3, width = 760;
  const junctionY = Math.max(140, Math.ceil(visible.length / columns) * 130 + 50);
  const height = junctionY + Math.max(1, Math.ceil(variables.length / 6)) * 62 + 35;
  svg.replaceChildren(); svg.setAttribute('viewBox', `0 0 ${width} ${height}`);
  svg.setAttribute('aria-label', options.label ?? 'Ordered-port hypergraph');
  svg.append(svgNode('title', {}, options.label ?? 'Ordered-port hypergraph'));
  const wires = svgNode('g', { class: 'wires', 'aria-hidden': 'true' });
  const nodes = svgNode('g'); svg.append(wires, nodes);
  const positions = new Map(variables.map((v, i) => [v, { x: 62 + (i % 6) * 125, y: junctionY + Math.floor(i / 6) * 62 }]));
  if (!visible.length) svg.append(svgNode('text', { x: 28, y: 65, class: 'graph-empty' }, options.readonly ? 'No facts in this alternative.' : 'Add a relation to this view.'));
  visible.forEach(({ node, path: itemPath }, index) => {
    const atom = atomOf(node), x = 28 + (index % columns) * 246, y = 30 + Math.floor(index / columns) * 130;
    const color = atom ? relationColor(atom.relation) : node.kind === 'equal' ? 'violet' : 'neutral';
    const selected = JSON.stringify(options.selected?.path) === JSON.stringify(itemPath);
    const group = svgNode('g', { class: `relation-node ${color}${selected ? ' selected' : ''}` });
    group.append(svgNode('rect', { x, y, width: 212, height: 66, rx: atom ? 7 : 2, class: node.kind === 'or' ? 'or-boundary' : '' }));
    let title = atom ? `${atom.relation} / ${atom.args.length}` : node.kind === 'equal' ? `${node.left} = ${node.right}` :
      node.items ? `${node.kind === 'and' ? 'And' : 'Or'} · ${node.items.length} ${node.kind === 'and' ? 'items' : 'branches'}` : node.kind;
    group.append(svgNode('text', { x: x + 12, y: y + 27, class: 'node-name' }, title.length > 26 ? title.slice(0, 23) + '…' : title));
    group.append(svgNode('title', {}, title));
    if (node.items) group.append(svgNode('text', { x: x + 12, y: y + 49, class: 'node-note' }, 'Select to open group'));
    if (atom && options.occurrences) group.append(svgNode('text', { x: x + 12, y: y + 48, class: 'node-note' }, `occurrence ${options.occurrences[page * pageSize + index]}`));
    if (!options.readonly) interactive(group, `Select ${title}`, () => options.onSelect?.({ path: itemPath }));
    nodes.append(group);
    const args = atom?.args ?? (node.kind === 'equal' ? [node.left, node.right] : []);
    const start = atom ? portPage * portsPerPage : 0;
    const shown = args.slice(start, start + portsPerPage);
    shown.forEach((variable, i) => {
      const px = x + 18 + i * 25, py = y + 66, dest = positions.get(variable);
      if (!dest) return;
      wires.append(svgNode('path', { d: `M${px},${py} C${px},${py + 38} ${dest.x},${dest.y - 40} ${dest.x},${dest.y}`, class: `wire ${color}` }));
      const activePort = selected && options.selected?.port === start + i;
      const port = svgNode('g', { class: `port ${color}${activePort ? ' selected' : ''}` });
      port.append(svgNode('circle', { cx: px, cy: py, r: 10 }), svgNode('text', { x: px, y: py + 3.5, 'text-anchor': 'middle' }, start + i + 1));
      if (!options.readonly && atom) interactive(port, `Select port ${start + i + 1} of ${atom.relation}, connected to ${variable}`, () => options.onSelect?.({ path: itemPath, port: start + i }));
      nodes.append(port);
    });
    if (args.length > portsPerPage) nodes.append(svgNode('text', { x: x + 12, y: y + 100, class: 'node-note' }, shown.length ? `ports ${start + 1}–${Math.min(start + portsPerPage, args.length)} of ${args.length}` : 'No ports on this page'));
  });
  for (const [name, { x, y }] of positions) {
    const junction = svgNode('g', { class: 'junction' });
    junction.append(svgNode('circle', { cx: x, cy: y, r: 5 }), svgNode('text', { x, y: y + 23, 'text-anchor': 'middle' }, name));
    if (!options.readonly) interactive(junction, `Connect selected port to ${name}`, () => options.onConnect?.(name));
    nodes.append(junction);
  }
  return { pages, page, portPages, portPage, count: entries.length };
}
