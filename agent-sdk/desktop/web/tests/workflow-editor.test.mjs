// ============================================================================
// 用户自定义工作流（桌面 Web）纯逻辑测试 —— tests/workflow-editor.test.mjs
//
// 运行：node tests/workflow-editor.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert，无 DOM、无网络。
//
// 覆盖面：JSON 解析与本地轻量检查、依赖边与预览行、请求体组装（校验/保存/AI 草稿/
//         导出/导入）、评审结果与列表项文案。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const source = readFileSync(join(here, "../core/workflow-editor.js"), "utf8");
const scope = {};
new Function("window", source)(scope);
const E = scope.OwoWorkflowEditor._test;

function loadEditor() {
  const scope = {};
  new Function("window", source)(scope);
  return scope.OwoWorkflowEditor._test;
}


const GOOD = {
  template_id: "video-recreate",
  name: "视频复刻流水线",
  mode: "team",
  applicability: "视频复刻 分镜",
  roles: [
    { role: "script", assignee: "agent", depends_on: [], tools: ["read_file"], write_paths: ["script/"] },
    { role: "render", assignee: "agent", depends_on: ["script"], tools: ["write_file"], verify: "non_empty" },
    { role: "acceptance", assignee: "human", depends_on: ["render"] },
  ],
};

test("parseTemplate：空输入/非法 JSON/非对象都要明确报错", () => {
  assert.equal(E.parseTemplate("").ok, false);
  assert.match(E.parseTemplate("{ 不是 json").error, /JSON 解析失败/);
  assert.match(E.parseTemplate("[1,2]").error, /必须是一个 JSON 对象/);
  const parsed = E.parseTemplate(JSON.stringify(GOOD));
  assert.equal(parsed.ok, true);
  assert.equal(parsed.template.template_id, "video-recreate");
});

test("quickCheck：必填/角色重复/assignee/依赖缺失都提示（服务端仍为最终权威）", () => {
  assert.deepEqual(E.quickCheck(GOOD), []);
  assert.ok(E.quickCheck({}).some((p) => p.includes("template_id")));
  assert.ok(E.quickCheck({ template_id: "x", name: "n", roles: [] }).some((p) => p.includes("roles 至少")));
  const duplicated = JSON.parse(JSON.stringify(GOOD));
  duplicated.roles.push({ role: "script", assignee: "agent" });
  assert.ok(E.quickCheck(duplicated).some((p) => p.includes("角色名重复")));
  const badAssignee = JSON.parse(JSON.stringify(GOOD));
  badAssignee.roles[0].assignee = "robot";
  assert.ok(E.quickCheck(badAssignee).some((p) => p.includes("assignee 非法")));
  const ghost = JSON.parse(JSON.stringify(GOOD));
  ghost.roles[1].depends_on = ["ghost"];
  assert.ok(E.quickCheck(ghost).some((p) => p.includes("依赖不存在的角色")));
});

test("buildEdges/previewLines：依赖边与角色预览行", () => {
  assert.deepEqual(E.buildEdges(GOOD), [
    { from: "script", to: "render" },
    { from: "render", to: "acceptance" },
  ]);
  const lines = E.previewLines(GOOD);
  assert.equal(lines.length, 3);
  assert.match(lines[0], /^- script（agent）/);
  assert.match(lines[1], /← script 工具：write_file 校验：non_empty/);
  assert.equal(E.buildEdges(null).length, 0);
});

test("layoutGraph：最长路径分层、扇出与环检测", () => {
  const E2 = loadEditor();
  const linear = {
    roles: [
      { role: "a", assignee: "agent", depends_on: [] },
      { role: "b", assignee: "agent", depends_on: ["a"] },
      { role: "c", assignee: "human", depends_on: ["b"] },
    ],
  };
  const layout = E2.layoutGraph(linear);
  assert.equal(layout.ok, true);
  assert.equal(layout.layers, 3);
  assert.deepEqual(
    layout.nodes.map((node) => [node.id, node.layer]),
    [["a", 0], ["b", 1], ["c", 2]]
  );
  assert.deepEqual(layout.edges, [
    { from: "a", to: "b" },
    { from: "b", to: "c" },
  ]);

  // 扇出：同一上游的两个下游同层，行号不同。
  const fanOut = E2.layoutGraph({
    roles: [
      { role: "root", depends_on: [] },
      { role: "left", depends_on: ["root"] },
      { role: "right", depends_on: ["root"] },
    ],
  });
  const left = fanOut.nodes.find((node) => node.id === "left");
  const right = fanOut.nodes.find((node) => node.id === "right");
  assert.equal(left.layer, 1);
  assert.equal(right.layer, 1);
  assert.notEqual(left.row, right.row);

  // 成环：返回 ok=false，不产出错误图形。
  const cyclic = E2.layoutGraph({
    roles: [
      { role: "x", depends_on: ["y"] },
      { role: "y", depends_on: ["x"] },
    ],
  });
  assert.equal(cyclic.ok, false);
  assert.match(cyclic.error, /环/);

  // 未知依赖按无上游处理（校验器负责报错）；空模板不报错。
  const ghost = E2.layoutGraph({ roles: [{ role: "solo", depends_on: ["ghost"] }] });
  assert.equal(ghost.ok, true);
  assert.equal(ghost.nodes[0].layer, 0);
  assert.deepEqual(E2.layoutGraph({}).nodes, []);
});

test("请求体组装：草稿/导出/导入（含 review-only）都做前置校验", () => {
  assert.equal(E.buildDraftRequest("   ").ok, false);
  assert.deepEqual(E.buildDraftRequest("做一条视频流水线", "必须人工", "draft-1"), {
    ok: true,
    path: "/workflows/draft",
    body: { description: "做一条视频流水线", constraints: "必须人工", refine_of: "draft-1" },
  });
  assert.deepEqual(E.buildDraftRequest("x"), { ok: true, path: "/workflows/draft", body: { description: "x" } });
  // 轮次 >1 → 收敛接口（并在本地限幅到 5）；=1/缺省仍走单轮生成。
  assert.deepEqual(E.buildDraftRequest("x", null, null, 3), {
    ok: true,
    path: "/workflows/draft/converge",
    body: { description: "x", max_rounds: 3 },
  });
  assert.equal(E.buildDraftRequest("x", null, null, 99).body.max_rounds, 5);
  assert.equal(E.buildDraftRequest("x", null, null, 1).path, "/workflows/draft");
  assert.equal(E.buildDraftRequest("x", null, "draft-1", 1).body.refine_of, "draft-1");

  assert.equal(E.buildExportRequest("").ok, false);
  assert.equal(E.buildExportRequest("a b").path, "/workflows/user/a%20b/export");

  assert.equal(E.buildImportRequest("").ok, false);
  assert.equal(E.buildImportRequest(" pkg ", true).path, "/workflows/import/review");
  assert.deepEqual(E.buildImportRequest("pkg", false).body, { package_b64: "pkg" });
});

test("buildTeamCreateRequest：目标必填，带上模板 id 与 mode", () => {
  const E2 = loadEditor();
  assert.equal(E2.buildTeamCreateRequest({ template_id: "a" }, "  ").ok, false);
  assert.equal(E2.buildTeamCreateRequest({}, "目标").ok, false);
  assert.deepEqual(E2.buildTeamCreateRequest({ template_id: "a", mode: "swarmflow" }, "  做一条视频  ", true), {
    ok: true,
    path: "/teams",
    body: {
      objective: "做一条视频",
      mode: "swarmflow",
      template_id: "a",
      acknowledge_risk: true,
    },
  });
  assert.deepEqual(E2.buildTeamCreateRequest({ template_id: "a" }, "x").body, {
    objective: "x",
    mode: "team",
    template_id: "a",
    acknowledge_risk: false,
  });
  assert.equal(E2.buildTeamCreateRequest({ template_id: "a" }, "x", true).body.acknowledge_risk, true);
});

test("describeTrajectory：每轮错误数/通过/未改善停止", () => {
  assert.deepEqual(E.describeTrajectory(null), []);
  const lines = E.describeTrajectory([
    { round: 1, roles: 4, errors: 2, valid: false, accepted: true },
    { round: 2, roles: 4, errors: 1, valid: false, accepted: true },
    { round: 3, roles: 3, errors: 0, valid: true, accepted: true },
  ]);
  assert.equal(lines[0], "  #1 角色 4 / 错误 2");
  assert.match(lines[2], /角色 3 \/ 错误 0（通过）/);
  const stopped = E.describeTrajectory([{ round: 2, roles: 2, errors: 2, accepted: false }]);
  assert.match(stopped[0], /（未改善，停止）/);
  const failed = E.describeTrajectory([{ round: 2, error: "502", detail: "模型调用失败" }]);
  assert.match(failed[0], /#2 失败：模型调用失败/);
});

test("describeDraftHealth：通过 ≠ 低风险（风险/工具/MCP/任意写都要提示）", () => {
  assert.deepEqual(E.describeDraftHealth({}), []);
  assert.deepEqual(E.describeDraftHealth({ health: null }), []);
  assert.deepEqual(E.describeDraftHealth({ health: { risk_level: "low" } }), ["风险：低"]);
  const lines = E.describeDraftHealth({
    health: {
      risk_level: "high",
      elevated_tools: ["run_command", "web_fetch"],
      mcp_tools: ["mcp__owo_plugin_translate__translate"],
      whole_workspace_writers: ["impl"],
    },
  });
  assert.equal(lines[0], "风险：高");
  assert.ok(lines.some((line) => line.includes("执行/联网工具：run_command, web_fetch")));
  assert.ok(lines.some((line) => line.includes("MCP 依赖：mcp__owo_plugin_translate__translate")));
  assert.ok(lines.some((line) => line.includes("工作区任意写：impl")));

  // 轨迹里的风险也按中文标签显示。
  const trajectory = E.describeTrajectory([{ round: 2, roles: 3, errors: 0, valid: true, risk_level: "high" }]);
  assert.match(trajectory[0], /错误 0 \/ 风险 高（通过）/);
});

test("roleHealthRows：逐角色风险标记（执行/MCP/任意写/人工/无工具）", () => {
  const rows = E.roleHealthRows({
    roles: [
      { role: "impl", assignee: "agent", tools: ["write_file"], write_paths: [] },
      { role: "fetch", assignee: "agent", tools: ["run_command", "mcp__owo_plugin_translate__translate"], write_paths: ["out/"] },
      { role: "accept", assignee: "human", tools: [] },
      { role: "talker", assignee: "agent", tools: [] },
    ],
  });
  assert.equal(rows.length, 4);
  assert.deepEqual(rows[0].risks, ["未声明写范围（工作区任意写）"]);
  assert.ok(rows[1].risks.includes("执行/联网工具:run_command"), rows[1].risks);
  assert.ok(rows[1].risks.some((risk) => risk.indexOf("MCP 依赖:") === 0), rows[1].risks);
  assert.deepEqual(rows[2].risks, ["需要人工确认"]);
  assert.deepEqual(rows[3].risks, ["无工具（只产出文本）"]);
  // 只读工具 + 声明写范围：没有风险标记。
  const safe = E.roleHealthRows({ roles: [{ role: "reader", assignee: "agent", tools: ["read_file"], write_paths: [] }] });
  assert.deepEqual(safe[0].risks, [], "只读角色不应被标成任意写");
  assert.deepEqual(E.roleHealthRows(null), []);
});

test("describeStatsRow：成功率/时间/失败定位（无终态显示 —）", () => {
  const full = E.describeStatsRow({
    template_id: "video-recreate",
    name: "视频复刻",
    origin: "community",
    risk_level: "high",
    runs: 3,
    succeeded: 1,
    failed: 2,
    cancelled: 0,
    running: 1,
    awaiting_human: 0,
    success_rate: 0.3333,
    last_run_at: "2026-10-18T21:42:00+00:00",
    last_failure: {
      team_id: "team-1",
      at: "2026-10-18T21:40:00+00:00",
      event: "team.failed",
      detail: "步骤失败且 replan 未启用：[\"s-impl\"]",
    },
  });
  assert.equal(full.riskLabel, "高");
  assert.equal(full.successRate, "33%");
  assert.equal(full.lastRunAt, "10-18 21:42");
  assert.equal(full.lastFailure.event, "team.failed");
  assert.match(full.lastFailure.detail, /replan 未启用/);

  // 草稿的上次验证结果（采纳后用同一目标再跑的结论）。
  assert.equal(E.describeVerifyOutcome(null), "");
  assert.equal(E.describeVerifyOutcome({}), "");
  assert.equal(
    E.describeVerifyOutcome({
      verify_outcome: { status: "succeeded", compared_status: "failed", team_id: "team-1" },
    }),
    "上次验证：succeeded（基线 failed） · 状态有变化"
  );
  assert.match(
    E.describeVerifyOutcome({ verify_outcome: { status: "failed", compared_status: "failed" } }),
    /未康复（与基线相同）/
  );
  assert.match(
    E.describeVerifyOutcome({ draft: { verify_outcome: { status: "failed" } } }),
    /无基线对比/
  );

  // 最近一次运行：无记录 / 有记录（状态中文 + 可复用目标）。
  // 按运行证据改进：草稿请求带上 based_on；based_on 与 refine_of 互斥。
  const basedRequest = E.buildDraftRequest("按证据改进", "", "", 1, "flaky-flow");
  assert.equal(basedRequest.ok, true);
  assert.equal(basedRequest.path, "/workflows/draft");
  assert.equal(basedRequest.body.based_on, "flaky-flow");
  const convergeRequest = E.buildDraftRequest("多轮改进", "", "", 3, "flaky-flow");
  assert.equal(convergeRequest.path, "/workflows/draft/converge");
  assert.equal(convergeRequest.body.based_on, "flaky-flow");
  assert.equal(convergeRequest.body.max_rounds, 3);
  const conflicting = E.buildDraftRequest("冲突", "", "draft-1", 1, "flaky-flow");
  assert.equal(conflicting.ok, false);
  assert.match(conflicting.error, /不能同时使用/);

  const none = E.describeLastRun({ template_id: "x", last_run: null });
  assert.equal(none.hasRun, false);
  assert.match(none.hint, /从未运行/);

  const last = E.describeLastRun({
    template_id: "x",
    last_run: {
      team_id: "team-1",
      objective: "复现上次失败：改哪里",
      status: "failed",
      created_at: "2026-10-18T22:00:00Z",
    },
  });
  assert.equal(last.hasRun, true);
  assert.equal(last.teamId, "team-1");
  assert.equal(last.objective, "复现上次失败：改哪里");
  assert.equal(last.statusLabel, "失败");
  assert.match(last.hint, /沿用上次目标/);

  const listItem = E.describeWorkflow({
    template: { template_id: "broken", name: "坏流程", roles: [{}, {}] },
    origin: "user",
    valid: true,
    runs: 3,
    attention: ["从未成功（失败 3 次）"],
  });
  assert.equal(listItem.runs, 3);
  assert.deepEqual(listItem.attention, ["从未成功（失败 3 次）"]);

  const quiet = E.describeWorkflow({ template: { template_id: "ok" }, origin: "user" });
  assert.deepEqual(quiet.attention, []);
  assert.equal(quiet.runs, 0);

  const flagged = E.describeStatsRow({
    template_id: "broken",
    runs: 2,
    failed: 2,
    success_rate: 0,
    last_run_failed: true,
    attention: ["从未成功（失败 2 次）", "最近一次运行失败"],
  });
  assert.equal(flagged.lastRunFailed, true);
  assert.deepEqual(flagged.attention, ["从未成功（失败 2 次）", "最近一次运行失败"]);

  const noTerminal = E.describeStatsRow({ template_id: "x", runs: 2, running: 2, success_rate: null });
  assert.equal(noTerminal.successRate, "—", "没有终态时不编造 0%");
  assert.equal(noTerminal.lastFailure, null);
  assert.equal(noTerminal.lastRunAt, "");
  assert.equal(E.describeStatsRow(null).id, "?");
});

test("buildStatsUri：排序白名单与 limit 限幅", () => {
  assert.equal(E.buildStatsUri("", null), "/workflows/stats");
  assert.equal(E.buildStatsUri("failed"), "/workflows/stats?sort=failed");
  assert.equal(E.buildStatsUri("bogus", 5), "/workflows/stats?limit=5");
  assert.equal(E.buildStatsUri("last", 999), "/workflows/stats?sort=last&limit=200");
  assert.equal(E.buildStatsUri("success_rate", 0), "/workflows/stats?sort=success_rate");
});

test("describeReview：校验错误、finding 分级与门禁提示", () => {
  const blocked = {
    blocked: true,
    findings: [
      { level: "block", category: "schema", detail: "角色 a 依赖不存在的角色：b" },
      { level: "warn", category: "capability", detail: "包含执行/联网类工具：run_command" },
    ],
  };
  const lines = E.describeReview({ review: blocked, errors: ["x"] });
  assert.ok(lines[0].startsWith("[错误] x"));
  assert.ok(lines.some((l) => l.startsWith("[block] schema：")));
  assert.ok(lines.some((l) => l.startsWith("[warn] capability：")));
  assert.ok(lines.some((l) => l.startsWith("[门禁]")));
  assert.deepEqual(E.describeReview({ review: { blocked: false, findings: [] } }), ["校验通过，没有发现需要关注的问题"]);
});

test("describeWorkflow：列表项文案（origin/角色数/可用性/能力面提示）", () => {
  const row = E.describeWorkflow({ origin: "community", valid: true, template: GOOD });
  assert.equal(row.id, "video-recreate");
  assert.equal(row.name, "视频复刻流水线");
  assert.equal(row.origin, "community");
  assert.equal(row.roles, 3);
  assert.equal(row.valid, true);
  assert.equal(row.tag, "可创建团队");
  assert.equal(row.warnings, 0);
  assert.equal(row.blocked, false);
  assert.deepEqual(row.findings, []);
  assert.equal(
    E.describeWorkflow({ origin: "user", valid: true, warnings: 2, template: { template_id: "x", roles: [] } }).warnings,
    2
  );
  assert.equal(E.describeWorkflow({ origin: "user", valid: false, template: { template_id: "x" } }).tag, "校验未通过");
});

test("describeLocalDiff：导入时展示与本地同名工作流的差异与风险", () => {
  assert.deepEqual(E.describeLocalDiff({ diff_vs_local: null }), []);
  assert.deepEqual(E.describeLocalDiff({}), []);
  const lines = E.describeLocalDiff({
    diff_vs_local: {
      local_origin: "community",
      local_version: "1.0.0",
      incoming_version: "1.1.0",
      summary: ["元数据 version：1.0.0 → 1.1.0", "[风险] 角色 impl 新增执行/联网工具：run_command"],
    },
  });
  assert.match(lines[0], /与本地同名工作流（community，版本 1\.0\.0 → 1\.1\.0）/);
  assert.ok(lines.some((line) => line.includes("元数据 version")));
  assert.ok(lines.some((line) => line.includes("[风险]")));
  const noChange = E.describeLocalDiff({
    diff_vs_local: { local_origin: "user", local_version: "1.0.0", incoming_version: "1.0.0", summary: [] },
  });
  assert.ok(noChange.some((line) => line.includes("没有差异")));
});

test("riskLabel / describeWorkflow：健康等级映射与缺省", () => {
  assert.equal(E.riskLabel("low"), "低");
  assert.equal(E.riskLabel("medium"), "中");
  assert.equal(E.riskLabel("high"), "高");
  assert.equal(E.riskLabel("unknown"), "");
  assert.equal(E.riskLabel(""), "");

  const risky = E.describeWorkflow({
    origin: "user",
    valid: true,
    health: { risk_level: "high", roles: 3, tools: 5, max_depth: 2, human_roles: 1 },
    template: GOOD,
  });
  assert.equal(risky.risk, "high");
  assert.equal(risky.riskLabel, "高");
  assert.equal(E.describeWorkflow({ template: GOOD }).riskLabel, "", "没有 health 时不臆造风险");
});

test("provenanceLabel / describeWorkflow：社区导入溯源展示（时间按 RFC3339 截取）", () => {
  assert.equal(E.provenanceLabel(null), "");
  assert.equal(E.provenanceLabel({}), "");
  assert.equal(
    E.provenanceLabel({ imported_at: "2026-10-18T19:39:29+00:00", sha256: "08063fccf1b2aaaa" }),
    "导入@10-18 19:39 · sha256 08063fcc"
  );
  // 时间字段异常时不臆造日期，只给 sha256。
  assert.equal(E.provenanceLabel({ imported_at: "?", sha256: "abcdef12" }), "导入@? · sha256 abcdef12");
  assert.equal(E.provenanceLabel({ sha256: "abcdef12" }), "sha256 abcdef12");

  const row = E.describeWorkflow({
    origin: "community",
    valid: true,
    template: GOOD,
    provenance: { imported_at: "2026-10-18T19:39:29+00:00", sha256: "08063fccf1b2" },
  });
  assert.equal(row.provenanceLabel, "导入@10-18 19:39 · sha256 08063fcc");
  assert.equal(E.describeWorkflow({ template: GOOD }).provenanceLabel, "", "无溯源时不显示");
});

test("buildTeamCreateConfirm：warn 一次性确认；blocked 直接拒绝；无发现返回 null", () => {
  assert.equal(E.buildTeamCreateConfirm({ findings: [], template: { template_id: "a" } }), null);
  const warn = E.buildTeamCreateConfirm({
    findings: [
      { level: "warn", category: "capability", detail: "包含执行/联网类工具：run_command" },
      { level: "info", category: "summary", detail: "角色 2 个" },
    ],
    template: { template_id: "a" },
  });
  assert.match(warn, /确认后才会创建团队/);
  assert.match(warn, /capability：包含执行\/联网类工具：run_command/);
  assert.doesNotMatch(warn, /角色 2 个/, "info 级不进入确认文案");

  const blocked = E.buildTeamCreateConfirm({
    findings: [{ level: "block", category: "schema", detail: "角色 a 依赖不存在的角色：b" }],
    blocked: true,
    template: { template_id: "a" },
  });
  assert.match(blocked, /不能建队/);
  assert.match(blocked, /依赖不存在的角色/);
});
