// ============================================================================
// WorkSwarm 面板 Node 前端测试 —— desktop/web/tests/workswarm.panel.test.mjs
//
// 运行：node tests/workswarm.panel.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert / node 内置模块；无 DOM、无网络。
//
// 覆盖面（第四路二轮验收的结构/行为守卫）：
//   - 纯逻辑：状态归一化、运行摘要计算、重试目标/门控、冻结 retry 请求体；
//   - 渲染守卫：DAG 重试按钮可见性、详情骨架（摘要/中断横幅/审计焦点）、
//     列表三态、无 "## 选择器" 回归；
//   - 行为：重试提交锁（快速双击只发一次请求）、终态成功/取消拒绝重试、
//     interrupted 标记经详情响应流入面板状态；
//   - 错误文案：400/404/409/网络/凭据提示映射；
//   - 样式守卫：style.css 第 16 节防溢出/中断/重试/焦点规则在场。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const panel = require(join(here, "../panels/workswarm.panel.js"));
const T = panel._test;
const shellCss = readFileSync(join(here, "../style.css"), "utf8");

const drain = () => new Promise((r) => setTimeout(r, 5));

const TASKS_MIXED = [
  { task_id: "plan", role: "planner", worker: "m-planner", status: "Succeeded", attempts: 1, depends_on: [], error: null },
  {
    task_id: "build",
    role: "builder",
    worker: "m-builder",
    status: "Failed",
    attempts: 3,
    depends_on: ["plan"],
    error: "验证失败：输出缺少单元测试（此处是很长的错误信息，用于验证摘要行的省略与 tooltip 行为）",
  },
  { task_id: "review", role: "critic", worker: "m-critic", status: "Pending", attempts: 0, depends_on: ["build"], error: null },
];

function resetState() {
  T.setRoot(null);
  T.state.conversationSessionAfter = null;
  T.state.handoffBefore = null;
  T.state.handoffNextBefore = null;
  T.state.handoffHasMore = false;
  T.state.handoffBusy = false;
  T.state.current = null;
  T.state.team = null;
  T.state.teamStatus = "";
  T.state.tasks = [];
  T.state.interrupted = false;
  T.state.artifactCount = null;
  T.state.active = false;
  // —— 四期字段 ——
  T.state.progress = null;
  T.state.lastProgressSeq = 0;
  T.state.cancelling = false;
  T.state.pollRequest = null;
  T.state.reviewBusy = {};
  T.state.artifacts = [];
  T.state.artifactLoadError = "";
  T.state.handoffArtsFetched = false;
  T.state.handoffArtsLoading = false;
  T.state.handoffArtsError = "";
  T.state.handoffArtsRequest = null;
  T.state.handoffRefs = { arts: [], evid: [] };
  T.state.handoffRefQuery = { arts: "", evid: "" };
  T.state.reviewResult = null;
  // —— 五期字段 ——
  T.state.strategyDecision = null;
  T.state.metrics = null;
  T.state.deliverables = null;
  T.state.deliverablesOpen = false;
  T.state.reworkBusy = {};
  T.state.reworkResult = null;
  T.state.historyReviews = {};
  T.state.diagnostic = null;
  // —— 八/九期 ChangeSet 字段 ——
  T.state.changeSets = null;
  T.state.csApprovalBlock = null;
  T.state.csBusy = {};
  T.state.csResults = {};
}

// 可记录属性读写的假按钮（模拟 lockBtn/unlockBtn 所需的最小接口）。
function fakeBtn(stepId) {
  const attrs = { "data-ws-retry": stepId };
  return {
    disabled: false,
    textContent: "↻ 重试此节点",
    __idleText: null,
    getAttribute: (k) => (k in attrs ? attrs[k] : null),
    setAttribute: (k, v) => {
      attrs[k] = String(v);
    },
    removeAttribute: (k) => {
      delete attrs[k];
    },
  };
}

test("切换 Team 时保留在途产物评审和返工提交锁", () => {
  resetState();
  T.state.reviewBusy["artifact-1"] = true;
  T.state.reworkBusy["artifact-1"] = true;

  panel.open("team-next");

  assert.equal(T.state.current, "team-next");
  assert.equal(T.state.reviewBusy["artifact-1"], true, "切换团队不能丢失评审中的提交锁");
  assert.equal(T.state.reworkBusy["artifact-1"], true, "切换团队不能丢失返工中的提交锁");
});

test("模块导出与浏览器骨架完好", () => {
  assert.equal(panel.id, "workswarm");
  assert.equal(typeof panel.mount, "function");
  const html = panel.nav();
  assert.match(html, /data-panel="workswarm"/);
  assert.match(html, /<style>/);
  assert.ok(!html.includes("##"), "禁止出现 '##' 非法选择器（历史空白面板回归）");
});

test("WorkSwarm live polling deduplicates slow snapshots and aborts stale requests", async () => {
  resetState();
  const previous = T.getTransport();
  const requests = [
    { resolve: null, reject: null, signal: null },
    { resolve: null, reject: null, signal: null },
  ];
  const pending = requests.map((item) => new Promise((resolve, reject) => {
    item.resolve = resolve;
    item.reject = reject;
  }));
  let index = 0;
  T.state.current = "team-a";
  T.setTransport({
    get(path, options) {
      const item = requests[index];
      item.path = path;
      item.signal = options && options.signal;
      return pending[index++];
    },
  });
  try {
    const first = T.pollOnce();
    const duplicate = T.pollOnce();
    assert.equal(first, duplicate, "interval ticks during a slow poll must join the same request pair");
    assert.equal(index, 2, "only one event snapshot and one detail request should be in flight");
    assert.ok(requests.every((item) => item.signal && item.signal === requests[0].signal));
    assert.ok(requests[0].path.includes("/teams/team-a/events"));
    assert.equal(requests[1].path, "/teams/team-a");

    T.stopLive("已离开团队页面");
    assert.equal(requests[0].signal.aborted, true, "leaving live mode should abort both fetches");
    requests[0].resolve({ active: true, status: "Running", audit: [] });
    requests[1].resolve({ team: { team_id: "team-a" }, tasks: [] });
    await Promise.all([first, duplicate]);
    assert.equal(T.state.streamNote, "已离开团队页面", "late responses must not replace the stop reason");
    assert.equal(T.state.pollRequest, null);
  } finally {
    T.setTransport(previous);
    resetState();
  }
});

test("normStatus / isTerminalTeam 状态归一化", () => {
  assert.equal(T.normStatus("AwaitingHuman"), "awaiting_human");
  assert.equal(T.normStatus("Running"), "running");
  assert.equal(T.normStatus(null), "");
  assert.equal(T.isTerminalTeam("Succeeded"), true);
  assert.equal(T.isTerminalTeam("Cancelled"), true);
  assert.equal(T.isTerminalTeam("Failed"), true);
  assert.equal(T.isTerminalTeam("Running"), false);
});

test("任务选择器将未知状态和所有动态字段作为文本转义", () => {
  const task = {
    task_id: 'task"><img src=x onerror=alert(1)>',
    role: '<svg onload=alert(1)>',
    status: '<img src=x onerror=alert(2)>',
  };

  const html = T.taskOptionHtml(task);
  assert.equal(
    html,
    '<option value="task&quot;&gt;&lt;img src=x onerror=alert(1)&gt;">&lt;svg onload=alert(1)&gt;（&lt;img src=x onerror=alert(2)&gt;）</option>',
  );
  assert.equal(T.taskStatusLabel(task), "&lt;img src=x onerror=alert(2)&gt;");
});

test("computeRunSummary：混合 DAG 计数/失败步骤/累计尝试/阻塞", () => {
  const s = T.computeRunSummary({ team: { status: "Failed" }, tasks: TASKS_MIXED, artifactCount: 2 });
  assert.deepEqual(s.counts, { total: 3, succeeded: 1, failed: 1, waiting: 1, running: 0, blocked: 1 });
  assert.equal(s.totalAttempts, 4);
  assert.equal(s.failedStep.task_id, "build");
  assert.equal(s.failedStep.attempts, 3);
  assert.equal(s.phase, "已停止：存在失败步骤");
  assert.equal(s.canRetry, true);
  assert.equal(s.artifactCount, 2);
});

test("computeRunSummary：中断团队不再显示为正常执行中", () => {
  const s = T.computeRunSummary({ team: { status: "Running" }, tasks: [], interrupted: true });
  assert.equal(s.statusKey, "interrupted");
  assert.equal(s.statusLabel, "已中断（可恢复）");
  assert.equal(s.phase, "已中断，可恢复");
  assert.notEqual(s.phase, "调度中");
});

test("computeRunSummary：终态中断标记不覆盖终态显示；产物未知显示 null", () => {
  const s = T.computeRunSummary({ team: { status: "Failed" }, tasks: TASKS_MIXED, interrupted: true, artifactCount: null });
  assert.equal(s.statusKey, "failed");
  assert.equal(s.statusLabel, "失败");
  assert.equal(s.artifactCount, null);
});

test("computeRunSummary：等待人节点/运行阶段推导", () => {
  const awaiting = T.computeRunSummary({
    team: { status: "AwaitingHuman" },
    tasks: [{ task_id: "h", role: "human-review", worker: "u-1", status: "Ready", attempts: 0, depends_on: [] }],
  });
  assert.equal(awaiting.phase, "等待人节点结果");
  const running = T.computeRunSummary({
    team: { status: "Running" },
    tasks: [{ task_id: "b", role: "builder", worker: "m-b", status: "Running", attempts: 1, depends_on: [] }],
  });
  assert.equal(running.phase, "执行：builder");
  assert.equal(running.counts.running, 1);
});

test("shouldShowRetry / retryableTasks 门控矩阵", () => {
  assert.equal(T.shouldShowRetry(TASKS_MIXED, "Succeeded"), false);
  assert.equal(T.shouldShowRetry(TASKS_MIXED, "Cancelled"), false);
  assert.equal(T.shouldShowRetry(TASKS_MIXED, "Failed"), true);
  assert.equal(T.shouldShowRetry(TASKS_MIXED, "Running"), true);
  assert.equal(T.shouldShowRetry([], "Failed"), false);
  assert.deepEqual(
    T.retryableTasks(TASKS_MIXED).map((t) => t.task_id),
    ["build"]
  );
  assert.equal(T.isRetryableStep({ status: "Aborted" }), true);
  assert.equal(T.isRetryableStep({ status: "Succeeded" }), false);
});

test("buildRetryBody：第三路冻结契约形状", () => {
  const b = T.buildRetryBody("builder");
  assert.deepEqual(b, { command: "retry", step_id: "builder", note: "重试此节点：builder" });
  assert.equal(Object.keys(b).sort().join(","), "command,note,step_id");
  const b2 = T.buildRetryBody(" builder ", " 修复输入后重试 ");
  assert.equal(b2.step_id, "builder");
  assert.equal(b2.note, "修复输入后重试");
});

test("dagSvg：仅 Failed/Aborted 节点出现重试按钮；终态成功团队不出现", () => {
  const svgFailed = T.dagSvg(TASKS_MIXED, "Failed");
  assert.match(svgFailed, /data-ws-retry="build"/);
  assert.ok(!svgFailed.includes('data-ws-retry="plan"'));
  assert.ok(!svgFailed.includes('data-ws-retry="review"'));
  assert.match(svgFailed, /class="owo-ws-node st-pending blocked"/);
  assert.match(svgFailed, /marker-end="url\(#ws-arrow-dead\)"/);
  const svgDone = T.dagSvg(TASKS_MIXED, "Succeeded");
  assert.ok(!svgDone.includes("data-ws-retry"), "终态成功团队不得出现重试入口");
});

test("三态提示具备读屏告警与加载语义", () => {
  assert.match(T.stateBox("error", "请求失败", "detail"), /role="alert"/);
  assert.match(T.stateBox("loading", "正在加载"), /role="status" aria-live="polite"/);
  assert.match(T.stateBox("error", "请求失败", "detail"), /data-retry="detail"/);
});

test("renderDetail 骨架：运行摘要 / 中断横幅 / 审计键盘焦点 / retry 提示", () => {
  const html = T.renderDetail();
  assert.match(html, /id="ws-d-summary" class="owo-ws-sumbox" aria-live="polite"/);
  assert.match(html, /id="ws-d-interrupted" role="alert"/);
  assert.match(html, /运行已中断，可恢复/);
  assert.match(html, /id="ws-d-audit"[^>]*tabindex="0"/);
  assert.match(html, /继续 \/ 重试失败节点 \/ 转向 \/ 更换成员 \/ 取消/, "运行操作提示必须用产品语言，不得出现 API 路径");
  assert.match(html, /id="ws-act-continue"/);
  assert.match(html, /id="ws-act-cancel"/);
  assert.ok(!html.includes("##"));
});

test("renderList 骨架：加载三态在场", () => {
  const html = T.renderList();
  assert.match(html, /id="ws-list-body"/);
  assert.match(html, /owo-ws-loading/);
});

test("explainError：400/404/409/网络/凭据映射", () => {
  const e409 = T.explainError({ message: '409: {"error":"任务 build 已成功，不能 retry"}' }, "重试");
  assert.match(e409, /状态冲突（409）/);
  const e404 = T.explainError({ message: '404: {"error":"任务 x 不存在"}' }, "重试");
  assert.match(e404, /资源不存在（404）/);
  const e400 = T.explainError({ message: '400: {"error":"retry 需要 step_id 字段"}' }, "重试");
  assert.match(e400, /请求被拒绝（400）/);
  assert.match(T.explainError({ message: "Failed to fetch" }), /无法连接 owo-agent-server/);
  const cred = T.explainError({ message: '500: {"error":"OPENAI_API_KEY missing"}' });
  assert.match(cred, /环境变量/);
});

test("WorkSwarm 错误态中的交付物与评审历史重试按钮会重新发起对应请求", async () => {
  resetState();
  T.state.current = "team-1";
  T.state.team = { team_id: "team-1", project_space_id: "project-1" };
  T.state.deliverablesOpen = true;
  const historyTarget = {
    innerHTML: "",
    getAttribute(name) { return name === "data-art-history-box" ? "art-1" : null; },
  };
  const artifactBox = {
    querySelectorAll() { return [historyTarget]; },
  };
  const deliverableBox = { hidden: false, innerHTML: "" };
  T.setRoot({
    querySelector(selector) {
      if (selector === "#ws-d-artifacts") return artifactBox;
      if (selector === "#ws-dlv-box") return deliverableBox;
      return null;
    },
  });

  const calls = [];
  T.setTransport({
    get(path) {
      calls.push(path);
      if (path === "/projects/project-1/deliverables") {
        if (calls.filter((x) => x === path).length === 1) return Promise.reject(new Error("deliverables unavailable"));
        return Promise.resolve({ approved: [{ artifact_id: "art-approved", kind: "report", version: 1 }] });
      }
      if (path === "/artifacts/art-1/history") {
        if (calls.filter((x) => x === path).length === 1) return Promise.reject(new Error("history unavailable"));
        return Promise.resolve({ reviews: [{ review_id: "review-1", decision: "approve" }] });
      }
      throw new Error("unexpected GET " + path);
    },
  });

  await T.loadDeliverables();
  assert.match(deliverableBox.innerHTML, /role="alert"/);
  assert.match(deliverableBox.innerHTML, /data-retry="deliverables"/);
  await T.retryAction("deliverables");
  assert.equal(T.state.deliverables.approved[0].artifactId, "art-approved");
  assert.doesNotMatch(deliverableBox.innerHTML, /data-retry="deliverables"/);

  await T.loadArtifactHistory("art-1");
  assert.match(historyTarget.innerHTML, /role="alert"/);
  assert.match(historyTarget.innerHTML, /data-retry="history-art-1"/);
  await T.retryAction("history-art-1");
  assert.equal(T.state.historyReviews["art-1"][0].review_id, "review-1");
  assert.doesNotMatch(historyTarget.innerHTML, /data-retry="history-art-1"/);
  assert.deepEqual(calls, [
    "/projects/project-1/deliverables",
    "/projects/project-1/deliverables",
    "/artifacts/art-1/history",
    "/artifacts/art-1/history",
  ]);
});

test("handleRetryClick：快速双击只产生一次请求；结束后解锁并恢复文案", async () => {
  resetState();
  T.state.current = "team-1";
  T.state.team = { team_id: "team-1", status: "Failed" };
  T.state.teamStatus = "Failed";
  const posts = [];
  T.setTransport({
    get: () =>
      Promise.resolve({ team: { team_id: "team-1", status: "Failed" }, tasks: TASKS_MIXED, interrupted: false, audit_tail: [] }),
    post: (path, body) => {
      posts.push({ path, body });
      return Promise.resolve({ team_id: "team-1", status: "Running", interrupted: false });
    },
  });
  const btn = fakeBtn("build");
  assert.equal(T.handleRetryClick(btn), true);
  assert.equal(btn.getAttribute("data-busy"), "1"); // 锁在入口同步生效
  assert.equal(T.handleRetryClick(btn), false); // 双击第二击被丢弃
  assert.equal(T.handleRetryClick(fakeBtnDisabled()), false); // 禁用不触发
  await drain();
  await drain();
  assert.equal(posts.length, 1, "快速双击必须只发一次请求");
  assert.equal(posts[0].path, "/teams/team-1/steer");
  assert.deepEqual(posts[0].body, { command: "retry", step_id: "build", note: "重试此节点：build" });
  assert.equal(btn.getAttribute("data-busy"), null, "结束后必须解锁");
  assert.equal(btn.getAttribute("aria-busy"), null);
  assert.equal(btn.disabled, false);
  assert.equal(btn.textContent, "↻ 重试此节点");
});

function fakeBtnDisabled() {
  const b = fakeBtn("build");
  b.disabled = true;
  return b;
}

test("submitRetry：终态成功/取消团队拒绝且零请求", async () => {
  resetState();
  T.state.current = "team-2";
  T.state.team = { team_id: "team-2", status: "Succeeded" };
  T.state.teamStatus = "Succeeded";
  const posts = [];
  T.setTransport({ post: (p, b) => (posts.push({ p, b }), Promise.resolve({})) });
  await T.submitRetry("build");
  assert.equal(posts.length, 0);
  T.state.teamStatus = "Cancelled";
  T.state.team = { team_id: "team-2", status: "Cancelled" };
  await T.submitRetry("build");
  assert.equal(posts.length, 0);
});

test("syncDetail：interrupted 标记从详情响应流入面板状态", async () => {
  resetState();
  T.state.current = "team-3";
  T.setTransport({
    get: () =>
      Promise.resolve({ team: { team_id: "team-3", status: "Running" }, tasks: [], interrupted: true, audit_tail: [] }),
  });
  await T.syncDetail();
  assert.equal(T.state.interrupted, true);
  assert.equal(T.state.team.team_id, "team-3");
});

test("style.css 第 16 节守卫：inline 换行/中断徽标/重试按钮/表格包裹/窄栏断点/焦点环", () => {
  const cssText = shellCss.replace(/\s+/g, " ");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-inline { display: flex; flex-wrap: wrap"), "inline 行必须可换行（防 1280 溢出）");
  assert.ok(cssText.includes(".owo-ws-badge.st-interrupted"), "中断徽标样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-interrupted"), "中断横幅样式在场");
  assert.ok(cssText.includes("button.owo-ws-retry"), "重试按钮样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-tablewrap"), "表格横向滚动包裹在场");
  assert.ok(cssText.includes("@media (max-width: 1280px)"), "窄栏断点在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-audit:focus-visible"), "审计区键盘焦点环在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-ellip"), "长文本省略工具类在场");
});

// ==================== 四期：实时进度（progress 事件） ====================

// 纯函数测试固定参考时钟：fixture 与断言共用同一时刻，禁止读取系统时钟
// （重构方案 3.2：消除耗时/退避/TTL 类断言对真实时钟的依赖，避免慢 CI 偶发失败）。
const FIXED_NOW = Date.parse("2026-01-01T00:00:00.000Z");

const PROGRESS_EVT = {
  seq: 18,
  team_id: "team-1",
  status: "Running",
  active: true,
  current_steps: [
    { step_id: "builder", worker: "builder", status: "Running", attempts: 1, started_at: new Date(FIXED_NOW - 90_000).toISOString() },
  ],
  counts: { pending: 2, running: 1, succeeded: 1, failed: 0 },
  updated_at: new Date(FIXED_NOW).toISOString(),
};

test("applyProgress：合法事件入状态并同步 teamStatus/active", () => {
  resetState();
  assert.equal(T.applyProgress(PROGRESS_EVT), true);
  assert.equal(T.state.progress.seq, 18);
  assert.equal(T.state.teamStatus, "Running");
  assert.equal(T.state.active, true);
  assert.equal(T.state.lastProgressSeq, 18);
});

test("applyProgress：seq 单调守卫——旧/重复/无 seq 一律跳过（断线重连去重）", () => {
  resetState();
  T.applyProgress({ ...PROGRESS_EVT, seq: 10 });
  assert.equal(T.applyProgress({ ...PROGRESS_EVT, seq: 10 }), false, "重复 seq 跳过");
  assert.equal(T.applyProgress({ ...PROGRESS_EVT, seq: 9 }), false, "旧 seq 跳过");
  assert.equal(T.state.progress.seq, 10, "状态未被旧事件覆盖");
  assert.equal(T.applyProgress({ status: "Running", current_steps: [] }), false, "无 seq 不采纳");
  assert.equal(T.applyProgress(null), false);
  assert.equal(T.applyProgress("junk"), false);
  assert.equal(T.state.lastProgressSeq, 10);
  // 新 seq 采纳
  assert.equal(T.applyProgress({ ...PROGRESS_EVT, seq: 11 }), true);
  assert.equal(T.state.progress.seq, 11);
});

test("applyProgress：current_steps 缺 step_id 的行被剔除、缺失字段归零", () => {
  resetState();
  T.applyProgress({ seq: 1, current_steps: [{ worker: "x", status: "Running" }, { step_id: "ok", worker: "w", status: "running" }], counts: null });
  assert.equal(T.state.progress.current_steps.length, 1);
  assert.equal(T.state.progress.current_steps[0].step_id, "ok");
  assert.deepEqual(T.state.progress.counts, { pending: 0, running: 0, succeeded: 0, failed: 0 });
});

test("computeProgressView：耗时基于 started_at，running 标记与状态中文", () => {
  resetState();
  T.applyProgress(PROGRESS_EVT);
  const vm = T.computeProgressView(FIXED_NOW);
  assert.equal(vm.seq, 18);
  assert.deepEqual(vm.counts, { pending: 2, running: 1, succeeded: 1, failed: 0 });
  assert.equal(vm.rows.length, 1);
  assert.equal(vm.rows[0].step_id, "builder");
  assert.equal(vm.rows[0].running, true);
  assert.equal(vm.rows[0].statusCn, "运行中");
  assert.equal(vm.rows[0].elapsedMs, 90_000, "固定时钟下耗时精确等于 started_at 至 FIXED_NOW 的间隔");
  assert.equal(vm.cancelling, false);
});

test("computeProgressView：无 progress → null；started_at 非法 → elapsed null", () => {
  resetState();
  assert.equal(T.computeProgressView(FIXED_NOW), null);
  T.applyProgress({ seq: 2, current_steps: [{ step_id: "s", worker: "w", status: "Succeeded", attempts: 2, started_at: "not-a-date" }] });
  const vm = T.computeProgressView(FIXED_NOW);
  assert.equal(vm.rows[0].elapsedMs, null);
  assert.equal(vm.rows[0].running, false);
});

test("computeProgressView：取消中标志进入视图", () => {
  resetState();
  T.applyProgress({ seq: 3, current_steps: [], counts: {} });
  T.state.cancelling = true;
  assert.equal(T.computeProgressView(FIXED_NOW).cancelling, true);
});

test("renderProgress：空态/计数徽标/取消中/步骤行（worker·状态·尝试·耗时）", () => {
  resetState();
  assert.match(T.renderProgress(null), /暂无实时进度事件/);
  T.applyProgress(PROGRESS_EVT);
  T.state.cancelling = true;
  const html = T.renderProgress(T.computeProgressView(FIXED_NOW));
  assert.match(html, /seq #18/);
  assert.match(html, /等待 <b>2<\/b>/);
  assert.match(html, /运行 <b>1<\/b>/);
  assert.match(html, /完成 <b>1<\/b>/);
  assert.match(html, /失败 <b>0<\/b>/);
  assert.match(html, /取消中/);
  assert.match(html, /第 1 次尝试/);
  assert.match(html, /已运行 /);
  assert.match(html, /owo-ws-prog-step run/, "Running 步骤带高亮类");
});

test("fmtElapsed：秒/分/时边界", () => {
  assert.equal(T.fmtElapsed(0), "0s");
  assert.equal(T.fmtElapsed(59_000), "59s");
  assert.equal(T.fmtElapsed(60_000), "1m");
  assert.equal(T.fmtElapsed(61_000), "1m1s");
  assert.equal(T.fmtElapsed(3_600_000), "1h0m");
  assert.equal(T.fmtElapsed(-5), "—");
  assert.equal(T.fmtElapsed(null), "—");
  assert.equal(T.fmtElapsed(NaN), "—");
});

test("handleEventFrame：嵌套 progress 帧（第二路实现形状）与扁平形状均采纳", () => {
  resetState();
  // 嵌套形状：{type:"progress", progress:{...}}
  T.handleEventFrame({ type: "progress", progress: { seq: 5, status: "Running", active: true, current_steps: [], counts: { pending: 1 } } });
  assert.equal(T.state.progress.seq, 5);
  assert.equal(T.state.teamStatus, "Running");
  // 旧 seq 跳过（含嵌套）
  T.handleEventFrame({ type: "progress", progress: { seq: 5, status: "Running", current_steps: [], counts: {} } });
  assert.equal(T.state.progress.seq, 5);
  // 扁平形状（计划原形）
  T.handleEventFrame({ type: "progress", seq: 6, status: "Running", current_steps: [], counts: {} });
  assert.equal(T.state.progress.seq, 6);
  // state 终态帧清除"取消中"
  T.state.cancelling = true;
  T.handleEventFrame({ type: "state", status: "Cancelled", active: false });
  assert.equal(T.state.cancelling, false);
});

test("handleEventFrame：audit 帧去重入列", () => {
  resetState();
  const audit = { type: "audit", ts: "t1", event: "step.retry", detail: "d" };
  T.handleEventFrame(audit);
  T.handleEventFrame(audit); // 重复帧（断线重放）不得重复渲染
  assert.equal(T.state.audit.length, 1);
});

test("四期样式守卫：style.css 第 18 节 WorkSwarm 进度区/评审闭环/徽标守卫在场", () => {
  const cssText = shellCss.replace(/\s+/g, " ");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-prog-step.run"), "运行中步骤高亮样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-prog-cancelling"), "取消中徽标样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-art-row.head"), "版本链链头样式在场");
  assert.ok(cssText.includes(".owo-ws-badge.rv-ok"), "已批准徽标样式在场");
  assert.ok(cssText.includes(".owo-ws-badge.rv-bad"), "已驳回徽标样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-review-act:disabled"), "评审按钮锁定样式在场");
  assert.ok(cssText.includes(".owo-ws-panel .owo-ws-review-act:focus-visible"), "评审按钮焦点环在场");
  assert.ok(cssText.includes("@keyframes owo-ws-pulse"), "取消中脉冲动画在场");
});

// ============================================================================
// 五期：自适应组队 / 角色指标 / 版本时间线 / 差异 / 返工 / 交付物 / 诊断
// ============================================================================
const STRAT = {
  mode: "single",
  roles: [{ role: "writer", duty: "独立完成交付物" }],
  parallelism: 1,
  budget_per_role: 4,
  reasons: ["无并行子任务", "单 Agent 历史成功率 80%"],
};

test("strategyDecisionOf：完整判定/字符串 reasons/字符串角色/非法输入", () => {
  resetState();
  const d = T.strategyDecisionOf({ strategy_decision: STRAT });
  assert.equal(d.mode, "single");
  assert.equal(d.roles.length, 1);
  assert.equal(d.roles[0].role, "writer");
  assert.equal(d.parallelism, 1);
  assert.equal(d.budgetPerRole, 4);
  assert.equal(d.reasons.length, 2);
  // reason 单数字符串 + roles 字符串形态
  const d2 = T.strategyDecisionOf({ strategy: { mode: "team", reason: "需要独立评审", roles: ["producer", "critic"] } });
  assert.equal(d2.mode, "team");
  assert.deepEqual(d2.reasons, ["需要独立评审"]);
  assert.equal(d2.roles[1].role, "critic");
  // 非法/缺失 → null
  assert.equal(T.strategyDecisionOf(null), null);
  assert.equal(T.strategyDecisionOf({}), null);
  assert.equal(T.strategyDecisionOf({ strategy_decision: { mode: "unknown" } }), null);
  assert.equal(T.strategyDecisionOf({ strategy_decision: { mode: "single" }, team: 1 }).mode, "single");
  const resolved = T.strategyDecisionOf({ strategy_decision: { mode: "team", resolved_roles: [
    { role: "implementer", assignee: "agent", budget_calls: 16, write_paths: ["src"] },
    { role: "reviewer", assignee: "agent", budget_calls: 11 }
  ] } });
  assert.deepEqual(resolved.resolvedRoles.map(function (r) { return [r.role, r.budgetCalls]; }), [["implementer", 16], ["reviewer", 11]]);
  assert.deepEqual(resolved.resolvedRoles[0].writePaths, ["src"]);
});

test("strategyBoxHtml：单/多 Agent 徽标与理由列表", () => {
  resetState();
  const html = T.strategyBoxHtml(T.strategyDecisionOf({ strategy_decision: STRAT }));
  assert.match(html, /单 Agent/);
  assert.match(html, /writer/);
  const budgeted = T.strategyBoxHtml(T.strategyDecisionOf({ strategy_decision: { mode: "team", resolved_roles: [
    { role: "implementer", budget_calls: 16 }, { role: "reviewer", budget_calls: 11 }
  ] } }));
  assert.match(budgeted, /implementer 16/);
  assert.match(budgeted, /reviewer 11/);
  assert.match(html, /每角色调用预算 4/);
  assert.match(html, /无并行子任务/);
  const teamHtml = T.strategyBoxHtml(T.strategyDecisionOf({ strategy_decision: { mode: "team", roles: ["producer", "critic"], reasons: [] } }));
  assert.match(teamHtml, /多 Agent 团队/);
  assert.match(teamHtml, /producer → critic/);
  // 无判定时给可理解空态
  assert.match(T.strategyBoxHtml(null), /策略判定/);
});

test("metricsFromPayload：标准形状/别名形状/空形状", () => {
  resetState();
  const std = T.metricsFromPayload({
    workers: [
      { worker: "m-writer", role: "writer", task_id: "task-7", attempt_id: "attempt-2", phase_epoch: 9, duration_ms: 1200, budget_reservation_wait_ms: 14, model_calls: 3, tokens_in: 100, tokens_out: 50, est_cost: 0.002, attempts: 1, terminal: "Succeeded", failure_reason: "", artifact_ids: ["a1"] },
    ],
    execution_epochs: { epoch_count: 1, items: [
      { phase_epoch: 9, worker_span_count: 1, task_attempt_count: 1, failed_spans: 0, model_calls: 3, prompt_tokens: 100, completion_tokens: 50, usage_known: true, worker_wall_ms_sum: 1200, worker_window_ms: 1200, parallel_overlap_factor: 1.0 },
    ] },
    lifecycle: { epochs: [ { phase_epoch: 9, phase_duration_ms_sum: 2500 } ] },
    context_assembly: {
      sample_count: 2, sample_cap: 1000, sample_truncated: false, audit_scan_truncated: false,
      failures: 0, duration_ms_p50: 12, duration_ms_p90: 18,
      by_role: [{ role: "writer", sample_count: 2, failures: 0, duration_ms_p50: 12, duration_ms_p90: 18 }],
      by_epoch: [{ phase_epoch: 9, sample_count: 2, failures: 0, duration_ms_p50: 12, duration_ms_p90: 18 }],
    },
    summary: { wall_clock_ms: 5000, total_model_calls: 3, budget_reservation_wait_ms_sum: 14, total_tokens_in: 100, total_tokens_out: 50, total_est_cost: 0.002, slowest_worker: "m-writer", failures: 0, reworks: 0, artifact_versions: 1 },
  });
  assert.equal(std.workers.length, 1);
  assert.equal(std.workers[0].modelCalls, 3);
  assert.equal(std.workers[0].durationMs, 1200);
  assert.equal(std.workers[0].budgetReservationWaitMs, 14);
  assert.equal(std.workers[0].taskId, "task-7");
  assert.equal(std.workers[0].attemptId, "attempt-2");
  assert.equal(std.workers[0].phaseEpoch, 9);
  assert.equal(std.executionEpochs.length, 1);
  assert.equal(std.executionEpochs[0].phaseDurationMs, 2500);
  assert.equal(std.executionEpochs[0].usageKnown, true);
  assert.equal(std.contextAssembly.sampleCount, 2);
  assert.equal(std.contextAssembly.byRole[0].durationMsP90, 18);
  assert.equal(std.contextAssembly.byEpoch[0].phaseEpoch, 9);
  assert.equal(std.summary.totalBudgetReservationWaitMs, 14);
  const epochHtml = T.metricsCardsHtml(std, null);
  assert.match(epochHtml, /上下文装配（Worker 内部耗时，不与墙钟相加）/);
  assert.match(epochHtml, /writer <b>P50 12ms \/ P90 18ms/);
  assert.match(epochHtml, /epoch 9 上下文 P50 12ms \/ P90 18ms/);
  assert.match(epochHtml, /执行代次（最近 1）/);
  assert.match(epochHtml, /epoch 9<\/b> · 编排 2.5s · Worker窗口 1.2s/);
  const identityHtml = T.metricsCardsHtml(std, null);
  assert.match(identityHtml, /task task-7 · attempt attempt-2 · epoch 9/);
  assert.equal(std.summary.wallClockMs, 5000);
  assert.equal(std.summary.slowestWorker, "m-writer");
  // 别名：roles/items + calls/cost_usd
  const alias = T.metricsFromPayload({ roles: [{ name: "critic", calls: 1, cost_usd: 0.01 }] });
  assert.equal(alias.workers[0].worker, "critic");
  assert.equal(alias.workers[0].modelCalls, 1);
  assert.equal(alias.workers[0].estCost, 0.01);
  // 预算耗尽原因透传
  const budget = T.metricsFromPayload({ summary: { wall_clock_ms: 1, budget_exhausted: true, budget_reason: "调用预算用尽" } });
  assert.equal(budget.summary.budgetExhausted, true);
  assert.equal(budget.summary.budgetReason, "调用预算用尽");
  const lifecycleOnly = T.metricsFromPayload({ lifecycle: { span_count: 2, stages: [
    { stage: "phase_orchestration", count: 2, failures: 0, duration_ms_p50: 900, duration_ms_p90: 1200 },
  ] } });
  assert.equal(lifecycleOnly.lifecycleStages[0].durationMsP90, 1200);
  const lifecycleHtml = T.metricsCardsHtml(lifecycleOnly, null);
  assert.match(lifecycleHtml, /运行阶段耗时/);
  assert.match(lifecycleHtml, /P50 900ms \/ P90 1.2s/);
  // 完全无指标 → null（UI 保持空态）
  assert.equal(T.metricsFromPayload(null), null);
  assert.equal(T.metricsFromPayload({}), null);
});

test("metricsFromPayload：上下文耗时可独立呈现且标明截断采样", () => {
  const contextOnly = T.metricsFromPayload({
    context_assembly: {
      sample_count: 1000, sample_cap: 1000, sample_truncated: true, audit_scan_truncated: true,
      attribution_missing_count: 2, failures: 1, duration_ms_p50: 5, duration_ms_p90: 20,
      by_role: [{ role: "reviewer", sample_count: 1000, failures: 1, duration_ms_p50: 5, duration_ms_p90: 20 }],
      by_epoch: [],
    },
  });
  assert.equal(contextOnly.contextAssembly.sampleTruncated, true);
  assert.equal(contextOnly.contextAssembly.auditScanTruncated, true);
  assert.match(T.metricsCardsHtml(contextOnly, null), /最近样本，统计可能不完整/);
  assert.match(T.metricsCardsHtml(contextOnly, null), /未归因 2/);
  const legacyOnly = T.metricsFromPayload({ context_assembly: { sample_count: 0, attribution_missing_count: 3 } });
  assert.match(T.metricsCardsHtml(legacyOnly, null), /样本 0.*未归因 3/);
});

test("metricsCardsHtml：汇总条/卡片/预算余量/空态", () => {
  resetState();
  const empty = T.metricsCardsHtml(null, null);
  assert.match(empty, /暂无角色指标/);
  const html = T.metricsCardsHtml(
    T.metricsFromPayload({
      workers: [{ worker: "m-w", role: "writer", duration_ms: 900, budget_reservation_wait_ms: 12, model_calls: 3, terminal: "Succeeded" }],
      summary: { wall_clock_ms: 900, total_model_calls: 3, budget_reservation_wait_ms_sum: 12, failures: 1, reworks: 0 },
    }),
    4
  );
  assert.match(html, /总墙钟/);
  assert.match(html, /预算预留等待/);
  assert.match(html, /最慢|总调用/);
  assert.match(html, /writer/);
  assert.match(html, /余 1）/, "预算余量 = budget - calls");
  assert.match(html, /失败 <b class="bad">1<\/b>/);
  // 无 workers 只有 summary → 仅汇总条
  const sumOnly = T.metricsCardsHtml(T.metricsFromPayload({ summary: { wall_clock_ms: 800 } }), null);
  assert.ok(!sumOnly.includes("owo-ws-mgrid"));
});

test("fmtMs：毫秒/秒/分秒档位", () => {
  assert.equal(T.fmtMs(null), "—");
  assert.equal(T.fmtMs(500), "500ms");
  assert.equal(T.fmtMs(1200), "1.2s");
  assert.equal(T.fmtMs(65000), "1m5s");
});

test("diffLines/diffHtml：LCS 行差异与 +/- 渲染", () => {
  const a = "行一\n行二\n行三";
  const b = "行一\n行二改\n行三\n行四";
  const rows = T.diffLines(a, b);
  const dels = rows.filter((r) => r.t === "-").map((r) => r.s);
  const adds = rows.filter((r) => r.t === "+").map((r) => r.s);
  assert.deepEqual(dels, ["行二"]);
  assert.deepEqual(adds, ["行二改", "行四"]);
  const html = T.diffHtml(a, b, "v1", "v2");
  assert.match(html, /dl-del/);
  assert.match(html, /dl-add/);
  assert.match(html, /差异 v1 → v2/);
  // 空对空：两行都归一为单条空上下文行（LCS 语义），不抛错
  assert.deepEqual(T.diffLines(null, ""), [{ t: " ", s: "" }]);
});

test("artifactTimelineHtml：版本节点/状态徽标/箭头数", () => {
  resetState();
  const chain = [
    { artifact_id: "p:critic:v1", version: 1, review_state: "Superseded" },
    { artifact_id: "p:critic:v2", version: 2, review_state: "Approved" },
  ];
  const html = T.artifactTimelineHtml(chain);
  assert.match(html, /data-timeline-len="2"/);
  assert.match(html, /v1/);
  assert.match(html, /已被取代|draft|草稿/);
  assert.match(html, /已批准/);
  assert.equal((html.match(/owo-ws-tl-arrow/g) || []).length, 1);
  assert.equal(T.artifactTimelineHtml([]), "");
});

test("buildReworkBody：冻结契约四字段 + 校验 + 幂等键自动生成", () => {
  resetState();
  T.state.current = "team-9";
  const body = T.buildReworkBody({ artifactId: "p:critic:v1", reviewId: "rev-1", instruction: "修正 scope 字段", idempotencyKey: "k-1" });
  assert.deepEqual(Object.keys(body).sort(), ["idempotency_key", "instruction", "review_id", "team_id"]);
  assert.equal(body.team_id, "team-9");
  assert.equal(body.review_id, "rev-1");
  assert.equal(body.idempotency_key, "k-1");
  // 缺省幂等键自动生成（含 rework 意图标记 + 序号）
  const b2 = T.buildReworkBody({ artifactId: "p:critic:v1", reviewId: "rev-1", instruction: " 修正 " });
  assert.match(b2.idempotency_key, /rework/);
  assert.equal(b2.instruction, "修正");
  // 校验：缺 artifact / review / instruction / team
  assert.throws(() => T.buildReworkBody({ reviewId: "r", instruction: "x" }), /artifactId/);
  assert.throws(() => T.buildReworkBody({ artifactId: "a", instruction: "x" }), /review_id/);
  T.state.current = null;
  assert.throws(() => T.buildReworkBody({ artifactId: "a", reviewId: "r", instruction: "x" }), /team_id/);
  T.state.current = "team-9";
  assert.throws(() => T.buildReworkBody({ artifactId: "a", reviewId: "r", instruction: "  " }), /不能为空/);
});

test("explainReworkError：409 幂等冲突/404/回退文案", () => {
  assert.match(T.explainReworkError(new Error("409: dup")), /已创建过返工任务/);
  assert.match(T.explainReworkError(new Error("404: no")), /不存在/);
  assert.match(T.explainReworkError(new Error("boom")), /返工提交/);
});

test("submitRework：成功记录 flash + 忙锁 + 失败文案（注入传输层）", async () => {
  resetState();
  T.state.current = "team-9";
  const calls = [];
  const old = T.getTransport();
  T.setTransport({
    get: old.get,
    post: (url, body) => {
      calls.push({ url, body });
      return calls.length === 1 ? Promise.resolve({ replayed: false }) : Promise.reject(new Error("409: dup"));
    },
  });
  await T.submitRework({ artifactId: "a1", reviewId: "rev-1", instruction: "修", idempotencyKey: "k1" });
  assert.equal(calls[0].url, "/artifacts/a1/rework");
  assert.equal(calls[0].body.review_id, "rev-1");
  assert.equal(calls[0].body.team_id, "team-9");
  assert.equal(T.state.reworkResult.ok, true);
  assert.match(T.state.reviewFlash.text, /返工任务已受理/);
  // 失败路径：409 → 计划文案进 flash
  await assert.rejects(() => T.submitRework({ artifactId: "a1", reviewId: "rev-1", instruction: "修", idempotencyKey: "k2" }));
  assert.equal(T.state.reworkResult.ok, false);
  assert.match(T.state.reviewFlash.text, /已创建过返工任务/);
  assert.equal(T.state.reworkBusy.a1, undefined, "失败后解锁");
  // 忙锁：同产物进行中拒绝并发
  T.state.reworkBusy.a1 = true;
  await assert.rejects(() => T.submitRework({ artifactId: "a1", reviewId: "rev-1", instruction: "修" }), /进行中/);
  T.setTransport(old);
});

test("deliverablesFromPayload：分列形状/单列分桶/空形状", () => {
  resetState();
  const bucketed = T.deliverablesFromPayload({
    project_id: "proj-1",
    approved: [{ artifact_id: "a-ok", kind: "doc", version: 2, producer: "m-w", review_state: "Approved" }],
    pending: [{ artifact_id: "a-p", review_state: "PendingReview" }],
    rejected_or_superseded: [{ artifact_id: "a-x", review_state: "Rejected" }],
  });
  assert.equal(bucketed.approved.length, 1);
  assert.equal(bucketed.approved[0].artifactId, "a-ok");
  assert.equal(bucketed.pending.length, 1);
  assert.equal(bucketed.other.length, 1);
  // 单列表形状：按 review_state 自动分桶
  const flat = T.deliverablesFromPayload({
    items: [
      { artifact_id: "a1", review_state: "Approved" },
      { artifact_id: "a2", review_state: "pending_review" },
      { artifact_id: "a3", review_state: "Superseded" },
    ],
  });
  assert.equal(flat.approved.length, 1);
  assert.equal(flat.pending.length, 1);
  assert.equal(flat.other.length, 1);
  // 空容器 → 空三桶（不崩）
  const empty = T.deliverablesFromPayload({});
  assert.equal(empty.approved.length + empty.pending.length + empty.other.length, 0);
  assert.equal(T.deliverablesFromPayload(null), null);
});

test("metricsFromPayload：服务端实弹形状（五期 team-metrics live shape）", () => {
  resetState();
  // 形状取自 GET /teams/{id}/metrics 实测响应（workers 行 + summary 别名 + budget）。
  const live = T.metricsFromPayload({
    team_id: "team-x",
    workers: [
      {
        member_id: "m-critic",
        role: "critic",
        outcome: "succeeded",
        attempt: 1,
        wall_ms: 24,
        model_calls: 0,
        cost_usd: 0.0,
        artifact: { artifact_id: "team-x:critic:v1", kind: "review", version: 1 },
      },
    ],
    summary: {
      wall_window_ms: 59548,
      model_calls: 0,
      prompt_tokens: null,
      completion_tokens: null,
      cost_usd: 0.0,
      failed_spans: 0,
      succeeded_spans: 2,
      span_count: 2,
      rework_count: 1,
      artifact_versions: 2,
      slowest_worker: { role: "critic", span_id: "span-1", step_id: "s-critic" },
    },
    budget: { exceeded: false, max_cost_usd: null, spent_usd: 0.0, reason: null },
  });
  assert.ok(live, "实弹形状应产出指标");
  assert.equal(live.workers.length, 1);
  const w = live.workers[0];
  assert.equal(w.worker, "m-critic");
  assert.equal(w.terminal, "succeeded");
  assert.equal(w.durationMs, 24);
  assert.equal(w.attempts, 1);
  assert.deepEqual(w.artifactIds, ["team-x:critic:v1"], "artifact 对象应提取 artifact_id");
  assert.equal(live.summary.wallClockMs, 59548, "wall_window_ms 别名");
  assert.equal(live.summary.reworks, 1);
  assert.equal(live.summary.failures, 0, "failed_spans 别名");
  assert.equal(live.summary.slowestWorker, "critic", "slowest_worker 对象提 role");
  assert.equal(live.summary.budgetExhausted, false);
  // budget.exceeded=true 透传（附最小 wall_clock 以通过空态守卫）
  const boomed = T.metricsFromPayload({ summary: { wall_clock_ms: 1 }, budget: { exceeded: true, reason: "费用超限" } });
  assert.equal(boomed.summary.budgetExhausted, true);
  assert.equal(boomed.summary.budgetReason, "费用超限");
});

test("deliverablesFromPayload：服务端实弹形状（pending_review + null 桶 + manifest）", () => {
  resetState();
  const live = T.deliverablesFromPayload({
    project_id: "proj-x",
    approved: [],
    pending_review: [
      { artifact_id: "team-x:critic:v1", review_state: "draft", version: 1, kind: "review", producer: "m-critic" },
      { artifact_id: "team-x:critic:v2", review_state: "PendingReview", version: 2, kind: "review", producer: "m-critic" },
    ],
    rejected_or_superseded: null,
    complete: false,
    delivery_manifest_ref: "cas://sha256:abc",
    rework_tasks: [{ task_id: "t1" }],
  });
  assert.equal(live.approved.length, 0);
  assert.equal(live.pending.length, 2, "pending_review 键应并入待评审桶");
  assert.equal(live.other.length, 0, "null 桶应容错为空");
  assert.equal(live.manifestRef, "cas://sha256:abc");
  assert.equal(live.complete, false);
  assert.equal(live.reworkCount, 1);
  const html = T.deliverablesBoxHtml(live);
  assert.match(html, /待评审 2/);
  assert.match(html, /返工中 1/);
  assert.match(html, /cas:\/\/sha256:abc/);
  assert.doesNotMatch(html, /交付完成/);
});

test("deliverablesBoxHtml：计数徽标/交付列表/全空提示", () => {
  resetState();
  const dl = T.deliverablesFromPayload({
    approved: [{ artifact_id: "a-ok", kind: "doc", version: 2, producer: "m-w", review_state: "Approved" }],
    pending: [{ artifact_id: "a-p", review_state: "PendingReview" }],
    rejected_or_superseded: [],
  });
  const html = T.deliverablesBoxHtml(dl);
  assert.match(html, /已批准 1/);
  assert.match(html, /待评审 1/);
  assert.match(html, /data-dlv-art="a-ok"/);
  assert.match(T.deliverablesBoxHtml(T.deliverablesFromPayload({})), /暂无交付物/);
});

test("latestChangesReviewId：取最近一次要求修改评审 id", () => {
  resetState();
  T.state.historyReviews["a1"] = [
    { review_id: "rev-1", decision: "request_changes", comment: "修 scope" },
    { review_id: "rev-2", decision: "approve" },
  ];
  assert.equal(T.latestChangesReviewId("a1"), "rev-1");
  T.state.historyReviews["a2"] = [{ review_id: "rev-3", decision: "reject" }];
  assert.equal(T.latestChangesReviewId("a2"), "");
  assert.equal(T.latestChangesReviewId("missing"), "");
});

test("交接引用候选失败可读可重试，重试成功后候选及时更新", async () => {
  resetState();
  T.state.current = "team-1";
  T.state.team = { team_id: "team-1", project_space_id: "project-1" };
  const listeners = {};
  function fakeBox(id) {
    return {
      id,
      innerHTML: "",
      addEventListener(type, handler) { listeners[type + ":" + id] = handler; },
      querySelector() { return null; },
    };
  }
  const artifactAttrs = {};
  const artifactBox = {
    innerHTML: "",
    getAttribute(name) { return artifactAttrs[name] || null; },
    setAttribute(name, value) { artifactAttrs[name] = String(value); },
    addEventListener() {},
  };
  const artsBox = fakeBox("arts");
  const evidBox = fakeBox("evid");
  const elements = {
    "#ws-d-artifacts": artifactBox,
    "#ws-x-arts-pick": artsBox,
    "#ws-x-evid-pick": evidBox,
  };
  T.setRoot({ querySelector(selector) { return elements[selector] || null; } });

  let attempts = 0;
  T.setTransport({
    get(path) {
      assert.equal(path, "/projects/project-1/artifacts");
      attempts += 1;
      if (attempts === 1) return Promise.reject(new Error("artifact options unavailable"));
      const id = attempts === 2 ? "art-1" : "art-2";
      return Promise.resolve({
        artifacts: [{ artifact_id: id, kind: "markdown", version: attempts - 1, producer: "m-writer", review_state: "PendingReview", preview: "# Result" }],
      });
    },
  });

  T.bindHandoffRefPickers();
  T.paintHandoffRefPickers();
  await T.loadHandoffArtifacts();
  assert.equal(T.state.handoffArtsFetched, false);
  assert.equal(T.state.handoffArtsLoading, false);
  assert.match(artsBox.innerHTML, /role="alert"/);
  assert.match(artsBox.innerHTML, /data-ws-ref-retry-artifacts/);
  assert.match(artsBox.innerHTML, /仍可手动输入引用/);

  listeners["click:arts"]({ target: {
    hasAttribute(name) { return name === "data-ws-ref-retry-artifacts"; },
    getAttribute() { return null; },
  } });
  await T.loadHandoffArtifacts();
  assert.equal(attempts, 2);
  assert.equal(T.state.handoffArtsFetched, true, T.state.artifactLoadError || T.state.handoffArtsError || "candidate load did not complete");
  assert.equal(T.state.handoffArtsError, "");
  assert.match(artsBox.innerHTML, /data-ws-ref-add="arts:art-1"/);
  assert.doesNotMatch(artsBox.innerHTML, /data-ws-ref-retry-artifacts/);

  await T.loadHandoffArtifacts(true);
  assert.equal(attempts, 3);
  assert.match(artsBox.innerHTML, /data-ws-ref-add="arts:art-2"/);
  assert.doesNotMatch(artsBox.innerHTML, /data-ws-ref-add="arts:art-1"/);
});

test("artifactRowHtml：链内最新 Draft/Rejected 行带返工表单，其余状态不带", () => {
  resetState();
  const v1 = { artifact_id: "p:critic:v1", kind: "doc", version: 1, producer: "m-critic", review_state: "Superseded", supersedes_artifact_id: null, preview: "x" };
  const v2 = { artifact_id: "p:critic:v2", kind: "doc", version: 2, producer: "m-critic", review_state: "Draft", supersedes_artifact_id: "p:critic:v1", preview: "y" };
  const chain = T.groupArtifactChain([v1, v2])[0];
  const html = T.artifactRowHtml(chain.items[1], chain);
  assert.match(html, /根据评审意见返工/);
  assert.match(html, /data-rework-go="p:critic:v2"/);
  const approved = T.artifactRowHtml({ artifact_id: "p:critic:v3", kind: "doc", version: 3, producer: "m-critic", review_state: "Approved", supersedes_artifact_id: "p:critic:v2", preview: "z" }, { items: [v1, v2, { artifact_id: "p:critic:v3", version: 3 }], approvedHead: null });
  assert.ok(!approved.includes("根据评审意见返工"), "已批准版本无返工入口");
});

test("五期样式守卫：style.css 第 19 节策略/指标/时间线/差异/交付物样式在场", () => {
  const flat = shellCss.replace(/\s+/g, "");
  assert.ok(flat.includes(".owo-ws-strategy"), "策略判定区样式在场");
  assert.ok(flat.includes(".owo-ws-mgrid"), "角色指标网格样式在场");
  assert.ok(flat.includes(".owo-ws-timeline"), "版本时间线样式在场");
  assert.ok(flat.includes(".owo-ws-diff.dl-add"), "差异增行样式在场");
  assert.ok(flat.includes(".owo-ws-rework"), "返工表单样式在场");
  assert.ok(flat.includes(".owo-ws-dlv-item"), "交付物条目样式在场");
});

// ============================================================================
// 七期（第四路）：Worker 能力 / 写租约 / 文件变更 / 产物校验与下载交付
// ============================================================================

test("七期 workerProfilesTable：角色×权限×预算表 + 空态容错", () => {
  resetState();
  const html = T.workerProfilesTable([
    {
      role: "code_analyzer",
      visible_tools: ["fs_read", "grep"],
      read_only: true,
      write_allowed_paths: null,
      max_turns: 12,
      can_run_command: false,
      can_use_browser: false,
    },
    {
      role: "implementer",
      visible_tools: ["fs_read", "fs_write", "shell"],
      read_only: false,
      write_allowed_paths: ["src/", "tests/"],
      max_turns: 30,
      can_run_command: true,
      can_use_browser: false,
    },
  ]);
  assert.match(html, /<table class="owo-ws-table">/);
  assert.match(html, /code_analyzer/);
  assert.match(html, /implementer/);
  assert.match(html, /fs_read、grep/, "可见工具顿号连接");
  assert.match(html, /src\/、tests\//, "允许写路径顿号连接");
  assert.match(html, /<td>30<\/td>/, "最大轮次（预算）列");
  assert.match(html, /只读/);
  // 容错：未声明写路径的可写角色给出拒绝写入提示；空/缺字段安全
  const sparse = T.workerProfilesTable([{ role: "x", read_only: false }]);
  assert.match(sparse, /未声明（写入将被拒绝）/);
  assert.match(T.workerProfilesTable([]), /暂无 WorkerProfile/);
  assert.match(T.workerProfilesTable(null), /暂无 WorkerProfile/);
  assert.match(T.workerProfilesTable([null, 42]), /暂无 WorkerProfile/, "非对象条目被过滤");
});

test("七期 writeLeaseBox：持有/已释放/无租约三态", () => {
  resetState();
  const held = T.writeLeaseBox({ holder_role: "implementer", holder_step_id: "s-2", acquired_at_ms: 1700000000000 });
  assert.match(held, /写租约持有中/);
  assert.match(held, /implementer/);
  assert.match(held, /s-2/);
  assert.match(held, /自 .* 起持有/);
  const released = T.writeLeaseBox({ holder_role: "implementer", holder_step_id: "s-2", acquired_at_ms: 1700000000000, released_at_ms: 1700000001000 });
  assert.match(released, /已于 .* 释放/);
  assert.match(T.writeLeaseBox(null), /独立输出范围可并行写入/);
  assert.match(T.writeLeaseBox(undefined), /独立输出范围可并行写入/);
  assert.match(T.writeLeaseBox("junk"), /独立输出范围可并行写入/, "非对象容错");
});

test("七期 changesListHtml：变更行状态徽标 + ±行数 + diff 预览 + 空态", () => {
  resetState();
  const html = T.changesListHtml([
    { path: "src/lib.rs", state: "modified", added_lines: 12, deleted_lines: 3, diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new" },
    { path: "tests/new_test.rs", state: "added" },
    { path: "old.rs", state: "deleted", added_lines: 0, deleted_lines: 40 },
  ]);
  assert.match(html, /共 3 个文件变更/);
  assert.match(html, /src\/lib\.rs/);
  assert.match(html, /\+12 \/ -3/);
  assert.match(html, /<details class="owo-ws-chg-diff">/);
  assert.match(html, /<pre class="owo-ws-diff">/, "diff 复用差异配色容器");
  assert.match(html, /\+0 \/ -40/);
  assert.ok(html.includes("new") && html.includes("old"), "diff 正文转义后在场");
  // 状态徽标语义色
  assert.match(html, /owo-ws-badge st-awaiting_human">修改</);
  assert.match(html, /owo-ws-badge st-running">新增</);
  assert.match(html, /owo-ws-badge st-failed">删除</);
  // 空态与非对象容错
  assert.match(T.changesListHtml([]), /暂无文件变更/);
  assert.match(T.changesListHtml(null), /暂无文件变更/);
  assert.match(T.changesListHtml([null, 7]), /暂无文件变更/);
});

test("七期 validationBadgeHtml + artifactFileName：校验徽标三态与扩展名推断", () => {
  resetState();
  assert.match(T.validationBadgeHtml({ valid: true }), /校验通过/);
  const bad = T.validationBadgeHtml({ valid: false, reason: "JSON 解析失败：禁止 Markdown 围栏" });
  assert.match(bad, /校验未通过/);
  assert.match(bad, /title="JSON 解析失败：禁止 Markdown 围栏"/);
  assert.equal(T.validationBadgeHtml(null), "", "字段缺失不渲染（旧产物兼容）");
  assert.equal(T.validationBadgeHtml({}), "", "valid 缺失不渲染");
  assert.equal(T.artifactFileName({ artifact_id: "a-1", format: "markdown" }), "a-1.md");
  assert.equal(T.artifactFileName({ artifact_id: "a-2", format: "JSON" }), "a-2.json");
  assert.equal(T.artifactFileName({ artifact_id: "a-3", format: "research" }), "a-3.md");
  assert.equal(T.artifactFileName({ artifact_id: "a-4", format: "weird" }), "a-4.weird");
  assert.equal(T.artifactFileName({ artifact_id: "a-5" }), "a-5.txt");
  assert.equal(T.artifactFileName(null), "artifact.txt");
});

test("七期 deliveryManifestText：容错文本化（版本/哈希/大小/批准态/引用）", () => {
  resetState();
  const text = T.deliveryManifestText({
    project_id: "proj-t-1",
    generated_at: "2025-01-01T00:00:00Z",
    manifest: [
      { artifact_id: "a-1", kind: "code_diff", format: "markdown", version: 2, sha256: "abc123", size_bytes: 512, approved: true, content_url: "/artifacts/a-1/content" },
      { artifact_id: "a-2", kind: "report", format: "json", version: 1, sha256: null, size_bytes: null, approved: false, content_url: "" },
      null,
    ],
  });
  assert.match(text, /project_id: proj-t-1/);
  assert.match(text, /artifacts: 2/, "null 条目不计");
  assert.match(text, /a-1 {2}code_diff\/markdown {2}v2 {2}sha256:abc123 {2}512B {2}已批准/);
  assert.match(text, /a-2/);
  assert.match(text, /未批准/);
  assert.match(T.deliveryManifestText(null), /project_id: —/);
  assert.match(T.deliveryManifestText({}), /artifacts: 0/);
});

test("七期 stopping/stopped：门控与状态中文化 + 模块 CSS 在场", () => {
  resetState();
  assert.equal(T.isGatedTeam("stopping"), true);
  assert.equal(T.isGatedTeam("Stopped"), true, "Debug/大小写归一");
  assert.equal(T.isGatedTeam("cancelled"), true, "终态同样门控");
  assert.equal(T.isGatedTeam("failed"), true, "failed 终态仍门控运行操作");
  assert.equal(T.isGatedTeam("running"), false);
  assert.equal(T.isGatedTeam(""), false);
  // 运行摘要：状态标签与阶段中文化
  const stopping = T.computeRunSummary({ team: { status: "stopping" }, tasks: [] });
  assert.equal(stopping.statusLabel, "正在停止");
  assert.equal(stopping.phase, "正在停止（Worker 退出中）");
  const stopped = T.computeRunSummary({ team: { status: "stopped" }, tasks: [] });
  assert.equal(stopped.statusLabel, "已停止");
  assert.equal(stopped.phase, "已停止");
  // 模块内联 CSS：停止态徽标 + rv-* 评审徽标 + 变更列表样式
  const css = T.css();
  assert.ok(css.includes(".owo-ws-badge.st-stopping"), "st-stopping 徽标样式在场");
  assert.ok(css.includes(".owo-ws-badge.st-stopped"), "st-stopped 徽标样式在场");
  assert.ok(css.includes(".owo-ws-badge.rv-ok") && css.includes(".owo-ws-badge.rv-bad"), "rv-* 评审徽标配色在场");
  assert.ok(css.includes(".owo-ws-chg-row") && css.includes(".owo-ws-lease"), "变更列表与写租约样式在场");
});

test("七期 artifactRowHtml：校验徽标/证据/Handoff/sha256/下载按钮在场", () => {
  resetState();
  const a = {
    artifact_id: "p:w:v2",
    kind: "code_diff",
    version: 2,
    producer: "m-implementer",
    review_state: "PendingReview",
    supersedes_artifact_id: null,
    validation: { valid: false, reason: "CSV 列数不一致" },
    evidence_refs: ["cas://sha256:aa", "file://src/lib.rs"],
    sha256: "deadbeefcafe1234567890",
    handoff: { completed_summary: "完成" },
  };
  const html = T.artifactRowHtml(a, { items: [a] });
  assert.match(html, /校验未通过/);
  assert.match(html, /title="CSV 列数不一致"/);
  assert.match(html, /证据 2 条/);
  assert.match(html, /title="证据引用：cas:\/\/sha256:aa，file:\/\/src\/lib\.rs"/);
  assert.match(html, /含 Handoff/);
  assert.match(html, /sha256 deadbeefca…/, "短哈希 10 字符截断");
  assert.match(html, /title="sha256: deadbeefcafe1234567890"/, "完整哈希在 tooltip");
  assert.match(html, /data-art-dl="p:w:v2"/, "每行一个下载按钮");
  // 无七期字段的旧产物：不渲染徽标/按钮外的附加节点（下载按钮恒在）
  const old = T.artifactRowHtml({ artifact_id: "p:w:v1", kind: "doc", version: 1, producer: "m-w", review_state: "Approved", supersedes_artifact_id: null }, { items: [{ artifact_id: "p:w:v1", version: 1 }] });
  assert.ok(!old.includes("校验通过"), "validation 缺失时不渲染校验徽标");
  assert.ok(!old.includes("含 Handoff"));
  assert.match(old, /data-art-dl="p:w:v1"/);
});

// ============================================================================
// 七期（二路交接）：GET /projects/{pid}/workspace/changes 变更追踪读取面
// ============================================================================

test("七期 changesRemoteView：端点载荷归一 + 全字段容错", () => {
  resetState();
  const view = T.changesRemoteView({
    team_id: "team-1",
    git: true,
    changed_files: ["src/calc.rs", "out/fix-report.md"],
    diff_summary: " src/calc.rs | 2 +-",
    has_violation: false,
    records: [
      { role: "implementer", step: "s-2", at: 1753680000000, git: true, changed_files: ["src/calc.rs"], diff_summary: " src/calc.rs | 2 +-", diff_ref: "team-1-changes/s-2.patch", violation: null },
      { role: "finalizer", step: "s-3", git: false, violation: "越界写入 /etc" },
    ],
  });
  assert.equal(view.team_id, "team-1");
  assert.equal(view.git, true);
  assert.deepEqual(view.changed_files, ["src/calc.rs", "out/fix-report.md"]);
  assert.equal(view.has_violation, false);
  assert.equal(view.records.length, 2);
  assert.equal(view.records[0].diff_ref, "team-1-changes/s-2.patch");
  assert.equal(view.records[0].violation, null);
  assert.equal(view.records[1].at, null, "缺 at 容错为 null");
  assert.deepEqual(view.records[1].changed_files, []);

  // 容错：null / 空对象 / records 内非对象条目
  const empty = T.changesRemoteView(null);
  assert.deepEqual(empty.changed_files, []);
  assert.deepEqual(empty.records, []);
  assert.equal(empty.has_violation, false);
  const sparse = T.changesRemoteView({ records: [null, 42, { role: "r" }] });
  assert.equal(sparse.records.length, 3, "非对象记录归一为缺省行");
  assert.equal(sparse.records[0].role, "—");
  assert.equal(sparse.records[2].step, "unknown", "缺 step 缺省 unknown（与服务端一致）");
});

test("七期 changeRecordsHtml：越界/通过徽标 + 文件行 + diff 容器", () => {
  resetState();
  const html = T.changeRecordsHtml([
    { role: "implementer", step: "s-2", at: 1753680000000, git: true, changed_files: ["src/calc.rs"], diff_summary: " 1 file changed", diff_ref: "team-1-changes/s-2.patch", violation: null },
    { role: "finalizer", step: "s-3", at: null, git: false, changed_files: [], diff_summary: "", diff_ref: null, violation: "越界写入 /etc" },
  ]);
  assert.match(html, /owo-ws-badge rv-ok">通过</);
  assert.match(html, /owo-ws-badge rv-bad" title="越界写入 \/etc">越界</);
  assert.match(html, /implementer/);
  assert.match(html, /finalizer/);
  assert.match(html, /步骤 s-2/);
  assert.match(html, /patch team-1-changes\/s-2\.patch/);
  assert.match(html, /src\/calc\.rs/);
  assert.match(html, /<pre class="owo-ws-diff"> 1 file changed<\/pre>/);
  assert.match(html, /（本窗口无新增变更文件）/, "空文件窗口给出占位");
  assert.equal(T.changeRecordsHtml([]), "", "空 records 输出空串");
  assert.equal(T.changeRecordsHtml(null), "");
});

test("七期 changesRuntimeHtml：远程优先 + 详情文件清单补充 + 双空态", () => {
  resetState();
  // 远程有数据：越界警示 + diff 摘要 + 记录
  const remote = T.changesRemoteView({
    team_id: "team-1",
    git: true,
    changed_files: ["src/calc.rs"],
    diff_summary: " src/calc.rs | 2 +-",
    has_violation: true,
    records: [{ role: "implementer", step: "s-2", at: 1, git: true, changed_files: ["src/calc.rs"], diff_summary: " x", diff_ref: null, violation: null }],
  });
  const html = T.changesRuntimeHtml(remote, []);
  assert.match(html, /存在白名单越界写记录/, "has_violation 警示行");
  assert.match(html, /最近 diff 摘要/);
  assert.match(html, /<pre class="owo-ws-diff"> src\/calc\.rs \| 2 \+-<\/pre>/);
  assert.match(html, /步骤 s-2/);
  // 远程有数据 + 详情文件清单：两者都渲染
  const both = T.changesRuntimeHtml(remote, [{ path: "src/lib.rs", state: "modified", added_lines: 1, deleted_lines: 0, diff: "" }]);
  assert.match(both, /步骤 s-2/);
  assert.match(both, /src\/lib\.rs/);
  // 远程空记录（无摘要/无越界）→ 回退详情文件清单
  const fallback = T.changesRuntimeHtml({ team_id: "t", git: false, changed_files: [], diff_summary: "", has_violation: false, records: [] }, [{ path: "a.rs", state: "added" }]);
  assert.match(fallback, /共 1 个文件变更/);
  assert.ok(!fallback.includes("最近 diff 摘要"));
  // 双空 → 既有空态
  assert.match(T.changesRuntimeHtml(null, []), /暂无文件变更/);
  assert.match(T.changesRuntimeHtml(null, null), /暂无文件变更/);
  // 远程无数据但详情有 → 详情文件清单
  assert.match(T.changesRuntimeHtml(null, [{ path: "b.rs", state: "added" }]), /b\.rs/);
});

test("七期 loadWorkspaceChanges：成功落地归一视图；404/失败容错为 null", async () => {
  resetState();
  const view = T.changesRemoteView({
    team_id: "team-1",
    git: true,
    changed_files: ["src/calc.rs"],
    diff_summary: " 1 file changed",
    has_violation: false,
    records: [],
  });
  T.setTransport({
    get(path) {
      if (path === "/projects/proj-t-1/workspace/changes") return Promise.resolve(view);
      if (path === "/projects/proj-bad/workspace/changes") return Promise.reject(new Error("404: project not found"));
      return Promise.reject(new Error("unexpected GET " + path));
    },
    post() {
      return Promise.reject(new Error("unexpected POST"));
    },
  });
  // rootEl 为 null（无 DOM）：paintSevenRuntime 内部 el() 全部守卫，不触碰 DOM。
  T.state.current = "t-1";
  T.state.team = { team_id: "t-1", project_space_id: null };
  await T.loadWorkspaceChanges();
  assert.deepEqual(T.state.changesRemote, view, "成功 → 归一视图落地");

  T.state.current = "bad";
  T.state.team = { team_id: "bad", project_space_id: null };
  await T.loadWorkspaceChanges();
  assert.equal(T.state.changesRemote, null, "404 → 容错为 null（详情 changes[] 兜底）");
});

// ============================================================================
// 八期（第四路）守卫：ChangeSet 审批闭环（列表 / 状态徽标 / accept-reject-revert）
// ============================================================================

function csFixture(id, status, extra) {
  return Object.assign(
    { change_set_id: id, team_id: "team-a", step_id: "s-impl", role: "implementer",
      changed_files: ["src/calc.rs"], diff_ref: null,
      status: status || "pending_review", created_at: "2026-08-30T04:00:00Z" },
    extra || {}
  );
}

test("normCsStatus：Debug/混合形式归一", () => {
  assert.equal(T.normCsStatus("PendingReview"), "pending_review");
  assert.equal(T.normCsStatus("pending-review"), "pending_review");
  assert.equal(T.normCsStatus("accepted"), "accepted");
});

test("changeSetsView：归一 + 容错（非对象条目/缺 id 丢弃/文件清单数组化）", () => {
  const list = T.changeSetsView({
    change_sets: [
      csFixture("cs1", "PendingReview", { diff_ref: "patch-1" }),
      csFixture("cs2", "conflicted", { changed_files: "not-array" }),
      { broken: true },
      null,
      csFixture(""),
    ],
  });
  assert.equal(list.length, 2);
  assert.equal(list[0].status, "pending_review");
  assert.equal(list[0].diff_ref, "patch-1");
  assert.equal(list[1].status, "conflicted");
  assert.deepEqual(list[1].changed_files, []);
  assert.equal(T.changeSetsView(null).length, 0);
});

test("changeSetBadge：状态徽标配色（accepted 绿 / conflicted 红 / pending 警示）", () => {
  assert.match(T.changeSetBadge("accepted"), /rv-ok/);
  assert.match(T.changeSetBadge("conflicted"), /rv-bad/);
  assert.match(T.changeSetBadge("pending_review"), /rv-warn/);
  assert.match(T.changeSetBadge("reverted"), /已撤销/);
  assert.match(T.changeSetBadge("superseded"), /\u5df2\u88ab\u65b0\u7248\u672c\u53d6\u4ee3/);
  assert.match(T.changeSetBadge("superseded"), /rv-off/);
});

test("changeSetsHtml：空态 + 待审批动作（data 属性/提交锁）+ 结果区", () => {
  assert.match(T.changeSetsHtml([]), /暂无 ChangeSet/);
  assert.match(T.changeSetsHtml(null), /暂无 ChangeSet/);

  T.state.csBusy = {};
  T.state.csResults = {};
  const html = T.changeSetsHtml([csFixture("cs1")]);
  assert.match(html, /data-cs-act="accept"/);
  assert.match(html, /data-cs-act="reject"/);
  assert.match(html, /data-cs-act="revert"/);
  assert.match(html, /data-cs-id="cs1"/);
  assert.match(html, /src\/calc\.rs/);

  // 非待审批状态：无动作按钮
  const html2 = T.changeSetsHtml([csFixture("cs2", "accepted")]);
  assert.ok(!html2.includes('data-cs-act="accept"'));

  // 提交锁 + 结果文本
  T.state.csBusy["cs1:accept"] = true;
  T.state.csResults["cs1"] = { ok: true, text: "已接受。" };
  const html3 = T.changeSetsHtml([csFixture("cs1")]);
  assert.match(html3, /disabled/);
  assert.match(html3, /已接受。/);
  T.state.csBusy = {};
  T.state.csResults = {};
});

test("startChangeSetAction()：成功路径（POST /change-sets/{id}/{action}）+ 幂等重放文案", async () => {
  resetState();
  T.state.current = "team-a";
  const log = [];
  T.setTransport({
    get(path) {
      if (path === "/teams/team-a/change-sets") {
        return Promise.resolve({ team_id: "team-a", change_sets: [csFixture("cs1", "accepted")] });
      }
      return Promise.reject(new Error("404: " + path));
    },
    post(path) {
      log.push("POST " + path);
      return Promise.resolve({ change_set: csFixture("cs1", "accepted"), replayed: false });
    },
  });
  await T.startChangeSetAction("cs1", "accept");
  assert.deepEqual(log, ["POST /change-sets/cs1/accept"]);
  assert.match(T.state.csResults["cs1"].text, /已接受。/);
  assert.equal(T.state.csBusy["cs1:accept"], false);
  // 动作后重拉列表
  assert.equal(T.state.changeSets.length, 1);
  assert.equal(T.state.changeSets[0].status, "accepted");
});

test("startChangeSetAction()：幂等重放（replayed）提示", async () => {
  resetState();
  T.state.current = "team-a";
  T.setTransport({
    get() { return Promise.resolve({ change_sets: [] }); },
    post() { return Promise.resolve({ change_set: csFixture("cs1", "accepted"), replayed: true }); },
  });
  await T.startChangeSetAction("cs1", "accept");
  assert.match(T.state.csResults["cs1"].text, /幂等重放/);
});

test("startChangeSetAction()：409 冲突文案（不覆盖用户新内容）", async () => {
  resetState();
  T.state.current = "team-a";
  T.setTransport({
    get() { return Promise.resolve({ change_sets: [csFixture("cs1", "conflicted")] }); },
    post() { return Promise.reject(new Error("409: {\"error\":\"文件已被用户修改\"}")); },
  });
  await T.startChangeSetAction("cs1", "revert");
  assert.equal(T.state.csResults["cs1"].ok, false);
  assert.match(T.state.csResults["cs1"].text, /冲突（409）/);
  assert.match(T.state.csResults["cs1"].text, /未覆盖新内容/);
});

test("startChangeSetAction()：提交锁双击只发一次；未知动作零请求", async () => {
  resetState();
  T.state.current = "team-a";
  const log = [];
  T.setTransport({
    get() { return Promise.resolve({ change_sets: [] }); },
    post(path) { log.push("POST " + path); return Promise.resolve({}); },
  });
  const p1 = T.startChangeSetAction("cs1", "reject");
  const p2 = T.startChangeSetAction("cs1", "reject");
  await Promise.all([p1, p2]);
  assert.equal(log.length, 1);
  const before = log.length;
  await T.startChangeSetAction("cs1", "bogus");
  assert.equal(log.length, before, "未知动作零请求");
});

test("loadChangeSets()：404 容错为 null（端点未上线空态）", async () => {
  resetState();
  T.state.current = "team-a";
  T.state.csApprovalBlock = { blocked: true, reason: "stale" };
  T.setTransport({
    get() { return Promise.reject(new Error("404: not found")); },
    post() { return Promise.reject(new Error("404: not found")); },
  });
  await T.loadChangeSets();
  assert.equal(T.state.changeSets, null);
  assert.deepEqual(T.state.csApprovalBlock, null, "门控状态随失败清空");
});

test("loadChangeSets()（九期）：捕获 approval_blocked/approval_block_reason 门控视图", async () => {
  resetState();
  T.state.current = "team-a";
  T.setTransport({
    get() {
      return Promise.resolve({
        team_id: "team-a",
        approval_blocked: true,
        approval_block_reason: "团队存在待审批 ChangeSet：cs1",
        change_sets: [csFixture("cs1")],
      });
    },
    post() { return Promise.reject(new Error("unexpected POST")); },
  });
  await T.loadChangeSets();
  assert.deepEqual(T.state.csApprovalBlock, { blocked: true, reason: "团队存在待审批 ChangeSet：cs1" });
});

// ============================================================================
// 九期（第二路）守卫：状态口径 / 批准门控联动 / idempotency_key 请求体
// ============================================================================

test("csStatusHint / approvalBlockView：九期状态口径与门控视图归一", () => {
  assert.equal(T.csStatusHint("pending_review"), "等待接受或拒绝");
  assert.equal(T.csStatusHint("PendingReview"), "等待接受或拒绝", "Debug 形式容错");
  assert.match(T.csStatusHint("conflicted"), /存在冲突，禁止批准/);
  assert.equal(T.csStatusHint("accepted"), "");

  const v = T.approvalBlockView({
    approval_blocked: true,
    approval_block_reason: "存在待审批 ChangeSet",
  });
  assert.deepEqual(v, { blocked: true, reason: "存在待审批 ChangeSet" });
  assert.deepEqual(T.approvalBlockView(null), { blocked: false, reason: "" });
  assert.deepEqual(T.approvalBlockView({ approval_blocked: false }), { blocked: false, reason: "" });
});

test("changeSetsHtml（九期）：conflicted 行提供可重试动作 + 冲突清单；门控横幅展示阻断原因", () => {
  resetState();
  T.state.csApprovalBlock = { blocked: true, reason: "团队存在待审批 ChangeSet：cs1" };
  const html = T.changeSetsHtml([
    csFixture("cs1", "pending_review"),
    csFixture("cs2", "conflicted", { conflicts: ["src/user-edit.rs"] }),
  ]);
  // 门控横幅（Artifact 批准阻断原因置顶）
  assert.match(html, /data-cs-approval-block="1"/);
  assert.match(html, /批准被门控阻断/);
  assert.match(html, /团队存在待审批 ChangeSet：cs1/);
  // pending_review：明确「等待接受或拒绝」
  assert.match(html, /data-cs-status-hint="pending_review"/);
  assert.match(html, /等待接受或拒绝/);
  // conflicted：明确「存在冲突，禁止批准」+ 冲突文件 + 仍提供可重试动作
  assert.match(html, /data-cs-status-hint="conflicted"/);
  assert.match(html, /存在冲突，禁止批准/);
  assert.match(html, /data-cs-conflicts="cs2"/);
  assert.match(html, /src\/user-edit\.rs/);
  assert.equal(
    (html.match(/data-cs-act="accept"/g) || []).length,
    2,
    "conflicted 行同样提供接受/拒绝/撤销动作"
  );

  // 无门控 → 无横幅
  resetState();
  const html2 = T.changeSetsHtml([csFixture("cs1", "accepted")]);
  assert.ok(!html2.includes("data-cs-approval-block"));
  assert.ok(!html2.includes('data-cs-act="accept"'), "已接受行无动作");
});

test("artifactRowHtml（九期）：门控阻断时批准按钮禁用并展示原因，要求修改/驳回不受影响", () => {
  resetState();
  T.state.csApprovalBlock = { blocked: true, reason: "存在待审批 ChangeSet" };
  const artifact = {
    artifact_id: "a-1", version: 2, review_state: "pending_review",
    kind: "document", producer: "m-impl", created_at: "2026-08-30T04:00:00Z",
  };
  const html = T.artifactRowHtml(artifact, { items: [artifact] });
  assert.ok(/data-art-act="approve"[^>]*disabled/.test(html), "批准按钮应禁用");
  assert.match(html, /data-art-approve-blocked="a-1"/);
  assert.match(html, /存在待审批 ChangeSet/);
  assert.ok(!/data-art-act="request_changes"[^>]*disabled/.test(html), "要求修改不受门控影响");
  assert.ok(!/data-art-act="reject"[^>]*disabled/.test(html), "驳回不受门控影响");

  // 无门控 → 批准可用、无阻断提示
  resetState();
  const html2 = T.artifactRowHtml(artifact, { items: [artifact] });
  assert.ok(!/data-art-act="approve"[^>]*disabled/.test(html2));
  assert.ok(!html2.includes("data-art-approve-blocked"));
});

test("startChangeSetAction()（九期）：请求体携带 idempotency_key（修复空体 422）", async () => {
  resetState();
  T.state.current = "team-a";
  const posts = [];
  T.setTransport({
    get(path) {
      if (path === "/teams/team-a/change-sets") {
        return Promise.resolve({ team_id: "team-a", change_sets: [csFixture("cs1", "accepted")] });
      }
      return Promise.reject(new Error("404: " + path));
    },
    post(path, body) {
      posts.push({ path, body });
      return Promise.resolve({ change_set: csFixture("cs1", "accepted"), replayed: false });
    },
  });
  await T.startChangeSetAction("cs1", "reject");
  assert.equal(posts.length, 1);
  assert.equal(posts[0].path, "/change-sets/cs1/reject");
  assert.ok(
    posts[0].body && typeof posts[0].body.idempotency_key === "string" &&
      posts[0].body.idempotency_key.length > 0,
    "必须携带 idempotency_key（服务端缺键 422）"
  );
  // 每次点击新键：提交锁保证单次发送；失败后重试是新一轮真实决定。
  assert.notEqual(T.csIdemKey("cs1", "reject"), T.csIdemKey("cs1", "reject"));
});

test("创建团队表单使用任务语言并提供可访问的状态入口", () => {
  const html = T.renderCreate();
  assert.match(html, /团队目标/);
  assert.match(html, /运行模式/);
  assert.match(html, /组队策略/);
  assert.match(html, /多 Agent 协作/);
  assert.match(html, /单 Agent 执行/);
  assert.match(html, /DAG 工作流/);
  assert.match(html, /ws-create-template-retry/);
  assert.match(html, /aria-live="polite"/);
  assert.doesNotMatch(html, /目标 objective|模式 mode|角色规格 roles/);
  assert.equal(html.includes(">strategy<"), false);
  const roleHtml = T.roleRowHtml({ role: "planner", assignee: "agent", worker: "agent", depends_on: [] });
  assert.match(roleHtml, /aria-label="角色名称"/);
  assert.match(roleHtml, /Agent 执行/);
  assert.match(roleHtml, /自定义 Worker 名称或用户 ID/);
  const defaultRoleHtml = T.roleRowHtml({});
  assert.match(defaultRoleHtml, /<option value="agent" selected>/);
  assert.match(defaultRoleHtml, /class="owo-ws-role-worker"[^>]* hidden/);

});

test("模板加载失败时可就地重试，手工角色配置仍保留", async () => {
  resetState();
  T.state.templates = [];
  const elements = {
    "#ws-template": { value: "", innerHTML: "" },
    "#ws-create-template-retry": { hidden: true, disabled: false, textContent: "" },
    "#ws-create-template-status": { textContent: "" },
  };
  T.setRoot({ querySelector: (selector) => elements[selector] || null });
  const previous = T.getTransport();
  let calls = 0;
  T.setTransport({
    get: async () => {
      calls += 1;
      if (calls === 1) throw new Error("desktop authorization required");
      return { templates: [{ template_id: "review", name: "代码审查", mode: "team" }] };
    },
    post: previous.post,
  });
  try {
    const failed = await T.loadCreateTemplates();
    assert.equal(failed, null);
    assert.equal(elements["#ws-create-template-retry"].hidden, false);
    assert.match(elements["#ws-create-template-status"].textContent, /仍可直接填写角色规格创建团队/);
    assert.match(elements["#ws-template"].innerHTML, /按上方角色规格/);
    const loaded = await T.loadCreateTemplates();
    assert.equal(loaded.length, 1);
    assert.equal(elements["#ws-create-template-retry"].hidden, true);
    assert.match(elements["#ws-template"].innerHTML, /代码审查/);
  } finally {
    T.setTransport(previous);
    T.setRoot(null);
    T.state.templates = [];
  }
});


test("dagSvg dependency endpoints are numeric and inside the viewport", () => {
  const tasks = [
    { task_id: "source", role: "source", status: "Succeeded", depends_on: [] },
    { task_id: "left", role: "left", status: "Running", depends_on: ["source"] },
    { task_id: "right", role: "right", status: "Pending", depends_on: ["source"] },
    { task_id: "merge", role: "merge", status: "Pending", depends_on: ["left", "right"] },
  ];
  const svg = T.dagSvg(tasks, "Running");
  const width = Number(svg.match(/width="([0-9]+)"/)[1]);
  const edges = [...svg.matchAll(/<line x1="([^"]+)" y1="([^"]+)" x2="([^"]+)" y2="([^"]+)"/g)];
  assert.equal(edges.length, 4);
  assert.equal(Number(edges[0][1]), 222);
  for (const edge of edges) {
    const [x1, y1, x2, y2] = edge.slice(1).map(Number);
    assert.ok([x1, y1, x2, y2].every(Number.isFinite));
    assert.ok(x1 >= 0 && x1 < x2 && x2 < width, JSON.stringify({ x1, x2, width }));
  }
});


test("持久事件高频刷新只合并一次在途会话请求，离开页面取消请求并忽略迟到数据", async () => {
  T.stopLive(); resetState();
  const previous = T.getTransport();
  const callbacks = [];
  T.state.current = "team-a"; T.state.view = "detail"; T.state.handoffs = null;
  T.setTransport({ get(path, options) {
    return new Promise(resolve => callbacks.push({ resolve, path, signal: options.signal }));
  }});
  try {
    const first = T.loadTeamConversation("team-a", true);
    assert.equal(T.loadTeamConversation("team-a", true), first);
    assert.equal(T.loadTeamConversation("team-a", true), first);
    assert.equal(callbacks.length, 1);
    T.stopLive("已离开");
    assert.equal(callbacks[0].signal.aborted, true);
    callbacks[0].resolve({ handoffs: [{handoff_id:"late"}], sessions: [], next_cursors: {}, has_more: true });
    await first; await drain();
    assert.equal(callbacks.length, 1);
    assert.equal(T.state.handoffs, null);
    assert.equal(T.state.conversationPageTimer, null);
  } finally { T.stopLive(); T.setTransport(previous); resetState(); }
});

test("会话补页定时器在退出时清理，不产生详情页外的幽灵请求", async () => {
  T.stopLive(); resetState();
  const previous = T.getTransport();
  globalThis.OwoWorkswarmRenderConversation = require("../panels/workswarm/render_conversation.js");
  let calls = 0;
  T.state.current = "team-a"; T.state.view = "detail";
  T.state.conversationReplay = {}; T.state.conversationCursors = {};
  T.setTransport({ get: async () => {
    calls++;
    return { handoffs: [], sessions: [], next_cursors: {}, has_more: true };
  }});
  try {
    await T.loadTeamConversation("team-a", true);
    assert.notEqual(T.state.conversationPageTimer, null);
    T.stopLive();
    await drain();
    assert.equal(calls, 1);
    assert.equal(T.state.conversationPageTimer, null);
  } finally { T.stopLive(); T.setTransport(previous); resetState(); }
});

test("交接按实际游标翻页，刷新保留历史页，返回最新不携带旧游标", async () => {
  T.stopLive(); resetState();
  const previous = T.getTransport();
  globalThis.OwoWorkswarmRenderConversation = require("../panels/workswarm/render_conversation.js");
  const paths = [];
  T.state.current = "team-a"; T.state.view = "detail";
  T.setTransport({ get: async path => {
    paths.push(path);
    const older = new URL(path,"http://fixture").searchParams.has("handoff_before");
    return { handoffs: [{handoff_id:older ? "h-01" : "h-65"}], sessions: [],
      next_cursors:{}, next_session_after:"team-a-worker-next", has_more:false, next_handoff_before:older ? "h-01" : "h-65", handoffs_has_more:!older };
  }});
  try {
    await T.loadTeamConversation("team-a",true);
    assert.equal(T.state.handoffNextBefore,"h-65"); assert.equal(T.state.handoffHasMore,true);
    await T.navigateHandoffs(T.state.handoffNextBefore);
    assert.equal(new URL(paths.at(-1),"http://fixture").searchParams.get("session_after"),"team-a-worker-next");
    assert.equal(T.state.handoffBefore,"h-65"); assert.equal(T.state.handoffs[0].handoff_id,"h-01");
    assert.equal(T.state.handoffHasMore,false);
    assert.match(T.handoffPagingHtml(),/较早交接/);
    assert.match(T.handoffPagingHtml(),/data-handoff-page="older" disabled/);
    await T.loadTeamConversation("team-a",true);
    assert.equal(new URL(paths.at(-1),"http://fixture").searchParams.get("handoff_before"),"h-65");
    await T.navigateHandoffs(null);
    assert.equal(new URL(paths.at(-1),"http://fixture").searchParams.has("handoff_before"),false);
    assert.equal(T.state.handoffs[0].handoff_id,"h-65");
  } finally { T.stopLive(); T.setTransport(previous); resetState(); }
});

test("交接读取失败保留已读数据，退出取消且迟到页不覆盖当前团队", async () => {
  T.stopLive(); resetState();
  const previous = T.getTransport();
  const callbacks = [];
  T.state.current="team-a"; T.state.view="detail"; T.state.handoffs=[{handoff_id:"kept"}];
  T.setTransport({ get: () => Promise.reject(new Error("offline")) });
  try {
    await T.navigateHandoffs("h-65");
    assert.equal(T.state.handoffs[0].handoff_id,"kept");
    assert.ok(T.state.conversationError); assert.equal(T.state.handoffBusy,false);
    T.setTransport({ get: (path,options) => new Promise(resolve => callbacks.push({resolve,signal:options.signal})) });
    const pending=T.navigateHandoffs(null);
    assert.equal(T.state.handoffBusy,true);
    await T.navigateHandoffs("ignored");
    assert.equal(callbacks.length,1,"busy page navigation is serialized");
    T.stopLive();
    assert.equal(callbacks[0].signal.aborted,true);
    callbacks[0].resolve({handoffs:[{handoff_id:"late"}],sessions:[],next_cursors:{},has_more:false,
      next_handoff_before:"late",handoffs_has_more:true});
    await pending;
    assert.equal(T.state.handoffs[0].handoff_id,"kept");
    assert.equal(T.state.handoffBusy,false);
  } finally { T.stopLive(); T.setTransport(previous); resetState(); }
});
