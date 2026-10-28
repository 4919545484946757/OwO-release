// ============================================================================
// 项目与运行历史面板 Node 前端测试 —— desktop/web/tests/project-history.panel.test.mjs
//
// 运行：node --test "tests/*.test.mjs"（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert；无 DOM、无网络（传输层经 _test.setTransport 注入）。
//
// 覆盖面（八期第四路验收守卫）：
//   - 纯逻辑：状态归一、耗时人类可读、计划预算容错、任务计数、模板/项目过滤选项
//     （动态组队桶）、四维过滤、可见列表排序截断、objective 提取三级回退
//     （additive → audit 启发式 → null）、复跑请求体（模板/工作区六期冻结形状）；
//   - 渲染守卫：面板骨架、过滤下拉 selected、行卡片（徽标/耗时/预算/交付物/
//     data 属性）、复跑提交锁禁用、结果区、空态文案；
//   - 行为：load() 聚合落 state（deliverables 404 静默、workspace 404 → 未绑定、
//     详情失败降级不阻塞）、startRerun 成功/失败/提交锁（快速双击只发一次）、
//     缺 objective 不发请求；
//   - 接线守卫：index.html 脚本、app.js PANEL_ORDER、style.css 面板样式节在场。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const panel = require(join(here, "../panels/project-history.panel.js"));
const T = panel._test;
const shellCss = readFileSync(join(here, "../style.css"), "utf8");
const indexHtml = readFileSync(join(here, "../index.html"), "utf8");
const appJs = readFileSync(join(here, "../app.js"), "utf8");
const appDomainJs = readFileSync(join(here, "../app-domain.js"), "utf8");

// ---------- fixture 工具 ----------

function team(id, status, extra) {
  return Object.assign(
    {
      team_id: id,
      status: status,
      mode: "Team",
      template_id: null,
      project_space_id: "proj-" + id,
      created_at: "2026-08-30T04:00:00+00:00",
      updated_at: "2026-08-30T04:02:00+00:00",
      budget: {},
    },
    extra || {}
  );
}

function detail(teamId, tasks, auditTail, extra) {
  return Object.assign(
    {
      team: { team_id: teamId, mode: "Team", template_id: null },
      tasks: tasks || [],
      interrupted: false,
      audit_tail: auditTail || [],
    },
    extra || {}
  );
}

function resetState() {
  if (T.state.filterEnrichTimer) clearTimeout(T.state.filterEnrichTimer);
  T.invalidateEnrichment();
  T.state.filterEnrichTimer = null;
  T.state.loading = false;
  T.state.loadedOnce = false;
  T.state.teams = [];
  T.state.details = {};
  T.state.detailErrors = {};
  T.state.deliverables = {};
  T.state.workspaces = {};
  T.state.workspaceErrors = {};
  T.state.filters = { status: "", template: "", project: "", q: "" };
  T.state.rerunBusy = {};
  T.state.rerunResults = {};
  T.state.errors = [];
  T.state.lastLoadedAt = "";
  T.setRoot(null);
}

// ---------- 纯逻辑 ----------

test("normStatus：Debug 形式与 snake_case 归一一致", () => {
  assert.equal(T.normStatus("Succeeded"), "succeeded");
  assert.equal(T.normStatus("AwaitingHuman"), "awaiting_human");
  assert.equal(T.normStatus("awaiting_human"), "awaiting_human");
  assert.equal(T.normStatus(null), "");
});

test("durationText：人类可读且容错缺失/倒挂", () => {
  assert.equal(T.durationText("2026-08-30T04:00:00Z", "2026-08-30T04:00:45Z"), "45s");
  assert.equal(T.durationText("2026-08-30T04:00:00Z", "2026-08-30T04:02:00Z"), "2m 0s");
  assert.equal(T.durationText("2026-08-30T04:00:00Z", "2026-08-30T05:02:00Z"), "1h 2m");
  assert.equal(T.durationText(null, "2026-08-30T04:00:00Z"), "");
  assert.equal(T.durationText("2026-08-30T05:00:00Z", "2026-08-30T04:00:00Z"), "");
  assert.equal(T.durationText("not-a-date", "also-bad"), "");
});

test("plannedCalls：strategy_decision.budget_calls_total 容错", () => {
  assert.equal(T.plannedCalls({ strategy_decision: { budget_calls_total: 7 } }), 7);
  assert.equal(T.plannedCalls({ strategy_decision: null }), null);
  assert.equal(T.plannedCalls({}), null);
  assert.equal(T.plannedCalls({ strategy_decision: { budget_calls_total: 0 } }), null);
});

test("taskCounts：成功/失败归类；空任务 → null", () => {
  const c = T.taskCounts({
    tasks: [
      { status: "Succeeded" },
      { status: "succeeded" },
      { status: "Failed" },
      { status: "Pending" },
    ],
  });
  assert.deepEqual(c, { total: 4, done: 2, failed: 1 });
  assert.equal(T.taskCounts({ tasks: [] }), null);
  assert.equal(T.taskCounts(null), null);
});

test("templateOptions/projectOptions：去重 + 动态组队桶 + 派生项目键", () => {
  const teams = [
    team("t1", "succeeded", { template_id: "code-change-v1" }),
    team("t2", "failed", { template_id: "code-change-v1" }),
    team("t3", "running", { template_id: null }),
    team("t4", "succeeded", { template_id: null, project_space_id: null }),
  ];
  const tpls = T.templateOptions(teams);
  assert.deepEqual(
    tpls.map((o) => o.value),
    ["__owo_dynamic__", "code-change-v1"]
  );
  assert.equal(tpls.find((o) => o.value === "__owo_dynamic__").label, "动态组队（无模板）");
  const projs = T.projectOptions(teams);
  assert.deepEqual(
    projs.map((o) => o.value).sort(),
    ["proj-t1", "proj-t2", "proj-t3", "proj-t4"]
  );
  assert.equal(T.projectKeyOf({ team_id: "t9", project_space_id: null }), "proj-t9");
});

test("matchesFilters：四维过滤（状态归一/模板含动态桶/项目/搜索）", () => {
  const t = team("team-a", "Succeeded", { template_id: "code-change-v1" });
  assert.ok(T.matchesFilters(t, {}));
  assert.ok(T.matchesFilters(t, { status: "succeeded" }));
  assert.ok(!T.matchesFilters(t, { status: "failed" }));
  assert.ok(T.matchesFilters(t, { template: "code-change-v1" }));
  assert.ok(!T.matchesFilters(t, { template: "research-brief-v1" }));
  assert.ok(T.matchesFilters(t, { project: "proj-team-a" }));
  assert.ok(!T.matchesFilters(t, { project: "proj-other" }));
  assert.ok(T.matchesFilters(t, { q: "TEAM-A" }));
  assert.ok(!T.matchesFilters(t, { q: "zzz" }));
  assert.ok(!T.matchesFilters(null, {}));
});

test("visibleTeams：过滤 + created_at 降序 + 截断", () => {
  const teams = [
    team("old", "succeeded", { created_at: "2026-08-28T00:00:00Z" }),
    team("new", "failed", { created_at: "2026-08-30T00:00:00Z" }),
    team("mid", "succeeded", { created_at: "2026-08-29T00:00:00Z" }),
    team("hidden", "failed", { created_at: "2026-08-31T00:00:00Z" }),
  ];
  const out = T.visibleTeams(teams, { status: "succeeded" }, 2);
  assert.deepEqual(
    out.map((t) => t.team_id),
    ["mid", "old"]
  );
});

test("objectiveFromDetail：additive 优先 → audit「目标 …」启发式 → null", () => {
  assert.equal(T.objectiveFromDetail({ objective: " 直接目标 " }), "直接目标");
  assert.equal(
    T.objectiveFromDetail({
      audit_tail: [
        { event: "team.succeeded", detail: "目标达成" },
        { event: "goal.started", detail: "目标 调研 A 与 B 的差异" },
      ],
    }),
    "调研 A 与 B 的差异"
  );
  assert.equal(T.objectiveFromDetail({ audit_tail: [{ detail: "无关" }] }), null);
  assert.equal(T.objectiveFromDetail(null), null);
});

test("Dynamic template filter has a distinct value from all templates", () => {
  assert.ok(T.matchesFilters(team("d","succeeded"),{template:"__owo_dynamic__"}));
  assert.ok(!T.matchesFilters(team("t","succeeded",{template_id:"custom"}),{template:"__owo_dynamic__"}));
  assert.ok(T.matchesFilters(team("t","succeeded",{template_id:"custom"}),{q:"CUSTOM"}));
  assert.equal(T.objectiveFromDetail({audit_tail:[{detail:"目标达成：完成所有步骤"}]}),null);
  assert.equal(T.localWhen("bad"),"时间未知");
});

// ---------- 渲染守卫 ----------

test("shellHtml：骨架完整（标题/刷新/错误区/过滤区/列表区）", () => {
  const html = T.shellHtml();
  assert.match(html, /查找历史任务/);
  assert.match(html, /id="ph-refresh"/);
  assert.match(html, /id="ph-errors"/);
  assert.match(html, /id="ph-filters-box"/);
  assert.match(html, /id="ph-list"/);
});

test("filtersHtml：状态下拉 selected 与模板/项目选项", () => {
  T.state.teams = [
    team("t1", "succeeded", { template_id: "code-change-v1" }),
    team("t2", "failed", { template_id: null }),
  ];
  T.state.filters = { status: "failed", template: "", project: "", q: "" };
  const html = T.filtersHtml(T.state);
  assert.match(html, /value="failed" selected/);
  assert.match(html, /动态组队（无模板）/);
  assert.match(html, /code-change-v1/);
});

test("rowHtml：徽标/耗时/预算/交付物/动作 data 属性 + 提交锁 + 结果区", () => {
  const t = team("team-x", "Succeeded", {
    template_id: "code-change-v1",
    strategy_decision: { budget_calls_total: 7 },
  });
  const html = T.rowHtml(t, {
    detail: { tasks: [{ status: "Succeeded" }, { status: "Failed" }] },
    deliverables: { approved: 2, pending: 1 },
    workspace: null,
    objective: "做一个研究简报",
  });
  assert.match(html, /data-ph-row="team-x"/);
  assert.match(html, /ph-st-succeeded/);
  assert.match(html, /已完成/);
  assert.match(html, /运行跨度 2m 0s/);
  assert.match(html, /预算 7 次调用|预算/);
  assert.match(html, /任务 1\/2（失败 1）/);
  assert.match(html, /批准 2 · 待评审 1/);
  assert.match(html, /复用目标/);
  assert.match(html, /data-ph-open="team-x"/);
  assert.match(html, /data-ph-rerun="team-x"/);
  assert.match(html, /做一个研究简报/);

  // 提交锁：busy 禁用 + 结果文本
  T.state.rerunBusy["team-x"] = true;
  T.state.rerunResults["team-x"] = { ok: true, text: "已按同配置创建新运行：team-y。" };
  const html2 = T.rowHtml(t, {});
  assert.match(html2, /disabled/);
  assert.match(html2, /已按同配置创建新运行：team-y。/);
  delete T.state.rerunBusy["team-x"];
  delete T.state.rerunResults["team-x"];
});

test("listHtml：空态文案", () => {
  const html = T.listHtml([], T.state);
  assert.match(html, /没有匹配的历史运行/);
});

// ---------- 行为（传输层注入） ----------

function transportMock(map, log) {
  return {
    get(path) {
      log.push("GET " + path);
      const h = map[path];
      if (h === undefined) return Promise.reject(new Error("404: not found"));
      if (h instanceof Error) return Promise.reject(h);
      return Promise.resolve(h);
    },
    post(path, body) {
      log.push("POST " + path + " " + JSON.stringify(body));
      const h = map["POST " + path];
      if (h === undefined) return Promise.reject(new Error("404: not found"));
      if (h instanceof Error) return Promise.reject(h);
      return Promise.resolve(typeof h === "function" ? h(body) : h);
    },
  };
}

test("load()：列表 + 详情/交付物/工作区富化；404 静默降级", async () => {
  resetState();
  const log = [];
  const teams = {
    teams: [
      team("t1", "Succeeded", { project_space_id: "proj-p1" }),
      team("t2", "Failed", { project_space_id: null }),
    ],
  };
  const map = {
    "/teams": teams,
    "/teams/t1": detail("t1", [{ status: "Succeeded" }], [
      { detail: "目标 调研要点" },
    ]),
    "/teams/t2": new Error("500: boom"),
    "/projects/proj-p1/artifacts": { artifacts: [{review_state:"Approved"}, {review_state:"PendingReview"}, {review_state:"Draft"}] },
    "/projects/proj-p1/workspace": { root: "T:/ws", read_only: true },
    "/projects/proj-t2/artifacts": new Error("404: none"),
    "/projects/proj-t2/workspace": new Error("404: none"),
  };
  T.setTransport(transportMock(map, log));
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.load();
  assert.equal(T.state.teams.length, 2);
  assert.ok(T.state.details["t1"]);
  assert.ok(T.state.detailErrors["t2"]);
  assert.deepEqual(T.state.deliverables["proj-p1"], { approved: 1, pending: 1 });
  // 404 交付物 → 空计数且无错误文案
  assert.deepEqual(T.state.deliverables["proj-t2"], { approved: 0, pending: 0, error: "" });
  // 404 工作区 → 未绑定（null）
  assert.equal(T.state.workspaces["proj-p1"].root, "T:/ws");
  assert.equal(T.state.workspaces["proj-t2"], null);
  assert.equal(T.state.loading, false);
  const paths = log.map((x) => x.split(" ")[1]);
  assert.ok(paths.includes("/teams/t1"));
  assert.ok(paths.includes("/projects/proj-p1/artifacts"));
  assert.ok(!log.some((x) => x.startsWith("POST ")));
});

test("enrichVisible：相同团队/项目合并并发请求，完成后复用缓存", async () => {
  resetState();
  const rows = [
    team("team-1", "Succeeded", { project_space_id: "project-shared" }),
    team("team-2", "Failed", { project_space_id: "project-shared" }),
  ];
  T.state.teams = rows;
  const pending = {};
  const calls = [];
  T.setTransport({
    get(path) {
      calls.push(path);
      return new Promise((resolve) => { pending[path] = resolve; });
    },
  });

  const first = T.enrichVisible(rows);
  const second = T.enrichVisible(rows);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(calls.slice().sort(), [
    "/teams/team-1",
    "/teams/team-2",
    "/projects/project-shared/artifacts",
    "/projects/project-shared/workspace",
  ].sort());

  pending["/teams/team-1"]({ team: { team_id: "team-1" } });
  pending["/teams/team-2"]({ team: { team_id: "team-2" } });
  pending["/projects/project-shared/artifacts"]({ artifacts: [{review_state:"Approved"}] });
  pending["/projects/project-shared/workspace"]({ root: "T:/shared" });
  await Promise.all([first, second]);
  await T.enrichVisible(rows);

  assert.equal(calls.length, 4);
  assert.ok(T.state.details["team-1"]);
  assert.ok(T.state.details["team-2"]);
  assert.deepEqual(T.state.deliverables["project-shared"], { approved: 1, pending: 0 });
});

test("项目历史搜索输入防抖：只为最终命中行补充详情", async () => {
  resetState();
  T.state.teams = [
    team("team-1", "Succeeded", { project_space_id: "project-1" }),
    team("team-2", "Failed", { project_space_id: "project-2" }),
  ];
  const elements = {
    "#ph-list": { innerHTML: "" },
    "#ph-errors": { innerHTML: "" },
    "#ph-meta": { textContent: "" },
  };
  T.setRoot({ querySelector(selector) { return elements[selector] || null; } });
  const calls = [];
  T.setTransport({
    get(path) {
      calls.push(path);
      if (path.startsWith("/teams/")) return Promise.resolve(detail(path.slice("/teams/".length)));
      if (path.endsWith("/artifacts")) return Promise.resolve({ artifacts: [] });
      if (path.endsWith("/workspace")) return Promise.resolve({ root: "T:/project" });
      return Promise.reject(new Error("unexpected GET " + path));
    },
  });

  ["t", "team-", "team-2"].forEach((value) => {
    T.onFilterChange({ target: { id: "ph-f-q", value } });
  });
  assert.deepEqual(calls, [], "本地过滤即时更新，输入过程中不应请求补充数据");
  await new Promise((resolve) => setTimeout(resolve, 220));
  await new Promise((resolve) => setTimeout(resolve, 5));

  assert.deepEqual(calls.sort(), [
    "/teams/team-2",
    "/projects/project-2/artifacts",
    "/projects/project-2/workspace",
  ].sort());
  assert.match(elements["#ph-list"].innerHTML, /data-ph-row="team-2"/);
  assert.doesNotMatch(elements["#ph-list"].innerHTML, /data-ph-row="team-1"/);
});

test("dispose：切页时取消待处理的搜索补充", async () => {
  resetState();
  T.state.teams = [team("team-1", "Succeeded", { project_space_id: "project-1" })];
  const elements = {
    "#ph-list": { innerHTML: "" },
    "#ph-errors": { innerHTML: "" },
    "#ph-meta": { textContent: "" },
  };
  T.setRoot({ querySelector(selector) { return elements[selector] || null; } });
  const calls = [];
  T.setTransport({ get(path) { calls.push(path); return Promise.resolve({}); } });

  T.onFilterChange({ target: { id: "ph-f-q", value: "team-1" } });
  assert.notEqual(T.state.filterEnrichTimer, null);
  panel.dispose();
  await new Promise((resolve) => setTimeout(resolve, 220));

  assert.deepEqual(calls, []);
  assert.equal(T.state.filterEnrichTimer, null);
  assert.equal(T.state.loading, false);
});

test("dispose：忽略卸载前发出的迟到团队列表响应", async () => {
  resetState();
  let resolveTeams;
  const calls = [];
  T.setTransport({
    get(path) {
      calls.push(path);
      return new Promise((resolve) => { resolveTeams = resolve; });
    },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });

  const pendingLoad = T.load();
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(calls, ["/teams"]);
  panel.dispose();
  resolveTeams({ teams: [team("late-team", "Succeeded")] });
  await pendingLoad;

  assert.deepEqual(T.state.teams, []);
  assert.equal(T.state.loading, false);
  assert.deepEqual(calls, ["/teams"]);
});

test("load() 刷新成功后失效旧富化缓存并重新读取", async () => {
  resetState();
  const calls = [];
  let revision = 1;
  T.setTransport({
    get(path) {
      calls.push(path);
      if (path === "/teams") return Promise.resolve({ teams: [team("team-r", "Succeeded", { project_space_id: "project-r" })] });
      if (path === "/teams/team-r") return Promise.resolve({ team: { team_id: "team-r", objective: "rev-" + revision } });
      if (path.endsWith("/artifacts")) return Promise.resolve({ artifacts: [] });
      if (path.endsWith("/workspace")) return Promise.resolve({ root: "T:/revision-" + revision });
      return Promise.reject(new Error("unexpected GET " + path));
    },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });

  await T.load();
  assert.equal(T.state.details["team-r"].team.objective, "rev-1");
  revision = 2;
  await T.load();
  assert.equal(T.state.details["team-r"].team.objective, "rev-2");
  assert.equal(calls.filter((path) => path === "/teams/team-r").length, 2);
});

test("Goal reuse opens launcher and prefills only objective without creating a run", async () => {
  resetState();
  const events=[];
  const old=globalThis.OwoPanels["project-launcher"];
  globalThis.OwoPanels["project-launcher"]={setObjective:value=>events.push(["objective",value])};
  T.setHelpers({get:async()=>({objective:"复用真实目标"}),openPanel:id=>events.push(["panel",id]),post:()=>assert.fail("reuse must not create a run")});
  await Promise.all([T.startRerun("reuse"),T.startRerun("reuse")]);
  assert.deepEqual(events,[["panel","project-launcher"],["objective","复用真实目标"]]);
  globalThis.OwoPanels["project-launcher"]=old;
});

test("Goal reuse ignores stale detail after leaving the panel", async () => {
  resetState();
  let finish, opens=0;
  T.setHelpers({get:()=>new Promise(r=>{finish=r;}),openPanel:()=>opens++});
  const pending=T.startRerun("stale");
  await Promise.resolve(); await Promise.resolve();
  T.dispose();
  finish({objective:"不能跳转"});
  await pending;
  assert.equal(opens,0);
  assert.equal(T.state.rerunBusy.stale,undefined);
});

test("Missing goal and failed details surface errors without any POST", async () => {
  resetState();
  T.setHelpers({get:async()=>({audit_tail:[{detail:"目标达成"}]}),post:()=>assert.fail("no automatic creation")});
  await T.startRerun("no-goal");
  assert.match(T.state.rerunResults["no-goal"].text,/无法自动提取原目标/);
  T.setHelpers({get:async()=>{throw new Error("503: failed");}});
  await T.startRerun("failed-detail");
  assert.match(T.state.rerunResults["failed-detail"].text,/503/);
});

test("History list can reveal records beyond the original 50-row cap", () => {
  resetState();
  const rows=Array.from({length:65},(_,i)=>team("team-"+i,"succeeded"));
  T.state.rowLimit=50;
  assert.match(T.listHtml(rows,T.state),/data-ph-more/);
  T.state.rowLimit=100;
  assert.doesNotMatch(T.listHtml(rows,T.state),/data-ph-more/);
  assert.equal((T.listHtml(rows,T.state).match(/data-ph-row=/g)||[]).length,65);
  T.state.rowLimit=50;
});

// ---------- 接线守卫 ----------

test("index.html 引入 project-history 面板脚本", () => {
  assert.match(indexHtml, /panels\/project-history\.panel\.js/);
});

test("PANEL_ORDER 注册 project-history（唯一来源 app-domain.js）", () => {
  assert.ok(!/const\s+PANEL_ORDER\s*=/.test(appJs), "PANEL_ORDER 不得回归 app.js 定义");
  const panelOrder = /const PANEL_ORDER = \[([\s\S]*?)\]/.exec(appDomainJs);
  assert.ok(panelOrder && panelOrder[1].includes("\"project-history\""), "app-domain.js PANEL_ORDER 未注册 project-history");
});

test("style.css 提供面板样式节（.owo-ac-* 复用 + ph 专属类）", () => {
  assert.match(shellCss, /\.ph-row\s*\{|\.ph-row\b/);
  assert.match(shellCss, /\.ph-st-/);
});


test("Workspace accepts the actual wrapped binding and explicit null without false errors", async () => {
  resetState();
  const rows=[team("wrapped","succeeded"),team("unbound","succeeded")];
  T.setTransport({get:async path=>{
    if (path.endsWith("/artifacts")) return {artifacts:[]};
    if (path.endsWith("/workspace")) return path.includes("unbound") ? {workspace:null} : {workspace:{root:"T:/real",read_only:true}};
    return {team:{}};
  }});
  await T.enrichVisible(rows);
  assert.equal(T.state.workspaces["proj-wrapped"].root,"T:/real");
  assert.equal(T.state.workspaces["proj-unbound"],null);
  assert.deepEqual(T.state.workspaceErrors,{});
});

test("Only the exact completion audit event can recover a completed goal", () => {
  assert.equal(T.objectiveFromDetail({audit_tail:[{event:"team.succeeded",detail:"目标达成：原始任务目标"}]}),"原始任务目标");
  assert.equal(T.objectiveFromDetail({audit_tail:[{event:"log",detail:"目标达成：不能作为原始目标"}]}),null);
});


test("Search matches the visible fallback role title even without an objective", () => {
  assert.ok(T.matchesFilters(team("role-title","failed",{members:[{role:"构建检查"}]}),{q:"构建检查"}));
});
