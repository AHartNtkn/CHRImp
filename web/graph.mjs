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
    case 'set-port':
      if(node.kind==='equal'){require(op.index===0||op.index===1,'Select an equality port.');node[op.index===0?'left':'right']=op.variable;}
      else {port();atom.args[op.index]=op.variable;}break;
    case 'insert-port': require(atom, 'Select a relation.'); atom.args.push(op.variable??freshVariables(next,path,1)[0]); break;
    case 'remove-port': port(); atom.args.splice(op.index, 1); break;
    case 'move-port': {
      port(); require(Number.isInteger(op.to) && op.to >= 0 && op.to < atom.args.length, 'Choose a port position.');
      atom.args.splice(op.to, 0, atom.args.splice(op.index, 1)[0]); break;
    }
    case 'equal': require(node.kind === 'equal', 'Select an equality.'); node.left = op.left; node.right = op.right; break;
    case 'replace': replace(clone(op.node)); break;
    case 'add-alternative': require(node.kind==='or','Select a disjunction.');node.items.push({kind:'true'});break;
    case 'append': {
      if (Array.isArray(node)) {
        require(['kept', 'removed'].includes(path.at(-1)) && op.node.kind === 'atom', 'Rule heads contain relations only.');
        node.push(clone(op.node.atom));
      } else if (node.kind === 'and') node.items.push(clone(op.node));
      else if(node.kind==='true')replace(clone(op.node));
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

// Variables are local to a query or a rule, including all its alternatives.
export function freshVariables(model,path,count=1) {
  const scope=path[0]==='program'?path.slice(0,3):['query'];
  const used=new Set(variablesIn(at(model,scope))),names=[];
  for(let i=0;names.length<count;i++){const name=`V${i}`;if(!used.has(name)){used.add(name);names.push(name);}}
  return names;
}
export function insertionPath(model,path) {
  let scope=path[0]==='program'?[...path.slice(0,3),['kept','removed'].includes(path[3])?path[3]:'body']:['query'];
  for(let i=scope.length;i<path.length;i++)if(path[i]==='items'&&at(model,path.slice(0,i)).kind==='or')scope=path.slice(0,i+2);
  return scope;
}

export function relationColor(name) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return ['cyan', 'ochre', 'violet'][(hash >>> 0) % 3];
}

// Source paths remain the edit authority; geometry never changes execution order.
export function diagramScene(model, path) {
  const rootPath = path[0] === 'program' ? path.slice(0,3) : path[0] === 'query' ? ['query'] : path;
  function build(node, path, label) {
    const atom = atomOf(node);
    if (atom) return {kind:'atom',relation:atom.relation,args:atom.args,path};
    if (node.kept) return {kind:'rule',path,label:node.name || 'Rule',children:[
      build(node.kept,[...path,'kept'],'Kept'),build(node.removed,[...path,'removed'],'Removed'),{kind:'and',path:[...path,'body'],label:'Body',children:[build(node.body,[...path,'body'])]}]};
    const children = Array.isArray(node) ? node : node.items;
    return {kind:Array.isArray(node)?'and':node.kind,path,label,
      args:node.kind==='equal'?[node.left,node.right]:[],
      ...(children ? {children:children.map((child,i)=>build(child,[...path,...(Array.isArray(node)?[]:['items']),i]))} : {})};
  }
  const result=build(at(model,rootPath),rootPath);
  return rootPath[0]==='query'?{kind:'and',path:rootPath,label:'Query',children:[result]}:result;
}
const keyOf = path => JSON.stringify(path);
const overlaps = (a,b) => a.x <= b.x+b.width && a.x+a.width >= b.x && a.y <= b.y+b.height && a.y+a.height >= b.y;

// Order connected siblings together in linear incidence work, without changing ports.
function connectedOrder(children) {
  const incidence=new Map();
  children.forEach((c,i)=>c.names.forEach(name=>{if(!incidence.has(name))incidence.set(name,[]);incidence.get(name).push(i);}));
  const seen=new Set(), order=[];
  for(let i=0;i<children.length;i++) {
    if(seen.has(i))continue;
    const queue=[i];seen.add(i);
    for(let j=0;j<queue.length;j++) {
      const index=queue[j];order.push(children[index]);
      for(const name of children[index].names) {
        for(const other of incidence.get(name)??[])if(!seen.has(other)){seen.add(other);queue.push(other);}
        incidence.delete(name);
      }
    }
  }
  return order;
}
export function layoutScene(scene, positions = new Map()) {
  function measure(node) {
    if(node.kind==='and'&&!node.label)node={...node,compact:true};
    let children=(node.children??[]).map(child=>measure(node.kind==='or'&&child.kind==='and'?{...child,compact:true}:child));
    const names=new Set(node.args??[]);
    children.forEach(c=>c.names.forEach(n=>names.add(n)));
    if(node.kind==='and')children=connectedOrder(children);
    if(node.kind==='true')return {...node,names,children:[],compact:true,width:0,height:0};
    if(!node.children)return {...node,names,width:Math.max(node.kind==='atom'?166:92,(node.args?.length??0)*28+24,(node.relation?.length??0)*9+48),height:86};
    if(node.kind==='or') {
      const width=children.reduce((width,child)=>Math.max(width,child.width+48),190);let y=38;
      children=children.map((child,i)=>{child.dx=24;child.dy=32;const section={kind:'branch',path:child.path,label:`Alternative ${i+1}`,names:child.names,children:[child],dx:0,dy:y,width,height:child.height+54};y+=section.height;return section;});
      return {...node,children,names,width,height:y};
    }
    const columns=node.kind==='rule'?3:Math.min(4,Math.max(1,children.length));
    if(node.kind==='rule') {
      let x=0;children.forEach(child=>{child.dx=x;child.dy=0;x+=child.width+34;});
      return {...node,children,names,width:x-34,height:Math.max(...children.map(c=>c.height))};
    }
    const widths=Array(columns).fill(0), heights=[];
    children.forEach((c,i)=>{widths[i%columns]=Math.max(widths[i%columns],c.width);heights[Math.floor(i/columns)]=Math.max(heights[Math.floor(i/columns)]??0,c.height);});
    const padding=node.compact?0:node.label?14:24,top=node.compact?0:node.label?38:40;
    let y=top;
    children.forEach((c,i)=>{
      const row=Math.floor(i/columns),col=i%columns;
      if(col===0&&row)y+=heights[row-1]+52;
      c.dx=padding+widths.slice(0,col).reduce((sum,w)=>sum+w+44,0);c.dy=y;
    });
    return {...node,children,names,width:Math.max(166,2*padding+widths.reduce((s,w)=>s+w,0)+44*(columns-1)),height:Math.max(86,y+(heights.at(-1)??0)+(node.compact?0:22))};
  }
  const root=measure(scene),items=[],ports=new Map();
  const add=item=>{item.order=items.length;items.push(item);return item;};
  function place(node,x,y,depth=0,ancestors=[]) {
    const id=keyOf(node.path),offset=positions.get(id)??{x:0,y:0};
    x+=offset.x;y+=offset.y;
    if(node.children) {
      if(node.compact){node.children.forEach(c=>place(c,x+c.dx,y+c.dy,depth,ancestors));return;}
      const label=node.label??(node.kind==='or'?'Or':node.kind==='rule'?'Rule':'And');
      const boundary=add({type:'boundary',kind:node.kind,path:node.path,id,x,y,width:node.width,height:node.height,label,depth,region:!!node.label&&node.kind!=='branch'});
      if(node.kind==='or') {
        const sections=[];let bottom=y+38;
        for(const child of node.children) {
          const start=items.length;
          place(child,x,bottom,depth+1,[...ancestors,boundary]);
          const section=items[start];
          // Keep every alternative inside its own contiguous compartment after a drag.
          const shift=bottom-section.y;
          for(let i=start;i<items.length;i++)items[i].y+=shift;
          bottom+=section.height;sections.push(section);
        }
        const left=sections.reduce((min,s)=>Math.min(min,s.x),x);
        const right=sections.reduce((max,s)=>Math.max(max,s.x+s.width),x+node.width);
        Object.assign(boundary,{x:left,y,width:right-left,height:bottom-y});
        sections.forEach(section=>Object.assign(section,{x:left,width:right-left}));
        for(const parent of ancestors){parent.width=Math.max(parent.x+parent.width,right)-Math.min(parent.x,left);parent.x=Math.min(parent.x,left);parent.height=Math.max(parent.height,bottom-parent.y);}
      } else node.children.forEach(c=>place(c,x+c.dx,y+c.dy,depth+1,[...ancestors,boundary]));
      return;
    }
    for(const boundary of ancestors){if(!positions.size)continue;const right=Math.max(boundary.x+boundary.width,x+node.width+28),bottom=Math.max(boundary.y+boundary.height,y+node.height+28);boundary.x=Math.min(boundary.x,x-28);boundary.y=Math.min(boundary.y,y-48);boundary.width=right-boundary.x;boundary.height=bottom-boundary.y;}
    const label=node.kind==='atom'?`${node.relation} / ${node.args.length}`:node.kind==='equal'?'=':node.kind;
    add({type:'node',...node,id,x,y,width:node.width,height:62,label});
    (node.args??[]).forEach((name,port)=>{
      const item=add({type:'port',name,relation:node.relation,path:node.path,id:`${id}:${port}`,port,x:x+20+28*port-10,y:y+62-10,width:20,height:20});
      if(!ports.has(name))ports.set(name,[]);ports.get(name).push(item);
    });
  }
  place(root,28,28);
  let routingError=null;const routes=[];
  try{routeTrees(items,ports,item=>routes.push(item));routes.forEach(add);}
  catch(error){if(!error.routing)throw error;routingError=error.message;}
  // A balanced bounding-volume tree makes repaint proportional to visible geometry.
  function index(objects,depth=0) {
    if(!objects.length)return null;
    let x=Infinity,y=Infinity,right=-Infinity,bottom=-Infinity;
    for(const o of objects){x=Math.min(x,o.x);y=Math.min(y,o.y);right=Math.max(right,o.x+o.width);bottom=Math.max(bottom,o.y+o.height);}
    const box={x,y,width:right-x,height:bottom-y};
    if(objects.length<=12)return {...box,items:objects};
    const axis=depth%2?'y':'x';objects.sort((a,b)=>a[axis]-b[axis]);const middle=Math.floor(objects.length/2);
    return {...box,left:index(objects.slice(0,middle),depth+1),right:index(objects.slice(middle),depth+1)};
  }
  const tree=index([...items]);
  return {items,index:tree,routingError,width:Math.max(root.width+56,(tree?.x??0)+(tree?.width??0)+28),height:Math.max(root.height+56,(tree?.y??0)+(tree?.height??0)+28)};
}
export function visibleItems(layout,view) {
  const result=[],pending=[layout.index];
  while(pending.length){const node=pending.pop();if(!node||!overlaps(node,view))continue;
    if(node.items)for(const item of node.items){if(overlaps(item,view))result.push(item);}
    else pending.push(node.left,node.right);
  }
  return result.sort((a,b)=>a.order-b.order);
}
// Incremental rectilinear Steiner approximation: attach each terminal to the
// existing tree by an obstacle-avoiding shortest path, including bend costs.
// ponytail: a 4px routing lattice bounds geometric precision; use a visibility
// graph if diagrams need channels narrower than this spacing.
function routeTrees(items, ports, add) {
  const step=4,key=(x,y)=>`${x},${y}`,point=k=>k.split(',').map(Number);
  const obstacles=new Map(),occupied=new Map(),usedEdges=new Set(),reserved=new Map();
  const anchor=p=>({port:p,x:Math.round((p.x+10)/step)*step,y:Math.ceil((p.y+28)/step)*step});
  for(const [name,ends] of ports)for(const p of ends){const a=anchor(p);reserved.set(key(a.x,a.y),name);}
  const edgeKey=(a,b)=>a<b?`${a}|${b}`:`${b}|${a}`;
  const bucket=(x,y)=>key(Math.floor(x/64),Math.floor(y/64));
  const boxes=items.filter(i=>i.type==='node').map(i=>({...i,bottomClearance:18}));
  for(const item of items)if(item.type==='boundary'&&item.kind!=='rule')boxes.push({x:item.x+12,y:item.y+7,width:item.label.length*7+8,height:18,bottomClearance:6});
  for(const box of boxes) {
    for(let x=Math.floor((box.x-6)/64);x<=Math.floor((box.x+box.width+6)/64);x++)
      for(let y=Math.floor((box.y-6)/64);y<=Math.floor((box.y+box.height+box.bottomClearance)/64);y++) {
        const k=key(x,y);if(!obstacles.has(k))obstacles.set(k,[]);obstacles.get(k).push(box);
      }
  }
  const blocked=(x,y)=> (obstacles.get(bucket(x,y))??[]).some(b=>x>b.x-6&&x<b.x+b.width+6&&y>b.y-6&&y<b.y+b.height+b.bottomClearance);
  const margin=40+step*[...ports.values()].filter(ends=>ends.length>1).length;
  const bounds=items.reduce((b,i)=>({left:Math.min(b.left,i.x-margin),top:Math.min(b.top,i.y-margin),right:Math.max(b.right,i.x+i.width+margin),bottom:Math.max(b.bottom,i.y+i.height+margin)}),{left:0,top:0,right:0,bottom:0});
  for(const [name,ends] of ports) {
    if(ends.length<2)continue;
    const tree=new Map(),terminals=new Map();
    const connect=(a,b)=>{if(a===b)return;for(const [u,v] of [[a,b],[b,a]]){if(!tree.has(u))tree.set(u,new Set());tree.get(u).add(v);}};
    const anchors=ends.map(anchor);
    const cells=new Set([key(anchors[0].x,anchors[0].y)]);
    let minX=anchors[0].x,maxX=minX,minY=anchors[0].y,maxY=minY;
    for(const terminal of anchors.slice(1)) {
      const start=key(terminal.x,terminal.y);if(cells.has(start))continue;
      const heuristic=(x,y)=>Math.max(minX-x,0,x-maxX)+Math.max(minY-y,0,y-maxY);
      const queue=[],distance=new Map(),parents=new Map();let serial=0;
      const push=value=>{queue.push(value);let i=queue.length-1;while(i){const p=(i-1)>>1;if(queue[p].score<value.score||(queue[p].score===value.score&&queue[p].serial<value.serial))break;queue[i]=queue[p];i=p;}queue[i]=value;};
      const pop=()=>{const first=queue[0],last=queue.pop();if(queue.length){let i=0;while(i*2+1<queue.length){let c=i*2+1;if(c+1<queue.length&&(queue[c+1].score<queue[c].score||(queue[c+1].score===queue[c].score&&queue[c+1].serial<queue[c].serial)))c++;if(last.score<queue[c].score||(last.score===queue[c].score&&last.serial<queue[c].serial))break;queue[i]=queue[c];i=c;}queue[i]=last;}return first;};
      const initial={x:terminal.x,y:terminal.y,axis:2,cost:0,id:`${start}:2`,score:heuristic(terminal.x,terminal.y),serial:serial++};
      push(initial);distance.set(initial.id,0);let found;
      while(queue.length) {
        const current=pop();if(current.cost!==distance.get(current.id))continue;
        const here=key(current.x,current.y);
        if(cells.has(here)){found=current;break;}
        for(const [dx,dy,axis] of [[step,0,1],[0,step,2],[-step,0,1],[0,-step,2]]) {
          const x=current.x+dx,y=current.y+dy,next=key(x,y),foreign=occupied.get(here);
          if((reserved.has(next)&&reserved.get(next)!==name)||x<bounds.left||x>bounds.right||y<bounds.top||y>bounds.bottom||blocked(x,y)||blocked(current.x+dx/2,current.y+dy/2))continue;
          // Cross another tree straight through; never share a segment or turn at its crossing.
          if(usedEdges.has(edgeKey(here,next))||(foreign&&(foreign.axis===3||foreign.degree!==2||foreign.axis===axis||current.axis!==axis)))continue;
          const incoming=occupied.get(next);
          if(incoming&&(incoming.axis===3||incoming.degree!==2||incoming.axis===axis))continue;
          const cost=current.cost+step+(current.axis&&current.axis!==axis?12:0)+(incoming?32:0),id=`${next}:${axis}`;
          if(cost>=(distance.get(id)??Infinity))continue;
          distance.set(id,cost);parents.set(id,current);
          push({x,y,axis,cost,id,score:cost+heuristic(x,y),serial:serial++});
        }
      }
      if(!found)throw Object.assign(new Error(`Connections unavailable for ${name}. Move relations apart or use Arrange.`),{routing:true});
      for(let current=found;current;current=parents.get(current.id)) {
        const k=key(current.x,current.y),previous=parents.get(current.id);
        cells.add(k);minX=Math.min(minX,current.x);maxX=Math.max(maxX,current.x);minY=Math.min(minY,current.y);maxY=Math.max(maxY,current.y);
        if(previous)connect(k,key(previous.x,previous.y));
      }
    }
    for(const {port,x,y} of anchors) {
      const tip=key(port.x+10,port.y+10),exit=key(port.x+10,port.y+24),aligned=key(x,port.y+24),anchor=key(x,y);
      terminals.set(tip,port.id);connect(tip,exit);connect(exit,aligned);connect(aligned,anchor);
    }
    for(const [a,neighbors] of tree) {
      if(cells.has(a)) {
        let axis=0;const [x,y]=point(a);
        for(const b of neighbors){const [bx,by]=point(b);axis|=bx===x?2:by===y?1:3;if(cells.has(b))usedEdges.add(edgeKey(a,b));}
        occupied.set(a,{axis:axis|(occupied.get(a)?.axis??0),degree:neighbors.size});
      }
      if(neighbors.size>=3){const [x,y]=point(a);add({type:'junction',name,id:`junction:${name}:${a}`,x:x-24,y:y-12,width:48,height:24});}
    }
    const walked=new Set();
    for(const [a,neighbors] of tree) {
      if(neighbors.size===2&&!terminals.has(a))continue;
      for(const b of neighbors) {
        if(walked.has(edgeKey(a,b)))continue;
        const points=[point(a)];let previous=a,current=b;
        while(true) {
          walked.add(edgeKey(previous,current));points.push(point(current));
          if(tree.get(current).size!==2||terminals.has(current))break;
          const next=[...tree.get(current)].find(k=>k!==previous);previous=current;current=next;
        }
        const bends=points.filter((p,i)=>!i||i===points.length-1||!((points[i-1][0]===p[0]&&p[0]===points[i+1][0])||(points[i-1][1]===p[1]&&p[1]===points[i+1][1])));
        const box=points.reduce((r,[x,y])=>({x:Math.min(r.x,x),y:Math.min(r.y,y),right:Math.max(r.right,x),bottom:Math.max(r.bottom,y)}),{x:Infinity,y:Infinity,right:-Infinity,bottom:-Infinity});
        add({type:'wire',name,from:terminals.get(a)??'',to:terminals.get(current)??'',points:bends,x:box.x-3,y:box.y-3,width:box.right-box.x+6,height:box.bottom-box.y+6,d:wirePath(bends)});
      }
    }
  }
}
const wirePath=points=>points.map(([x,y],i)=>`${i?'L':'M'}${x},${y}`).join(' ');
const NS='http://www.w3.org/2000/svg';
function svgNode(tag,attrs={},text) {
  const node=document.createElementNS(NS,tag);
  for(const [key,value] of Object.entries(attrs))node.setAttribute(key,value);
  if(text!==undefined)node.textContent=text;
  return node;
}
function interactive(node,label,action) {
  node.setAttribute('tabindex','0');node.setAttribute('role','button');node.setAttribute('aria-label',label);
  node.addEventListener('click',event=>{event.stopPropagation();action();});
  node.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();event.stopPropagation();action();}});
}
const canvases=new WeakMap();
let layoutWorker, nextCanvas=0;
const requests=new Map();
function computeLayout(state,scene,done) {
  layoutWorker??=new Worker(new URL('./graph.mjs',import.meta.url),{type:'module'});
  layoutWorker.onmessage=({data})=>{const request=requests.get(data.id);if(request?.version===data.version){requests.delete(data.id);request.done(data.layout,data.error);}};
  const version=(state.version??0)+1;state.version=version;
  const finish=(layout,error)=>{
    if(error){state.loading=false;state.svg.removeAttribute('aria-busy');state.svg.replaceChildren(svgNode('text',{x:20,y:35,class:'graph-empty'},`Diagram error: ${error}`));return;}
    done(layout);
  };
  layoutWorker.onerror=event=>{for(const request of requests.values())request.done(null,event.message);requests.clear();layoutWorker.terminate();layoutWorker=undefined;};
  requests.set(state.id,{version,done:finish});
  layoutWorker.postMessage({id:state.id,version,scene,positions:[...state.positions]});
}
if(typeof document==='undefined'&&typeof self!=='undefined') {
  const scenes=new Map();
  self.onmessage=({data})=>{try{if(data.scene)scenes.set(data.id,data.scene);self.postMessage({id:data.id,version:data.version,layout:layoutScene(scenes.get(data.id),new Map(data.positions))});}catch(error){self.postMessage({id:data.id,version:data.version,error:error.message});}};
}

export function diagramControl(svg,action) {canvases.get(svg)?.control(action);}
export function renderGraph(svg,model,path,options={}) {
  const key=keyOf(path[0]==='program'?path.slice(0,3):path[0]==='query'?['query']:path),state=canvases.get(svg);
  const scene=state?.model===model&&state.key===key?state.scene:diagramScene(model,path);
  const result=renderScene(svg,scene,{...options,key});result.model=model;return result;
}
export function renderScene(svg,scene,options={}) {
  let state=canvases.get(svg);
  const key=options.key??'scene';
  if(!state){state={svg,id:++nextCanvas,positions:new Map(),camera:null};canvases.set(svg,state);state.resize=new ResizeObserver(()=>{if(state.layout&&!state.loading){state.camera=null;drawScene(svg,state);}});state.resize.observe(svg);}
  if(state.key!==key){state.positions.clear();state.camera=null;state.key=key;}
  const changed=state.scene!==scene;
  state.options=options;state.scene=scene;
  if(changed) {
    state.loading=true;svg.setAttribute('aria-busy','true');svg.replaceChildren(svgNode('text',{x:20,y:35,class:'graph-empty'},'Laying out diagram…'));
    svg.onpointerdown=svg.onpointermove=svg.onpointerup=svg.onkeydown=null;state.control=null;
    computeLayout(state,scene,layout=>{state.layout=layout;state.loading=false;svg.removeAttribute('aria-busy');drawScene(svg,state);});
  } else if(state.layout&&!state.loading)drawScene(svg,state);
  return state;
}
function drawScene(svg,state) {
  const {scene,options}=state;
  const size=()=>({width:svg.clientWidth||800,height:svg.clientHeight||480});
  const fit=()=>{const {width,height}=size();const scale=Math.max(.15,Math.min(1,width/state.layout.width,height/state.layout.height));state.camera={x:(state.layout.width-width/scale)/2,y:(state.layout.height-height/scale)/2,scale};};
  if(!state.camera)fit();
  function paint() {
    const {width,height}=size(),{x,y,scale}=state.camera;
    const view={x,y,width:width/scale,height:height/scale};
    svg.setAttribute('viewBox',`${x} ${y} ${view.width} ${view.height}`);
    svg.setAttribute('role','group');svg.setAttribute('tabindex','0');svg.setAttribute('aria-label',options.label??'Relational diagram');
    svg.replaceChildren(svgNode('title',{},options.label??'Relational diagram'));
    const boundaries=svgNode('g'),wires=svgNode('g',{'aria-hidden':'true'}),nodes=svgNode('g');svg.append(boundaries,wires,nodes);
    for(const item of visibleItems(state.layout,view)) {
      const {x,y,width,height}=item;
      if(item.type==='wire'){const wire=svgNode('path',{d:item.d,class:`wire ${relationColor(item.name)}`,'data-variable':item.name,'data-from':item.from,'data-to':item.to,'data-points':JSON.stringify(item.points)});wire.append(svgNode('title',{},item.name));if(!options.readonly&&options.selected?.port!==undefined)wire.addEventListener('click',()=>options.onConnect?.(item.name));wires.append(svgNode('path',{d:item.d,class:'wire-clearance'}),wire);continue;}
      const selected=options.selected&&keyOf(options.selected.path)===keyOf(item.path)&&(!options.selected.compartment||item.type==='boundary');
      const group=svgNode('g',{'data-item':item.id,'data-type':item.type,...(item.name?{'data-variable':item.name}:{})});
      if(item.name)group.append(svgNode('title',{},item.name));
      if(item.type==='boundary') {
        group.setAttribute('class',`diagram-boundary ${item.kind}${item.region?' region':''}${selected?' selected':''}${options.insertion&&keyOf(options.insertion)===keyOf(item.path)&&(item.region||item.kind==='branch')?' insertion-target':''}`);
        if(item.kind==='branch')group.append(svgNode('rect',{x,y,width,height,fill:'transparent',class:'compartment-hit'}));
        if(item.kind==='branch')group.append(svgNode('line',{x1:x,y1:y,x2:x+width,y2:y,class:'branch-divider'}));
        else group.append(svgNode('rect',{x,y,width,height,rx:10}));
        group.append(svgNode('text',{x:x+14,y:y+23,class:'boundary-label'},item.label));
        if(!options.readonly)interactive(group,`Select ${item.label}`,()=>options.onSelect?.({path:item.path,compartment:item.region||item.kind==='branch'}));
        boundaries.append(group);continue;
      }
      if(item.type==='node') {
        group.setAttribute('class',`relation-node ${relationColor(item.relation??'equal')}${selected?' selected':''}`);
        group.append(svgNode('rect',{x,y,width,height,rx:6}),svgNode('text',{x:x+12,y:y+28,class:'node-name'},item.label));
        if(item.kind==='equal')group.append(svgNode('title',{},`${item.args[0]} = ${item.args[1]}`));
        if(item.occurrence!==undefined)group.append(svgNode('title',{},`Occurrence ${item.occurrence}`));
        if(!options.readonly)interactive(group,`Select ${item.label}`,()=>{if(!state.moved)options.onSelect?.({path:item.path});});
      } else if(item.type==='port') {
        group.setAttribute('class',`port ${relationColor(item.relation??'equal')}${selected&&options.selected.port===item.port?' selected':''}`);
        group.append(svgNode('circle',{cx:x+10,cy:y+10,r:10}),svgNode('text',{x:x+10,y:y+13.5,'text-anchor':'middle'},item.port+1));
        if(!options.readonly)interactive(group,`Select port ${item.port+1} of ${item.relation??'equality'}, connected to ${item.name}`,()=>{if(!state.moved)options.onSelect?.({path:item.path,port:item.port});});
      } else {
        group.setAttribute('class','junction');group.append(svgNode('rect',{x,y,width,height,fill:'transparent'}),svgNode('circle',{cx:x+24,cy:y+12,r:5}));
        if(!options.readonly&&options.selected?.port!==undefined)interactive(group,`Connect selected port to ${item.name}`,()=>{if(!state.moved)options.onConnect?.(item.name);});
      }
      nodes.append(group);
    }
    if(state.layout.routingError)svg.append(svgNode('text',{x:x+12,y:y+24,class:'graph-error',role:'alert'},state.layout.routingError));
    if(!state.layout.items.length)nodes.append(svgNode('text',{x:x+view.width/2,y:y+view.height/2,class:'graph-empty','text-anchor':'middle'},'Empty state'));
    options.onView?.(Math.round(scale*100));
  }
  const zoom=(factor,point={x:size().width/2,y:size().height/2})=>{
    const old=state.camera.scale,next=Math.min(4,Math.max(.15,old*factor));
    state.camera.x+=point.x/old-point.x/next;state.camera.y+=point.y/old-point.y/next;state.camera.scale=next;paint();
  };
  state.control=action=>{if(action==='expand'){svg.parentElement.requestFullscreen().then(()=>{fit();paint();svg.focus();}).catch(error=>{svg.append(svgNode('text',{x:state.camera.x+20,y:state.camera.y+35,class:'graph-empty'},error.message));});return;}if(action==='fit')fit();else if(action==='layout'){state.positions.clear();computeLayout(state,null,layout=>{state.layout=layout;fit();paint();});return;}else if(action==='in')return zoom(1.25);else if(action==='out')return zoom(.8);paint();};
  svg.onpointerover=event=>{
    const variable=event.target.closest('[data-variable]')?.dataset.variable;
    svg.toggleAttribute('data-highlight',variable!==undefined);
    for(const element of svg.querySelectorAll('[data-variable]'))element.classList.toggle('highlight',variable!==undefined&&element.dataset.variable===variable);
  };
  svg.onpointerleave=()=>svg.removeAttribute('data-highlight');
  svg.onwheel=event=>{event.preventDefault();const rect=svg.getBoundingClientRect();zoom(Math.exp(-event.deltaY*.002),{x:event.clientX-rect.left,y:event.clientY-rect.top});};
  const cancelDrag=()=>{if(state.drag?.offset)state.positions.set(state.drag.item.id,state.drag.offset);state.drag=null;state.moved=false;paint();};
  svg.onkeydown=event=>{
    const target=event.target.closest('[data-item]');
    if(event.altKey&&['ArrowLeft','ArrowRight','ArrowUp','ArrowDown'].includes(event.key)&&target?.dataset.type==='node') {
      event.preventDefault();const id=target.dataset.item,offset=state.positions.get(id)??{x:0,y:0};
      state.positions.set(id,{x:offset.x+(event.key==='ArrowLeft'?-20:event.key==='ArrowRight'?20:0),y:offset.y+(event.key==='ArrowUp'?-20:event.key==='ArrowDown'?20:0)});
      computeLayout(state,null,layout=>{state.layout=layout;drawScene(svg,state);for(const element of svg.querySelectorAll('[data-item]'))if(element.dataset.item===id&&element.dataset.type===target.dataset.type)element.focus();});return;
    }
    if(event.key==='Escape'){cancelDrag();return;}
    if(event.target!==svg)return;
    if(['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','+','-','0'].includes(event.key))event.preventDefault();
    if(event.key==='+'||event.key==='=')zoom(1.25);else if(event.key==='-')zoom(.8);else if(event.key==='0'){fit();paint();}
    else {const delta=64/state.camera.scale;if(event.key==='ArrowLeft')state.camera.x-=delta;if(event.key==='ArrowRight')state.camera.x+=delta;if(event.key==='ArrowUp')state.camera.y-=delta;if(event.key==='ArrowDown')state.camera.y+=delta;paint();}
  };
  svg.onpointerdown=event=>{
    if(event.button!==0&&event.button!==1)return;
    const target=event.target.closest('[data-item]');const item=target&&state.layout.items.find(i=>i.id===target.dataset.item&&i.type===target.dataset.type);
    const movable=item&&item.type==='node';
    state.moved=false;
    state.drag={startX:event.clientX,startY:event.clientY,x:state.camera.x,y:state.camera.y,item:event.button===1?null:item,
      offset:movable?(state.positions.get(item.id)??{x:0,y:0}):null};
  };
  svg.onpointermove=event=>{
    const drag=state.drag;if(!drag)return;
    const dx=(event.clientX-drag.startX)/state.camera.scale,dy=(event.clientY-drag.startY)/state.camera.scale;
    if(Math.abs(dx)+Math.abs(dy)<4&&!state.moved)return;
    state.moved=true;svg.setPointerCapture(event.pointerId);
    if(drag.item?.type==='port'&&!options.readonly) {
      paint();const rect=svg.getBoundingClientRect(),x=state.camera.x+(event.clientX-rect.left)/state.camera.scale,y=state.camera.y+(event.clientY-rect.top)/state.camera.scale;
      svg.append(svgNode('path',{d:`M${drag.item.x+10},${drag.item.y+10} L${x},${y}`,class:'wire connection-preview'}));
    } else if(drag.offset) {
      state.positions.set(drag.item.id,{x:drag.offset.x+dx,y:drag.offset.y+dy});
      for(const element of svg.querySelectorAll('[data-item]'))if(element.dataset.item===drag.item.id||element.dataset.item.startsWith(drag.item.id+':'))element.setAttribute('transform',`translate(${dx} ${dy})`);
      for(const wire of svg.querySelectorAll('[data-from]')) {
        const from=wire.dataset.from.startsWith(drag.item.id+':'),to=wire.dataset.to.startsWith(drag.item.id+':');
        if(from||to){const points=JSON.parse(wire.dataset.points);if(from){points[0][0]+=dx;points[0][1]+=dy;}if(to){points.at(-1)[0]+=dx;points.at(-1)[1]+=dy;}const d=wirePath(points);wire.setAttribute('d',d);wire.previousElementSibling.setAttribute('d',d);}
      }
    } else {state.camera.x=drag.x-dx;state.camera.y=drag.y-dy;paint();}
  };
  svg.onpointerup=event=>{
    const drag=state.drag;state.drag=null;
    if(drag?.item?.type==='port'&&state.moved&&!options.readonly) {
      const target=document.elementFromPoint(event.clientX,event.clientY)?.closest('[data-variable]');
      if(target)options.onWire?.({path:drag.item.path,port:drag.item.port},target.dataset.variable);
    }
    if(state.moved&&drag?.offset)computeLayout(state,null,layout=>{state.layout=layout;drawScene(svg,state);});
    else if(state.moved)paint();
    setTimeout(()=>{state.moved=false;},0);
  };
  svg.onpointercancel=cancelDrag;
  paint();return state;
}
