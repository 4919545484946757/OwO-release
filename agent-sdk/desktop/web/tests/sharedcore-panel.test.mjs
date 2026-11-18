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

test("mount fetches state and fills the body", async () => {
  const { panel } = harness();
  const root = fakeRoot();
  await panel.mount(root, { get: async () => state, esc });
  await flush();
  assert.match(root.querySelector(".owo-sc-body").innerHTML, /并行计划/);
  assert.match(root.querySelector(".owo-sc-meta").textContent, /并行批次 1/);
});
