import * as graph from './graph.mjs';

const check = (value, message) => { if (!value) throw new Error(message); };
const equal = (actual, expected, message) => check(JSON.stringify(actual) === JSON.stringify(expected), message);
const atom = relation => ({kind:'atom',atom:{relation,args:[]}});
const model = {query:{kind:'and',items:[atom('a'),atom('b'),atom('c')]}};
const path = i => ['query','items',i];
const id = i => JSON.stringify(path(i));

check(typeof graph.graphPositions === 'function', 'Layout snapshot API is available');
check(typeof graph.restoreGraphPositions === 'function', 'Layout restoration API is available');
check(typeof graph.exportGraphSvg === 'function', 'Full SVG export API is available');
const scene = graph.diagramScene(model,['query']);
const original = graph.layoutScene(scene);
const moved = graph.layoutScene(scene,new Map([[id(0),{x:-200,y:40}],[id(1),{x:-200,y:40}]]));
for (const i of [0,1,2]) {
  const before=original.items.find(n=>n.type==='node'&&n.id===id(i));
  const after=moved.items.find(n=>n.type==='node'&&n.id===id(i));
  equal([after.x-before.x,after.y-before.y],i<2?[-200,40]:[0,0],'Offsets move only their own nodes');
}
const single=graph.diagramScene({query:atom('single')},['query']);
const singleBefore=graph.layoutScene(single).items.find(item=>item.type==='node');
const singleAfter=graph.layoutScene(single,new Map([['["query"]',{x:40,y:20}]])).items.find(item=>item.type==='node');
equal([singleAfter.x-singleBefore.x,singleAfter.y-singleBefore.y],[40,20],'A node sharing a compartment path moves once');

// Import this module in a browser and await browserChecks() to exercise the real DOM and worker.
export async function browserChecks() {
  const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
  svg.style.cssText='width:800px;height:480px';document.body.append(svg);
  let selection=[],layouts=[];
  const options={onSelection:value=>{selection=value;},onLayout:value=>layouts.push(value)};
  const ready=async()=>{for(let i=0;i<500;i++){if(!svg.hasAttribute('aria-busy'))return;await new Promise(r=>setTimeout(r,10));}throw Error('Layout timeout');};
  const node=i=>[...svg.querySelectorAll('[data-type="node"]')].find(n=>n.dataset.item===id(i));
  const click=(i,shiftKey=false)=>node(i).dispatchEvent(new MouseEvent('click',{bubbles:true,shiftKey}));
  const pointer=(type,target,x,y,extra={})=>svg[`onpointer${type}`]({target,clientX:x,clientY:y,button:0,pointerId:1,preventDefault(){},...extra});
  // Synthetic pointers do not have a browser capture session.
  svg.setPointerCapture=()=>{};
  try {
    const state=graph.renderGraph(svg,model,['query'],options);await ready();
    click(0);click(1,true);
    equal(selection.map(s=>s.path),[path(0),path(1)],'Shift-click adds a second node');
    check(node(0).classList.contains('selected')&&node(1).classList.contains('selected'),'Both nodes are highlighted');
    pointer('down',node(0),100,100);pointer('move',svg,140,120);pointer('up',svg,140,120);await ready();
    equal(graph.graphPositions(svg),[[id(0),{x:40,y:20}],[id(1),{x:40,y:20}]],'Dragging moves the whole selection');
    check(layouts.length===1,'Completed move emits one layout');
    await new Promise(r=>setTimeout(r,0));click(1,true);
    equal(selection.map(s=>s.path),[path(0)],'Shift-click toggles a member off');
    pointer('down',node(0),100,100);pointer('move',svg,180,160);svg.onkeydown({key:'Escape',target:svg});
    equal(graph.graphPositions(svg),[[id(0),{x:40,y:20}],[id(1),{x:40,y:20}]],'Escape restores the entire layout');
    graph.restoreGraphPositions(svg,[[id(2),{x:-1200,y:1600}]]);await ready();
    check(layouts.length===1,'Restoring does not echo onLayout');
    const snapshot=graph.graphPositions(svg);snapshot[0][1].x=999;
    equal(graph.graphPositions(svg),[[id(2),{x:-1200,y:1600}]],'Snapshots are detached');
    let rejected=false;try{graph.restoreGraphPositions(svg,[[id(0),{x:NaN,y:0}]]);}catch{rejected=true;}
    check(rejected,'Non-finite offsets are rejected');
    equal(graph.graphPositions(svg),[[id(2),{x:-1200,y:1600}]],'Invalid restore is atomic');
    const camera={...state.camera},before=svg.innerHTML;
    const exported=new DOMParser().parseFromString(graph.exportGraphSvg(svg),'image/svg+xml');
    check(!exported.querySelector('parsererror'),'Export is well-formed SVG');
    check(exported.querySelectorAll('[data-type="node"]').length===3,'Export includes offscreen nodes');
    const box=exported.documentElement.getAttribute('viewBox').split(' ').map(Number);
    check(box[0]<-600&&box[3]>1500,'Export includes negative and distant geometry');
    check(exported.querySelector('.node-name').hasAttribute('style'),'Export embeds computed styles');
    equal(state.camera,camera,'Export preserves camera');check(svg.innerHTML===before,'Export preserves live DOM');
    check(graph.focusGraphPath(svg,path(2)),'Search finds a node outside the viewport');
    check(document.activeElement===node(2),'Search paints and focuses the node');
    const distant=state.layout.items.find(item=>item.type==='node'&&item.id===id(2));
    check(Math.abs(state.camera.x+400/state.camera.scale-distant.x-distant.width/2)<.001,'Search centers camera on the node');
    check(!graph.focusGraphPath(svg,['query','items',100]),'Missing search result is reported');
    graph.diagramControl(svg,'layout');await ready();
    equal(graph.graphPositions(svg),[],'Arrange resets offsets');check(layouts.length===2,'Arrange emits layout');
    graph.renderScene(svg,scene,{...options,key:'new',positions:[[id(1),{x:70,y:80}]]});await ready();
    equal(graph.graphPositions(svg),[[id(1),{x:70,y:80}]],'New key restores supplied layout even for same scene');
    graph.renderScene(svg,scene,{...options,key:'new',positions:[]});await ready();
    equal(graph.graphPositions(svg),[],'Changed supplied positions restore within the same key');
    options.onSelection=value=>{selection=value;graph.renderScene(svg,scene,{...options,key:'new',selection:value,positions:[]});};
    click(0);click(1,true);
    pointer('down',node(0),100,100);pointer('move',svg,140,120);
    graph.renderScene(svg,scene,{...options,key:'new',selection,positions:[]});
    check(state.drag,'Repainting with serialized positions preserves an active drag');
    pointer('move',svg,160,130);pointer('up',svg,160,130);await ready();
    equal(graph.graphPositions(svg),[[id(0),{x:60,y:30}],[id(1),{x:60,y:30}]],'Group movement survives selection callback repaint');
    await new Promise(r=>setTimeout(r,0));
    graph.renderScene(svg,scene,{...options,key:'new',selection:[],positions:graph.graphPositions(svg)});
    const target=state.layout.items.find(item=>item.type==='node'&&item.id===id(2)),rect=svg.getBoundingClientRect();
    const client=(x,y)=>[rect.left+(x-state.camera.x)*state.camera.scale,rect.top+(y-state.camera.y)*state.camera.scale];
    const start=client(target.x+target.width/2-5,target.y-2),end=client(target.x+target.width+2,target.y+target.height+2);
    pointer('down',svg,...start,{shiftKey:true});pointer('move',svg,...end);pointer('up',svg,...end);
    equal(selection.map(value=>value.path),[path(2)],'Marquee selects intersecting nodes in graph coordinates');
    await ready();await new Promise(r=>setTimeout(r,0));
    let saved=graph.graphPositions(svg),notifications=0;
    const repaint=()=>graph.renderScene(svg,scene,{key:'new',selection:[{path:path(0)}],positions:structuredClone(saved),onLayout:save,onView:()=>equal(graph.graphPositions(svg),saved,'Layout callback precedes repaint hooks')});
    const save=positions=>{saved=positions;notifications++;repaint();};
    repaint();
    pointer('down',node(0),100,100);pointer('move',svg,110,110);repaint();pointer('up',svg,110,110);await ready();
    check(notifications===1,'Layout callback can synchronously repaint using saved positions');
    equal(graph.graphPositions(svg),saved,'Callback repaint keeps the completed layout');
    graph.renderScene(svg,scene,{key:'new',selection:[{path:path(0)}]});
    pointer('down',node(0),100,100);pointer('move',svg,120,120);
    graph.restoreGraphPositions(svg,[[id(1),{x:15,y:25}]]);
    check(state.drag,'Explicit restore waits for active gesture');pointer('up',svg,120,120);await ready();
    equal(graph.graphPositions(svg),[[id(1),{x:15,y:25}]],'Deferred restore takes effect after release');
    graph.renderScene(svg,scene,{key:'new',selection:[{path:['program','rules',0,'body']},{path:path(0)}],onSelection:value=>{selection=value;}});
    await new Promise(r=>setTimeout(r,0));click(1,true);
    equal(selection.map(value=>value.path),[path(0),path(1)],'Selection never carries paths from another graph');
    svg.onkeydown({target:node(0),altKey:true,key:'ArrowRight',preventDefault(){}});await ready();
    equal(new Map(graph.graphPositions(svg)).get(id(0)),{x:20,y:0},'Keyboard movement moves the first selected node');
    equal(new Map(graph.graphPositions(svg)).get(id(1)),{x:35,y:25},'Keyboard movement moves the other selected node');
    graph.renderScene(svg,scene,{key:'new',readonly:true});
    const locked=graph.graphPositions(svg);pointer('down',node(0),100,100);pointer('move',svg,120,120);pointer('up',svg,120,120);
    equal(graph.graphPositions(svg),locked,'Read-only graphs pan without moving nodes');
    const wired={query:{kind:'and',items:['X','Y','X','X'].map((variable,i)=>({kind:'atom',atom:{relation:`p${i}`,args:[variable]}}))}};
    let connection,wire,singleSelection;
    graph.renderGraph(svg,wired,['query'],{key:'query:wires',selection:[{path:path(0),port:0}],onConnect:value=>{connection=value;},onWire:(port,name)=>{wire=[port,name];},onSelect:value=>{singleSelection=value;}});
    check(graph.focusGraphPath(svg,path(1)),'Search can queue during worker layout');await ready();
    check(document.activeElement===node(1),'Queued search focuses after layout');
    graph.diagramControl(svg,'fit');
    const junction=svg.querySelector('.junction');check(junction,'Shared variable has a junction');
    await new Promise(r=>setTimeout(r,0));junction.dispatchEvent(new MouseEvent('click',{bubbles:true}));
    equal(connection,'X','Selection-array ports retain junction connection semantics');
    const port=i=>[...svg.querySelectorAll('[data-type="port"]')].find(element=>element.dataset.item===`${id(i)}:0`);
    port(0).dispatchEvent(new MouseEvent('click',{bubbles:true}));
    equal(singleSelection,{path:path(0),port:0},'Single onSelect preserves numbered port payload');
    const from=port(0).getBoundingClientRect(),to=port(1).getBoundingClientRect();
    pointer('down',port(0),from.x+10,from.y+10);pointer('move',svg,to.x+10,to.y+10);pointer('up',svg,to.x+10,to.y+10);
    equal(wire,[{path:path(0),port:0},'Y'],'Port drag still calls onWire with the target variable');
    equal(graph.graphPositions(svg),[],'Wiring never persists a node movement');
  } finally {graph.disposeGraph(svg);svg.remove();}
  return 'Graph editing browser checks passed';
}
console.log('Graph editing pure checks passed');

// Run in a browser: export uses the real layout worker, SVG geometry and styles.
export async function answerExportBrowserChecks() {
  const args=Array.from({length:300},(_,i)=>`V${i}`);
  const answer={number:7,completion:'18446744073709551615',alternative:'19',
    bindings:Array.from({length:30},(_,slot)=>({slot,name:slot===29?'Last <binding> & name':'Q'+slot,variable:String(slot)})),
    facts:[{kind:'atom',path:[0],relation:'wide',args,occurrence:'18446744073709551615'}],
    pending:[{event:'51',scene:{kind:'or',path:[1],args:[],children:[
      {kind:'and',path:[2],args:[],children:[
        {kind:'atom',path:[3],relation:'pending_only',args:['V901','V902']},
        {kind:'equal',path:[4],args:['V903','V904']}]},
      {kind:'true',path:[5],args:[]}, {kind:'fail',path:[6],args:[]}]}},
      {event:'52',scene:{kind:'and',path:[7],args:[],children:[]}},
      {event:'18446744073709551615',scene:{kind:'or',path:[8],args:[],children:[]}}]};
  const before=JSON.stringify(answer), children=document.body.childElementCount;
  const svgText=await graph.exportAnswerSvg(answer);
  const exported=new DOMParser().parseFromString(svgText,'image/svg+xml');
  check(!exported.querySelector('parsererror'),'Answer export is well-formed SVG');
  const text=[...exported.querySelectorAll('text')].map(node=>node.textContent).join('\n');
  for(const value of ['Completion 18446744073709551615','Alternative 19','Last <binding> & name = V29','Pending event 51','Pending event 52','Pending event 18446744073709551615','pending_only / 2','true','fail'])check(text.includes(value),`Export includes ${value}`);
  check(exported.querySelectorAll('.port').length===304,'Every fact and pending port is exported');
  equal([...exported.querySelectorAll('.port')].slice(0,300).map(n=>n.querySelector('text').textContent),args.map((_,i)=>String(i+1)),'Fact ports retain their order');
  check(exported.querySelectorAll('.diagram-boundary.branch').length===3,'All pending alternatives are present');
  check(exported.querySelectorAll('.bindings text').length===30,'Bindings are not paginated');
  equal(JSON.parse(exported.querySelector('metadata').textContent),answer,'SVG retains complete answer metadata');
  check(exported.querySelector('.node-name').style.fontSize,'Styles are embedded');
  const box=exported.documentElement.getAttribute('viewBox').split(' ').map(Number);
  check(box[2]>8000&&box[3]>600,'Export bounds include wide facts and the binding header');
  equal(JSON.stringify(answer),before,'Export does not mutate the answer');
  equal(document.body.childElementCount,children,'Temporary export elements are cleaned up');
  const rendered=document.importNode(exported.documentElement,true);document.body.append(rendered);
  try {
    for(const region of rendered.querySelectorAll('.diagram-boundary.region')) {
      const label=region.querySelector('.boundary-label'),rect=region.querySelector('rect');
      check(label.getBBox().width<=Number(rect.getAttribute('width'))-28,`Section width includes ${label.textContent}: ${label.getBBox().width} within ${rect.getAttribute('width')}`);
    }
  } finally {rendered.remove();}
  const empty=await graph.exportAnswerSvg({number:1,completion:'1',alternative:'0',bindings:[],facts:[],pending:[]});
  check(empty.includes('No bindings')&&empty.includes('No facts')&&empty.includes('No pending bodies'),'Empty answer is explicit');
  const WorkerClass=globalThis.Worker;
  try {
    for(const mode of ['message','error','constructor','post','routing']) {
      globalThis.Worker=class {
        constructor(){if(mode==='constructor')throw new Error('worker constructor failed');}
        postMessage(data){
          if(data.dispose)return;
          if(mode==='post')throw new Error('worker post failed');
          queueMicrotask(()=>mode==='routing'?this.onmessage({data:{id:data.id,version:data.version,layout:{items:[],index:null,width:100,height:100,routingError:'worker routing failed'}}})
            :mode==='message'?this.onmessage({data:{id:data.id,version:data.version,error:'worker message failed'}}):this.onerror({message:'worker error failed'}));
        }
        terminate(){}
      };
      const isolated=await import(`./graph.mjs?answer-export-failure=${mode}`);
      let error;
      try { await isolated.exportAnswerSvg(answer); } catch(caught) { error=caught; }
      check(error?.message===`worker ${mode} failed`,`${mode} failure rejects export`);
      equal(document.body.childElementCount,children,`${mode} failure cleans up temporary SVG`);
    }
  } finally {globalThis.Worker=WorkerClass;}
  return 'Complete answer SVG and worker failure checks passed';
}
