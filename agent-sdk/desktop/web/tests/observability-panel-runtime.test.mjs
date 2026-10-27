import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/observability.panel.js", import.meta.url), "utf8");

function harness(helpers, windowOptions = {}) {
  const elements = new Map();
  function element() {
    return {
      innerHTML: "", textContent: "", style: {}, disabled: false, handlers: {},
      addEventListener(name, callback) { this.handlers[name] = callback; },
      setAttribute(name, value) { this[name] = value; },
      removeAttribute(name) { delete this[name]; },
      querySelector(selector) { return get(selector.replace(/^#/, "")); },
    };
  }
  function get(key) {
    if (!elements.has(key)) elements.set(key, element());
    return elements.get(key);
  }
  const section = element();
  section.querySelector = selector => get(selector.replace(/^#/, ""));
  const root = {
    set innerHTML(_) {},
    querySelector(selector) { return selector === '[data-panel="observability"]' ? section : get(selector.replace(/^#/, "")); },
  };
  const document = {
    getElementById: get,
    querySelector(selector) { return get(selector.replace(/^#/, "")); },
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" }, ...windowOptions };
  runInNewContext(source, { window, document, Promise, String, JSON, Number, Math, Object, isFinite });
  const panel = window.OwoPanels.observability;
  panel.mount(root, helpers);
  return { panel, elements, section, get };
}

function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

function dataFor(path) {
  if (path === "/metrics/overview") return { traces_count: 1, avg_turn_ms: 2, updated_at: "2030-01-01T00:00:00Z" };
  if (path === "/metrics/turns?limit=50") return { turns: [] };
  if (path === "/metrics/tools") return { tools: [] };
  if (path === "/metrics/health") return { components: {} };
  if (path === "/metrics/runtime") return {};
  if (path === "/metrics/slo") return { slo: [] };
  if (path === "/usage/summary") return { dimensions: [] };
  if (path === "/metrics/slo/alerts") return { rules: [], alerts: [] };
  if (path === "/metrics/telemetry/status") return { enabled: false };
  return {};
}

test("observability refresh uses only the latest response and invalidates work on disposal", async () => {
  const overview = [];
  const h = harness({
    get(path) {
      if (path === "/metrics/overview") {
        const request = deferred();
        overview.push(request);
        return request.promise;
      }
      return Promise.resolve(dataFor(path));
    },
    esc: value => String(value), friendlyError: error => String(error),
  });
  const first = h.panel.refresh();
  const second = h.panel.refresh();
  overview[2].resolve({ traces_count: 22, avg_turn_ms: 2 });
  await second;
  overview[1].resolve({ traces_count: 11, avg_turn_ms: 1 });
  await first;
  assert.match(h.get("owo-mtr-cards").innerHTML, />22</);
  assert.doesNotMatch(h.get("owo-mtr-cards").innerHTML, />11</);

  const afterDispose = h.panel.refresh();
  h.panel.dispose();
  overview[3].resolve({ traces_count: 33, avg_turn_ms: 3 });
  await afterDispose;
  assert.match(h.get("owo-mtr-cards").innerHTML, />22</);
});

test("observability panel initializes default API helpers and reports HTTP failures", async () => {
  const paths = [];
  const h = harness(undefined, {
    OwoApi: {
      async stream(path) {
        paths.push(path);
        return { ok: false, status: 503, json: async () => ({ message: "metrics unavailable" }) };
      },
    },
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(paths.includes("/metrics/overview"));
  assert.match(h.get("owo-mtr-cards").textContent, /metrics unavailable/);
});

test("observability report requests are fenced when the page is disposed", async () => {
  const pending = deferred();
  const h = harness({
    get(path) { return path.includes("/report?") ? pending.promise : Promise.resolve(dataFor(path)); },
    esc: value => String(value), friendlyError: error => String(error),
  });
  const report = h.panel._test.loadReport();
  h.panel.dispose();
  pending.resolve({ period_days: 7, slo: [{ name: "late report" }] });
  await report;
  assert.doesNotMatch(h.get("owo-mtr-report").innerHTML, /late report/);
});


test("observability weekly report keeps an inline retry action after request failure", async () => {
  let reportCalls = 0;
  const h = harness({
    get(path) {
      if (path.includes("/report?")) {
        reportCalls += 1;
        return reportCalls === 1
          ? Promise.reject(new Error("report service offline"))
          : Promise.resolve({ period_days: 7, slo: [{ name: "availability", samples: 10, achieving: true }] });
      }
      return Promise.resolve(dataFor(path));
    },
    esc: value => String(value), friendlyError: error => String(error),
  });

  await h.panel._test.loadReport();
  assert.match(h.get("owo-mtr-report").innerHTML, /report service offline/);
  const retry = h.get("owo-mtr-report-refresh");
  assert.equal(retry.textContent, "重试");
  assert.equal(typeof retry.handlers.click, "function");
  await retry.handlers.click();
  assert.equal(reportCalls, 2);
  assert.match(h.get("owo-mtr-report").innerHTML, /availability/);
});

test("observability weekly report prevents duplicate requests while loading", async () => {
  const request = deferred();
  let reportCalls = 0;
  const h = harness({
    get(path) {
      if (path.includes("/report?")) { reportCalls += 1; return request.promise; }
      return Promise.resolve(dataFor(path));
    },
    esc: value => String(value), friendlyError: error => String(error),
  });

  const first = h.panel._test.loadReport();
  assert.match(h.get("owo-mtr-report").innerHTML, /正在加载周报/);
  assert.equal(await h.panel._test.loadReport(), null);
  assert.equal(reportCalls, 1);
  request.resolve({ period_days: 7, slo: [] });
  await first;
  assert.match(h.get("owo-mtr-report").innerHTML, /重新加载/);
});


test("observability metrics escape labels and reject malformed numeric payloads", async () => {
  const malicious = "<img src=x onerror=alert(1)>";
  const h = harness({
    get(path) {
      if (path === "/metrics/overview") return Promise.resolve({ traces_count: 1, avg_turn_ms: 2 });
      if (path === "/metrics/turns?limit=50") return Promise.resolve({ turns: [{ duration_ms: malicious }] });
      if (path === "/metrics/tools") return Promise.resolve({ tools: [{ tool: malicious, calls: malicious, failures: malicious, failure_rate: malicious }] });
      if (path === "/metrics/health") return Promise.resolve({ components: { plugins: { count: malicious }, notes: { count: 0 }, traces: { count: 0 } } });
      if (path === "/metrics/runtime") return Promise.resolve({ queue_depth: malicious, sse: { active_connections: malicious } });
      if (path === "/metrics/slo") return Promise.resolve({ slo: [{ name: malicious, target_ms: malicious, p95_ms: malicious, success_rate: malicious, samples: malicious, error_budget: { bad: malicious, allowed_bad: 0 } }] });
      if (path === "/usage/summary") return Promise.resolve({ count: malicious, dimensions: [{ dimension: malicious, calls: malicious, total_tokens: malicious, cost_usd: malicious }] });
      if (path === "/metrics/slo/alerts") return Promise.resolve({ count: malicious, rules: [{ name: malicious, slo_name: malicious, kind: "rate", threshold: malicious, consecutive: malicious, severity: malicious }] });
      if (path === "/metrics/telemetry/status") return Promise.resolve({ enabled: false, counters: { [malicious]: malicious }, error_codes: {}, performance: {} });
      if (path.includes("/metrics/slo/report?")) return Promise.resolve({ period_days: malicious, slo: [{ name: malicious, p95_ms: malicious, success_rate: malicious, samples: malicious, violations_in_window: malicious }] });
      return Promise.resolve({});
    },
    esc: value => String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\"/g, "&quot;"),
    friendlyError: error => String(error),
  });

  await h.panel.refresh();
  await h.panel._test.loadReport();
  const rendered = [
    "owo-mtr-chart", "owo-mtr-tools", "owo-mtr-health", "owo-mtr-runtime",
    "owo-mtr-slo", "owo-mtr-usage", "owo-mtr-alerts", "owo-mtr-telemetry", "owo-mtr-report",
  ].map(id => h.get(id).innerHTML).join("\n");
  assert.doesNotMatch(rendered, /<img|<script|<svg/);
  assert.match(rendered, /&lt;img/);
  assert.match(rendered, /—/);
  assert.doesNotMatch(rendered, /NaN/);
});

test("unknown price never reports zero estimated cost and missing telemetry is an error", async () => {
  const h=harness({get:async path=>path==="/usage/summary"?{count:1,price_per_mtok:0,dimensions:[{dimension:"session",calls:1,tokens:2,cost_usd:0}]}:path==="/metrics/telemetry/status"?{}:dataFor(path),esc:String,friendlyError:String});
  await h.panel.refresh();
  assert.match(h.get("owo-mtr-usage").innerHTML,/未配置有效单价/);
  assert.match(h.get("owo-mtr-usage").innerHTML,/—/);
  assert.match(h.get("owo-mtr-telemetry").textContent,/遥测开关状态未知/);
});

test("overview failure updates its status and retry recovers without an error color", async () => {
  let broken=true;
  const h=harness({get:async path=>path==="/metrics/overview"&&broken?{}:dataFor(path),esc:String,friendlyError:String});
  await h.panel.refresh();
  assert.match(h.get("owo-mtr-updated").textContent,/概览读取失败/);
  broken=false;await h.panel.refresh();
  assert.match(h.get("owo-mtr-cards").innerHTML,/>1</);
  assert.equal(h.get("owo-mtr-cards").style.color,"");
  assert.doesNotMatch(h.get("owo-mtr-updated").textContent,/读取失败/);
});

test("insight tabs maintain one selected tab and one visible pane", () => {
  const h=harness({get:async path=>dataFor(path),esc:String,friendlyError:String});
  h.panel._test.selectTab("quality");
  for(const name of ["overview","usage","quality","telemetry"]) {
    assert.equal(h.get("owo-mtr-tab-"+name)["aria-selected"],String(name==="quality"));
    assert.equal(h.get("owo-mtr-pane-"+name).hidden,name!=="quality");
  }
});
