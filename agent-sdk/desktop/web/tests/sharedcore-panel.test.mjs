import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/sharedcore.panel.js", import.meta.url), "utf8");

const esc = value =>
  String(value == null ? "" : value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

function harness() {
  const window = { OwoPanels: {}, OwoApi: { stream: async () => ({ ok: true, json: async () => ({}) }) } };
  const document = { createElement: () => ({ textContent: "", innerHTML: "" }) };
  vm.runInNewContext(source, { window, document, globalThis: window, Promise, String, Boolean, Object, navigator: {} }, {
    filename: "sharedcore.panel.js",
  });
  return { panel: window.OwoPanels.sharedcore };
}

function fakeRoot() {
  const nodes = new Map();
  return {
    innerHTML: "",
    listeners: {},
    addEventListener(type, callback) {
      this.listeners[type] = callback;
    },
    querySelector(selector) {
      if (!nodes.has(selector)) nodes.set(selector, { innerHTML: "", textContent: "" });
      return nodes.get(selector);
    },
  };
}

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

const state = {
  ok: true,
  exists: true,
  project: {
    name: "demo",
    goal: "并行开发记账应用",
    core_version: 2,
    context_revision: 7,
    baseline_commit: "abc123",
    rules: ["接口先定契约", "禁止越出写范围"],
    architecture: { backend: "Rust", frontend: "TS" },
  },
  tasks: {
    "T-001": { id: "T-001", title: "契约", status: "done", write_scope: ["contracts/"] },
    "T-002": { id: "T-002", title: "后端", status: "ready", owner: "dev-a", depends_on: ["T-001"], write_scope: ["src/backend/"] },
    "T-003": { id: "T-003", title: "前端", status: "ready", depends_on: ["T-001"], write_scope: ["src/frontend/"] },
  },
  plan: { batches: [["T-002", "T-003"]], ready: ["T-002", "T-003"], blocked: { "T-004": ["T-002"] }, conflicts: [] },
  contracts: { "auth-api": { version: 2, status: "accepted" } },
  events: [{ type: "task.completed", resource: "tasks/T-001", ts: "2026-11-18T00:00:00Z" }],
};

test("paint renders project, batches, tasks and contracts", async () => {
  const { panel } = harness();
  const root = fakeRoot();
  await panel.mount(root, { get: async () => state, esc });
  await flush();
  const html = root.querySelector(".owo-sc-body").innerHTML;
  assert.match(html, /demo/);
  assert.match(html, /接口先定契约/);
  assert.match(html, /并行计划/);
  assert.match(html, /批次 1/);
  assert.match(html, /T-002/);
  assert.match(html, /T-004/);
  assert.match(html, /阻塞/);
  assert.match(html, /auth-api/);
  assert.match(html, /task\.completed/);
});

test("empty state points to sharedcore_init", async () => {
  const { panel } = harness();
  const root = fakeRoot();
  await panel.mount(root, { get: async () => ({ ok: true, exists: false, dir: ".sharedcore" }), esc });
  await flush();
  const html = root.querySelector(".owo-sc-body").innerHTML;
  assert.match(html, /sharedcore_init/);
  assert.match(html, /\.sharedcore/);
});

test("renderDag draws layered SVG nodes with dependency edges", () => {
  const { panel } = harness();
  const html = panel._test.renderDag({
    "T-001": { id: "T-001", title: "契约", status: "done", depends_on: [] },
    "T-002": { id: "T-002", title: "后端", status: "running", depends_on: ["T-001"] },
    "T-003": { id: "T-003", title: "前端", status: "ready", depends_on: ["T-001"] },
    "T-004": { id: "T-004", title: "文档", status: "blocked", depends_on: ["T-002", "T-003"] },
  }, { batches: [] });
  assert.match(html, /<svg class="owo-sc-dag"/);
  assert.match(html, /任务图/);
  // 4 个节点 + 4 条依赖连线（T-002/T-003 各依赖 T-001，T-004 依赖两者）。
  assert.equal((html.match(/owo-sc-dag-node/g) || []).length, 4);
  assert.equal((html.match(/marker-end="url\(#owo-sc-arrow\)"/g) || []).length, 4);
  // 状态配色：完成绿 / 进行中主色 / 阻塞虚线。
  assert.match(html, /fill="var\(--green-soft\)"/);
  assert.match(html, /stroke="var\(--accent\)"/);
  assert.match(html, /stroke-dasharray="4 3"/);
  // 分层：T-001 在第 0 列，T-004 在第 2 列。
  assert.match(html, /viewBox="0 0 /);
});

test("renderDag handles cycles without hanging", () => {
  const { panel } = harness();
  const html = panel._test.renderDag({
    "T-001": { id: "T-001", title: "A", status: "ready", depends_on: ["T-002"] },
    "T-002": { id: "T-002", title: "B", status: "ready", depends_on: ["T-001"] },
  }, { batches: [] });
  assert.match(html, /<svg class="owo-sc-dag"/);
  assert.equal((html.match(/owo-sc-dag-node/g) || []).length, 2);
});

test("renderDag returns empty without tasks", () => {
  const { panel } = harness();
  assert.equal(panel._test.renderDag({}, {}), "");
});

test("mount fetches state and fills the body", async () => {
  const { panel } = harness();
  const root = fakeRoot();
  await panel.mount(root, { get: async () => state, esc });
  await flush();
  assert.match(root.querySelector(".owo-sc-body").innerHTML, /并行计划/);
  assert.match(root.querySelector(".owo-sc-meta").textContent, /并行批次 1/);
});
