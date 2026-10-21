import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const C = require("../panels/workswarm/render_conversation.js");
const D = require("../panels/workswarm/domain.js");
const R = require("../panels/workswarm/render.js");
const row = (seq, payload, role = "分镜导演") => ({
 session_id: "team-a-worker-1", turn_id: role, seq, created_at: "2026-10-19T01:00:00Z", payload
});
test("reconnect pages deduplicate sequence numbers and retain node identity on empty polls", () => {
 let replay = C.mergeReplay({}, [{session_id:"s",role:"分镜导演",events:[row(1,{type:"token_delta",delta:"你好"})]}]);
 replay = C.mergeReplay(replay, [{session_id:"s",role:null,events:[row(1,{type:"token_delta",delta:"你好"}),row(2,{type:"final",text:"完成"})]}]);
 replay = C.mergeReplay(replay, [{session_id:"s",role:null,events:[]}]);
 assert.equal(replay.s.role, "分镜导演");
 assert.equal(replay.s.events.length, 2);
 const html = R.teamConversationHtml([], [], null, "", replay);
 assert.match(html, /分镜导演/); assert.match(html, /完成/);
 assert.doesNotMatch(html, /你好/);
});
test("tool parameters and failures render as expandable, escaped details", () => {
 const replay=C.mergeReplay({},[{session_id:"s",role:"音效",events:[
  row(1,{type:"tool_use",tool:"mcp__audio__generate",args:{prompt:"<script>bad</script>"}}),
  row(2,{type:"tool_result",tool:"mcp__audio__generate",ok:false,error:"service unavailable"})
 ]}]);
 const html=R.teamConversationHtml([],[],null,"",replay);
 assert.match(html, /<details class="owo-ws-tool-detail" data-event-key=/);
 assert.match(html,/mcp__audio__generate/);assert.match(html,/service unavailable/);
 assert.doesNotMatch(html,/<script>/);assert.match(html,/&lt;script&gt;/);
});

test("成员对话支持按角色过滤（多节点并行时可读）", () => {
  const replay = C.mergeReplay({}, [
    { session_id: "a", role: "分镜导演", events: [row(1, { type: "final", text: "分镜完成" }, "分镜导演")] },
    { session_id: "b", role: "音效", events: [row(1, { type: "final", text: "音效完成" }, "音效")] },
  ]);
  assert.deepEqual(C.replayRoles(replay), ["分镜导演", "音效"]);

  const all = R.teamConversationHtml([], [], null, "", replay);
  assert.match(all, /分镜完成/);
  assert.match(all, /音效完成/);

  const only = R.teamConversationHtml([], [], null, "", replay, "音效");
  assert.match(only, /音效完成/);
  assert.doesNotMatch(only, /分镜完成/);

  // 不存在的角色 → 空（不偷偷回落到全部）。
  const none = R.teamConversationHtml([], [], null, "", replay, "不存在的角色");
  assert.doesNotMatch(none, /分镜完成|音效完成/);
});


test("成员过滤状态：角色仍存在才沿用，否则回到全部", () => {
  assert.equal(C.pickConversationRole("音效", ["分镜导演", "音效"]), "音效");
  assert.equal(C.pickConversationRole("已删除角色", ["分镜导演", "音效"]), "all");
  assert.equal(C.pickConversationRole("", ["音效"]), "all");
  assert.equal(C.pickConversationRole(null, []), "all");
  assert.equal(C.pickConversationRole("all", ["音效"]), "all");
});


test("团队详情的模板行带「查看该工作流」入口（动态组队则没有）", () => {
  const withTemplate = R.templateBoxHtml({ template_id: "demo-parallel-writes" }, null);
  assert.match(withTemplate, /data-open-workflows="demo-parallel-writes"/);
  assert.match(withTemplate, /查看该工作流/);
  const dynamic = R.templateBoxHtml({}, null);
  assert.doesNotMatch(dynamic, /data-open-workflows/);
});

test("custom workflow editing round-trips model, tools, paths and delivery requirements", () => {
 const role={role:"世界设计师",assignee:"agent",model:"visual-model",tools:["mcp__mesh__generate","write_file"],
 write_paths:["models"],handoff_contract:"交付 glb",capabilities:["custom"],verify:"non_empty"};
 assert.deepEqual(D.sanitizeRoles([role])[0],role);
});

test("denied tools render their terminal failure without a tool-use event", () => {
 const replay=C.mergeReplay({},[{session_id:"s",role:"视频制作",events:[
  row(1,{type:"tool_result",id:"denied",tool:"mcp__video__generate",ok:false,
   error:"permission denied: approval declined",command_receipt:null})
 ]}]);
 const html=R.teamConversationHtml([],[],null,"",replay);
 assert.match(html,/mcp__video__generate<\/b> · <span class="owo-ws-badge rv-bad">失败<\/span>/);
 assert.match(html,/permission denied: approval declined/);
 assert.doesNotMatch(html,/调用参数|已完成/);
});
