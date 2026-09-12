import assert from 'node:assert/strict';
import {test} from 'node:test';
import {at, clone, variablesIn} from './graph.mjs';
import {emptyNotebook, validateDocument, serializeDocument, parseDocument, executionModel,
  replaceExecutionModel, fragmentFor, pasteFragment, removeSelection, searchNotebook} from './documents.mjs';

const atom = (relation, ...args) => ({kind:'atom', atom:{relation, args}});
const and = (...items) => ({kind:'and', items});
const rule = (name = 'reduce') => ({name, kept:[{relation:'keep', args:['X','Y']}],
  removed:[{relation:'take', args:['Y','X']}], body:atom('result','X','Y')});
const model = () => ({program:{rules:[rule()]}, query:and(atom('p','X','Y','X'),
  {kind:'or', items:[and({kind:'equal', left:'X', right:'Z'}, atom('q','Z','Y')), atom('r','Y')]})});
const fragment = (...items) => ({format:'chr-fragment', version:1, kind:'expressions', items});
const bodyPath = ['program','rules',0,'body'];

test('portable round trip, detached clones, active-query replacement and layouts', () => {
  const doc = emptyNotebook();
  doc.queries.push({id:'other', name:'Another query', body:atom('other','W')});
  doc.activeQuery = 'other';
  doc.layouts.other = [['["query"]', {x:-1.25, y:20}]];
  const snapshot = clone(doc), replacement = model();
  const next = replaceExecutionModel(doc, replacement);
  assert.deepEqual(doc, snapshot);
  assert.deepEqual(next.queries[0].body, {kind:'true'});
  assert.deepEqual(next.queries[1].body, replacement.query);
  assert.deepEqual(next.layouts, doc.layouts);
  assert.deepEqual(parseDocument(serializeDocument(next)), next);
  const executable = executionModel(next);
  assert.deepEqual(executable, replacement);
  executable.program.rules[0].kept[0].args[0] = 'Changed';
  replacement.query.items.length = 0;
  assert.equal(next.program.rules[0].kept[0].args[0], 'X');
  assert.equal(next.queries[1].body.items.length, 2);
  const validated = validateDocument(next);
  validated.queries[0].name = 'Changed';
  assert.equal(next.queries[0].name, 'Query 1');
});

test('malformed documents and inactive queries are rejected', () => {
  assert.throws(() => parseDocument('{'));
  assert.throws(() => parseDocument(null));
  for (const mutate of [
    doc => {doc.format = 'other';}, doc => {doc.version = '1';},
    doc => {doc.queries = [];}, doc => {doc.queries.push(clone(doc.queries[0]));},
    doc => {doc.queries[0].id = ' ';}, doc => {doc.queries[0].name = ' ';},
    doc => {doc.activeQuery = 'absent';}, doc => {doc.title = null;},
    doc => {doc.queries.push({id:'bad', name:'Bad', body:{kind:'or', items:[]}});},
    doc => {doc.program.rules = [{...rule(), kept:[], removed:[]}];},
    doc => {doc.layouts = [];}, doc => {doc.layouts.a = [['a',{x:Infinity,y:0}]];},
    doc => {doc.layouts.a = [['a',{x:0,y:0}],['a',{x:1,y:1}]];},
  ]) {
    const doc = emptyNotebook(); mutate(doc);
    assert.throws(() => validateDocument(doc));
  }
});

test('layout ids must encode unique safe paths before files reach rendering', () => {
  const doc = emptyNotebook(), offset = {x:-2.5,y:3.75};
  for (const id of [
    'not JSON', '{', 'null', '{}', '"query"', '12', '[]',
    '["query",null]', '["query",true]', '["query",{}]', '["query",[]]',
    '["query","items",-1]', '["query","items",1.5]', '["query","items",1e400]',
    '["query","items",9007199254740992]', '["query",""]',
    '["query","__proto__"]', '["query","constructor"]', '["query","prototype"]',
  ]) {
    doc.layouts = {'query:query-1':[[id,offset]]};
    assert.throws(() => validateDocument(doc), id);
    assert.throws(() => parseDocument(JSON.stringify(doc)), id);
  }
  doc.layouts = {'query:query-1':[['["query","items",0]',offset],['[ "query", "items", 0 ]',offset]]};
  assert.throws(() => parseDocument(JSON.stringify(doc)), /unique/i);
  doc.layouts = {'query:query-1':[['[ "query", "items", 0 ]',offset]]};
  assert.deepEqual(parseDocument(serializeDocument(doc)),doc);
  for (const invalidOffset of [{x:NaN,y:0},{x:0,y:Infinity},{x:'1',y:0}]) {
    doc.layouts = {'query:query-1':[['["query"]',invalidOffset]]};
    assert.throws(() => validateDocument(doc));
  }
});

test('rule removal remaps surviving layout keys and nested paths in source order', () => {
  const doc = emptyNotebook();
  doc.program.rules = [rule('a'),rule(null),rule('c'),rule(null)];
  doc.program.rules.forEach(rule => { rule.body = and(rule.body); });
  // Unnamed rules can be structurally identical; match their occurrences in order.
  for (let i = 0; i < 4; i++) doc.layouts[`rule:${i}`] = [
    [JSON.stringify(['program','rules',i,'kept',0]),{x:10 + i,y:20 + i}],
    [JSON.stringify(['program','rules',i,'body','items',0]),{x:30 + i,y:40 + i}],
  ];
  doc.layouts['query:query-1'] = [['["query"]',{x:9,y:8}]];
  const before = clone(doc);
  const next = replaceExecutionModel(doc,removeSelection(executionModel(doc),[
    ['program','rules',0],['program','rules',2],
  ]));
  assert.deepEqual(next.layouts, {
    'rule:0':[
      ['["program","rules",0,"kept",0]',{x:11,y:21}],
      ['["program","rules",0,"body","items",0]',{x:31,y:41}],
    ],
    'rule:1':[
      ['["program","rules",1,"kept",0]',{x:13,y:23}],
      ['["program","rules",1,"body","items",0]',{x:33,y:43}],
    ],
    'query:query-1':doc.layouts['query:query-1'],
  });
  assert.deepEqual(doc,before);
  next.layouts['rule:0'][0][1].x = 100;
  assert.equal(doc.layouts['rule:1'][0][1].x,11);
  const empty = replaceExecutionModel(doc,{program:{rules:[]},query:{kind:'true'}});
  assert.deepEqual(empty.layouts,{'query:query-1':doc.layouts['query:query-1']});
});

test('same-index rule edits and renames retain layouts, including when adding rules', () => {
  const doc = replaceExecutionModel(emptyNotebook(),model());
  doc.layouts['rule:0'] = [['["program","rules",0,"kept",0]',{x:7,y:12}]];
  const replacement = executionModel(doc);
  replacement.program.rules[0].name = 'renamed';
  replacement.program.rules[0].kept[0].relation = 'edited';
  assert.deepEqual(replaceExecutionModel(doc,replacement).layouts,doc.layouts);
  replacement.program.rules.push(rule('added'));
  assert.deepEqual(replaceExecutionModel(doc,replacement).layouts,doc.layouts);
});

test('shifted rules without a saved layout do not inherit another rule position', () => {
  const doc = emptyNotebook();
  doc.program.rules = [rule('a'),rule('b'),rule('c')];
  doc.layouts['rule:0'] = [['["program","rules",0,"body"]',{x:1,y:2}]];
  doc.layouts['rule:2'] = [['["program","rules",2,"body"]',{x:3,y:4}]];
  const next = replaceExecutionModel(doc,removeSelection(executionModel(doc),[['program','rules',0]]));
  assert.deepEqual(next.layouts,{'rule:1':[['["program","rules",1,"body"]',{x:3,y:4}]]});
});

test('program traversal stays constant as query count grows; execution validates every body', () => {
  const doc = replaceExecutionModel(emptyNotebook(), model());
  const body = doc.program.rules[0].body;
  let reads = 0;
  Object.defineProperty(doc.program.rules[0], 'body', {enumerable:true, get() { reads++; return body; }});
  validateDocument(doc);
  const singleQueryReads = reads;
  assert.ok(singleQueryReads > 0);
  for (let i = 0; i < 8; i++) doc.queries.push({id:`extra-${i}`,name:`Extra ${i}`,body:atom('extra','X')});
  reads = 0;
  validateDocument(doc);
  assert.equal(reads, singleQueryReads);
  assert.deepEqual(executionModel(doc).query, doc.queries[0].body);
  for (const query of doc.queries) {
    const saved = query.body;
    query.body = {kind:'or',items:[]};
    assert.throws(() => validateDocument(doc));
    assert.throws(() => executionModel(doc));
    query.body = saved;
  }
  doc.program.rules[0].kept = [];
  doc.program.rules[0].removed = [];
  assert.throws(() => executionModel(doc));
});

test('duplicate fragments freshen every variable while retaining nested sharing and port order', () => {
  const source = model(), snapshot = clone(source);
  const copied = fragmentFor(source, [['query','items',0], ['query','items',1]]);
  const savedFragment = clone(copied);
  const pasted = pasteFragment(source, ['query'], copied);
  assert.deepEqual(source, snapshot);
  assert.deepEqual(copied, savedFragment);
  assert.deepEqual(pasted.paths, [['query','items',2], ['query','items',3]]);
  const p = at(pasted.model, pasted.paths[0]).atom;
  const branch = at(pasted.model, pasted.paths[1]).items[0].items;
  assert.equal(p.relation, 'p');
  assert.equal(p.args[0], p.args[2]);
  assert.notEqual(p.args[0], p.args[1]);
  assert.equal(branch[0].left, p.args[0]);
  assert.equal(branch[0].right, branch[1].atom.args[0]);
  assert.equal(branch[1].atom.args[1], p.args[1]);
  const originalNames = new Set(variablesIn(source.query));
  for (const name of variablesIn(pasted.paths.map(path => at(pasted.model, path)))) assert.ok(!originalNames.has(name));
  const twice = pasteFragment(pasted.model, ['query'], copied);
  const previous = new Set(variablesIn(pasted.model.query));
  for (const name of variablesIn(twice.paths.map(path => at(twice.model, path)))) assert.ok(!previous.has(name));
  const intoEmpty = pasteFragment({program:{rules:[]},query:{kind:'true'}}, ['query'], copied);
  for (const name of variablesIn(intoEmpty.model.query)) assert.ok(!originalNames.has(name));
});

test('freshening reserves source names, destination scope, and separate source scopes', () => {
  const source = model();
  source.program.rules[0].kept[0].args.push('X_1');
  const pasted = pasteFragment(source, bodyPath, fragment(atom('new','X','X_1'), {kind:'equal',left:'X',right:'X_1'}));
  const items = at(pasted.model, bodyPath).items;
  assert.deepEqual(items[1].atom.args, ['X_2','X_1_1']);
  assert.deepEqual(items[2], {kind:'equal',left:'X_2',right:'X_1_1'});
  const mixed = fragmentFor(source, [['query','items',0], ['program','rules',0,'kept',0]]);
  assert.equal(mixed.items[0].atom.args[0], mixed.items[0].atom.args[2]);
  assert.notEqual(mixed.items[0].atom.args[0], mixed.items[1].atom.args[0]);
});

test('copy across alternatives projects Or structure and preserves sharing on paste', () => {
  const source = model();
  source.query = {kind:'or',items:[
    and(atom('p','X','Y','X'),atom('unused','U')),
    and(atom('q','Y','X'),{kind:'equal',left:'X',right:'Y'}),
    atom('unselected','Z'),
  ]};
  const before = clone(source);
  const copied = fragmentFor(source, [
    ['query','items',0,'items',0], ['query','items',1,'items',0], ['query','items',1,'items',1],
  ]);
  assert.deepEqual(copied.items, [{kind:'or',items:[
    and(atom('p','X','Y','X')), and(atom('q','Y','X'),{kind:'equal',left:'X',right:'Y'}),
  ]}]);
  const pasted = pasteFragment(source, ['query'], copied);
  assert.equal(pasted.paths.length, 1);
  const inserted = at(pasted.model,pasted.paths[0]);
  assert.equal(inserted.kind, 'or');
  assert.equal(inserted.items.length, 2);
  const p = inserted.items[0].items[0].atom, q = inserted.items[1].items[0].atom;
  assert.deepEqual(p.args, ['X_1','Y_1','X_1']);
  assert.deepEqual(q.args, ['Y_1','X_1']);
  assert.deepEqual(inserted.items[1].items[1], {kind:'equal',left:'X_1',right:'Y_1'});
  assert.deepEqual(source,before);
  assert.deepEqual(fragmentFor(source,[['query']]).items,[source.query]);
  const sameBranch = fragmentFor(source,[['query','items',1,'items',0],['query','items',1,'items',1]]);
  assert.deepEqual(sameBranch.items,source.query.items[1].items);
});

test('nested body Or boundaries survive projection alongside head selections', () => {
  const source = model();
  source.program.rules[0].body = and(atom('outside','X'), {kind:'or',items:[
    and(atom('p','X'), {kind:'or',items:[atom('q','X'),atom('omit','Y')]}),
    atom('r','X'), atom('omit','Z'),
  ]});
  const copied = fragmentFor(source,[
    ['program','rules',0,'kept',0], [...bodyPath,'items',0],
    [...bodyPath,'items',1,'items',0,'items',1,'items',0], [...bodyPath,'items',1,'items',1],
  ]);
  assert.deepEqual(copied.items,[atom('keep','X','Y'),atom('outside','X'),{kind:'or',items:[
    and({kind:'or',items:[atom('q','X')]}),atom('r','X'),
  ]}]);
  const pasted = pasteFragment(source,['query'],copied);
  assert.deepEqual(pasted.paths.map(path => at(pasted.model,path).kind),['atom','atom','or']);
  assert.deepEqual(variablesIn(pasted.paths.map(path => at(pasted.model,path))),['X_1','Y_1']);
});

test('head atoms convert to expressions; multi-selection appends into one compartment', () => {
  const source = model();
  const copied = fragmentFor(source, [['program','rules',0,'kept',0], ['program','rules',0,'removed',0]]);
  assert.equal(copied.kind, 'expressions');
  assert.deepEqual(copied.items.map(item => item.kind), ['atom','atom']);
  const pasted = pasteFragment(source, ['query'], copied);
  assert.deepEqual(pasted.paths.map(path => at(pasted.model,path).atom.args), [['X_1','Y_1'],['Y_1','X_1']]);
  const head = pasteFragment(source, ['program','rules',0,'kept'], copied);
  assert.deepEqual(head.paths.map(path => at(head.model,path).relation), ['keep','take']);
  assert.deepEqual(head.paths, [['program','rules',0,'kept',1], ['program','rules',0,'kept',2]]);
  const group = pasteFragment(source, bodyPath, fragment(and(atom('a','A'),atom('b','A'))));
  assert.deepEqual(at(group.model,bodyPath).items.map(item => item.atom.relation), ['result','a','b']);
});

test('rules copy with fresh names and unchanged local variables; descendants are pruned', () => {
  const source = model();
  source.program.rules.push(rule('reduce_1'));
  const copied = fragmentFor(source, [['program','rules',0],bodyPath,['program','rules',0]]);
  assert.equal(copied.kind, 'rules'); assert.equal(copied.items.length, 1);
  const pasted = pasteFragment(source, ['program','rules'], copied);
  assert.deepEqual(pasted.paths, [['program','rules',2]]);
  assert.deepEqual(pasted.model.program.rules[2], {...rule(), name:'reduce_2'});
  const empty = executionModel(emptyNotebook());
  assert.equal(pasteFragment(empty, ['program','rules'], copied).model.program.rules[0].name, 'reduce_1');
  assert.throws(() => fragmentFor(source, [['program','rules',0],['query']]));
  assert.throws(() => fragmentFor(source, [['query','items']]));
  assert.throws(() => fragmentFor(source, [['query','items',0,'atom']]));
});

test('removal uses original indices and validates the entire transaction', () => {
  const source = model(), snapshot = clone(source);
  source.query = and(atom('a'),atom('b'),atom('c'),atom('d'));
  const next = removeSelection(source, [['query','items',0],['query','items',2]]);
  assert.deepEqual(next.query.items.map(item => item.atom.relation), ['b','d']);
  assert.equal(source.query.items.length, 4);
  const nested = removeSelection(snapshot, [['query','items',1,'items',0],['query','items',1],['query','items',1,'items',1]]);
  assert.equal(nested.query.items.length, 1);
  const rules = removeSelection(snapshot, [['program','rules',0,'kept',0],['program','rules',0],['program','rules',0,'removed',0]]);
  assert.deepEqual(rules.program.rules, []);
  assert.throws(() => removeSelection(snapshot, [['program','rules',0,'kept',0],['program','rules',0,'removed',0]]));
  assert.throws(() => removeSelection(snapshot, [['query','items',1,'items',0],['query','items',1,'items',1]]));
  assert.deepEqual(snapshot, model());
  assert.deepEqual(removeSelection(snapshot, [['query'],['query','items',0]]).query, {kind:'true'});
});

test('invalid cross-kind and malformed pastes never modify either input', () => {
  const source = model(), snapshot = clone(source), rules = fragmentFor(source, [['program','rules',0]]);
  for (const [destination, copied] of [
    [['query'],rules], [['program','rules'],fragment(atom('a'))],
    [['program','rules',0,'kept'],fragment({kind:'equal',left:'X',right:'Y'})],
    [['program','rules',0,'removed'],fragment({kind:'or',items:[atom('a')]})],
    [['program','rules',0,'kept'],fragment(and(atom('a')))],
    [['query'],{...fragment(atom('a')),version:2}], [['query'],fragment(atom('INVALID'))],
    [['query'],fragment()], [['program','rules',0,'kept',0],fragment(atom('a'))],
    [['query','__proto__'],fragment(atom('a'))],
  ]) {
    const before = clone(copied);
    assert.throws(() => pasteFragment(source,destination,copied));
    assert.deepEqual(copied,before); assert.deepEqual(source,snapshot);
  }
});

test('search finds literal case-insensitive rule and relation names in every query', () => {
  const doc = replaceExecutionModel(emptyNotebook(), model());
  doc.queries.push({id:'other',name:'Other',body:atom('result','T')});
  assert.deepEqual(searchNotebook(doc,'REDUCE'), [{label:'reduce',path:['program','rules',0]}]);
  assert.deepEqual(searchNotebook(doc,'ReSuLt'), [
    {label:'result',path:bodyPath}, {label:'result',path:['query'],queryId:'other'}]);
  assert.deepEqual(searchNotebook(doc,'q'), [
    {label:'Query 1',path:['query'],queryId:'query-1'},
    {label:'q',path:['query','items',1,'items',0,'items',1],queryId:'query-1'}]);
  assert.deepEqual(searchNotebook(doc,'.*'), []);
  assert.deepEqual(searchNotebook(doc,'['), []);
  assert.deepEqual(searchNotebook(doc,''), []);
});

test('query-name search navigates active and inactive queries and treats punctuation literally', () => {
  const doc = emptyNotebook();
  doc.queries[0].name = 'Find [Open]';
  doc.queries.push({id:'other',name:'Find .* Closed',body:{kind:'true'}});
  assert.deepEqual(searchNotebook(doc,'fInD'), [
    {label:'Find [Open]',path:['query'],queryId:'query-1'},
    {label:'Find .* Closed',path:['query'],queryId:'other'}]);
  assert.deepEqual(searchNotebook(doc,'['), [{label:'Find [Open]',path:['query'],queryId:'query-1'}]);
  assert.deepEqual(searchNotebook(doc,'.*'), [{label:'Find .* Closed',path:['query'],queryId:'other'}]);
});
