// ============================================================================
// Action Center（待我处理）面板 Node 前端测试 —— desktop/web/tests/action-center.panel.test.mjs
//
// 运行：node --test "tests/*.test.mjs"（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert；无 DOM、无网络（传输层经 _test.setTransport 注入）。
//
// 覆盖面（七期第四路验收守卫）：
//   - 纯逻辑：状态归一、候选团队裁剪/排序、人节点任务闸门、失败步骤闸门、
//     写租约双路径容错、评审/校验产物过滤、项目空间派生、四类聚合；
//   - 冻结契约：POST /teams/{id}/steer retry 请求体形状；
//   - 渲染守卫：四类目节骨架、计数徽标、重试按钮 data 属性、提交锁禁用、
//     结果区 aria-live、空态文案、未知条目安全空串；
//   - 行为：load() 聚合落 state（404 产物静默、详情失败降级）、
//     submitRetry 提交锁（快速双击只发一次）+ 失败兜底 + 成功后重载；
//   - 深链：无 DOM 环境安全（Node 下不抛错）；
//   - 接线守卫：index.html 脚本、app-domain.js PANEL_ORDER、style.css 第 21 节在场。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const panel = require(join(here, "../panels/action-center.panel.js"));
const T = panel._test;
const shellCss = readFileSync(join(here, "../style.css"), "utf8");
const indexHtml = readFileSync(join(here, "../index.html"), "utf8");
const appJs = readFileSync(join(here, "../app.js"), "utf8");
const appDomainJs = readFileSync(join(here, "../app-domain.js"), "utf8");

// ---------- fixture 工具 ----------

function member(id, kind) {
  const binding =
    kind === "human"
      ? { kind: "human", user_id: "u-" + id }
      : kind === "agent"
        ? { kind: "agent", agent_id: "a-" + id }
        : { kind: "worker", worker_name: "w-" + id };
  return { member_id: id, runtime_binding: binding };
}

function task(id, status, worker, extra) {
  return Object.assign({ task_id: id, status: status, worker: worker, role: worker, attempts: 1 }, extra || {});
}

function detail(teamId, teamStatus, tasks, members, extra) {
  return Object.assign(
    {
      team: { team_id: teamId, status: teamStatus, members: members || [] },
      tasks: tasks || [],
      interrupted: false,
      audit_tail: [],
    },
    extra || {}
  );
}

function resetState() {
  panel.dispose();
  T.state.loading = false;
  T.state.loadedOnce = false;
  T.state.teams = [];
  T.state.details = {};
  T.state.detailErrors = {};
  T.state.artifacts = {};
  T.state.items = { human: [], review: [], failed: [], lease: [] };
  T.state.errors = [];
  T.state.retryBusy = {};
  T.state.retryResults = {};
  T.state.lastLoadedAt = "";
  // 八期三路：Inbox 优先 + 回退口径
  T.state.inbox = [];
  T.state.inboxSource = "";
  T.state.inboxBusy = {};
  T.state.inboxResults = {};
  T.setRoot(null);
}

// ---------- 注册与纯逻辑 ----------

test("dispose 存在：离开面板时可隔离在途请求", () => {
  assert.equal(typeof panel.dispose, "function");
});

test("刷新 Inbox 与初始加载竞争时，较新的刷新结果优先", async () => {
  resetState();
  let resolveInitial;
  let gets = 0;
  T.setTransport({
    get(path) {
      assert.equal(path, "/human/inbox");
      gets++;
      if (gets === 1) return new Promise((resolve) => { resolveInitial = resolve; });
      return Promise.resolve({ items: [inboxItem("fresh", "step_retry", "open")] });
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  const initial = T.load();
  await T.refreshInbox();
  resolveInitial({ items: [inboxItem("stale", "step_retry", "open")] });
  await initial;
  assert.deepEqual(T.state.inbox.map((item) => item.item_id), ["fresh"]);
  assert.equal(T.state.loading, false);
});

test("dispose：忽略离开面板前发出的 Inbox 响应", async () => {
  resetState();
  let resolveInbox;
  T.setTransport({
    get(path) {
      assert.equal(path, "/human/inbox");
      return new Promise((resolve) => { resolveInbox = resolve; });
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  const pending = T.load();
  panel.dispose();
  resolveInbox({ items: [inboxItem("late", "step_retry", "open")] });
  await pending;
  assert.equal(T.state.inboxSource, "");
  assert.equal(T.state.inbox.length, 0);
  assert.equal(T.state.loadedOnce, false);
  assert.equal(T.state.loading, false);
});

test("dispose：忽略离开面板前发出的待办处理结果", async () => {
  resetState();
  let resolvePost;
  let gets = 0;
  T.state.inbox = T.inboxItemsOf({ items: [inboxItem("late-action", "step_retry", "open")] });
  T.state.inboxSource = "inbox";
  let postCount = 0;
  T.setRoot({ innerHTML: "", querySelector: () => null });
  T.setTransport({
    get() { gets++; return Promise.resolve({ items: [] }); },
    post() { postCount++; return new Promise((resolve) => { resolvePost = resolve; }); },
  });
  const pending = T.startInboxAction("late-action", "resolve");
  await Promise.resolve();
  panel.dispose();
  const duplicate = T.startInboxAction("late-action", "resolve");
  await duplicate;
  assert.equal(postCount, 1, "卸载后原请求未完成时不能再次处理同一 Inbox 条目");
  resolvePost({ replayed: false });
  await pending;
  assert.equal(T.state.inboxResults["late-action"], undefined);
  assert.equal(T.state.inboxBusy["late-action:resolve"], undefined);
  assert.equal(gets, 0, "卸载后不应触发列表刷新");
});

// ---------- 注册与纯逻辑 ----------

test("面板注册：id/title/mount/_test 齐备", () => {
  assert.equal(panel.id, "action-center");
  assert.equal(panel.title, "待我处理");
  assert.equal(typeof panel.mount, "function");
  assert.equal(typeof T.load, "function");
  assert.equal(typeof T.aggregate, "function");
});

test("normStatus/isRetryableStep：Debug 帧归一 + Failed/Aborted 可重试", () => {
  assert.equal(T.normStatus("AwaitingHuman"), "awaiting_human");
  assert.equal(T.normStatus("RUNNING"), "running");
  assert.equal(T.normStatus(null), "");
  assert.equal(T.isRetryableStep({ status: "Failed" }), true);
  assert.equal(T.isRetryableStep({ status: "aborted" }), true);
  assert.equal(T.isRetryableStep({ status: "running" }), false);
  assert.equal(T.isRetryableStep(null), false);
});

test("candidateTeams：排除 succeeded/cancelled，awaiting_human 优先，截断上限", () => {
  const teams = [
    { team_id: "t-old", status: "failed", updated_at: "2024-01-01" },
    { team_id: "t-done", status: "succeeded" },
    { team_id: "t-cancel", status: "cancelled" },
    { team_id: "t-wait", status: "AwaitingHuman", updated_at: "2024-01-02" },
    { team_id: "t-run", status: "running", updated_at: "2024-01-03" },
    { team_id: "t-run2", status: "running", updated_at: "2024-01-04" },
    { team_id: null, status: "running" },
  ];
  const out = T.candidateTeams(teams, 3);
  assert.deepEqual(out.map((t) => t.team_id), ["t-wait", "t-run2", "t-run"], "awaiting_human 优先，同类按 updated_at 降序");
  // 不传上限 → 默认 12；failed 保留（存在可 retry 步骤）
  const all = T.candidateTeams(teams);
  assert.equal(all.length, 4);
  assert.ok(all.some((t) => t.team_id === "t-old"));
  assert.equal(T.candidateTeams(null).length, 0);
});

test("humanTasksOf/humanItemsOf：人节点闸门（human 成员 + 未终态）+ awaiting_human 团队兜底", () => {
  const d = detail("t-1", "running", [task("s-1", "running", "m-h"), task("s-2", "succeeded", "m-h"), task("s-3", "running", "m-a")], [
    member("m-h", "human"),
    member("m-a", "agent"),
  ]);
  const hts = T.humanTasksOf(d);
  assert.deepEqual(hts.map((t) => t.task_id), ["s-1"], "仅人成员且未终态的任务");

  const items = T.humanItemsOf(d);
  assert.equal(items.length, 1);
  assert.equal(items[0].team_id, "t-1");
  assert.equal(items[0].awaiting_team, false);
  assert.deepEqual(items[0].tasks.map((t) => t.task_id), ["s-1"]);

  // 团队 awaiting_human 但人节点任务已终态 → 仍聚合（团队级等待信号）
  const d2 = detail("t-2", "awaiting_human", [task("s-1", "succeeded", "m-h")], [member("m-h", "human")]);
  const items2 = T.humanItemsOf(d2);
  assert.equal(items2.length, 1);
  assert.equal(items2[0].awaiting_team, true);
  assert.deepEqual(items2[0].tasks, []);

  // running 团队且无未完成人节点 → 不聚合
  const d3 = detail("t-3", "running", [task("s-1", "succeeded", "m-h")], [member("m-h", "human")]);
  assert.deepEqual(T.humanItemsOf(d3), []);
  // 容错：空/缺字段
  assert.deepEqual(T.humanItemsOf(null), []);
  assert.deepEqual(T.humanTasksOf({}), []);
});

test("failedItemsOf：Failed/Aborted 步骤，error/failure_code 双字段，终态团队不提供", () => {
  const d = detail("t-1", "running", [
    task("s-1", "failed", "impl", { attempts: 2, error: "boom" }),
    task("s-2", "aborted", "rev", { failure_code: "output_contract_invalid" }),
    task("s-3", "succeeded", "ana"),
  ]);
  const items = T.failedItemsOf(d);
  assert.equal(items.length, 2);
  assert.equal(items[0].step_id, "s-1");
  assert.equal(items[0].attempts, 2);
  assert.equal(items[0].error, "boom");
  assert.equal(items[1].error, "output_contract_invalid", "error 缺失时回落 failure_code");
  assert.equal(items[1].role, "rev");

  assert.deepEqual(T.failedItemsOf(detail("t-2", "succeeded", [task("s-1", "failed", "x")])), [], "succeeded 终态不提供重试");
  assert.deepEqual(T.failedItemsOf(detail("t-3", "cancelled", [task("s-1", "failed", "x")])), [], "cancelled 终态不提供重试");
  assert.deepEqual(T.failedItemsOf(null), []);
});

test("leaseOf/leaseItemsOf：写租约双路径容错读取，已释放不聚合", () => {
  const lease = { holder_role: "implementer", holder_step_id: "s-2", acquired_at_ms: 1700000000000 };
  // 路径一：详情顶层（七期契约首选）
  assert.equal(T.leaseOf({ write_lease: lease }), lease);
  // 路径二：team 对象内（生成描述允许的双形状）
  assert.equal(T.leaseOf({ team: { write_lease: lease } }), lease);
  assert.equal(T.leaseOf({}), null);
  assert.equal(T.leaseOf(null), null);

  const items = T.leaseItemsOf({ team: { team_id: "t-1" }, write_lease: lease });
  assert.equal(items.length, 1);
  assert.equal(items[0].kind, "lease");
  assert.equal(items[0].holder_role, "implementer");
  assert.equal(items[0].holder_step_id, "s-2");
  assert.equal(items[0].acquired_at_ms, 1700000000000);

  assert.deepEqual(
    T.leaseItemsOf({ team: { team_id: "t-1" }, write_lease: Object.assign({}, lease, { released_at_ms: 1700000001000 }) }),
    [],
    "已释放的写租约不需要处理"
  );
  assert.deepEqual(T.leaseItemsOf({ team: { team_id: "t-1" } }), []);
});

test("normReviewState/reviewItemsOf：pendingreview + 校验未通过，其余过滤，team_id 回落", () => {
  assert.equal(T.normReviewState("Pending_Review"), "pendingreview");
  assert.equal(T.normReviewState("APPROVED"), "approved");
  assert.equal(T.normReviewState(null), "");

  const entry = {
    team_id: "t-1",
    items: [
      { artifact_id: "a-1", version: 2, kind: "markdown", format: "markdown", review_state: "PendingReview" },
      { artifact_id: "a-2", version: 1, kind: "json", format: "json", review_state: "draft", validation: { valid: false, reason: "JSON 解析失败：禁止 Markdown 围栏" } },
      { artifact_id: "a-3", version: 1, kind: "csv", format: "csv", review_state: "approved", validation: { valid: true } },
      { artifact_id: "a-4", version: 3, review_state: "rejected" },
      { version: 9, review_state: "pendingreview" }, // 缺 artifact_id → 跳过
      { artifact_id: "a-5", review_state: "pendingreview" }, // 全缺省字段 → 容错聚合
    ],
  };
  const items = T.reviewItemsOf("p-1", entry);
  assert.deepEqual(items.map((x) => x.artifact_id), ["a-1", "a-2", "a-5"]);
  assert.equal(items[0].validation_failed, false);
  assert.equal(items[1].validation_failed, true);
  assert.equal(items[1].validation_reason, "JSON 解析失败：禁止 Markdown 围栏");
  assert.equal(items[0].team_id, "t-1", "产物缺 team_id 时回落到 pid 所属团队");
  assert.equal(items[2].version, "");
  assert.equal(items[2].format, "");

  // 产物自带 team_id 优先
  const own = T.reviewItemsOf("p-1", { team_id: "t-1", items: [{ artifact_id: "a-9", review_state: "pendingreview", team_id: "t-9" }] });
  assert.equal(own[0].team_id, "t-9");

  assert.deepEqual(T.reviewItemsOf("p-1", { items: [] }), []);
  assert.deepEqual(T.reviewItemsOf("p-1", null), []);
});

test("projectIdsOf：project_space_id 优先，缺省 proj-<team_id>，去重截断", () => {
  const out = T.projectIdsOf(
    [
      { team_id: "t-1", project_space_id: "p-1" },
      { team_id: "t-2", project_space_id: "p-1" }, // 同项目去重
      { team_id: "t-3" }, // 缺省派生
      { team_id: "t-4" },
    ],
    2
  );
  assert.deepEqual(out, [
    { pid: "p-1", team_id: "t-1" },
    { pid: "proj-t-3", team_id: "t-3" },
  ]);
  assert.equal(T.projectIdsOf(null).length, 0);
});

test("aggregate：四类聚合；详情/产物加载失败条目跳过", () => {
  const st = {
    details: {
      "t-1": detail("t-1", "awaiting_human", [task("s-1", "failed", "impl", { error: "x" })], [member("m-h", "human")]),
      "t-2": null, // 详情加载失败 → 全跳过
    },
    artifacts: {
      "p-1": { team_id: "t-1", error: "", items: [{ artifact_id: "a-1", review_state: "pendingreview" }] },
      "p-2": { team_id: "t-2", error: "网络错误", items: [{ artifact_id: "a-bad", review_state: "pendingreview" }] }, // 出错条目跳过
    },
  };
  const agg = T.aggregate(st);
  assert.equal(agg.human.length, 1);
  assert.equal(agg.failed.length, 1);
  assert.equal(agg.review.length, 1);
  assert.equal(agg.lease.length, 0);
  assert.deepEqual(Object.keys(agg).sort(), ["failed", "human", "lease", "review"]);
  assert.deepEqual(T.aggregate(null), { human: [], review: [], failed: [], lease: [] });
});

// ---------- 冻结契约 ----------

test("buildRetryBody：POST /teams/{id}/steer 冻结契约（command=retry）", () => {
  const b = T.buildRetryBody("s-1");
  assert.equal(b.command, "retry");
  assert.equal(b.step_id, "s-1");
  assert.equal(b.note, "重试此节点：s-1", "确定性中文缺省 note");
  const b2 = T.buildRetryBody(" s-2 ", "换思路重试");
  assert.equal(b2.step_id, "s-2");
  assert.equal(b2.note, "换思路重试");
});

// ---------- 渲染守卫 ----------

test("sectionsHtml：四类目节骨架 + 计数 + 重试按钮 data 属性 + 提交锁 + 结果区", () => {
  const items = {
    human: [{ kind: "human", team_id: "t-1", awaiting_team: true, tasks: [{ task_id: "s-1", role: "plan", status: "running" }] }],
    review: [
      { kind: "review", project_id: "p-1", artifact_id: "a-1", version: "2", kind_name: "markdown", format: "markdown", team_id: "t-1", review_state: "pendingreview", validation_failed: false, validation_reason: "" },
      { kind: "review", project_id: "p-1", artifact_id: "a-2", version: "1", kind_name: "json", format: "json", team_id: "t-1", review_state: "draft", validation_failed: true, validation_reason: "JSON 解析失败" },
    ],
    failed: [{ kind: "failed", team_id: "t-1", step_id: "s-1", role: "impl", attempts: 2, error: "boom", interrupted: false }],
    lease: [{ kind: "lease", team_id: "t-1", holder_role: "implementer", holder_step_id: "s-2", acquired_at_ms: 1700000000000 }],
  };
  const html = T.sectionsHtml(items, {
    retryBusy: { "t-1/s-1": true },
    retryResults: { "t-1/s-1": { ok: true, text: "重试指令已发送" } },
  });

  // 四节 + 计数
  for (const key of ["human", "review", "failed", "lease"]) {
    assert.ok(html.includes('data-ac-sec="' + key + '"'), "缺少类目节 " + key);
  }
  assert.ok(html.includes('data-ac-sec="review"'), "review 节在场");
  assert.match(html, /<span class="owo-ac-count has">1<\/span>/);
  assert.match(html, /<span class="owo-ac-count has">2<\/span>/);

  // ① 人节点：团队徽标 + 任务清单 + 深链（§8：徽标为用户可见中文语义，
  // 内部枚举 awaiting_human 不得出现在界面文本中）。
  assert.ok(html.includes("团队等待人工处理"), "awaiting_team 徽标行");
  assert.ok(!html.includes("awaiting_human"), "内部枚举不得泄漏到界面");
  assert.ok(html.includes('data-ac-goto="t-1"'));
  assert.ok(html.includes("在团队详情「人节点结果」区提交结果"), "人节点结果提交指引（不得出现 API 路径）");
  assert.ok(!/POST \//.test(html.match(/<span class="hint">[^<]*<\/span>/g)?.join("") || ""), "hint 文案不得包含 API 动词路径");

  // ② 评审：待评审 / 校验未通过双徽标
  assert.ok(html.includes("待评审"));
  assert.ok(html.includes("校验未通过"));
  assert.ok(html.includes("JSON 解析失败"));
  assert.ok(html.includes("data-ac-goto-artifact"));

  // ③ 失败步骤：重试按钮（busy 禁用 + 文案）、错误行、结果区 aria-live
  assert.match(html, /data-ac-retry[^>]*data-team="t-1"[^>]*data-step="s-1"[^>]*disabled/);
  assert.ok(html.includes("重试中…"));
  assert.ok(html.includes(">boom<"));
  assert.match(html, /data-ac-result="t-1\/s-1"[^>]*aria-live="polite"[^>]*>重试指令已发送</);

  // ④ 写租约：持有者 + 步骤 + 时间
  assert.ok(html.includes("写租约持有中"));
  assert.ok(html.includes("implementer"));
  assert.ok(html.includes("s-2"));

  // 空态：全部为空时输出四条空态文案
  const empty = T.sectionsHtml({}, null);
  assert.match(empty, /暂无等待人工结果的任务/);
  assert.match(empty, /暂无可重试的失败步骤/);
  assert.equal((empty.match(/owo-ac-empty/g) || []).length, 4);

  // 未知条目 → 安全空串
  assert.equal(T.itemHtml({ kind: "unknown" }, {}, {}), "");
});

// ---------- 行为（传输层注入，无 DOM） ----------

test("load()：四类聚合落 state；404 产物静默；详情失败降级并记错误", async () => {
  resetState();
  const gets = [];
  T.setTransport({
    get(path) {
      gets.push(path);
      if (path === "/human/inbox") {
        // 九期回退口径：仅 404 触发 legacy 聚合（本测试验证 legacy 路径本身）。
        return Promise.reject(new Error("404: inbox offline"));
      }
      if (path === "/teams") {
        return Promise.resolve({
          teams: [
            { team_id: "t-1", status: "running", project_space_id: "p-1", updated_at: "2024-01-04" },
            { team_id: "t-2", status: "succeeded", project_space_id: "p-2" },
            { team_id: "t-3", status: "awaiting_human", updated_at: "2024-01-05" },
          ],
        });
      }
      if (path === "/teams/t-1") {
        return Promise.resolve(
          detail("t-1", "running", [task("s-1", "failed", "m-a", { error: "boom" })], [member("m-a", "agent"), member("m-h", "human")], {
            // t-1 无未完成人节点任务（human 成员无任务）
          })
        );
      }
      if (path === "/teams/t-3") {
        return Promise.resolve(
          detail("t-3", "awaiting_human", [task("s-9", "running", "m-h")], [member("m-h", "human")], {
            write_lease: { holder_role: "critic", holder_step_id: "s-8", acquired_at_ms: 1700000000000 },
          })
        );
      }
      if (path === "/projects/p-1/artifacts") {
        return Promise.resolve({
          artifacts: [
            { artifact_id: "a-1", version: 2, review_state: "PendingReview" },
            { artifact_id: "a-2", version: 1, review_state: "approved", validation: { valid: true } },
            { artifact_id: "a-3", version: 1, review_state: "draft", validation: { valid: false, reason: "CSV 列数不一致" } },
          ],
        });
      }
      if (path === "/projects/proj-t-3/artifacts") {
        return Promise.reject(new Error("404: project not found"));
      }
      return Promise.reject(new Error("unexpected GET " + path));
    },
    post() {
      return Promise.reject(new Error("unexpected POST"));
    },
  });

  await T.load();

  // 候选：t-1/t-3（succeeded 排除）
  assert.ok(gets.includes("/teams/t-1") && gets.includes("/teams/t-3"));
  assert.ok(!gets.includes("/teams/t-2"), "succeeded 团队不拉详情");
  assert.deepEqual(T.state.detailErrors, {});

  // ① t-3 awaiting_human + 人节点任务；t-1 无
  assert.equal(T.state.items.human.length, 1);
  assert.equal(T.state.items.human[0].team_id, "t-3");
  assert.deepEqual(T.state.items.human[0].tasks.map((t) => t.task_id), ["s-9"]);

  // ③ t-1 失败步骤
  assert.equal(T.state.items.failed.length, 1);
  assert.equal(T.state.items.failed[0].team_id, "t-1");
  assert.equal(T.state.items.failed[0].step_id, "s-1");

  // ② p-1 待评审 2 条（pendingreview + 校验未通过）；proj-t-3 404 静默
  assert.equal(T.state.items.review.length, 2);
  assert.equal(T.state.artifacts["proj-t-3"].error, "", "404 视为暂无产物");
  assert.deepEqual(T.state.artifacts["proj-t-3"].items, []);

  // ④ t-3 写租约（顶层路径）
  assert.equal(T.state.items.lease.length, 1);
  assert.equal(T.state.items.lease[0].holder_role, "critic");

  assert.equal(T.state.loadedOnce, true);
  assert.equal(T.state.loading, false);
});

test("load()：详情请求失败 → 降级空聚合 + detailErrors 记录，不阻塞其余类目", async () => {
  resetState();
  T.setTransport({
    get(path) {
      if (path === "/human/inbox") return Promise.reject(new Error("404: inbox offline"));
      if (path === "/teams") return Promise.resolve({ teams: [{ team_id: "t-x", status: "running" }] });
      if (path === "/teams/t-x") return Promise.reject(new Error("500: boom"));
      if (/^\/projects\//.test(path)) return Promise.resolve({ artifacts: [] });
      return Promise.reject(new Error("unexpected GET " + path));
    },
    post() {
      return Promise.reject(new Error("unexpected POST"));
    },
  });
  await T.load();
  assert.equal(T.state.details["t-x"], undefined);
  assert.match(T.state.detailErrors["t-x"], /500/);
  assert.equal(T.state.items.human.length, 0);
  assert.equal(T.state.items.failed.length, 0);
  assert.equal(T.state.loadedOnce, true, "降级后仍完成一轮加载");
});

test("load()：团队列表失败 → errors 记录，终态安全", async () => {
  resetState();
  T.setTransport({
    get(path) {
      // Inbox 404 → legacy；legacy 的 /teams 再失败 → errors 记录。
      if (path === "/human/inbox") return Promise.reject(new Error("404: inbox offline"));
      return Promise.reject(new Error("Failed to fetch"));
    },
    post() {
      return Promise.reject(new Error("unexpected POST"));
    },
  });
  await T.load();
  assert.match(T.state.errors[0], /团队列表加载失败/);
  assert.equal(T.state.loading, false);
});

test("submitRetry：提交锁（快速双击只发一次）+ 冻结契约 + 失败兜底 + 成功后重载", async () => {
  resetState();
  const posts = [];
  let failNext = false;
  let getRuns = 0;
  T.setTransport({
    get(path) {
      if (path === "/human/inbox") return Promise.reject(new Error("404: inbox offline"));
      if (path === "/teams") {
        getRuns++;
        return Promise.resolve({ teams: [{ team_id: "t-9", status: "running" }] });
      }
      if (path === "/teams/t-9") {
        return Promise.resolve(detail("t-9", "running", [task("s-1", "failed", "impl", { error: "x" })]));
      }
      if (/^\/projects\//.test(path)) return Promise.resolve({ artifacts: [] });
      return Promise.reject(new Error("unexpected GET " + path));
    },
    post(path, body) {
      posts.push({ path: path, body: body });
      if (failNext) return Promise.reject(new Error("409: team busy"));
      return Promise.resolve({});
    },
  });

  await T.load(); // 初始聚合（真实 UI 流程：面板先加载，再点重试）
  assert.equal(getRuns, 1);
  assert.equal(T.state.items.failed.length, 1, "预置一条失败步骤");

  const p1 = T.submitRetry("t-9", "s-1");
  const p2 = T.submitRetry("t-9", "s-1"); // 提交期间第二次调用：直接返回，不发请求
  await Promise.all([p1, p2]);
  assert.equal(posts.length, 1, "提交锁：快速双击只发一次");
  assert.equal(posts[0].path, "/teams/t-9/steer");
  assert.equal(posts[0].body.command, "retry");
  assert.equal(posts[0].body.step_id, "s-1");
  assert.ok(T.state.retryResults["t-9/s-1"].ok, "成功结果落 state");
  assert.ok(getRuns >= 2, "成功后重载聚合");
  assert.ok(!T.state.retryBusy["t-9/s-1"], "锁释放");

  // 失败兜底：409 → ok:false + 友好文案
  failNext = true;
  await T.submitRetry("t-9", "s-1");
  const res = T.state.retryResults["t-9/s-1"];
  assert.equal(res.ok, false);
  assert.match(res.text, /重试失败/);
  assert.match(res.text, /409/);
  assert.ok(!T.state.retryBusy["t-9/s-1"], "失败同样释放锁");

  // 重复步骤/团队各成一条（不同 key 互不干扰）
  const before = posts.length;
  T.setTransport({ get: T.getTransport().get, post: (p, b) => { posts.push({ path: p, body: b }); return Promise.resolve({}); } });
  await T.submitRetry("t-9", "s-2");
  assert.equal(posts.length, before + 1);
  assert.equal(posts[posts.length - 1].body.step_id, "s-2");
});

test("submitRetry：dispose 后原重试未结束前保留提交锁", async () => {
  resetState();
  let resolvePost;
  let postCount = 0;
  T.setTransport({
    get() { return Promise.resolve({}); },
    post() { postCount++; return new Promise((resolve) => { resolvePost = resolve; }); },
  });

  const pending = T.submitRetry("t-lock", "s-lock");
  await Promise.resolve();
  panel.dispose();
  await T.submitRetry("t-lock", "s-lock");
  assert.equal(postCount, 1, "页面离开后不能重复发送仍在途的 retry");
  assert.equal(T.state.retryBusy["t-lock/s-lock"], true);

  resolvePost({});
  await pending;
  assert.equal(T.state.retryBusy["t-lock/s-lock"], undefined, "旧请求结束后释放跨页面锁");
});

test("gotoWorkswarm：无 DOM 环境安全（Node 下不抛错）", () => {
  assert.doesNotThrow(() => T.gotoWorkswarm(""));
  assert.doesNotThrow(() => T.gotoWorkswarm("t-1"));
});

// ---------- 接线守卫 ----------

test("接线守卫：index.html 脚本 + PANEL_ORDER 唯一来源 app-domain.js + style.css 第 21 节在场", () => {
  assert.ok(indexHtml.includes('<script src="panels/action-center.panel.js"></script>'), "index.html 缺 action-center 脚本");
  // 模块拆分（§12.3）后 PANEL_ORDER 唯一来源为 app-domain.js；app.js 不得复制回巨型文件。
  assert.ok(!/const\s+PANEL_ORDER\s*=/.test(appJs), "PANEL_ORDER 不得在 app.js 中定义（唯一来源 app-domain.js）");
  assert.match(appDomainJs, /const PANEL_ORDER = \[([\s\S]*?)\]/, "app-domain.js PANEL_ORDER 未定义");
  const panelOrder = /const PANEL_ORDER = \[([\s\S]*?)\]/.exec(appDomainJs)[1];
  assert.ok(panelOrder.includes("\"capabilities\"") && panelOrder.includes("\"action-center\""), "app-domain.js PANEL_ORDER 未注册核心面板");
  assert.match(indexHtml, /<script src="app-domain\.js"><\/script>[\s\S]*<script src="app\.js"><\/script>/, "app-domain.js 必须先于 app.js 载入（经典脚本顺序）");
  assert.ok(shellCss.includes("21. 七期：Action Center"), "style.css 缺第 21 节 Action Center");
  for (const cls of [".owo-ac-item", ".owo-ac-badge.warn", ".owo-ac-count.has", ".owo-ac-result.ok"]) {
    assert.ok(shellCss.includes(cls), "style.css 缺 " + cls);
  }
});

// ============================================================================
// 八期三路守卫：正式 Human Inbox（/human/inbox）优先 + 直接处理
// ============================================================================

function inboxItem(id, kind, status, extra) {
  return Object.assign(
    { item_id: id, kind: kind, team_id: "team-a", project_id: "proj-a", target_id: "target-1",
      status: status || "open", summary: "待办 " + id, created_at: "2026-08-30T04:00:00Z" },
    extra || {}
  );
}

test("inboxItemsOf：归一化 + 非法条目丢弃 + 字段缺省", () => {
  const items = T.inboxItemsOf({
    items: [
      inboxItem("i1", "artifact_review", "open"),
      inboxItem("i2", "change_set", "claimed", { assignee: "alice" }),
      { item_id: "", kind: "artifact_review" },            // 无 id → 丢弃
      { item_id: "i3", kind: "unknown_kind" },             // 未知 kind → 丢弃
      null,
    ],
  });
  assert.equal(items.length, 2);
  assert.equal(items[0].status, "open");
  assert.equal(items[0].team_id, "team-a");
  assert.equal(items[1].status, "claimed");
  assert.equal(items[1].assignee, "alice");
  assert.equal(T.inboxItemsOf(null).length, 0);
});

test("normItemStatus/groupInbox：四类分组，resolved 不再出现", () => {
  assert.equal(T.normItemStatus("Claimed"), "claimed");
  const items = T.inboxItemsOf({
    items: [
      inboxItem("i1", "artifact_review", "open"),
      inboxItem("i2", "change_set", "resolved"),
      inboxItem("i3", "step_retry", "claimed"),
      inboxItem("i4", "human_result", "open"),
    ],
  });
  const g = T.groupInbox(items);
  assert.equal(g.artifact_review.length, 1);
  assert.equal(g.change_set.length, 0, "resolved 已处理条目不再出现");
  assert.equal(g.step_retry.length, 1);
  assert.equal(g.human_result.length, 1);
});

test("buildInboxActionPath：id encodeURIComponent + 动作分段", () => {
  assert.equal(T.buildInboxActionPath("i 1", "claim"), "/human/inbox/i%201/claim");
  assert.equal(T.buildInboxActionPath("i1", "resolve"), "/human/inbox/i1/resolve");
});

test("buildResolveBody：按 kind 分派（冻结③）+ 非法动作报错", () => {
  assert.deepEqual(T.buildResolveBody("artifact_review", "approve"), { decision: "approve" });
  assert.deepEqual(T.buildResolveBody("artifact_review", "request_changes", { comment: "改吧" }),
    { decision: "request_changes", comment: "改吧" });
  assert.deepEqual(T.buildResolveBody("change_set", "accept"), { action: "accept" });
  assert.deepEqual(T.buildResolveBody("change_set", "reject"), { action: "reject" });
  assert.deepEqual(T.buildResolveBody("step_retry"), {});
  assert.deepEqual(T.buildResolveBody("human_result", "resolve", { result: " 结果文本 " }),
    { result: "结果文本" });
  assert.deepEqual(T.buildResolveBody("artifact_review", "bogus"), { error: "unknown_decision" });
  assert.deepEqual(T.buildResolveBody("change_set", "bogus"), { error: "unknown_action" });
  assert.deepEqual(T.buildResolveBody("human_result", "resolve", { result: "  " }),
    { error: "missing_result" });
});

test("Inbox list exposes only matching real items and no empty category containers", () => {
  T.state.filters = { status: "", kind: "", q: "" };
  const items = T.inboxItemsOf({items: [
    inboxItem("i1", "artifact_review", "claimed"), inboxItem("i2", "change_set", "claimed"),
    inboxItem("i3", "step_retry", "open")
  ]});
  const html = T.inboxSectionsHtml(items);
  assert.match(html, /data-ic-item-row="i1"/);
  assert.match(html, /data-ic-act="approve"/);
  assert.match(html, /data-ic-act="accept"/);
  assert.match(html, /data-ic-act="claim"/);
  assert.match(html, /data-ac-goto="team-a"/);
  assert.doesNotMatch(html, /data-ic-sec=/);
  T.state.filters = {status: "open"};
  assert.doesNotMatch(T.inboxSectionsHtml(items), /data-ic-item-row="i1"/);
  T.state.filters = {status: "", kind: "", q: ""};
});

test("load()：inbox 优先（不触发 /teams 客户端聚合）", async () => {
  resetState();
  const log = [];
  T.setTransport({
    get(path) {
      log.push("GET " + path);
      if (path === "/human/inbox") {
        return Promise.resolve({ items: [inboxItem("i1", "step_retry", "open")] });
      }
      throw new Error("unexpected GET " + path);
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.load();
  assert.equal(T.state.inboxSource, "inbox");
  assert.equal(T.state.inbox.length, 1);
  assert.ok(!log.some((x) => x.startsWith("GET /teams")));
  assert.ok(!log.some((x) => x.startsWith("GET /human/inbox/i1/claim")));
});

// —— 九期第二路：回退口径修订（仅 404 回退 legacy；在线为空/其余失败保持 Inbox） ——

test("load()（九期）：inbox 在线但列表为空 → 保持 Inbox 口径，不偷偷切换数据源", async () => {
  resetState();
  const log = [];
  T.setTransport({
    get(path) {
      log.push("GET " + path);
      if (path === "/human/inbox") return Promise.resolve({ items: [] });
      throw new Error("unexpected GET " + path);
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.load();
  assert.equal(T.state.inboxSource, "inbox", "空列表也是 Inbox 的真实状态");
  assert.equal(T.state.inbox.length, 0);
  assert.ok(!log.some((x) => x.startsWith("GET /teams")), "不得回退客户端聚合");
});

test("load()（九期）：404（Inbox 未部署）→ 回退七期客户端聚合", async () => {
  resetState();
  T.setTransport({
    get(path) {
      if (path === "/human/inbox") return Promise.reject(new Error("404: not found"));
      if (path === "/teams") return Promise.resolve({ teams: [] });
      return Promise.reject(new Error("404: " + path));
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.load();
  assert.equal(T.state.inboxSource, "legacy");
});

test("load()（九期）：非 404 失败 → 保持 Inbox 口径 + 错误行，不切数据源", async () => {
  resetState();
  const log = [];
  T.setTransport({
    get(path) {
      log.push("GET " + path);
      if (path === "/human/inbox") return Promise.reject(new Error("500: boom"));
      throw new Error("unexpected GET " + path);
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.load();
  assert.equal(T.state.inboxSource, "inbox", "非 404 不回退聚合");
  assert.ok(!log.some((x) => x.startsWith("GET /teams")));
  assert.ok(
    T.state.errors.some((x) => /Inbox 加载失败/.test(x)),
    "错误行如实提示（面板不静默降级）"
  );
});

test("refreshInbox()（九期）：动作后重取为空仍保持 Inbox 口径", async () => {
  resetState();
  T.state.inboxSource = "inbox";
  const log = [];
  T.setTransport({
    get(path) {
      log.push("GET " + path);
      if (path === "/human/inbox") return Promise.resolve({ items: [] });
      throw new Error("unexpected GET " + path);
    },
    post() { throw new Error("unexpected POST"); },
  });
  T.setRoot({ innerHTML: "", querySelector: () => null });
  await T.refreshInbox();
  assert.equal(T.state.inboxSource, "inbox", "处理完成后空列表不再回退聚合");
  assert.ok(!log.some((x) => x.startsWith("GET /teams")));
});

test("startInboxAction()：claim → POST /human/inbox/{id}/claim；提交锁双击一次", async () => {
  resetState();
  const log = [];
  T.state.inbox = T.inboxItemsOf({ items: [inboxItem("i1", "change_set", "open")] });
  T.state.inboxSource = "inbox";
  T.setTransport({
    get(path) {
      if (path === "/human/inbox") {
        return Promise.resolve({ items: [inboxItem("i1", "change_set", "claimed", { assignee: "me" })] });
      }
      return Promise.reject(new Error("404: " + path));
    },
    post(path) {
      log.push("POST " + path);
      return Promise.resolve({ item_id: "i1", replayed: false });
    },
  });
  const p1 = T.startInboxAction("i1", "claim");
  const p2 = T.startInboxAction("i1", "claim");
  await Promise.all([p1, p2]);
  const posts = log.filter((x) => x.startsWith("POST"));
  assert.equal(posts.length, 1, JSON.stringify(posts));
  assert.equal(posts[0], "POST /human/inbox/i1/claim");
  assert.equal(T.state.inboxResults["i1"].ok, true);
});

test("startInboxAction()：artifact_review resolve 按 decision 分派到 resolve 端点", async () => {
  resetState();
  const log = [];
  T.state.inbox = T.inboxItemsOf({ items: [inboxItem("i1", "artifact_review", "claimed")] });
  T.state.inboxSource = "inbox";
  T.setTransport({
    get() { return Promise.resolve({ items: [] }); },
    post(path, body) {
      log.push("POST " + path + " " + JSON.stringify(body));
      return Promise.resolve({ replayed: true });
    },
  });
  await T.startInboxAction("i1", "approve");
  assert.equal(log.length, 1);
  assert.equal(log[0], 'POST /human/inbox/i1/resolve {"decision":"approve","user":"本地用户"}');
  assert.match(T.state.inboxResults["i1"].text, /幂等重放/);
});

test("startInboxAction()：human_result 缺 result 不发请求", async () => {
  resetState();
  const log = [];
  T.state.inbox = T.inboxItemsOf({ items: [inboxItem("i1", "human_result", "claimed")] });
  T.state.inboxSource = "inbox";
  T.setTransport({
    get() { return Promise.resolve({ items: [] }); },
    post(path) { log.push("POST " + path); return Promise.resolve({}); },
  });
  await T.startInboxAction("i1", "resolve", { result: "   " });
  assert.equal(log.length, 0);
  assert.match(T.state.inboxResults["i1"].text, /无法构建请求/);
});

test("startInboxAction()：失败兜底（friendly 文本，不抛错）", async () => {
  resetState();
  T.state.inbox = T.inboxItemsOf({ items: [inboxItem("i1", "change_set", "claimed")] });
  T.state.inboxSource = "inbox";
  T.setTransport({
    get() { return Promise.resolve({ items: [] }); },
    post() { return Promise.reject(new Error("409: conflicted")); },
  });
  await T.startInboxAction("i1", "accept");
  assert.equal(T.state.inboxResults["i1"].ok, false);
  assert.match(T.state.inboxResults["i1"].text, /操作失败/);
});

test("Inbox preserves multiline result and review drafts across refresh and filtering", async () => {
  resetState();
  T.state.filters = {};
  T.state.drafts = Object.create(null);
  const it = inboxItem("draft-human", "human_result", "claimed");
  T.rememberInput({target:{value:"第一行\\n第二行", getAttribute:k => k === "data-ic-note" ? it.item_id : "result"}});
  assert.match(T.inboxItemHtml(it), /第一行\\n第二行/);
  T.state.inboxSource = "inbox";
  T.setTransport({get:async()=>({items:[it]})});
  await T.refreshInbox();
  assert.equal(T.state.drafts[it.item_id].result, "第一行\\n第二行");
  T.state.filters = {q:"not-found"};
  assert.match(T.inboxSectionsHtml([it]), /没有匹配/);
  T.state.filters = {};
  assert.match(T.inboxSectionsHtml([it]), /第一行\\n第二行/);
});

test("Inbox serializes different actions on the same item but allows another item", async () => {
  resetState();
  T.state.inbox = T.inboxItemsOf({items:[inboxItem("lock", "change_set", "claimed")]});
  let finish, posts=0;
  T.setTransport({post:()=>{posts++; return new Promise(r=>{finish=r;});},get:async()=>({items:[]})});
  const pending=T.startInboxAction("lock","accept");
  await Promise.resolve(); await Promise.resolve();
  await T.startInboxAction("lock","release");
  assert.equal(posts,1);
  finish({});
  await pending;
});

test("Inbox respects assignee and rejects prototype property kinds", () => {
  const other=inboxItem("owned", "human_result", "claimed", {assignee:"another-user"});
  assert.doesNotMatch(T.inboxItemHtml(other), /data-ic-act=|textarea/);
  assert.equal(T.inboxItemsOf({items:[{item_id:"bad",kind:"constructor"}]}).length,0);
  assert.equal(T.filteredInbox([other],{q:"ANOTHER-USER"}).length,1);
});
