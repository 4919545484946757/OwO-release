import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source=readFileSync(new URL('../panels/workflow.panel.js',import.meta.url),'utf8');
const defer=()=>{let resolve;const promise=new Promise(r=>resolve=r);return {promise,resolve};};
const flush=()=>new Promise(r=>setTimeout(r,0));
function harness(overrides={}) {
  const nodes=new Map(),streams=[],writes=[],timers=new Map();let timerId=0;
  function element(){return {innerHTML:'',textContent:'',value:'',hidden:false,disabled:false,handlers:{},attributes:{},children:[],isConnected:true,
    addEventListener(name,fn){this.handlers[name]=fn;},setAttribute(name,value){this.attributes[name]=value;},getAttribute(name){return this.attributes[name];},
    querySelectorAll(){return [];},appendChild(child){this.children.push(child);},removeChild(child){this.children.splice(this.children.indexOf(child),1);},get firstElementChild(){return this.children[0];}};}
  function node(id){if(!nodes.has(id))nodes.set(id,element());return nodes.get(id);}
  const root={innerHTML:'',isConnected:true,querySelector:selector=>node(selector.slice(1))};
  const window={OwoPanels:{},location:{origin:'http://localhost'}};
  vm.runInNewContext(source,{window,document:{createElement:element},Promise,JSON,String,Number,Math,Object,Array,Error,AbortController,
    setTimeout(fn,ms){const id=++timerId;timers.set(id,{fn,ms});return id;},clearTimeout(id){timers.delete(id);}});
  const helpers={get:async path=>path==='/workflow'?{flows:[{name:'safe',path:'safe.owflow'}]}:path.endsWith('/runs')?{runs:[]}:path.includes('/run/')?{run_id:path.split('/').pop(),state:'succeeded',steps:[]}:{valid:true,issues:[],definition:{name:'Safe assert',version:1,steps:[{kind:'assert',id:'check',expr:'true'}]}},
    post:async(path,body)=>{writes.push({path,body});return {run_id:'run-1'};},
    openEventStream(path,options){streams.push({path,options});return new Promise((resolve,reject)=>{options.signal.addEventListener('abort',()=>reject(new Error('aborted')));});},...overrides};
  const panel=window.OwoPanels.workflow;panel.mount(root,helpers);
  return {panel,root,node,nodes,writes,streams,timers,helpers,remount(){panel.dispose();nodes.clear();panel.mount(root,helpers);}};
}

test('workflow definition uses actual internally tagged steps and explicit backend choice',async()=>{
  const h=harness();await h.panel._test.loadFlow('safe');
  assert.match(h.node('owo-workflow-runner').innerHTML,/条件检查 · true/);
  assert.match(h.node('owo-workflow-runner').innerHTML,/沙箱（默认/);
  h.node('owo-workflow-backend').value='real';await h.panel._test.runFlow('safe','[]');assert.equal(h.writes.length,0);
  await h.panel._test.runFlow('safe','{"safe":true}');assert.equal(h.writes.length,1);assert.equal(h.writes[0].body.backend,'real');
  assert.match(h.node('owo-workflow-runner').innerHTML,/运行参数/,'a snapshot must preserve the editable definition form');h.panel.dispose();
});

test('workflow start lock survives remount and blocks duplicate starts',async()=>{
  const pending=defer(),writes=[];const h=harness({post:(path,body)=>{writes.push({path,body});return pending.promise;}});
  await h.panel._test.loadFlow('safe');const task=h.panel._test.runFlow('safe','{}');h.remount();await flush();
  assert.equal(h.node('owo-workflow-run-this').disabled,true);await h.panel._test.runFlow('safe','{}');assert.equal(writes.length,1);
  pending.resolve({run_id:'run-1'});await task;assert.equal(h.node('owo-workflow-run-this').disabled,false);h.panel.dispose();
});

test('workflow streams use authenticated transport, decode nested frames and abort on disposal',async()=>{
  const h=harness();await h.panel._test.viewRun('run-1',true);
  const {path,options}=h.streams[0];assert.equal(path,'/workflow/run/run-1/events');assert.ok(options.signal);
  options.onOpen(200);options.onEvent({event:'message',data:'event: action_start\ndata: {"detail":"<unsafe>","ts":"now"}\n\n'});
  const children=h.node('owo-workflow-events').children;assert.equal(children.length,1);assert.match(children[0].innerHTML,/action_start/);assert.match(children[0].innerHTML,/&lt;unsafe&gt;/);
  for(let i=0;i<205;i++)options.onEvent({data:JSON.stringify({detail:'event '+i})});assert.equal(children.length,200);
  h.panel.dispose();assert.equal(options.signal.aborted,true);await flush();assert.equal(h.timers.size,0);
});

test('later selected workflow run cannot be repainted by an earlier snapshot',async()=>{
  const pending=defer();const h=harness({get:path=>path==='/workflow'?Promise.resolve({flows:[]}):path.endsWith('old')?pending.promise:Promise.resolve({run_id:'new',state:'succeeded',steps:[]})});
  const first=h.panel._test.viewRun('old',true);await h.panel._test.viewRun('new',true);pending.resolve({run_id:'old',state:'failed',steps:[]});await first;
  assert.match(h.node('owo-workflow-snapshot').innerHTML,/new/);assert.doesNotMatch(h.node('owo-workflow-snapshot').innerHTML,/old/);h.panel.dispose();
});

test('editing pending validation invalidates its response but releases the submit lock',async()=>{
  const pending=defer();const h=harness({post:()=>pending.promise});h.node('owo-workflow-validate-dsl').value='{"id":"before"}';
  const task=h.panel._test.validate();h.node('owo-workflow-validate-dsl').value='{"id":"after"}';h.node('owo-workflow-validate-dsl').handlers.input();
  pending.resolve({valid:true,issues:[]});await task;assert.match(h.node('owo-workflow-validate-result').textContent,/尚未校验/);assert.equal(h.node('owo-workflow-validate-btn').disabled,false);h.panel.dispose();
});

test('approval and abort share one action lock and require an active snapshot',async()=>{
  const pending=defer(),writes=[];const h=harness({post:(path,body)=>{writes.push({path,body});return pending.promise;}});await h.panel._test.viewRun('run-1',true);
  await h.panel._test.startRunAction('run-1','abort',{});assert.equal(writes.length,0,'terminal run cannot be aborted');
  h.panel._test.renderSnapshot({run_id:'run-1',state:'waiting_approval',pending_approval:{id:'approval-1',prompt:'Confirm safe assert'},steps:[]});
  const task=h.panel._test.decideApproval('run-1','reject');await h.panel._test.startRunAction('run-1','abort',{});assert.equal(writes.length,1);assert.equal(writes[0].body.decision,'reject');
  pending.resolve({ok:true});await task;h.panel.dispose();
});

test('malformed catalog is an error and stale refresh cannot replace newer data',async()=>{
  const pending=defer();let calls=0;const h=harness({get:()=>++calls===1?pending.promise:Promise.resolve({})});await h.panel.refresh();
  assert.match(h.node('owo-workflow-list').textContent,/失败/);pending.resolve({flows:[]});await flush();assert.match(h.node('owo-workflow-list').textContent,/失败/);h.panel.dispose();
});


test('terminal event drains the remaining replay before closing the stream',async()=>{
  const h=harness();assert.match(h.panel.nav(),/data-layout="custom"/,'panel must opt out of legacy column sizing');
  await h.panel._test.viewRun('run-1',true);const {options}=h.streams[0];
  options.onEvent({data:'event: state_change\ndata: {"state":"failed"}\n\n'});
  options.onEvent({data:'event: rollback\ndata: {"detail":"checkpoint restored"}\n\n'});
  assert.equal(options.signal.aborted,false,'replay frames following the terminal event are retained');
  assert.match(h.node('owo-workflow-events').children.at(-1).innerHTML,/checkpoint restored/);
  const drain=Array.from(h.timers.values()).find(timer=>timer.ms===200);assert.ok(drain);drain.fn();assert.equal(options.signal.aborted,true);h.panel.dispose();
});


test('refreshing after a selected definition disappears removes its runnable form',async()=>{
  const h=harness();await h.panel._test.loadFlow('safe');
  h.helpers.get=async()=>({flows:[]});await h.panel.refresh();
  assert.match(h.node('owo-workflow-runner').textContent,/已不在当前项目/);
  await h.panel._test.runFlow('safe','{}');assert.equal(h.writes.length,0);h.panel.dispose();
});
