import test from "node:test";
import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {runInNewContext} from "node:vm";
import {createRequire} from "node:module";
const require=createRequire(import.meta.url);
const launcher=require("../panels/project-launcher.panel.js");
function homeRuntime() {
  const root={OwoSessionFacts:{fromDetail:()=>({outcome:"completed"})}};
  runInNewContext(readFileSync(new URL("../panels/home.panel.js",import.meta.url),"utf8"),root);
  return root.OwoPanels.home;
}
const host=()=>({innerHTML:"",scrollTop:0,setAttribute(){},contains(){return false;},querySelectorAll(){return [];}});
const helpers=(get,snapshot={sessionsKnown:true,sessions:[],workspace:"C:/project"})=>({esc:String,snapshot:()=>snapshot,get});
test("Home errors and malformed replies stay unavailable rather than reporting an empty project",async()=>{
  const panel=homeRuntime(),el=host();
  await panel.mount(el,helpers(async path=>path==="/teams"?{teams:{}}:path==="/human/inbox"?{items:null}:[]));
  assert.match(el.innerHTML,/团队状态不可用/);
  assert.match(el.innerHTML,/待办状态不可用/);
  assert.doesNotMatch(el.innerHTML,/还没有团队任务/);
  assert.doesNotMatch(el.innerHTML,/<span>待我处理<\/span><strong>0/);
  panel.dispose();
});
test("Home loading and verified empty states are distinct",async()=>{
  const panel=homeRuntime(),el=host();
  let finish;
  const pending=panel.mount(el,helpers(path=>path==="/teams"?new Promise(resolve=>finish=resolve):Promise.resolve({items:[]})));
  assert.match(el.innerHTML,/正在读取/);
  assert.doesNotMatch(el.innerHTML,/还没有团队任务/);
  await Promise.resolve(); await Promise.resolve();
  finish({teams:[]}); await pending;
  assert.match(el.innerHTML,/还没有团队任务/);
  assert.match(el.innerHTML,/<span>待我处理<\/span><strong>0/);
  panel.dispose();
});
test("Home detail failure is a read failure and does not fabricate an interrupted outcome",async()=>{
  const panel=homeRuntime(),el=host();
  await panel.mount(el,helpers(async path=>{
    if(path==="/teams")return {teams:[]};
    if(path==="/human/inbox")return {items:[]};
    throw Error("offline");
  },{sessionsKnown:true,sessions:[{id:"s",title:"task",workspace:"C:/project"}],workspace:"C:/project"}));
  assert.match(el.innerHTML,/状态读取失败/);
  assert.doesNotMatch(el.innerHTML,/终态未确认|已中断/);
  panel.dispose();
});
test("Home sorts without mutating source records and enriches goals from actual audit events",async()=>{
  const panel=homeRuntime(),el=host();
  const teams=[{team_id:"old",created_at:"2026-10-01"},{team_id:"new",created_at:"2026-10-24"}];
  await panel.mount(el,helpers(async path=>path==="/teams"?{teams}:path==="/human/inbox"?{items:[]}:
    {audit_tail:[{event:"team.succeeded",detail:"目标达成：真实目标 "+path}]}));
  assert.ok(el.innerHTML.indexOf('data-home-team="new"')<el.innerHTML.indexOf('data-home-team="old"'));
  assert.match(el.innerHTML,/真实目标/);
  assert.equal(teams[0].team_id,"old");
  assert.equal(teams[0].objective,undefined);
  assert.equal(panel._test.objectiveFromDetail({audit_tail:[{event:"other",detail:"目标达成：非目标"}]}),"");
  panel.dispose();
});
test("Home ignores an older host response after remount",async()=>{
  const panel=homeRuntime(),old=host(),next=host();
  let finish;
  const pending=panel.mount(old,helpers(path=>path==="/teams"?new Promise(resolve=>finish=resolve):Promise.resolve({items:[]})));
  await Promise.resolve(); await Promise.resolve();
  panel.dispose();
  await panel.mount(next,helpers(async path=>path==="/teams"?{teams:[]}:{items:[]}));
  finish({teams:[{team_id:"old-host"}]});await pending;
  assert.doesNotMatch(next.innerHTML,/old-host/);
  panel.dispose();
});
test("Clearing or replacing the project path updates the actual create request",()=>{
  const T=launcher._test;
  T.state.root="C:/old";
  T.setRoot({querySelector:selector=>selector==="#pl-root"?{value:""}:null});
  T.syncStateFromDom();
  assert.equal(T.state.root,"");
  assert.ok(T.validateState({...T.state,objective:"goal"}).some(error=>/项目目录/.test(error)));
  T.setRoot({querySelector:selector=>selector==="#pl-root"?{value:"new-project"}:null});
  T.syncStateFromDom();
  assert.equal(T.buildCreateBody(T.state).workspace.root,"new-project");
  launcher.dispose();
});
test("Write path parsing handles prototype names and rejects rooted paths and terminal parent traversal",()=>{
  const T=launcher._test;
  assert.deepEqual(T.parseWritePaths("constructor,__proto__,constructor"),["constructor","__proto__"]);
  for(const value of ["/absolute","\\absolute","\\\\server\\share","src/..","src\\.."])assert.equal(T.pathIsSafe(value),false,value);
  assert.equal(T.pathIsSafe("constructor/file"),true);
});
test("Leaving creation before deferred dispatch sends no POST to an obsolete page",async()=>{
  const T=launcher._test;let calls=0;
  T.state.objective="goal";T.state.root="C:/project";T.state.readOnly=true;T.state.writePathsRaw="";T.state.treeDepth=2;T.state.creating=false;
  T.setRoot({querySelector:()=>null});T.setTransport({post:async()=>{calls++;return {team_id:"wrong"};}});
  const pending=T.doCreate();launcher.dispose();await pending;
  assert.equal(calls,0);assert.equal(T.state.creating,false);
});

test("Catalog refresh clears a template that is no longer installed instead of submitting a hidden stale selection",async()=>{
  const T=launcher._test;
  T.state.selectedTemplate="removed";
  T.setRoot({querySelector:()=>null});
  T.setTransport({get:async()=>({catalog:[{template_id:"removed",installed:false,roles:[]}]})});
  await T.loadCatalog();
  assert.equal(T.state.selectedTemplate,"");
  assert.equal(T.buildCreateBody(T.state).template_id,undefined);
  launcher.dispose();
});
test("Leaving before deferred template installation sends no obsolete write",async()=>{
  const T=launcher._test;let posts=0;
  T.state.installBusy={};
  T.setRoot({querySelector:()=>null});
  T.setTransport({post:async()=>{posts++;return {installed:true};}});
  const pending=T.installTemplate("unused");
  launcher.dispose();await pending;
  assert.equal(posts,0);
  assert.equal(T.state.installBusy.unused,undefined);
});

test("Fresh worker histories and terminals without stats do not throw or invent completion",()=>{
  const root={};
  for(const file of ["core/completion-record.js","core/session-facts.js"])runInNewContext(readFileSync(new URL("../"+file,import.meta.url),"utf8"),root);
  const fresh=root.OwoSessionFacts.fromDetail({turn_history:{last_turn_id:null,last_terminal:null,last_stats:null}});
  assert.equal(fresh.outcome,"");
  assert.equal(fresh.lastExecutionModel,"");
  const failure=root.OwoSessionFacts.fromDetail({turn_history:{last_turn_id:"t",last_terminal:{turn_id:"t",payload:{type:"turn_failed",completion_status:"aborted"}},last_stats:null}});
  assert.equal(failure.outcome,"cancelled");
  assert.equal(failure.stats,null);
  const old = root.OwoSessionFacts.fromDetail({turn_history:{last_turn_id:"new",last_terminal:{turn_id:"old",payload:{type:"turn_stats",model_calls:[{model:"old-model"}]}}}});
  assert.equal(old.outcome,"interrupted");
  assert.equal(old.stats,null);
  assert.equal(old.lastExecutionModel,"");
});
