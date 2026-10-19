import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const read = path => readFileSync(new URL("../" + path, import.meta.url), "utf8");
const navigation = require("../core/workbench-navigation.js");
const domain = require("../permissions/permissions.domain.js");

test("all existing panels remain searchable, grouped and unique", () => {
  const ids = navigation.entries.map(item => item.id);
  assert.equal(ids.length, 23);
  assert.equal(new Set(ids).size, 23);
  assert.deepEqual(navigation.search("权限").map(item => item.id), ["plugin-market", "permissions", "command"]);
  assert.equal(navigation.search("project", ["notes"]).length, 0);
  assert.equal(navigation.find("missing"), null);
});

test("effective permissions initialize the draft without inventing defaults", () => {
  const overview = { dimensions: [
    { key: "filesystem", effective: "workspace_read" },
    { key: "command", effective: "deny" },
    { key: "network", effective: "allowlisted" },
    { key: "persistence", effective: "once" },
  ] };
  const draft = domain.draftFromOverview(overview);
  assert.deepEqual(draft, { filesystem: "workspace_read", command: "deny", network: "allowlisted", persistence: "once", scopes: [] });
  assert.deepEqual(domain.changesFromDraft(draft, draft), []);
  assert.deepEqual(domain.changesFromDraft({ ...draft, network: "deny" }, draft), ["network"]);
  assert.equal(domain.draftFromOverview({}).filesystem, "");
});

test("controller preserves server effective network value until the user edits it", async () => {
  const window = { OwoPermissionsDomain: domain, setTimeout, clearTimeout };
  runInNewContext(read("permissions/permissions.controller.js"), { window, Date, Map, Set, Promise, JSON });
  const controller = window.OwoPermissionsController.create({ domain, api: { overview: async () => ({
    profile: "workspace", spec: null,
    dimensions: ["filesystem", "command", "network", "persistence"].map((key, i) =>
      ({ key, effective: ["workspace_write", "allowlisted", "allowlisted", "task"][i] })),
  }) } });
  await controller.load();
  assert.equal(controller.snapshot().draft.network, "allowlisted");
  assert.equal(controller.snapshot().changedFields.length, 0);
  controller.setDimension("network", "deny");
  assert.deepEqual(Array.from(controller.snapshot().changedFields), ["network"]);
  controller.dispose();
});

test("tool names and old browser full/auto preferences cannot auto approve", () => {
  const source = read("app-domain.js");
  const start = source.indexOf("function showApproval(payload)");
  const end = source.indexOf("\n// 自动放行记录", start);
  assert.ok(start >= 0 && end > start);
  let rendered = 0;
  const approvals = new Map(), sends = [];
  const sandbox = { state: { sessionId: "s", pendingApprovals: approvals }, writeTargetSid: "s",
    Date, getAccessMode: () => "full", SAFE_TOOL_PATTERN: /read|diff|view/,
    queueAutoAllowed: tool => sends.push(tool), respondApproval: id => sends.push(id),
    renderApprovals: () => rendered++ };
  runInNewContext(source.slice(start, end) + "\nthis.show=showApproval;", sandbox);
  for (const tool of ["apply_diff", "preview_write", "spreadsheet", "find_and_replace", "read_file"]) {
    sandbox.show({ request_id: tool, tool, level: tool === "read_file" ? "read" : "write" });
  }
  assert.equal(approvals.size, 5);
  assert.equal(rendered, 5);
  assert.deepEqual(sends, []);
});

test("host telemetry supplies last model; missing metrics remain unknown", () => {
  const context = {};
  runInNewContext(read("core/completion-record.js"), context);
  const api = context.OwoCompletionRecord;
  assert.equal(api.fromTurnStats({ model: "picker-default" }).lastExecutionModel, "");
  assert.equal(api.fromTurnStats({ model_calls: [{ model: "first" }, { model: "actual" }] }).lastExecutionModel, "actual");
  assert.equal(api.fromTurnStats({ completion_status: "accepted" }).lastCompletionRecord, null);
});

test("answer completion and host acceptance are different visible states", () => {
  const window = {};
  runInNewContext(read("views/status-bar.view.js"), { window });
  const facts = value => window.OwoStatusBar.computeFacts(value, null);
  assert.equal(facts({ lastTurnOutcome: "completed" })[4].text, "回答完成 · 未验证");
  assert.equal(facts({ lastTurnOutcome: "completed", lastCompletionStatus: "accepted" })[4].text, "回答完成 · 未验证");
  assert.equal(facts({ lastTurnOutcome: "completed", lastCompletionRecord: { status: "accepted", evidence_receipt_ids: ["receipt"] } })[4].text, "验收通过");
  const selected = facts({ selectedModel: "next", lastExecutionModel: "previous", settings: { model: "default" } })[2];
  assert.equal(selected.text, "next");
  assert.match(selected.secondary, /previous/);
  assert.match(selected.detail, /default/);
  assert.match(facts({ selectedModel: "next" })[2].secondary, /未知/);
});

test("usage formatting distinguishes missing usage from a measured zero", () => {
  const source = read("app.js");
  const start = source.indexOf("function formatCount(value)");
  const end = source.indexOf("\nasync function refreshUsage()", start);
  const context = {};
  runInNewContext(source.slice(start, end) + "\nthis.format=formatCount;", context);
  assert.equal(context.format(null), "未知");
  assert.equal(context.format(undefined), "未知");
  assert.equal(context.format("bad"), "未知");
  assert.equal(context.format(0), "0");
  assert.equal(context.format(125), "125");
});

test("wizard stages retain all seven original sections in three steps", () => {
  const launcher = require("../panels/project-launcher.panel.js")._test;
  assert.ok(launcher);
  assert.deepEqual(Array.from({ length: 7 }, (_, i) => launcher.wizardStage(i)), [1, 2, 2, 2, 2, 3, 3]);
  const html = launcher.resultHtml({ team_id: "created", strategy_decision: { mode: "single", reasons: ["coupled <steps>"] } });
  assert.match(html, /single/);
  assert.match(html, /coupled &lt;steps&gt;/);
});

test("absent provider usage cannot be rendered as an aggregate zero", () => {
  const context = {};
  runInNewContext(read("core/completion-record.js"), context);
  const api = context.OwoCompletionRecord;
  const unknown = api.usageFromTurnStats({ total_tokens: 0, model_calls: [{ usage: null }] });
  assert.equal(unknown.text, "用量未知");
  const partial = api.usageFromTurnStats({ model_calls: [{ usage: { total_tokens: 0 } }, { usage: null }] });
  assert.equal(partial.known, 1);
  assert.equal(partial.total, 2);
  assert.match(partial.text, /0 Tokens/);
  assert.match(partial.text, /1\/2/);
});
test("a default zero cost with missing prices is unknown, not free", () => {
  const source = read("app.js"), start = source.indexOf("function formatUsageCost(data)"), end = source.indexOf("\nasync function refreshUsage()", start);
  const context = {};
  runInNewContext(source.slice(start, end) + "\nthis.format=formatUsageCost;", context);
  assert.match(context.format({ cost_usd: 0, budget: { input_price_per_mtok: 0, output_price_per_mtok: 0 } }), /未知/);
  assert.equal(context.format({ cost_usd: 0, budget: { input_price_per_mtok: 1, output_price_per_mtok: 1 } }), "$0.0000");
});
