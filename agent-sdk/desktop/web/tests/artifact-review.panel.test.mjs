// ============================================================================
// Artifact 评审闭环面板测试 —— desktop/web/tests/artifact-review.panel.test.mjs
//
// 运行：node tests/artifact-review.panel.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert / node 内置模块；无 DOM、无网络。
//
// 覆盖面（第四路四期 · 第三路 review API 的 UI 侧守卫）：
//   - 版本链分组：单件 / 线性链 / 多链混排 / 断链（supersedes 指向不存在产物）/
//     approved head 选取（最高版本已批准者）；
//   - 评审状态徽标映射：Draft/PendingReview/Approved/Changes Requested/Rejected；
//   - 评审可操作性门控：仅 PendingReview 且非提交中出现表单与按钮；
//   - 评审请求体：decision 白名单、reviewer 必填、生产者禁止自行批准、
//     expected_version 取自产物版本、幂等键生成；
//   - 提交行为（transport 注入）：成功 / 409 / 403 / 网络失败 / 忙锁（重复提交拒绝）；
//   - 评审历史渲染：不可变记录的徽标/评审者/评语/时间。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const panel = require(join(here, "../panels/workswarm.panel.js"));
const T = panel._test;

function resetState() {
  T.state.current = "team-1";
  T.state.artifacts = [];
  T.state.reviewBusy = {};
  T.state.reviewResult = null;
  T.state.reviewFlash = null;
}

const V1 = { artifact_id: "art-1", kind: "document", version: 1, producer: "m-builder", review_state: "Approved", created_at: "2025-01-01T00:00:00Z", preview: "v1 内容" };
const V2 = { artifact_id: "art-2", kind: "document", version: 2, producer: "m-builder", review_state: "Approved", supersedes_artifact_id: "art-1", created_at: "2025-01-02T00:00:00Z", preview: "v2 内容" };
const V3 = { artifact_id: "art-3", kind: "document", version: 3, producer: "m-critic", review_state: "PendingReview", supersedes_artifact_id: "art-2", created_at: "2025-01-03T00:00:00Z", preview: "v3 内容" };

// ---------------- 版本链分组 ----------------
test("groupArtifactChain：单件自成链，approved head 指向已批准版本", () => {
  const chains = T.groupArtifactChain([V1]);
  assert.equal(chains.length, 1);
  assert.equal(chains[0].items.length, 1);
  assert.equal(chains[0].approvedHead.artifact_id, "art-1");
});

test("groupArtifactChain：线性链按版本升序，head=最新，approvedHead=最高已批准", () => {
  const chains = T.groupArtifactChain([V3, V1, V2]); // 乱序输入
  assert.equal(chains.length, 1);
  const ids = chains[0].items.map((a) => a.artifact_id);
  assert.deepEqual(ids, ["art-1", "art-2", "art-3"]);
  assert.equal(chains[0].approvedHead.artifact_id, "art-2", "v3 尚未批准，head 应为 v2");
});

test("groupArtifactChain：断链（supersedes 指向不存在产物）另起一条链", () => {
  const orphan = { artifact_id: "art-x", kind: "code", version: 1, producer: "m-builder", review_state: "Draft", supersedes_artifact_id: "art-ghost", created_at: "2025-01-04T00:00:00Z" };
  const chains = T.groupArtifactChain([V1, orphan]);
  assert.equal(chains.length, 2);
});

test("groupArtifactChain：多条 kind 混排互不串链", () => {
  const code1 = { artifact_id: "c1", kind: "code", version: 1, producer: "m-builder", review_state: "PendingReview", created_at: "2025-01-05T00:00:00Z" };
  const chains = T.groupArtifactChain([V1, code1]);
  assert.equal(chains.length, 2);
});

test("groupArtifactChain：空数组/null → 空链；无 artifact_id 的脏行被剔除", () => {
  assert.deepEqual(T.groupArtifactChain([]), []);
  assert.deepEqual(T.groupArtifactChain(null), []);
  assert.equal(T.groupArtifactChain([{ kind: "x" }]).length, 0);
});

// ---------------- 状态徽标与可操作性 ----------------
test("normReviewState/reviewBadgeHtml：六种评审状态映射与徽标分类", () => {
  assert.equal(T.normReviewState("PendingReview"), "pendingreview");
  assert.equal(T.normReviewState("Changes_Requested"), "changesrequested");
  assert.equal(T.reviewBadgeHtml("Draft"), '<span class="owo-ws-badge rv-off" data-art-state="draft">草稿</span>');
  assert.equal(T.reviewBadgeHtml("pending_review"), '<span class="owo-ws-badge rv-warn" data-art-state="pendingreview">待评审</span>');
  assert.equal(T.reviewBadgeHtml("PendingReview"), '<span class="owo-ws-badge rv-warn" data-art-state="pendingreview">待评审</span>');
  assert.equal(T.reviewBadgeHtml("Approved"), '<span class="owo-ws-badge rv-ok" data-art-state="approved">已批准</span>');
  assert.equal(T.reviewBadgeHtml("changes_requested"), '<span class="owo-ws-badge rv-warn" data-art-state="changesrequested">要求修改</span>');
  assert.equal(T.reviewBadgeHtml("Rejected"), '<span class="owo-ws-badge rv-bad" data-art-state="rejected">已驳回</span>');
  assert.equal(T.reviewBadgeHtml("superseded"), '<span class="owo-ws-badge rv-off" data-art-state="superseded">已被取代</span>');
});

test("isReviewable：仅 PendingReview 且非提交中可评审", () => {
  resetState();
  T.state.artifacts = [V1, V2, V3];
  assert.equal(T.isReviewable(V1), false);
  assert.equal(T.isReviewable(V3), true);
  T.state.reviewBusy["art-3"] = true;
  assert.equal(T.isReviewable(V3), false, "提交进行中（按钮锁定）不可重复评审");
});

// ---------------- 评审请求体 ----------------
test("buildReviewBody：合法 approve 生成冻结契约六字段", () => {
  resetState();
  const body = T.buildReviewBody({ artifact: V3, decision: "approve", reviewer: "critic-alice", comment: "结构和证据通过", teamId: "team-1", idempotencyKey: "k-1" });
  assert.deepEqual(
    { team_id: body.team_id, decision: body.decision, reviewer: body.reviewer, comment: body.comment, expected_version: body.expected_version, idempotency_key: body.idempotency_key },
    { team_id: "team-1", decision: "approve", reviewer: "critic-alice", comment: "结构和证据通过", expected_version: 3, idempotency_key: "k-1" }
  );
});

test("buildReviewBody：未知动作 / 缺评审者 → 抛错", () => {
  resetState();
  assert.throws(() => T.buildReviewBody({ artifact: V3, decision: "maybe", reviewer: "critic" }), /未知评审动作/);
  assert.throws(() => T.buildReviewBody({ artifact: V3, decision: "approve", reviewer: "   " }), /评审者/);
});

test("buildReviewBody：生产者（含 m- 前缀形态）不能自行批准", () => {
  resetState();
  assert.throws(() => T.buildReviewBody({ artifact: V3, decision: "approve", reviewer: "m-critic" }), /生产者不能自行批准/);
  assert.throws(() => T.buildReviewBody({ artifact: V3, decision: "approve", reviewer: "critic" }), /生产者不能自行批准/);
  // 非生产者评审者批准通过；且 request_changes/reject 不受生产者限制
  assert.equal(T.buildReviewBody({ artifact: V3, decision: "approve", reviewer: "human-bob" }).decision, "approve");
  assert.equal(T.buildReviewBody({ artifact: V3, decision: "request_changes", reviewer: "critic" }).decision, "request_changes");
});

test("buildReviewBody：幂等键缺省时自动生成且含 artifact/decision/reviewer", () => {
  resetState();
  const k1 = T.buildReviewBody({ artifact: V3, decision: "reject", reviewer: "human-bob" }).idempotency_key;
  assert.match(k1, /^art-3:3:reject:human-bob:/);
  assert.notEqual(k1, T.buildReviewBody({ artifact: V3, decision: "reject", reviewer: "human-bob" }).idempotency_key, "不同意图生成新键");
});

// ---------------- 提交行为（transport 注入） ----------------
function ok(body) { return Promise.resolve({ accepted: true, body }); }

test("submitArtifactReview：成功路径记录 reviewResult 并解锁", async () => {
  resetState();
  T.state.artifacts = [V3];
  T.setTransport({ post: (url, body) => { calls.push(url); return ok(body); } });
  let calls = [];
  const resp = await T.submitArtifactReview({ artifactId: "art-3", decision: "approve", reviewer: "human-bob", comment: "通过", idempotencyKey: "k-9" });
  assert.equal(resp.accepted, true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0], "/artifacts/art-3/review");
  assert.equal(T.state.reviewResult.ok, true);
  assert.equal(T.state.reviewResult.decision, "approve");
  assert.equal(T.state.reviewBusy["art-3"], undefined, "提交完成后解锁");
});

test("submitArtifactReview：409 → 计划规定文案（版本已更新，请刷新后重试）", async () => {
  resetState();
  T.state.artifacts = [V3];
  T.setTransport({ post: () => Promise.reject({ status: 409, message: "version conflict" }) });
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-3", decision: "approve", reviewer: "human-bob" }));
  assert.equal(T.state.reviewResult.ok, false);
  assert.match(T.state.reviewResult.error, /版本已更新，请刷新后重试/);
  assert.equal(T.state.reviewBusy["art-3"], undefined, "失败后必须解锁");
});

test("submitArtifactReview：403 → 无评审权限文案", async () => {
  resetState();
  T.state.artifacts = [V3];
  T.setTransport({ post: () => Promise.reject({ status: 403, message: "forbidden" }) });
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-3", decision: "approve", reviewer: "human-bob" }));
  assert.match(T.state.reviewResult.error, /无评审权限（403）/);
});

test("submitArtifactReview：网络失败 → 可操作提示（连接/服务确认）", async () => {
  resetState();
  T.state.artifacts = [V3];
  T.setTransport({ post: () => Promise.reject(new Error("Failed to fetch")) });
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-3", decision: "reject", reviewer: "human-bob" }));
  assert.match(T.state.reviewResult.error, /owo-agent-server/);
  assert.equal(T.state.reviewBusy["art-3"], undefined);
});

test("submitArtifactReview：提交中重复提交被忙锁拒绝；非法产物拒绝", async () => {
  resetState();
  T.state.artifacts = [V3];
  let resolveGate;
  const gate = new Promise((r) => { resolveGate = r; });
  T.setTransport({ post: () => gate.then(() => ({ accepted: true })) });
  const p1 = T.submitArtifactReview({ artifactId: "art-3", decision: "approve", reviewer: "human-bob" });
  assert.equal(T.state.reviewBusy["art-3"], true);
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-3", decision: "reject", reviewer: "human-bob" }), /进行中/);
  resolveGate({ accepted: true });
  await p1;
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-ghost", decision: "approve", reviewer: "human-bob" }), /产物不存在/);
  // 客户端前置校验错误（生产者自批）不发出请求
  T.state.artifacts = [V3];
  await assert.rejects(() => T.submitArtifactReview({ artifactId: "art-3", decision: "approve", reviewer: "critic" }), /生产者不能自行批准/);
});

// ---------------- 评审历史渲染 ----------------
test("artifactHistoryHtml：不可变记录映射（空/含评语/决策徽标）", () => {
  assert.match(T.artifactHistoryHtml([]), /暂无评审记录/);
  assert.match(T.artifactHistoryHtml(null), /暂无评审记录/);
  const html = T.artifactHistoryHtml([
    { decision: "approve", reviewer: "human-bob", comment: "结构和证据通过", created_at: "2025-01-03T01:00:00Z" },
    { decision: "request_changes", reviewer: "critic-alice", comment: "补运行证据", created_at: "2025-01-02T01:00:00Z" },
    { decision: "reject", reviewer: "human-carol", comment: "", created_at: "2025-01-01T01:00:00Z" },
  ]);
  assert.match(html, /已批准/);
  assert.match(html, /要求修改/);
  assert.match(html, /已驳回/);
  assert.match(html, /human-bob/);
  assert.match(html, /结构和证据通过/);
  assert.match(html, /（无评语）/);
});

// ---------------- 行/链渲染守卫 ----------------
test("renderArtifactsChains：链头标记 + approved head + 评审表单仅出现在待评审版本", () => {
  resetState();
  T.state.artifacts = [V1, V2, V3];
  const chains = T.groupArtifactChain([V1, V2, V3]);
  const html = T.renderArtifactsChains(chains);
  assert.match(html, /版本链（3 个版本）/);
  assert.match(html, /当前 approved head：/);
  assert.match(html, /v2/);
  assert.match(html, /data-art-act="approve"/);
  assert.match(html, /data-art-act="request_changes"/);
  assert.match(html, /data-art-act="reject"/);
  // 只有 v3（PendingReview）带评审表单
  assert.equal((html.match(/owo-ws-review-form/g) || []).length, 1);
  assert.match(html, /取代 v2/);
});

test("renderArtifactsChains：无链渲染为空串（空态由外层处理）", () => {
  assert.equal(T.renderArtifactsChains([]), "");
  assert.equal(T.renderArtifactsChains(null), "");
});

test("artifactRowHtml：提交中的产物行评审按钮 disabled（锁定）", () => {
  resetState();
  T.state.artifacts = [V3];
  const chains = T.groupArtifactChain([V3]);
  const busyHtml = T.artifactRowHtml(V3, chains[0]);
  assert.ok(!/disabled/.test(busyHtml));
  T.state.reviewBusy["art-3"] = true;
  const lockedHtml = T.artifactRowHtml(V3, chains[0]);
  assert.match(lockedHtml, /disabled/);
});

test("artifactRowHtml：reviewFlash 存续——产物区重载后结果提示不丢", () => {
  resetState();
  T.state.artifacts = [V3];
  const chains = T.groupArtifactChain([V3]);
  assert.ok(!/评审已提交/.test(T.artifactRowHtml(V3, chains[0])), "无 flash 时不渲染");
  T.state.reviewFlash = { artifactId: "art-3", ok: true, text: "评审已提交：批准（记录不可变）" };
  const okHtml = T.artifactRowHtml(V3, chains[0]);
  assert.match(okHtml, /评审已提交：批准（记录不可变）/);
  T.state.reviewFlash = { artifactId: "art-3", ok: false, text: "版本已更新，请刷新后重试" };
  const badHtml = T.artifactRowHtml(V3, chains[0]);
  assert.match(badHtml, /版本已更新，请刷新后重试/);
  assert.match(badHtml, /owo-ws-review-result sub bad/, "失败 flash 带 bad 语义类");
});
