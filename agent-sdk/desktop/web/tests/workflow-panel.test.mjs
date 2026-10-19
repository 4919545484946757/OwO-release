// ============================================================================
// 工作流面板渲染测试 —— tests/workflow-panel.test.mjs
//
// 运行：node tests/workflow-panel.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert / node:vm，无 DOM、无网络。
//
// 守卫：面板必须注册到 OwoPanels["custom-workflows"]、纯渲染函数（列表行/评审文本/草稿预览）
//       输出契约稳定，且不出现 console.log / 硬编码 URL（panels-lint 另有全量扫描）。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const read = (name) => readFileSync(join(here, "..", name), "utf8");

function loadPanel() {
  const win = {};
  new Function("window", read("core/workflow-editor.js"))(win);
  new Function("window", read("panels/workflows.panel.js"))(win);
  return win;
}

test("面板注册到 custom-workflows 并暴露 mount/dispose", () => {
  const win = loadPanel();
  assert.ok(win.OwoPanels && win.OwoPanels["custom-workflows"], "缺少 custom-workflows 面板注册");
  assert.equal(typeof win.OwoPanels["custom-workflows"].mount, "function");
  assert.equal(typeof win.OwoPanels["custom-workflows"].dispose, "function");
  assert.equal(typeof win.OwoPanels["custom-workflows"].refresh, "function");
});

test("renderWorkflowRows：空列表与条目的 origin/角色数/可用性", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  assert.match(panel.renderWorkflowRows({ templates: [] }), /还没有自定义工作流/);
  const html = panel.renderWorkflowRows({
    templates: [
      { origin: "user", valid: true, template: { template_id: "a", name: "流水线 A", roles: [{}, {}] } },
      { origin: "community", valid: false, template: { template_id: "b", name: "流水线 B", roles: [] } },
    ],
  });
  assert.match(html, /data-id="a"/);
  assert.match(html, /流水线 A/);
  assert.match(html, /user · 2 个角色 · 可创建团队/);
  assert.match(html, /community · 0 个角色 · 校验未通过/);
  assert.match(html, /class="owo-wf-export"/);
  assert.match(html, /class="owo-wf-delete"/);
});

test("renderReviewText/renderDraftText：评审分级与草稿预览（含依赖与校验问题）", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const review = panel.renderReviewText({
    review: {
      blocked: true,
      findings: [{ level: "block", category: "schema", detail: "角色 x 依赖不存在的角色：y" }],
    },
  });
  assert.match(review, /\[block\] schema：角色 x 依赖不存在的角色：y/);
  assert.match(review, /\[门禁\] 评审未通过/);

  const draft = panel.renderDraftText({
    ok: false,
    template: {
      template_id: "d1",
      name: "关卡流水线",
      description: "设计 → 生成 → 验收",
      roles: [
        { role: "design", assignee: "agent", depends_on: [], tools: ["read_file"] },
        { role: "build", assignee: "agent", depends_on: ["design"] },
      ],
    },
    errors: ["角色 build 的 tools 非法：未知工具：x"],
  });
  assert.match(draft, /校验未通过：关卡流水线/);
  assert.match(draft, /设计 → 生成 → 验收/);
  assert.match(draft, /- design（agent） 工具：read_file/);
  assert.match(draft, /- build（agent） ← design/);
  assert.match(draft, /\[错误\] 角色 build 的 tools 非法/);

  // 收敛轨迹（多轮）出现在草稿预览里。
  const converged = panel.renderDraftText({
    ok: true,
    template: { template_id: "d4", name: "收敛流水线", roles: [] },
    trajectory: [
      { round: 1, roles: 3, errors: 2 },
      { round: 2, roles: 3, errors: 0, valid: true },
    ],
  });
  assert.match(converged, /收敛轨迹：/);
  assert.match(converged, /#1 角色 3 \/ 错误 2/);
  assert.match(converged, /#2 角色 3 \/ 错误 0（通过）/);

  // 通过但有风险时必须在预览里显示健康行。
  const risky = panel.renderDraftText({
    ok: true,
    template: { template_id: "d5", name: "有风险但通过", roles: [] },
    health: { risk_level: "high", elevated_tools: ["run_command"] },
  });
  assert.match(risky, /健康：风险：高 · 执行\/联网工具：run_command/);
});

test("renderWorkflowRows：显示健康风险徽标（来自 health.risk_level）", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const html = panel.renderWorkflowRows({
    templates: [
      { origin: "user", valid: true, health: { risk_level: "high" }, template: { template_id: "a", name: "A", roles: [{}] } },
      { origin: "user", valid: true, health: { risk_level: "low" }, template: { template_id: "b", name: "B", roles: [{}] } },
    ],
  });
  assert.match(html, /风险 高/);
  assert.match(html, /风险 低/);
});

test("renderDraftText：标注自动修复轮次与校验问题", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const repaired = panel.renderDraftText({
    ok: true,
    repaired: true,
    attempts: 2,
    template: { template_id: "d1", name: "关卡流水线", roles: [] },
  });
  assert.match(repaired, /校验通过：关卡流水线（已按校验错误自动修复 2 轮内）/);

  const plain = panel.renderDraftText({
    ok: true,
    repaired: false,
    template: { template_id: "d2", name: "普通", roles: [] },
  });
  assert.doesNotMatch(plain, /自动修复/);

  const broken = panel.renderDraftText({
    ok: false,
    repaired: false,
    template: { template_id: "d3", name: "坏的", roles: [] },
    errors: ["角色 x 依赖不存在的角色：y"],
  });
  assert.match(broken, /校验未通过：坏的/);
  assert.match(broken, /\[错误\] 角色 x 依赖不存在的角色：y/);
});

test("renderWorkflowRows：社区导入显示溯源（导入时间 + sha256 前缀）", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const html = panel.renderWorkflowRows({
    templates: [
      {
        origin: "community",
        valid: true,
        template: { template_id: "a", name: "A", roles: [{}] },
        provenance: { imported_at: "2026-10-18T19:39:29+00:00", sha256: "08063fccf1b2" },
      },
    ],
  });
  assert.match(html, /导入@10-18 19:39/);
  assert.match(html, /sha256 08063fcc/);
});

test("renderRoleRows：逐角色工具/写范围/风险（HTML）", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const html = panel.renderRoleRows({
    roles: [
      { role: "impl", assignee: "agent", tools: ["write_file"], write_paths: [] },
      { role: "reader", assignee: "agent", tools: ["read_file"], write_paths: ["docs/"] },
    ],
  });
  assert.match(html, /角色健康/);
  assert.match(html, /<strong>impl<\/strong> <code>agent<\/code> 工具 1/);
  assert.match(html, /未声明写范围（工作区任意写）/);
  assert.match(html, /写 docs\//);
  assert.equal(panel.renderRoleRows({ roles: [] }), "");
});

test("renderStatsRows：运行/成败/成功率/最近失败（HTML）", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  assert.match(panel.renderStatsRows({ templates: [] }), /还没有用这些工作流创建过团队/);
  const html = panel.renderStatsRows({
    templates: [
      {
        template_id: "video-recreate",
        name: "视频复刻",
        origin: "user",
        risk_level: "low",
        runs: 3,
        failed: 2,
        succeeded: 1,
        cancelled: 0,
        running: 0,
        awaiting_human: 1,
        success_rate: 0.3333,
        last_run_at: "2026-10-18T21:42:00+00:00",
        last_failure: { event: "team.failed", detail: "步骤失败且 replan 未启用" },
      },
    ],
  });
  assert.match(html, /视频复刻/);
  assert.match(html, /运行 3 · 成功 1 · 失败 2 · 成功率 33%/);
  assert.match(html, /最后 10-18 21:42/);
  assert.match(html, /最近失败：team.failed · 步骤失败且 replan 未启用/);
  assert.doesNotMatch(html, /需要修复/, "未提供 attention 时不显示提示");

  const rerunRows = panel.renderWorkflowRows({
    templates: [
      {
        template: { template_id: "flaky", name: "易错流程", roles: [{}] },
        origin: "user",
        valid: true,
        runs: 2,
        attention: [],
      },
    ],
  });
  assert.match(rerunRows, /class="owo-wf-rerun"/, "列表行应提供沿用上次目标再跑的入口");
  assert.match(
    rerunRows,
    /class="owo-wf-improve"/,
    "列表行应提供按运行证据改进的入口"
  );

  const rows = panel.renderWorkflowRows({
    templates: [
      {
        template: { template_id: "broken", name: "坏流程", roles: [{}] },
        origin: "user",
        valid: true,
        runs: 2,
        attention: ["从未成功（失败 2 次）"],
      },
      {
        template: { template_id: "fresh", name: "新流程", roles: [{}] },
        origin: "user",
        valid: true,
        runs: 0,
        attention: [],
      },
    ],
  });
  assert.match(rows, /⚠ 需要修复：从未成功（失败 2 次）/);
  assert.match(rows, /运行 2 次/);
  // "fresh"（未运行）那一条后面不应出现告警。
  assert.ok(
    !/需要修复/.test(rows.slice(rows.indexOf("fresh"))),
    "未运行的流程不应显示告警"
  );

  const preview = panel.renderDraftText({
    ok: true,
    verify_outcome: { status: "failed", compared_status: "failed", team_id: "team-9" },
    draft: { draft_id: "draft-1" },
    template: { template_id: "flaky", roles: [{}] },
  });
  assert.match(preview, /上次验证：failed（基线 failed）/);

  const statsWithRerun = panel.renderStatsRows({
    templates: [
      {
        template_id: "flaky",
        name: "易错流程",
        origin: "user",
        runs: 3,
        failed: 3,
        success_rate: 0,
        attention: ["从未成功（失败 3 次）"],
      },
    ],
  });
  assert.match(
    statsWithRerun,
    /<button type="button" class="owo-wf-rerun" data-id="flaky">再跑<\/button>/,
    "统计行也应提供沿用上次目标的再跑入口"
  );

  const flagged = panel.renderStatsRows({
    templates: [
      {
        template_id: "broken",
        runs: 2,
        failed: 2,
        success_rate: 0,
        attention: ["从未成功（失败 2 次）"],
      },
    ],
  });
  assert.match(flagged, /⚠ 需要修复：从未成功（失败 2 次）/);
  assert.match(html, /等待人工 1/);
});

test("renderGraphSvg：节点/边/分层坐标与环提示", () => {
  const panel = loadPanel().OwoPanels["custom-workflows"];
  const svg = panel.renderGraphSvg({
    roles: [
      { role: "a", assignee: "agent", depends_on: [] },
      { role: "b", assignee: "human", depends_on: ["a"] },
    ],
  });
  assert.match(svg, /<svg class="owo-wf-graph"/);
  assert.match(svg, /aria-label="工作流依赖图"/);
  assert.match(svg, />a</);
  assert.match(svg, />b</);
  assert.match(svg, /<path d="M /, "应有依赖边");
  assert.equal((svg.match(/<rect /g) || []).length, 2, "两个角色两个节点");

  const cyclic = panel.renderGraphSvg({
    roles: [
      { role: "x", depends_on: ["y"] },
      { role: "y", depends_on: ["x"] },
    ],
  });
  assert.match(cyclic, /环/);
  assert.doesNotMatch(cyclic, /<svg/);

  assert.equal(panel.renderGraphSvg({ roles: [] }), "");
});

test("describeLocalDiff 与评审输出可拼接（导入前看到本地差异）", () => {
  const win = loadPanel();
  const editor = win.OwoWorkflowEditor;
  const lines = editor.describeLocalDiff({
    diff_vs_local: {
      local_origin: "user",
      local_version: "1.0.0",
      incoming_version: "1.2.0",
      summary: ["[风险] 角色 a 的写范围从受限变为工作区任意写"],
    },
    review: { blocked: false, findings: [{ level: "warn", category: "mcp", detail: "依赖 MCP" }] },
  });
  const review = win.OwoPanels["custom-workflows"].renderReviewText({
    review: { blocked: false, findings: [{ level: "warn", category: "mcp", detail: "依赖 MCP" }] },
  });
  const combined = review + "\n\n" + lines.join("\n");
  assert.match(combined, /\[warn\] mcp：依赖 MCP/);
  assert.match(combined, /与本地同名工作流（user，版本 1\.0\.0 → 1\.2\.0）的差异/);
  assert.match(combined, /写范围从受限变为工作区任意写/);
});

test("面板源码遵守前端约定（无 console.log / 无硬编码 http URL）", () => {
  const source = read("panels/workflows.panel.js");
  assert.doesNotMatch(source, /console\.log/);
  assert.doesNotMatch(source, /https?:\/\//);
  assert.match(source, /window\.OwoPanels\.workflows/);
});
