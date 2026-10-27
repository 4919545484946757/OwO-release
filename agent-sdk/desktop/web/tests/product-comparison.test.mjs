
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { normalizeConfig, summarize, createController, reportHtml } = require("../panels/product-comparison.panel.js");
const flush = () => new Promise(r => setImmediate(r));
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const run = (id, status, report = null) => ({ run_id: id, status, execution: "live", report });
function fixture(api) {
  let n = 0; const timers = new Map();
  let visible = true;
  const controller = createController(api, { setTimeout: fn => { timers.set(++n, fn); return n; }, clearTimeout: id => timers.delete(id), visible: () => visible });
  return { controller, timers, visible: value => { visible = value; }, tick: async () => {
    const fns = [...timers.values()]; timers.clear(); for (const fn of fns) await fn(); await flush();
  } };
}
const config = { execution: "live", repetitions: 2, category: "", only: " task " };
function report() {
  const cell = (mode, status, wall, tokens, tools = null) => ({ key: { case_id: "blog", repetition: 0, agent_mode: mode }, model: "same-model", status, wall_ms: wall, total_tokens: tokens, model_calls: 2, tool_calls: tools, checker_passed: status === "passed" ? 10 : 8, checker_total: 10, artifact_refs: [], failed_steps: [] });
  return { execution: "live", model: "same-model", suite_hash: "suite", run_contract_sha256: "contract", evaluator_binary_sha256: "binary", provider_endpoint_sha256: "endpoint", pending: [], runs: [cell("single", "passed", 100, 1000), cell("multi", "failed", 200, 5000)] };
}

test("request config forces both host modes and validates repetitions/classification", () => {
  assert.deepEqual(normalizeConfig(config), { suite: "v1", execution: "live", modes: ["single", "workswarm"], repetitions: 2, category: null, only: "task" });
  for (const repetitions of [0, 21, 1.1, NaN]) assert.throws(() => normalizeConfig({ ...config, repetitions }));
  assert.throws(() => normalizeConfig({ ...config, execution: "mock" }));
  assert.throws(() => normalizeConfig({ ...config, category: "../file" }));
});
test("report includes failed attempts, paired cells, wall/token ratios, exact tool counters and checker counts", () => {
  const result = summarize(report());
  assert.equal(result.comparable, true); assert.equal(result.paired, 1);
  assert.equal(result.team.attempted, 1); assert.equal(result.team.passed, 0);
  assert.equal(result.team.checkerPassed, 8); assert.equal(result.speedRatio, 2); assert.equal(result.tokenRatio, 5);
  assert.equal(result.team.toolCalls, null);
});
test("exact tool counts are compared without inferring zero from an empty legacy log", () => {
  const data = report(); data.runs[0].tool_calls = 4; data.runs[1].tool_calls = 8;
  const result = summarize(data);
  assert.equal(result.single.toolCalls, 4); assert.equal(result.team.toolCalls, 8);
  assert.equal(result.toolRatio, 2);
  const html = reportHtml({ status: "completed", execution: "live", report: data });
  assert.match(html, /工具调用/); assert.match(html, /工具调用：2\.00 倍/);
  delete data.runs[0].tool_calls; data.runs[0].tool_log = [];
  assert.equal(summarize(data).single.toolCalls, null);
});
test("reference, missing fingerprints, mismatched models and incomplete pairs never claim comparability", () => {
  const cases = [
    r => { r.execution = "dry"; },
    r => { delete r.run_contract_sha256; },
    r => { r.runs[1].model = "other"; },
    r => { r.pending = [{}]; },
    r => { r.runs.pop(); },
    r => { r.runs.push(r.runs[1]); },
    r => { r.runs[1].key.repetition = 1; },
    r => { r.runs[1].key.case_id = ""; },
  ];
  for (const mutate of cases) {
    const data = report(); mutate(data); const result = summarize(data);
    assert.equal(result.comparable, false); assert.equal(result.speedRatio, null); assert.equal(result.tokenRatio, null);
  }
});
test("unknown token/call/checker metrics remain unknown and do not get coerced to zero", () => {
  const data = report(); data.runs[1].total_tokens = null; delete data.runs[1].checker_total; delete data.runs[1].model_calls;
  const result = summarize(data); assert.equal(result.team.totalTokens, null);
  assert.equal(result.team.checkerTotal, null); assert.equal(result.team.modelCalls, null); assert.equal(result.tokenRatio, null);
});
test("start submission is single-flight and uses exact registered host payload", async () => {
  const pending = deferred(), posts = [];
  const f = fixture({ post: (path, body) => { posts.push({ path, body }); return pending.promise; },
    get: async path => path.endsWith("/r1") ? run("r1", "completed") : { runs: [] } });
  const first = f.controller.start(config); await flush();
  await f.controller.start(config); assert.equal(posts.length, 1);
  pending.resolve(run("r1", "queued")); await first;
  assert.equal(f.controller.state.runId, "r1"); assert.equal(f.controller.state.submitting, false);
  assert.equal(f.timers.size, 0); assert.deepEqual(posts[0].body.modes, ["single", "workswarm"]); f.controller.dispose();
});
test("switching selections discards late results even when transport ignores abort", async () => {
  const a = deferred(), b = deferred();
  const f = fixture({ get: path => path.endsWith("/a") ? a.promise : b.promise });
  const first = f.controller.select("a"), second = f.controller.select("b");
  b.resolve(run("b", "completed")); await second; a.resolve(run("a", "running")); await first;
  assert.equal(f.controller.state.detail.run_id, "b"); assert.equal(f.timers.size, 0); f.controller.dispose();
});
test("polling has one timer, pauses in hidden view and stops at terminal", async () => {
  let reads = 0;
  const f = fixture({ get: async () => run("r", ++reads === 1 ? "running" : "completed") });
  await f.controller.select("r"); assert.equal(f.timers.size, 1);
  f.visible(false); await f.tick(); assert.equal(reads, 1); assert.equal(f.timers.size, 1);
  f.visible(true); await f.tick(); assert.equal(reads, 2); assert.equal(f.timers.size, 0); f.controller.dispose();
});
test("initial read failure retains selected identity and retries without accepting a result", async () => {
  let reads = 0;
  const f = fixture({ get: async () => { if (++reads === 1) throw Error("offline"); return run("r", "completed"); } });
  await f.controller.select("r"); assert.equal(f.controller.state.detail, null); assert.equal(f.timers.size, 1);
  await f.tick(); assert.equal(f.controller.state.detail.status, "completed"); assert.equal(f.controller.state.error, ""); f.controller.dispose();
});
test("cancellation fences outstanding reads so late running status cannot replace cancelled", async () => {
  const late = deferred(); let reads = 0, posts = 0;
  const f = fixture({ get: async () => ++reads === 1 ? run("r", "running") : late.promise,
    post: async path => { posts++; assert.equal(path, "/product-eval/runs/r/cancel"); return run("r", "cancelled"); } });
  await f.controller.select("r"); const reading = f.controller.refresh(); await flush();
  await f.controller.cancel(); late.resolve(run("r", "running")); await reading;
  assert.equal(f.controller.state.detail.status, "cancelled"); assert.equal(posts, 1); assert.equal(f.timers.size, 0);
  await f.controller.cancel(); assert.equal(posts, 1); f.controller.dispose();
});
test("dispose aborts the read and prevents polling or a late callback from mutating view", async () => {
  const pending = deferred(); let signal;
  const f = fixture({ get: (path, options) => { signal = options.signal; return pending.promise; } });
  const reading = f.controller.select("r"); f.controller.dispose();
  assert.equal(signal.aborted, true); pending.resolve(run("r", "running")); await reading;
  assert.equal(f.controller.state.detail, null); assert.equal(f.timers.size, 0);
});
test("report escapes errors, task names, artifacts and model text", () => {
  const data = run("r", "failed", report()); data.error = "<script>fail</script>";
  data.report.runs[1].key.case_id = "<img src=x onerror=alert(1)>";
  data.report.runs[1].artifact_refs = ["<script>artifact</script>"];
  const html = reportHtml(data);
  assert.ok(!html.includes("<script>")); assert.ok(!html.includes("<img src=x"));
  assert.ok(html.includes("&lt;script&gt;")); assert.ok(html.includes("第 1 轮"));
});

test("selected terminal detail updates the corresponding history status", async () => {
  let reads = 0;
  const f = fixture({ get: async path => path === "/product-eval/runs"
    ? { runs: [run("r", "running")] } : run("r", ++reads === 1 ? "running" : "completed") });
  await f.controller.refreshList(); await f.controller.select("r");
  await f.tick(); assert.equal(f.controller.state.runs[0].status, "completed");
  f.controller.dispose();
});

test("large report bounds initial sample DOM while summaries retain every failed attempt", () => {
  const data = report();
  data.runs = Array.from({ length: 120 }, (_, index) => ({ ...data.runs[index % 2],
    key: { case_id: "case-" + Math.floor(index / 2), repetition: 0, agent_mode: index % 2 ? "multi" : "single" } }));
  const html = reportHtml(run("r", "completed", data));
  assert.equal((html.match(/class="owo-pe-sample"/g) || []).length, 50);
  assert.ok(html.includes("显示 50 / 120")); assert.ok(html.includes("再显示 50 条"));
  assert.equal(summarize(data).team.attempted, 60); assert.equal(summarize(data).team.passed, 0);
});

test("renders the authoritative shared Team enablement verdict and each guardrail", () => {
  const data = report();
  data.comparison = {
    enabled: false, sample_sufficient: false,
    alignment_guardrails: [{ name: "任务配对完整", satisfied: false, detail: "仍有 1 个待执行矩阵单元" }],
    quality_guardrails: [{ name: "检查器质量不退化", satisfied: false, detail: "Single=0.8，Team=0.7" }],
    resource_guardrails: [{ name: "Token ≤ 1.50x", satisfied: false, detail: "Team = 5.0x" }],
    rules: [{ name: "成功率 +5%", satisfied: false, detail: "低于门槛" }],
  };
  const html = reportHtml(run("r", "completed", data));
  assert.match(html, /暂不自动启用 Team/);
  assert.match(html, /样本量不足/);
  assert.match(html, /任务配对完整：仍有 1 个待执行矩阵单元/);
  assert.match(html, /检查器质量不退化：Single=0\.8，Team=0\.7/);
  assert.match(html, /Token ≤ 1\.50x：Team = 5\.0x/);
  assert.match(html, /成功率 \+5%：低于门槛/);
  data.comparison.enabled = true;
  assert.match(reportHtml(run("r", "completed", data)), /报告启用标志：开启（未满足本页的比较前提）/);
  assert.doesNotMatch(reportHtml(run("r", "completed", data)), /判定：建议启用 Team/);
  data.comparison.sample_sufficient = true;
  for (const key of ["alignment_guardrails", "quality_guardrails", "resource_guardrails", "rules"]) data.comparison[key].forEach(rule => rule.satisfied = true);
  assert.match(reportHtml(run("r", "completed", data)), /判定：建议启用 Team/);
  data.execution = "reference";
  assert.doesNotMatch(reportHtml(run("r", "completed", data)), /判定：建议启用 Team/);
});


test("malformed historical responses do not replace an already loaded list with an empty state", async () => {
  let reads = 0;
  const f = fixture({ get: async () => ++reads === 1 ? {runs:[run('kept','completed')]} : {runs:{unexpected:true}} });
  await f.controller.refreshList(); await f.controller.refreshList();
  assert.equal(f.controller.state.runs[0].run_id, 'kept');
  assert.match(f.controller.state.error, /历史运行列表无效/); f.controller.dispose();
});

test("malformed samples and rules render diagnostics without throwing or inventing successful modes", () => {
  const data = report(); data.runs = [null, {key:{agent_mode:'constructor',case_id:'bad',repetition:0}, status:'passed'}];
  data.comparison = {enabled:true, sample_sufficient:true, rules:{bad:true}, quality_guardrails:[null]};
  const summary = summarize(data); assert.equal(summary.invalid, 2); assert.equal(summary.comparable, false);
  const html = reportHtml(run('bad','completed',data));
  assert.match(html,/身份未知|模式未知/); assert.doesNotMatch(html,/判定：建议启用 Team/);
});
