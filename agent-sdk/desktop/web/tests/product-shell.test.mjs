import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { createRequire } from "node:module";
const details = createRequire(import.meta.url)("../views/task-details.view.js");
const read = file => readFileSync(new URL("../" + file, import.meta.url), "utf8");
const facts = text => [{ key: "task", text, tone: "muted" },{ key: "backend", text: "可用", tone: "ok" }];

test("task detail gives pending decisions priority without fabricating execution or verification", () => {
  assert.equal(details.presentation({ statusFacts: facts("回答完成 · 未验证") }).status, "回答完成 · 未验证");
  assert.match(details.presentation({ statusFacts: facts("回答完成 · 未验证") }).hint, /检查结果和验证依据/);
  assert.equal(details.presentation({ statusFacts: facts("回答完成 · 未验证"),running:true,phase:"tool" }).status,"正在执行操作");
  const pending=details.presentation({ statusFacts:facts("运行中"),running:true,phase:"tool",pendingApprovals:2 });
  assert.equal(pending.status,"等待你确认");assert.equal(pending.canReview,true);
  assert.equal(details.presentation({ pendingQuestion:{},running:true }).status,"需要补充信息");
  assert.equal(details.presentation({ statusFacts:facts("空闲") }).status,"等待任务");
});

test("completion status remains owned by host facts, not the presentation layer", () => {
  const sandbox={document:{documentElement:{},visibilityState:"visible"},console};
  runInNewContext(read("views/status-bar.view.js"),sandbox);
  const host=sandbox.OwoStatusBar;
  const unproven=host.computeFacts({lastTurnOutcome:"completed",lastCompletionStatus:"accepted"});
  assert.equal(details.presentation({statusFacts:unproven}).status,"回答完成 · 未验证");
  const accepted=host.computeFacts({lastTurnOutcome:"completed",lastCompletionRecord:{status:"accepted",evidence_receipt_ids:["receipt"]}});
  assert.equal(details.presentation({statusFacts:accepted}).status,"验收通过");
  assert.equal(details.presentation({authFailed:true,statusFacts:accepted}).connectionLabel,"连接异常");
});

test("administrative identifiers and repeated raw UI events have readable summaries", () => {
  assert.equal(details.presentation({title:"会话 d52b8d28"}).title,"新的任务");
  assert.equal(details.presentation({title:"会话 d52b8d28",request:"整理项目资料\n第二行"}).title,"整理项目资料");
  assert.equal(details.describeActions(["ui_change","ui_change","focus","ui_change"]),"界面发生变化、切换了当前应用");
  assert.equal(details.describeActions([]),"暂未记录近期活动");
});

test("task-header hydration uses the real authorization flag and existing session state", () => {
  const fn=read("app.js").match(/function syncTaskDetails\(statusFacts\) \{[\s\S]*?\n\}/)[0];
  let captured;
  const state={sessionId:"s",sessionIndex:[{id:"s",title:"资料整理"}],lastSessionPrompts:new Map([["s","整理资料"]]),pendingApprovals:new Map(),pendingQuestion:null};
  const doc={body:{classList:{contains:()=>false}}};
  const context={state,document:doc,authorizationUnavailable:false,currentTurn:()=>null,
    $:id=>["messages","diffList"].includes(id)?{querySelector:()=>null}:{textContent:"research-studio"},
    window:{OwoTaskDetails:{render:(doc,input)=>{captured=input;}},OwoStatusBar:{snapshotFacts:()=>facts("空闲")}},Array,Boolean};
  const sync=runInNewContext(fn+"\nsyncTaskDetails;",context);
  sync(facts("空闲"));
  assert.equal(captured.authFailed,false);
  assert.equal(captured.request,"整理资料");assert.equal(captured.title,"资料整理");
  assert.equal(captured.pendingApprovals,0);assert.equal(captured.steps.length,0);
});

test("collapsed conversation groups remain reachable and restore after search", () => {
  const classes = values => {
    const set = new Set(values);
    return { contains: x => set.has(x), add: x => set.add(x), toggle: (x,on) => { if (on) set.add(x); else set.delete(x); } };
  };
  const caret={classList:classes([])};
  const group={classList:classes(["codex-session-group"]),dataset:{groupWorkspace:"workspace"},attrs:{},querySelector:()=>caret,setAttribute:(k,v)=>{group.attrs[k]=v;}};
  const row={classList:classes(["hidden"]),dataset:{groupWorkspace:"workspace"},textContent:"research task",nextElementSibling:null};
  group.nextElementSibling=row;
  const input={value:""};
  const source=read("app-domain.js").match(/function filterSessionList\(\) \{[\s\S]*?\n\}/)[0];
  const filter=runInNewContext(source+"\nfilterSessionList;",{$:id=>id==="sessionSearch"?input:{querySelectorAll:()=>[group,row]},collapsedGroups:new Set(["workspace"]),Boolean,String});
  filter();assert.equal(group.classList.contains("hidden"),false);assert.equal(row.classList.contains("hidden"),true);
  input.value="research";filter();assert.equal(row.classList.contains("hidden"),false);assert.equal(group.attrs["aria-expanded"],"true");
  input.value="";filter();assert.equal(row.classList.contains("hidden"),true);assert.equal(group.classList.contains("hidden"),false);
  input.value="missing";filter();assert.equal(group.classList.contains("hidden"),true);
});

test("late file-change responses cannot expose another session's revert action", async () => {
  let release;
  const old=new Promise(resolve=>{release=resolve;});
  const state={sessionId:"old"};
  const list={innerHTML:""};
  const revert={disabled:false};
  const fn=read("app-domain.js").match(/async function refreshDiff\(sessionId\) \{[\s\S]*?\n\}/)[0];
  // renderHunkActions（逐 hunk 审阅）是 refreshDiff 的新依赖：本测试只关心
  // "迟到的响应不得暴露别的会话的 revert"，故在沙箱里注入空实现。
  const refresh=runInNewContext(fn+"\nrefreshDiff;",{state,sessionDiffRefreshGeneration:0,$:id=>id==="diffList"?list:revert,api:path=>path.includes("/old/")?old:Promise.resolve([]),renderHunkActions:async()=>{}});
  const waiting=refresh("old");state.sessionId="new";await refresh("new");
  release([{path:"old.txt",before:"before",after:"after"}]);await waiting;
  assert.equal(revert.disabled,true);assert.match(list.innerHTML,/没有文件改动/);assert.ok(!list.innerHTML.includes("old.txt"));
});

test("restored successful tool steps stay readable and the panel reserves editor space", () => {
  const fn=read("app.js").match(/function syncTaskDetails\(statusFacts\) \{[\s\S]*?\n\}/)[0];
  let captured, reserve;
  const tool={classList:{contains:name=>name==="is-ok"},querySelector:selector=>({textContent:selector===".tool-step-state"?"":selector===".tool-step-label"?"读取文件":"brief.md"})};
  const view={querySelectorAll:()=>[tool]};
  const nodes={messages:{querySelector:()=>view},composerProjectName:{textContent:"workspace"},diffList:{querySelector:()=>null},chatForm:{getBoundingClientRect:()=>({top:500,height:180})},approvalBar:{classList:{contains:()=>true}},right:{style:{setProperty:(name,value)=>{reserve=value;}}}};
  const state={sessionId:"s",sessionIndex:[],lastSessionPrompts:new Map([["s","读取项目"]]),pendingApprovals:new Map(),pendingQuestion:null};
  const ctx={state,document:{body:{classList:{contains:()=>true}}},authorizationUnavailable:false,currentTurn:()=>null,$:id=>nodes[id],
    window:{innerHeight:700,OwoTaskDetails:{render:(doc,input)=>{captured=input;}}},Array,Boolean,Math};
  const sync=runInNewContext(fn+"\nsyncTaskDetails;",ctx);sync(facts("回答完成 · 未验证"));
  assert.equal(captured.steps[0].state,"已完成");
  assert.equal(reserve,"212px");
});

test("compact navigation opens at the same breakpoint as the shell and closes when widened", () => {
  const source=read("app.js").match(/function setMobileSidebarOpen\(open\) \{[\s\S]*?\n\}/)[0];
  const classes=new Set();
  const attributes={};
  let width=800;
  const context={window:{matchMedia:query=>({matches:width<=Number(query.match(/(\d+)px/)[1])})},
    document:{body:{classList:{toggle:(name,enabled)=>enabled?classes.add(name):classes.delete(name)}}},
    mobileSidebarToggle:{setAttribute:(key,value)=>{attributes[key]=value;}},Boolean,String};
  const toggle=runInNewContext(source+"\nsetMobileSidebarOpen;",context);
  toggle(true);assert.equal(classes.has("mobile-sidebar-open"),true);assert.equal(attributes["aria-expanded"],"true");
  toggle(false);assert.equal(classes.has("mobile-sidebar-open"),false);
  width=1200;toggle(true);assert.equal(classes.has("mobile-sidebar-open"),false);
});
