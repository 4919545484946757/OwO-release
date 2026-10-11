// ============================================================================
// Project Launcher 面板 Node 前端测试 —— desktop/web/tests/project-launcher.panel.test.mjs
//
// 运行：node --test "tests/*.test.mjs"（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert；无 DOM、无网络（传输层经 _test.setTransport 注入）。
//
// 覆盖面（六期第四路验收守卫）：
//   - 纯逻辑：写入路径解析/安全校验、状态校验、POST /teams 冻结契约请求体；
//   - 预览：模板/策略两种来源的角色·预算·权限视图模型与 HTML；
//   - 目录：未安装模板禁用、安装按钮、幂等选用；
//   - 行为：创建提交锁（快速双击只发一次）、校验失败零请求、成功直达按钮；
//   - 渲染守卫：七步骨架、无 "## 选择器" 回归、aria-live 错误区；
//   - 样式守卫：style.css 第 20 节 Launcher/工作区/徽标/窄栏守卫在场。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const panel = require(join(here, "../panels/project-launcher.panel.js"));
const T = panel._test;
const shellCss = readFileSync(join(here, "../style.css"), "utf8");

const drain = () => new Promise((r) => setTimeout(r, 5));

function resetState() {
  panel.dispose();
  T.state.objective = "";
  T.state.root = "";
  T.state.readOnly = true;
  T.state.writePathsRaw = "";
  T.state.treeDepth = 2;
  T.state.strategy = "auto";
  T.state.catalog = [];
  T.state.catalogLoaded = false;
  T.state.catalogError = "";
  T.state.treeError = "";
  T.state.treeLoadFailed = false;
  T.state.treePickerOpen = false;
  T.state.treeEntries = null;
  T.state.pickedRows = [];
  T.state.selectedTemplate = "";
  T.state.installBusy = {};
  T.state.creating = false;
  T.state.result = null;
  T.state.error = "";
}

// ---------- 纯逻辑 ----------

test("parseWritePaths：逗号/换行/去重/去空白", () => {
  assert.deepEqual(T.parseWritePaths("src/, tests/\ntarget\n,target"), ["src/", "tests/", "target"]);
  assert.deepEqual(T.parseWritePaths("  "), []);
  assert.deepEqual(T.parseWritePaths(null), []);
});

test("pathIsSafe：拒绝 .. 与绝对路径转义", () => {
  assert.equal(T.pathIsSafe(".."), false);
  assert.equal(T.pathIsSafe("../etc"), false);
  assert.equal(T.pathIsSafe("a/../b"), false);
  assert.equal(T.pathIsSafe("a\\..\\b"), false);
  assert.equal(T.pathIsSafe("C:/abs"), false);
  assert.equal(T.pathIsSafe(""), false);
  assert.equal(T.pathIsSafe("src/main.rs"), true);
  assert.equal(T.pathIsSafe("tests/"), true);
});

test("validateState：目标/目录必填 + 不安全路径 + 深度边界", () => {
  resetState();
  const errs0 = T.validateState(T.state);
  assert.equal(errs0.length, 2, "空目标+空目录 = 2 条错误");
  assert.match(errs0[0], /任务目标/);
  assert.match(errs0[1], /项目目录/);
  T.state.objective = "修复登录超时";
  T.state.root = "T:\\demo";
  T.state.writePathsRaw = "src, ../escape";
  const errs1 = T.validateState(T.state);
  assert.equal(errs1.length, 1);
  assert.match(errs1[0], /不安全路径/);
  T.state.writePathsRaw = "src";
  T.state.treeDepth = 9;
  const errs2 = T.validateState(T.state);
  assert.equal(errs2.length, 1);
  assert.match(errs2[0], /深度/);
  T.state.treeDepth = 2;
  assert.deepEqual(T.validateState(T.state), []);
});

test("buildCreateBody：冻结契约形状（workspace/strategy/mode/template）", () => {
  resetState();
  T.state.objective = " 修复登录超时 ";
  T.state.root = " T:\\demo ";
  T.state.writePathsRaw = "src/, tests/";
  T.state.treeDepth = 3;
  T.state.strategy = "team";
  T.state.selectedTemplate = "";
  const body = T.buildCreateBody(T.state);
  assert.equal(body.objective, "修复登录超时");
  assert.equal(body.strategy, "team");
  assert.equal(body.mode, "team", "team 策略显式带 mode");
  assert.equal(body.workspace.root, "T:\\demo");
  assert.equal(body.workspace.read_only, true, "默认只读");
  assert.deepEqual(body.workspace.write_allowed_paths, ["src/", "tests/"]);
  assert.equal(body.workspace.tree_depth, 3);
  assert.ok(!("template_id" in body));

  T.state.strategy = "single";
  T.state.selectedTemplate = "code-change-v1";
  const b2 = T.buildCreateBody(T.state);
  assert.equal(b2.mode, "single");
  assert.equal(b2.template_id, "code-change-v1");

  T.state.strategy = "auto";
  const b3 = T.buildCreateBody(T.state);
  assert.ok(!("mode" in b3), "auto 不带 mode（缺省 Team 由策略判定）");

  T.state.root = "";
  const b4 = T.buildCreateBody(T.state);
  assert.ok(!("workspace" in b4), "root 为空时不带 workspace");
});

test("buildCreateBody：tree_depth 越界收敛到 1–8", () => {
  resetState();
  T.state.objective = "x";
  T.state.root = "T:\\demo";
  T.state.treeDepth = 99;
  assert.equal(T.buildCreateBody(T.state).workspace.tree_depth, 8);
  T.state.treeDepth = 0;
  assert.equal(T.buildCreateBody(T.state).workspace.tree_depth, 1);
});

// ---------- 预览 ----------

test("previewFromTemplate：角色/依赖/预算/产物类型", () => {
  const entry = {
    template_id: "research-brief-v1",
    version: 1,
    title: "研究简报",
    roles: [{ role: "researcher", duty: "并行检索" }, { role: "verifier", duty: "证据核验" }],
    edges: [{ from: "researcher", to: "verifier" }],
    budget_calls_total: 9,
    artifact_kinds: ["report"],
  };
  const p = T.previewFromTemplate(entry);
  assert.equal(p.roleCount, 2);
  assert.equal(p.edgeCount, 1);
  assert.equal(p.budget, 9);
  assert.deepEqual(p.artifactKinds, ["report"]);
  assert.equal(T.previewFromTemplate(null), null);
});

test("previewFromStrategy：single 一角色 / team 双角色 / auto 交策略判定", () => {
  const s = T.previewFromStrategy("single");
  assert.equal(s.roleCount, 1);
  assert.match(s.roles[0].role, /producer/);
  const t = T.previewFromStrategy("team");
  assert.ok(t.roleCount === null && t.roles.length === 2);
  const a = T.previewFromStrategy("auto");
  assert.ok(a.roleCount === null && /判定理由/.test(a.note));
});

test("permissionsSummary：只读/受控写入/无允许路径告警", () => {
  resetState();
  assert.match(T.permissionsSummary(T.state), /只读/);
  T.state.readOnly = false;
  assert.match(T.permissionsSummary(T.state), /写入将被拒绝/);
  T.state.writePathsRaw = "src/, docs";
  const s = T.permissionsSummary(T.state);
  assert.match(s, /src/);
  assert.match(s, /docs/);
  assert.match(s, /深度 2/);
});

test("buildPreview：目录命中模板；未命中回退策略预览", () => {
  const catalog = [{ template_id: "code-change-v1", title: "代码变更", version: 1, roles: [{ role: "analyzer" }, { role: "writer" }, { role: "reviewer" }], budget_calls_total: 10 }];
  resetState();
  T.state.selectedTemplate = "code-change-v1";
  const p1 = T.buildPreview(T.state, catalog);
  assert.equal(p1.source, "template");
  assert.equal(p1.roleCount, 3);
  assert.equal(p1.budget, 10);
  T.state.selectedTemplate = "no-such";
  assert.equal(T.buildPreview(T.state, catalog).source, "strategy");
});

// ---------- 目录渲染 ----------

test("目录树读取失败时提供可访问的重试，成功后恢复目录选择器", async () => {
  resetState();
  const treeBox = { innerHTML: "" };
  const fields = {
    "#pl-root": { value: "C:\\demo" },
    "#pl-depth": { value: "2" },
    "#pl-treebox": treeBox,
  };
  T.setRoot({
    querySelector(selector) {
      return fields[selector] || null;
    },
  });
  let attempts = 0;
  T.setTransport({
    get(path) {
      assert.match(path, /^\/workspace\/tree\?/);
      attempts += 1;
      if (attempts === 1 || attempts === 3) return Promise.reject(new Error("tree unavailable"));
      return Promise.resolve({
        root: "C:\\demo",
        depth: 2,
        entries: [{ path: "src", type: "dir" }],
      });
    },
  });

  await T.loadTree();
  assert.equal(T.state.treeLoadFailed, true);
  assert.match(T.state.treeError, /tree unavailable/);
  assert.match(treeBox.innerHTML, /role="alert"/);
  assert.match(treeBox.innerHTML, /data-pl-tree-reload/);
  assert.match(T.viewHtml(), /data-pl-tree-reload/);

  await T.loadTree();
  assert.equal(attempts, 2);
  assert.equal(T.state.treeLoadFailed, false);
  assert.equal(T.state.treePickerOpen, true);
  assert.match(treeBox.innerHTML, /data-pl-tree-path="src"/);

  await T.loadTree();
  assert.equal(T.state.treeLoadFailed, true);
  assert.match(treeBox.innerHTML, /data-pl-tree-reload/);
  assert.match(T.viewHtml(), /目录树读取失败：[\s\S]*data-pl-tree-reload/);
});

test("模板目录失败时展示可访问的重试状态，重试成功后恢复候选", async () => {
  resetState();
  const catalogBox = { innerHTML: "" };
  const templateSelect = { innerHTML: "", value: "" };
  T.setRoot({
    querySelector(selector) {
      if (selector === "#pl-catalog") return catalogBox;
      if (selector === "#pl-template") return templateSelect;
      return null;
    },
  });
  let attempts = 0;
  T.setTransport({
    get() {
      attempts += 1;
      if (attempts === 1) return Promise.reject(new Error("catalog unavailable"));
      return Promise.resolve({
        catalog: [{ template_id: "code-change-v1", title: "代码变更", version: 1, installed: true }],
      });
    },
    post() { throw new Error("unexpected POST"); },
  });

  await T.loadCatalog();
  assert.equal(T.state.catalogLoaded, true);
  assert.match(T.state.catalogError, /catalog unavailable/);
  assert.match(catalogBox.innerHTML, /role="alert"/);
  assert.match(catalogBox.innerHTML, /data-pl-catalog-retry/);
  assert.match(T.viewHtml(), /data-pl-catalog-retry/);
  assert.match(T.viewHtml(), /手动创建/);

  await T.loadCatalog();
  assert.equal(attempts, 2);
  assert.equal(T.state.catalogError, "");
  assert.match(catalogBox.innerHTML, /代码变更/);
  assert.match(templateSelect.innerHTML, /code-change-v1/);
});

test("templateOptionsHtml：未安装禁用 + 选中态", () => {
  const catalog = [
    { template_id: "code-change-v1", title: "代码变更", version: 1, installed: true },
    { template_id: "research-brief-v1", title: "研究简报", version: 1, installed: false },
  ];
  const html = T.templateOptionsHtml(catalog, "code-change-v1");
  assert.match(html, /value="code-change-v1" selected/);
  assert.match(html, /value="research-brief-v1" disabled/);
  assert.match(html, /未安装/);
  assert.match(T.templateOptionsHtml([], ""), /不使用模板/);
});

test("catalogHtml：候选徽标/安装按钮/选用按钮/角色与预算行", () => {
  const catalog = [
    { template_id: "code-change-v1", title: "代码变更", version: 1, installed: true, roles: [{ role: "analyzer" }, { role: "writer" }], budget_calls_total: 10 },
    { template_id: "structured-extract-v1", title: "结构化抽取", installed: false },
  ];
  const html = T.catalogHtml(catalog, {});
  assert.match(html, /data-pl-install="structured-extract-v1"/);
  assert.match(html, /data-pl-install="structured-extract-v1"[^>]*data-core-action|data-core-action[^>]*data-pl-install="structured-extract-v1"/);
  const busyHtml = T.catalogHtml(catalog, { "structured-extract-v1": true });
  assert.match(busyHtml, /data-core-busy="true"[^>]*data-pl-install="structured-extract-v1"[^>]*disabled/);
  assert.match(T.viewHtml(), /id="pl-create"[^>]*data-core-action/);
  assert.match(html, /data-pl-select="code-change-v1"/);
  assert.match(html, /已安装/);
  assert.match(html, /预算 10 次/);
  assert.match(T.catalogHtml([], {}), /目录为空/);
});

test("normTemplate：三路实现嵌套形状（template.template_id/name/depends_on/预算逐角色）", () => {
  const raw = {
    installed: false,
    template: {
      template_id: "code-change-v1",
      name: "代码变更（分析 → 单写者实现 → 只读审查）",
      mode: "team",
      roles: [
        { role: "code_analyzer", depends_on: [], handoff_contract: "只读分析" },
        { role: "implementer", depends_on: ["code_analyzer"], handoff_contract: "单写者" },
        { role: "reviewer", depends_on: ["implementer"], handoff_contract: "只读审查" },
      ],
    },
    budget_calls_per_role: [
      { role: "code_analyzer", budget_calls: 3 },
      { role: "implementer", budget_calls: 5 },
      { role: "reviewer", budget_calls: 3 },
    ],
    artifact_kinds: ["analysis", "code"],
  };
  const n = T.normTemplate(raw);
  assert.equal(n.id, "code-change-v1");
  assert.match(n.title, /代码变更/);
  assert.equal(n.version, null);
  assert.equal(n.roles.length, 3);
  assert.equal(n.roles[1].duty, "单写者");
  assert.equal(n.edgeCount, 2, "depends_on 折算为边");
  assert.equal(n.budgetTotal, 11, "逐角色预算求和");
  assert.deepEqual(n.kinds, ["analysis", "code"]);
  assert.equal(T.normTemplate(null), null);
  assert.equal(T.normTemplate({ installed: true }), null, "无 id 拒绝");

  // 渲染走嵌套形状：安装按钮与角色行在场
  const html = T.catalogHtml([raw], {});
  assert.match(html, /data-pl-install="code-change-v1"/);
  assert.match(html, /code_analyzer/);
  assert.match(html, /预算 11 次/);
});

// ---------- 行为：创建流程（无 DOM，传输层注入） ----------

function makeTransport() {
  const calls = { post: [], get: [] };
  return {
    calls,
    get(path) {
      calls.get.push(path);
      return Promise.resolve({ catalog: [] });
    },
    post(path, body) {
      calls.post.push({ path, body });
      return Promise.resolve({ team_id: "team-x1", project_space_id: "proj-team-x1", strategy_decision: { mode: "single" } });
    },
  };
}

test("doCreate：校验失败零请求；成功一次 POST /teams 且结果进入状态", async () => {
  resetState();
  const tr = makeTransport();
  T.setTransport(tr);
  T.state.objective = "";
  T.doCreate(); // 校验失败：不得发请求
  await drain();
  assert.equal(tr.calls.post.length, 0);

  T.state.objective = "修复登录超时";
  T.state.root = "T:\\demo";
  T.state.strategy = "auto";
  T.doCreate();
  await drain();
  assert.equal(tr.calls.post.length, 1);
  assert.equal(tr.calls.post[0].path, "/teams");
  assert.equal(tr.calls.post[0].body.workspace.root, "T:\\demo");
  assert.equal(T.state.result.team_id, "team-x1");
  assert.equal(T.state.error, "");
});

test("doCreate：快速双击只发一次请求（提交锁）", async () => {
  resetState();
  const tr = makeTransport();
  T.setTransport(tr);
  T.state.objective = "修复";
  T.state.root = "T:\\demo";
  T.doCreate();
  T.doCreate(); // creating 锁内第二次调用
  T.doCreate();
  await drain();
  assert.equal(tr.calls.post.length, 1);
  assert.equal(T.state.creating, false, "结束后解锁");
});

test("doCreate：服务端 400 错误进入友好错误且解锁", async () => {
  resetState();
  T.setTransport({
    get() { return Promise.resolve({ catalog: [] }); },
    post() { return Promise.reject(new Error("400: 缺少 objective")); },
  });
  T.state.objective = "x";
  T.state.root = "T:\\demo";
  T.doCreate();
  await drain();
  assert.match(T.state.error, /400|请求被拒绝/);
  assert.equal(T.state.creating, false);
});

test("doCreate：当前页面创建成功后保留组队理由等待用户打开", async () => {
  resetState();
  const clicks = [];
  let opened = null;
  globalThis.document = {
    querySelector: (selector) => selector === '#panelNav button[data-panel="workswarm"]'
      ? { click: () => clicks.push("nav") }
      : null,
  };
  const panels = globalThis.OwoPanels;
  const previousWorkswarm = panels.workswarm;
  panels.workswarm = { open: (id) => { opened = id; } };
  T.setRoot({ querySelector() { return null; } });
  T.setTransport({
    get() { return Promise.resolve({ catalog: [] }); },
    post() { return Promise.resolve({ team_id: "team-created" }); },
  });
  T.state.objective = "创建并执行项目任务";
  T.state.root = "C:\demo";
  T.doCreate();
  await drain();
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.deepEqual(clicks, []);
  assert.equal(opened, null);
  assert.match(T.resultHtml(T.state.result), /pl-goto-team/);
  delete globalThis.document;
  if (previousWorkswarm === undefined) delete panels.workswarm;
  else panels.workswarm = previousWorkswarm;
});

test("gotoTeam：切导航按钮并调 workswarm.open(team_id)", async () => {
  // 最小 document/OwoPanels shim
  const clicks = [];
  const navBtn = { click: () => clicks.push("nav") };
  globalThis.document = { querySelector: (sel) => (sel === '#panelNav button[data-panel="workswarm"]' ? navBtn : null) };
  let opened = null;
  win().OwoPanels.workswarm = { open: (id) => (opened = id) };
  T.gotoTeam("team-x1");
  await new Promise((r) => setTimeout(r, 90));
  assert.deepEqual(clicks, ["nav"]);
  assert.equal(opened, "team-x1");
  delete globalThis.document;
});

function win() {
  return typeof window !== "undefined" ? window : globalThis;
}

// ---------- 渲染守卫 ----------

test("viewHtml：七步骨架 + 错误区 aria-live + 预览区在场", () => {
  resetState();
  const html = T.viewHtml();
  assert.match(html, /id="pl-objective"/);
  assert.match(html, /id="pl-root"/);
  assert.match(html, /id="pl-readonly" checked/);
  assert.match(html, /id="pl-writebox" hidden/);
  assert.match(html, /id="pl-strategy"/);
  assert.match(html, /id="pl-template"/);
  assert.match(html, /id="pl-preview"/);
  assert.match(html, /id="pl-create"[^>]*class="primary"/);
  assert.match(html, /id="pl-create"[^>]*data-core-action/);
  T.state.creating = true;
  const busyHtml = T.viewHtml();
  assert.match(busyHtml, /id="pl-create"[^>]*data-core-busy="true"[^>]*disabled[^>]*aria-busy="true"/);
  assert.match(html, /id="pl-errors" aria-live="polite"/);
  assert.match(html, /id="pl-catalog"/);
  assert.match(html, /id="pl-result"/);
  assert.ok(!html.includes("##"), "不得出现 '## 选择器' 回归");
});

test("viewHtml：写入模式展开 + 只读隐藏切换", () => {
  resetState();
  T.state.readOnly = false;
  T.state.writePathsRaw = "src/";
  const html = T.viewHtml();
  assert.ok(!html.includes('id="pl-writebox" hidden'), "写入模式不隐藏写路径框");
  assert.match(html, /允许写入路径/);
});

test("resultHtml：team_id 与直达按钮", () => {
  const html = T.resultHtml({ team_id: "team-x1", project_space_id: "proj-team-x1" });
  assert.match(html, /team-x1/);
  assert.match(html, /id="pl-goto-team"/);
  assert.equal(T.resultHtml(null), "");
});

// ---------- WorkSwarm 详情六期补充（同面板内纯函数） ----------

test("workspaceFromTeam：完整/缺字段/旧记录", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  const full = wsw.workspaceFromTeam({ workspace: { root: "T:\\demo", read_only: false, write_allowed_paths: ["src/"], tree_depth: 3 } });
  assert.equal(full.root, "T:\\demo");
  assert.equal(full.readOnly, false);
  assert.deepEqual(full.writePaths, ["src/"]);
  assert.equal(full.treeDepth, 3);
  assert.equal(wsw.workspaceFromTeam({ workspace: null }), null);
  assert.equal(wsw.workspaceFromTeam({ workspace: {} }), null, "无 root 视为未绑定");
  assert.equal(wsw.workspaceFromTeam(null), null);
});

test("workspaceBoxHtml：未绑定空态/绑定内容/目录树与 Git 按钮", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  assert.match(wsw.workspaceBoxHtml(null, null, "", false), /未绑定项目工作区/);
  const ws = { root: "T:\\demo", readOnly: false, writePaths: ["src/"], treeDepth: 2 };
  const html = wsw.workspaceBoxHtml(ws, null, "", false);
  assert.match(html, /id="ws-ws-tree"/);
  assert.match(html, /id="ws-ws-git"/);
  assert.match(html, /T:\\demo/);
  assert.match(html, /src\//);
});

test("workspaceTreeHtml：层级缩进渲染；空条目空态", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  const html = wsw.workspaceTreeHtml({ entries: [{ path: "src", type: "dir" }, { path: "src/main.rs", type: "file", size: 10 }] });
  assert.match(html, /src/);
  assert.match(html, /main\.rs/);
  assert.match(html, /padding-left/);
  assert.match(wsw.workspaceTreeHtml({ entries: [] }), /目录为空/);
});

test("gitStatusHtml：非 Git 仓库不报错；分支与变更行", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  assert.match(wsw.gitStatusHtml({ is_git_repo: false }), /不是 Git 仓库/);
  const html = wsw.gitStatusHtml({ is_git_repo: true, branch: "main", clean: false, entries: [{ path: "a.rs", state: "M" }] });
  assert.match(html, /main/);
  assert.match(html, /a\.rs/);
});

test("templateBoxHtml：动态组队提示/模板 id 与版本", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  assert.match(wsw.templateBoxHtml({}, null), /动态组队/);
  const html = wsw.templateBoxHtml({ template_id: "code-change-v1" }, { template_id: "code-change-v1", version: 1, title: "代码变更" });
  assert.match(html, /代码变更/);
  assert.match(html, /v1/);
});

test("failureCodeLabel / failureBadgeHtml：三码映射与 error 前缀兜底", () => {
  const wsw = require(join(here, "../panels/workswarm.panel.js"))._test;
  assert.equal(wsw.failureCodeLabel("output_contract_invalid"), "输出契约无效");
  assert.equal(wsw.failureCodeLabel("artifact_missing"), "缺少交付物");
  assert.equal(wsw.failureCodeLabel("scope_violation"), "越权访问");
  assert.equal(wsw.failureCodeLabel("other"), "");
  assert.match(wsw.failureBadgeHtml({ status: "Failed", failure_code: "artifact_missing" }), /缺少交付物/);
  assert.match(wsw.failureBadgeHtml({ status: "Failed", error: "output_contract_invalid: JSON 不可解析" }), /输出契约无效/);
  assert.equal(wsw.failureBadgeHtml({ status: "Succeeded" }), "");
  const sum = wsw.failureSummaryHtml([
    { task_id: "t1", status: "Failed", failure_code: "scope_violation", error: "越界写 /etc" },
    { task_id: "t2", status: "Succeeded" },
  ]);
  assert.match(sum, /t1/);
  assert.match(sum, /越权访问/);
  assert.ok(!sum.includes("t2"));
});

// ---------- 样式守卫 ----------

test("Project Launcher 当前 DOM 具有布局、长路径换行与目录树滚动样式", () => {
  assert.match(shellCss, /20\. Project Launcher/);
  assert.match(shellCss, /\.owo-pl \{ display: flex; flex-direction: column;[^}]*min-width: 0/);
  assert.match(shellCss, /\.owo-pl-step \{[^}]*min-width: 0/);
  assert.match(shellCss, /\.owo-pl-roles \{ display: flex; flex-wrap: wrap;/);
  assert.match(shellCss, /\.owo-pl-cats \{[^}]*min-width: 0/);
  assert.match(shellCss, /\.owo-pl-badge/);
  assert.match(shellCss, /\.owo-pl-tree-list \{[^}]*overflow: auto/);
  assert.match(shellCss, /\.owo-pl-tree-row \{[^}]*overflow-wrap: anywhere/);
  assert.match(shellCss, /@media \(max-width: 1280px\)/);
});

// ---------- 七期：预览"实际执行权限" ----------

test("rolePermissions：只读工作区全员只读；评审角色恒只读；可写按路径下发；未声明路径拒绝写入", () => {
  resetState();
  // 默认只读工作区：所有角色 read_only，无命令执行
  const ro = T.rolePermissions("producer", T.state);
  assert.equal(ro.read_only, true);
  assert.equal(ro.can_run_command, false);
  assert.deepEqual(ro.write_allowed_paths, []);
  assert.equal(ro.write_mode, "read_only");
  // 可写工作区：producer 按允许路径下发
  T.state.readOnly = false;
  T.state.writePathsRaw = "src/, tests/";
  const w = T.rolePermissions("producer", T.state);
  assert.equal(w.read_only, false);
  assert.equal(w.can_run_command, true);
  assert.deepEqual(w.write_allowed_paths, ["src/", "tests/"]);
  assert.equal(w.write_mode, "scoped");
  // 可写工作区但未声明路径：写入将被拒绝
  T.state.writePathsRaw = "";
  const denied = T.rolePermissions("producer", T.state);
  assert.equal(denied.read_only, false);
  assert.deepEqual(denied.write_allowed_paths, []);
  assert.equal(denied.write_mode, "denied");
  // 评审角色在工作区可写时仍然恒只读
  T.state.writePathsRaw = "src/";
  const critic = T.rolePermissions("critic", T.state);
  assert.equal(critic.read_only, true);
  assert.equal(critic.can_run_command, false);
  assert.deepEqual(critic.write_allowed_paths, []);
  assert.equal(critic.write_mode, "read_only");
  // 浏览器能力按角色画像推导，与读写正交（只读 researcher 也可浏览）
  assert.equal(T.rolePermissions("researcher", { readOnly: true }).can_use_browser, true);
  assert.equal(T.rolePermissions("implementer", T.state).can_use_browser, false);
  assert.equal(T.rolePermissions("Web Searcher", { readOnly: true }).can_use_browser, true);
  // 容错：空/null
  assert.equal(T.rolePermissions("", T.state).role, "");
  assert.equal(T.rolePermissions(null, null).read_only, true, "null 状态按默认只读处理");
});

test("rolePermText：三写模式文案 + 命令/浏览器附加能力", () => {
  assert.match(T.rolePermText({ role: "producer", read_only: true, can_run_command: false, can_use_browser: false, write_allowed_paths: [], write_mode: "read_only" }), /实际执行权限：只读/);
  const scoped = T.rolePermText({ role: "producer", read_only: false, can_run_command: true, can_use_browser: false, write_allowed_paths: ["src/", "tests/"], write_mode: "scoped" });
  assert.match(scoped, /可写 src\/、tests\//);
  assert.match(scoped, /可执行命令/);
  assert.match(T.rolePermText({ role: "p", read_only: false, can_run_command: true, can_use_browser: true, write_allowed_paths: [], write_mode: "denied" }), /写入被拒绝（未声明允许路径）/);
  assert.match(T.rolePermText({ role: "r", read_only: false, can_run_command: false, can_use_browser: true, write_allowed_paths: ["x/"], write_mode: "scoped" }), /可浏览网页/);
  assert.equal(T.rolePermText({ role: "" }), "");
  assert.equal(T.rolePermText(null), "");
});

test("previewHtml：每角色附实际执行权限行（模板与策略两来源）", () => {
  resetState();
  T.state.readOnly = false;
  T.state.writePathsRaw = "src/";
  // 模板来源：producer 可写路径、critic 只读
  const tpl = T.previewFromTemplate({
    template_id: "code-change-v1",
    version: 1,
    title: "代码变更",
    roles: [{ role: "producer", duty: "实现" }, { role: "critic", duty: "评审" }],
  });
  const html = T.previewHtml(tpl, T.state);
  assert.match(html, /owo-pl-role/);
  assert.match(html, /实际执行权限：可写 src\//, "producer 显示可写路径");
  assert.match(html, /实际执行权限：只读/, "critic 恒只读");
  // 默认只读工作区：所有角色只读
  T.state.readOnly = true;
  T.state.writePathsRaw = "";
  const htmlRo = T.previewHtml(tpl, T.state);
  assert.ok(!htmlRo.includes("可写"), "只读工作区预览不出现可写文案");
  assert.equal((htmlRo.match(/实际执行权限：只读/g) || []).length, 2, "两角色均只读");
  // 策略来源（team）：producer + critic 同样带权限行
  const strat = T.previewFromStrategy("team");
  assert.match(T.previewHtml(strat, T.state), /实际执行权限：只读/);
  // single 来源：单 producer
  const single = T.previewFromStrategy("single");
  const htmlSingle = T.previewHtml(single, { readOnly: false, writePathsRaw: "docs/" });
  assert.match(htmlSingle, /实际执行权限：可写 docs\//);
});

test("模板安装失败会释放锁，用户可以再次安装", async () => {
  resetState();
  const root = {
    querySelector(selector) {
      if (selector === "#pl-catalog") return { innerHTML: "", querySelector: () => null };
      if (selector === "#pl-errors") return { innerHTML: "" };
      return null;
    },
  };
  T.setRoot(root);
  let posts = 0;
  T.setTransport({
    get() { return Promise.resolve({ catalog: [] }); },
    post() {
      posts++;
      return Promise.reject(new Error("temporary install failure"));
    },
  });

  await T.installTemplate("example-v1");
  assert.equal(T.state.installBusy["example-v1"], undefined);
  await T.installTemplate("example-v1");
  assert.equal(posts, 2, "失败后第二次调用必须重新发送安装请求");
  assert.equal(T.state.installBusy["example-v1"], undefined);
});

test("过期目录请求不能覆盖后发出的模板目录", async () => {
  resetState();
  const catalogBox = { innerHTML: "" };
  const templateSelect = { innerHTML: "", value: "" };
  T.setRoot({
    querySelector(selector) {
      if (selector === "#pl-catalog") return catalogBox;
      if (selector === "#pl-template") return templateSelect;
      return null;
    },
  });
  let resolveOld;
  let request = 0;
  T.setTransport({
    get() {
      request++;
      if (request === 1) return new Promise((resolve) => { resolveOld = resolve; });
      return Promise.resolve({ catalog: [{ template_id: "new", title: "新目录模板", installed: true }] });
    },
  });
  const oldLoad = T.loadCatalog();
  await Promise.resolve();
  await T.loadCatalog();
  resolveOld({ catalog: [{ template_id: "old", title: "旧目录模板", installed: true }] });
  await oldLoad;
  assert.equal(T.state.catalog[0].template_id, "new");
  assert.match(catalogBox.innerHTML, /新目录模板/);
  assert.doesNotMatch(catalogBox.innerHTML, /旧目录模板/);
});

test("dispose 使在途目录树响应失效", async () => {
  resetState();
  const treeBox = { innerHTML: "" };
  T.state.root = "C:\\demo";
  T.setRoot({
    querySelector(selector) {
      if (selector === "#pl-root") return { value: "C:\\demo" };
      if (selector === "#pl-depth") return { value: "2" };
      if (selector === "#pl-treebox") return treeBox;
      return null;
    },
  });
  let resolveTree;
  T.setTransport({ get() { return new Promise((resolve) => { resolveTree = resolve; }); } });
  const pending = T.loadTree();
  await Promise.resolve();
  panel.dispose();
  resolveTree({ root: "C:\\demo", depth: 2, entries: [{ path: "late", type: "dir" }] });
  await pending;
  assert.equal(T.state.treeEntries, null);
  assert.equal(T.state.treePickerOpen, false);
  assert.equal(T.state.treeBusy, false);
});

test("dispose 在创建请求进行中可释放 UI 锁且不触碰已卸载根节点", async () => {
  resetState();
  T.state.objective = "创建项目";
  T.state.root = "C:\\demo";
  let rootQueries = 0;
  T.setRoot({ querySelector() { rootQueries++; return null; } });
  let resolveCreate;
  T.setTransport({ post() { return new Promise((resolve) => { resolveCreate = resolve; }); } });
  T.doCreate();
  await Promise.resolve();
  const queriesBeforeDispose = rootQueries;
  panel.dispose();
  resolveCreate({ team_id: "created-while-away" });
  await drain();
  assert.equal(T.state.creating, false);
  assert.equal(T.state.result.team_id, "created-while-away");
  assert.equal(rootQueries, queriesBeforeDispose, "卸载后不再查找或写入旧根节点");
});

test("旧面板的创建响应不污染后来挂载的页面，并释放新页面的创建锁", async () => {
  resetState();
  T.state.objective = "创建项目";
  T.state.root = "C:\\demo";
  let resolveCreate;
  T.setTransport({
    get() { return Promise.resolve({ catalog: [] }); },
    post() { return new Promise((resolve) => { resolveCreate = resolve; }); },
  });
  const oldRoot = { querySelector() { return null; } };
  T.setRoot(oldRoot);
  T.doCreate();
  await Promise.resolve();
  panel.dispose();

  const resultBox = { innerHTML: "新页面结果" };
  const errorsBox = { innerHTML: "新页面错误" };
  const button = { disabled: true, textContent: "创建中…" };
  const newRoot = {
    querySelector(selector) {
      if (selector === "#pl-result") return resultBox;
      if (selector === "#pl-errors") return errorsBox;
      if (selector === "#pl-create") return button;
      return null;
    },
  };
  T.setRoot(newRoot);
  resolveCreate({ team_id: "created-in-old-panel" });
  await drain();

  assert.equal(T.state.result.team_id, "created-in-old-panel", "后台创建结果仍保留");
  assert.equal(resultBox.innerHTML, "新页面结果", "旧请求不得覆盖新页面内容");
  assert.equal(errorsBox.innerHTML, "新页面错误");
  assert.equal(button.disabled, false, "新挂载页面的共享创建锁应解开");
  assert.equal(button.textContent, "创建团队并开始执行");
});
