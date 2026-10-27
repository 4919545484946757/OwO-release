import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { bindTabs, title } = require("../panels/workswarm/detail-layout.js");
function fixture(saved) {
  const names = ["conversation", "changes", "artifacts", "activity"];
  const tabs = names.map(name => ({
    dataset: { teamTab: name }, attrs: {}, handlers: {}, focused: false,
    setAttribute(name, value) { this.attrs[name] = value; },
    addEventListener(type, fn) { this.handlers[type] = fn; },
    focus() { this.focused = true; }
  }));
  const panels = names.map(() => ({ hidden: false, draft: { value: "尚未提交的人工验收意见" } }));
  const selections = [];
  const controller = bindTabs(tabs, panels, saved, name => selections.push(name));
  const key = (index, key, extra={}) => tabs[index].handlers.keydown({key, preventDefault(){}, ...extra});
  return { tabs, panels, selections, controller, key };
}
test("团队详情恢复所属团队的页签，失效选择回退对话", () => {
  const saved = fixture("artifacts");
  assert.equal(saved.tabs[2].attrs["aria-selected"], "true");
  assert.deepEqual(saved.panels.map(p=>p.hidden), [true,true,false,true]);
  assert.equal(fixture("removed").tabs[0].tabIndex, 0);
});
test("页签左右方向键、首尾键完整移动焦点及可读状态", () => {
  const {tabs, panels, key} = fixture();
  key(0,"ArrowLeft");
  assert.equal(tabs[3].attrs["aria-selected"], "true");
  assert.equal(tabs[3].focused, true);
  assert.equal(panels[3].hidden, false);
  key(3,"Home"); assert.equal(tabs[0].tabIndex,0);
  key(0,"End"); assert.equal(tabs[3].tabIndex,0);
  key(3,"ArrowRight"); assert.equal(tabs[0].tabIndex,0);
  key(0,"ArrowRight",{ctrlKey:true}); assert.equal(tabs[0].tabIndex,0);
});
test("切换页签保持原表单对象及未提交内容，无效目标不改变当前页", () => {
  const {tabs,panels,controller,selections} = fixture();
  const draft=panels[0].draft;
  tabs[2].handlers.click(); tabs[0].handlers.click();
  assert.equal(panels[0].draft,draft);
  assert.equal(panels[0].draft.value,"尚未提交的人工验收意见");
  assert.equal(controller.select("unknown"),false);
  assert.deepEqual(selections,["conversation","artifacts","conversation"]);
});
test("任务标题只读取实际目标事件，不把诊断内容当成任务名称", () => {
  assert.equal(title({},[{event:"goal.start",detail:"目标 检查团队页面"}]),"检查团队页面");
  assert.equal(title({},[{event:"team.failed",detail:"错误详情"}]),"团队运行");
  assert.equal(title({},[]),"团队运行");
});

test("角色表单展示并保留显式验收规则，缺省值保持未配置", () => {
  const panel = require("../panels/workswarm.panel.js")._test;
  const html = panel.roleRowHtml({role:"人工验收",verify:"contains:通过"});
  assert.match(html,/aria-label="输出验收规则"[^>]*value="contains:通过"/);
  assert.match(panel.roleRowHtml({role:"人工验收"}),/aria-label="输出验收规则"[^>]*value=""/);
  assert.match(html,/留空的角色输出保持未验证/);
});
test("对话显示简短本地时间，保留精确时间并安全转义异常时间", () => {
  const C = require("../panels/workswarm/render_conversation.js");
  const esc = text => String(text).replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");
  const stamp = "2026-10-23T13:26:54.572858100+00:00";
  assert.match(C.timeHtml(esc,stamp),/datetime="2026-10-23T13:26:54.572858100\+00:00"/);
  assert.match(C.timeHtml(esc,stamp),/title="2026-10-23T13:26:54.572858100\+00:00"/);
  assert.doesNotMatch(C.timeHtml(esc,"<script>"),/<script>/);
  assert.equal(C.timeHtml(esc,""),"");
});

test("产物刷新按真实产物 ID 保留评审草稿和展开状态，不移到新版本", () => {
  const {captureArtifactDrafts,restoreArtifactDrafts}=require("../panels/workswarm/detail-layout.js");
  const makeRow=(id,comment="",open=false)=>({
    fields: {".owo-ws-review-comment":{value:comment}, ".owo-ws-review":{open}},
    getAttribute(){return id;}, querySelector(selector){return this.fields[selector]||null;}
  });
  const old=makeRow("version-1","请补充验证证据",true);
  const drafts=captureArtifactDrafts({querySelectorAll:()=>[old]});
  const same=makeRow("version-1"),next=makeRow("version-2");
  restoreArtifactDrafts({querySelectorAll:()=>[same,next]},drafts);
  assert.equal(same.fields[".owo-ws-review-comment"].value,"请补充验证证据");
  assert.equal(same.fields[".owo-ws-review"].open,true);
  assert.equal(next.fields[".owo-ws-review-comment"].value,"");
});
test("人工提交成功后同步产物、对话及任务状态", async () => {
  const T=require("../panels/workswarm.panel.js")._test;
  const text={value:"检查通过"},notice={className:"",textContent:"",classList:{contains(){return false;}}};
  const box={innerHTML:"",querySelectorAll(){return [];}};
  const elements={"#ws-h-task":{value:"s-checker"},"#ws-h-text":text,"#ws-h-result":notice,"#ws-d-artifacts":box};
  T.setRoot({querySelector:selector=>elements[selector]||null,querySelectorAll:()=>[]});
  Object.assign(T.state,{view:"detail",current:"team-check",teamStatus:"AwaitingHuman",team:{team_id:"team-check",project_space_id:"proj-check",status:"AwaitingHuman",members:[]},tasks:[],audit:[],auditKeys:{}});
  const previous=T.getTransport(),requests=[];
  T.setTransport({
    post:async(path,body)=>{requests.push(path);assert.equal(body.team_id,"team-check");return {artifact:{artifact_id:"result-1",review_state:"draft"}};},
    get:async path=>{requests.push(path);return path.includes("/conversation")?{handoffs:[],sessions:[]}:path.startsWith("/projects/")?{artifacts:[]}:{team:T.state.team,tasks:[]};}
  });
  try {
    await T.doHumanResult();
    assert.equal(text.value,"");
    assert.ok(requests.includes("/projects/proj-check/artifacts"));
    assert.ok(requests.some(path=>path.startsWith("/teams/team-check/conversation")));
    assert.ok(requests.includes("/teams/team-check"));
    assert.match(notice.textContent,/result-1/);
  } finally { T.setRoot(null); T.setTransport(previous); }
});
test("人工提交迟到响应不会清空另一个团队的输入或刷新其产物", async () => {
  const T=require("../panels/workswarm.panel.js")._test;
  const text={value:"旧团队结果"},notice={className:"",textContent:"",classList:{contains(){return false;}}};
  T.setRoot({querySelector:selector=>({"#ws-h-task":{value:"s-checker"},"#ws-h-text":text,"#ws-h-result":notice}[selector]||null)});
  Object.assign(T.state,{view:"detail",current:"team-old",teamStatus:"AwaitingHuman"});
  const previous=T.getTransport();let finish;const gets=[];
  T.setTransport({post:()=>new Promise(resolve=>finish=resolve),get:async path=>{gets.push(path);return {};}});
  try {
    const pending=T.doHumanResult();
    T.state.current="team-new"; text.value="新团队尚未提交的结果";
    finish({artifact:{artifact_id:"old-result"}});await pending;
    assert.equal(text.value,"新团队尚未提交的结果");
    assert.equal(notice.textContent,"");
    assert.deepEqual(gets,[]);
  } finally { T.setRoot(null); T.setTransport(previous); }
});

test("只看成员同时过滤人工交接和模型回放，角色来自实际消息", () => {
  const C=require("../panels/workswarm/render_conversation.js");
  const R=require("../panels/workswarm/render.js");
  const members=[{member_id:"m-a",role:"检查员"},{member_id:"m-b",role:"人工验收"},{member_id:"m-c",role:"尚未发言"}];
  const handoffs=[{from_member:"m-a",completed_summary:"检查完成"},{from_member:"m-b",completed_summary:"验收通过"}];
  assert.deepEqual(C.conversationRoles({},handoffs,members),["检查员","人工验收"]);
  const html=R.teamConversationHtml(handoffs,members,null,"",{},"人工验收");
  assert.match(html,/验收通过/);assert.doesNotMatch(html,/检查完成/);
  assert.deepEqual(C.replayRoles({s:{role:"constructor",events:[]}}),["constructor"]);
});
