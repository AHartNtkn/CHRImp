import assert from 'node:assert/strict';
import {renderGraph, renderScene, relationColor} from './graph.mjs';

// Minimal SVG DOM: render and dispatch real renderer callbacks without a browser.
class Element {
  constructor(tag) { this.tag = tag; this.attributes = {}; this.children = []; this.listeners = {}; this.textContent = ''; }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  addEventListener(type, action) { this.listeners[type] = action; }
  fire(type, key) {
    const event = {key, stopped:false, prevented:false, stopPropagation() { this.stopped = true; }, preventDefault() { this.prevented = true; }};
    this.listeners[type]?.(event); return event;
  }
}
globalThis.document = {createElementNS(_namespace, tag) { return new Element(tag); }};
const all = element => [element, ...element.children.flatMap(all)];
const withClass = (svg, name) => all(svg).filter(e => e.attributes.class?.split(' ').includes(name));
const texts = svg => all(svg).filter(e => e.tag === 'text').map(e => String(e.textContent));

// Indexed proxies reject off-window reads, including Array.map/reduce scans.
function windowed(length, start, end, value) {
  let reads = 0;
  const array = new Proxy(new Array(length), {
    get(target, property, receiver) {
      if (/^(0|[1-9][0-9]*)$/.test(String(property))) {
        const index = Number(property);
        assert.ok(index >= start && index < end, `off-window read ${index} outside ${start}..${end}`);
        reads++; return value(index);
      }
      return Reflect.get(target, property, receiver);
    },
    has(target, property) { return /^(0|[1-9][0-9]*)$/.test(String(property)) || Reflect.has(target, property); },
  });
  return {array, reads:() => reads};
}

const svg = new Element('svg');
const chosen = [], connected = [];
const ports = windowed(100_000, 24, 32, i => `V${i}`);
const nodes = windowed(1_000_000, 18 * 100, 18 * 101, () => ({kind:'atom', atom:{relation:'edge', args:ports.array}}));
const info = renderGraph(svg, {query:{kind:'and', items:nodes.array}}, ['query'], {
  page:100, portPage:3, selected:{path:['query','items',1800],port:24},
  onSelect:selection => chosen.push(selection), onConnect:name => connected.push(name),
});
assert.deepEqual(info, {page:100,pages:55556,portPage:3,portPages:12500,count:1_000_000});
assert.equal(nodes.reads(), 18); assert.equal(ports.reads(), 18 * 8);
assert.equal(withClass(svg,'relation-node').length, 18);
assert.equal(withClass(svg,'port').length, 144);
assert.ok(texts(svg).includes('edge / 100000'));
assert.ok(texts(svg).includes('ports 25–32 of 100000'));
assert.equal(svg.attributes.role, 'group');
const firstPort = withClass(svg,'port')[0];
assert.equal(firstPort.attributes.role, 'button');
assert.equal(firstPort.attributes.tabindex, '0');
assert.ok(firstPort.attributes.class.includes('selected'));
const key = firstPort.fire('keydown', 'Enter');
assert.ok(key.prevented && key.stopped);
assert.deepEqual(chosen.pop(), {path:['query','items',1800],port:24});
withClass(svg,'relation-node')[0].fire('click');
assert.deepEqual(chosen.pop(), {path:['query','items',1800]});
withClass(svg,'junction')[0].fire('keydown', ' ');
assert.equal(connected.pop(), 'V24');

// A normalized saved window has no complete AST or complete argument arrays.
const scene = {
  entries:[
    {kind:'atom', relation:'edge', arity:100_000, args:['V24','V25'], portStart:24, path:['saved',5], occurrence:'9007199254740993'},
    {kind:'or', args:[], portStart:0, count:400_000, path:['saved',6]},
    {kind:'equal', args:['V24','V25'], portStart:0, path:['saved',7]},
  ], page:999,pages:1000,portPage:3,portPages:12500,count:17985,
};
const original = structuredClone(scene), opened = [];
assert.deepEqual(renderScene(svg, scene, {readonly:true,onOpen:path => opened.push(path),label:'Pending graph'}),
  {page:999,pages:1000,portPage:3,portPages:12500,count:17985});
assert.deepEqual(scene, original);
assert.equal(svg.attributes['aria-label'], 'Pending graph');
assert.equal(svg.attributes.role, 'group');
assert.ok(withClass(svg,'relation-node')[0].attributes.class.includes(relationColor('edge')));
assert.ok(texts(svg).includes('occurrence 9007199254740993'));
assert.ok(texts(svg).includes('ports 25–26 of 100000'));
assert.ok(texts(svg).includes('V24 = V25'));
assert.deepEqual(withClass(svg,'port').map(p => p.children[1].textContent).map(String), ['25','26','1','2']);
const group = withClass(svg,'relation-node')[1];
assert.equal(group.attributes['aria-label'], 'Open Or · 400000 branches');
group.fire('keydown', ' ');
assert.deepEqual(opened, [['saved',6]]);
assert.ok(!withClass(svg,'relation-node')[0].listeners.click);
assert.ok(withClass(svg,'port').every(p => !p.listeners.click));
assert.ok(withClass(svg,'junction').every(p => !p.listeners.click));

// AST group cards read only child count; opening them reads only their window.
const nestedChildren = windowed(100_000, 0, 18, i => ({kind:'atom',atom:{relation:`branch${i}`,args:['X']}}));
const nested = {query:{kind:'and',items:[{kind:'true'}, {kind:'or',items:nestedChildren.array}, {kind:'fail'}]}};
let nestedPath;
renderGraph(svg, nested, ['query'], {readonly:true,onOpen:path => { nestedPath = path; }});
assert.equal(nestedChildren.reads(), 0);
assert.ok(texts(svg).includes('true')); assert.ok(texts(svg).includes('fail'));
withClass(svg,'relation-node')[1].fire('click');
assert.deepEqual(nestedPath, ['query','items',1]);
assert.equal(renderGraph(svg, nested, nestedPath, {readonly:true}).count, 100_000);
assert.equal(nestedChildren.reads(), 18);
assert.ok(texts(svg).includes('branch0 / 1'));
assert.equal(svg.attributes.role, 'img');

// Heads use plain atoms and absolute source indexes, including the last page.
const heads = Array.from({length:19}, (_, i) => ({relation:`p${i}`,args:['X']}));
const headInfo = renderGraph(svg, {heads}, ['heads'], {page:999, portPage:99});
assert.deepEqual(headInfo, {page:1,pages:2,portPage:0,portPages:1,count:19});
assert.ok(texts(svg).includes('p18 / 1'));

// A huge unseen arity must not affect the selected node window's port pages.
Object.defineProperty(heads[0], 'args', {get() { throw new Error('read hidden arity'); }});
assert.equal(renderGraph(svg, {heads}, ['heads'], {page:1}).portPages, 1);
// Mixed arities keep equality ports visible on a later relation-port page.
renderGraph(svg, {query:{kind:'and',items:[
  {kind:'atom',atom:{relation:'wide',args:Array.from({length:10},(_,i)=>`V${i}`)}},
  {kind:'equal',left:'X',right:'Y'},
  {kind:'atom',atom:{relation:'small',args:['X']}},
]}}, ['query'], {portPage:1});
assert.deepEqual(withClass(svg,'port').map(p => String(p.children[1].textContent)), ['9','10','1','2']);
assert.ok(texts(svg).includes('wide / 10')); assert.ok(texts(svg).includes('small / 1'));
renderGraph(svg, {query:{kind:'and',items:[]}}, ['query'], {readonly:true});
assert.equal(svg.attributes.role, 'img');
assert.ok(texts(svg).includes('No facts in this alternative.'));
assert.equal(withClass(svg,'relation-node').length, 0);

assert.throws(() => renderScene(svg, {...scene,entries:Array(19).fill(scene.entries[0])}), /18/);
assert.throws(() => renderScene(svg, {...scene,entries:[{...scene.entries[0],args:Array(9).fill('X')}]}), /8/);
console.log('Bounded AST and normalized scene windows, absolute ports, shared visuals and keyboard interactions passed.');
