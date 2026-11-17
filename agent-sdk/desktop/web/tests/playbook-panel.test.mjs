import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/playbook.panel.js", import.meta.url), "utf8");

const esc = value =>
  String(value == null ? "" : value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

function harness() {
  const window = { OwoPanels: {}, OwoApi: { stream: async () => ({ ok: true, json: async () => ({}) }) } };
  const document = { createElement: () => ({ textContent: "", innerHTML: "" }) };
  vm.runInNewContext(source, { window, document, globalThis: window, Promise, String, Boolean, navigator: {} }, {
    filename: "playbook.panel.js",
  });
  return { panel: window.OwoPanels.playbook };
}

function fakeRoot() {
  const nodes = new Map();
  return {
    innerHTML: "",
    isConnected: true,
    listeners: {},
    addEventListener(type, callback) {
      this.listeners[type] = callback;
    },
    querySelector(selector) {
      if (!nodes.has(selector)) {
        nodes.set(selector, {
          innerHTML: "",
          textContent: "",
          hidden: false,
          listeners: {},
          addEventListener(type, callback) {
            this.listeners[type] = callback;
          },
          classList: { toggle() {} },
        });
      }
      return nodes.get(selector);
    },
  };
}

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

const listPayload = {
  dir: "playbooks",
  count: 1,
  playbooks: [
    {
      id: "code-review",
      name: "代码评审",
      description: "结构化评审",
      path: "playbooks/code-review.json",
      steps: 5,
      ok: true,
      issues: [],
      warnings: 2,
      progress: { total: 5, passed: 1, failed: 0, skipped: 0, pending: 4, blocked: 0, percent: 20, summary: "1/5（20%）" },
      next: "build",
    },
  ],
};

test("mark() maps step states to glyphs", () => {
  const { panel } = harness();
  assert.equal(panel._test.mark("passed"), "✔");
  assert.equal(panel._test.mark("failed"), "✘");
  assert.equal(panel._test.mark("skipped"), "⊘");
  assert.equal(panel._test.mark("pending"), "○");
});

test("bar() clamps percent into 0..100", () => {
  const { panel } = harness();
  assert.match(panel._test.bar({ percent: 150 }), /width:100%/);
  assert.match(panel._test.bar({ percent: -5 }), /width:0%/);
});

test("card() escapes names and surfaces warnings/next", () => {
  const { panel } = harness();
  const view = fakeRoot();
  panel.mount(view, { get: async () => listPayload, esc });
  const html = panel._test.card({
    ...listPayload.playbooks[0],
    name: '<img src=x onerror=alert(1)>',
  });
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /&lt;img/);
  assert.match(html, /高风险命令 2 条/);
  assert.match(html, /下一步：build/);
  assert.match(html, /结构化评审/);
  assert.match(html, /data-pb-archive="playbooks\/code-review\.json"/);
});

test("stepRow() renders warnings and hides actions for passed steps", () => {
  const { panel } = harness();
  const view = fakeRoot();
  panel.mount(view, { get: async () => listPayload, esc });
  const risky = panel._test.stepRow({
    id: "build",
    title: "编译",
    kind: "check",
    state: "pending",
    command: "rm -rf build",
    warnings: ["命令含高风险片段 `rm -rf`"],
    attempts: 0,
  });
  assert.match(risky, /⚠/);
  assert.match(risky, /data-pb-record="build"/);
  assert.match(risky, /data-pb-copy=/);

  const passed = panel._test.stepRow({ id: "scope", title: "范围", kind: "model", state: "passed", note: "ok" });
  assert.match(passed, /〇|✔/);
  assert.doesNotMatch(passed, /data-pb-record/);
});

test("compareRows() lists transitions and flags regressions", () => {
  const { panel } = harness();
  panel.mount(fakeRoot(), { get: async () => ({ playbooks: [] }), esc });
  const identical = panel._test.compareRows({ a: "passed" }, { steps: { a: "passed" } });
  assert.match(identical, /与当前状态一致/);

  const html = panel._test.compareRows(
    { a: "pending", b: "failed" },
    { steps: { a: "passed", b: "pending" } }
  );
  assert.match(html, /a<\/code><span>passed → pending/);
  assert.match(html, /退化/);
  assert.match(html, /b<\/code><span>pending → failed/);

  const escaped = panel._test.compareRows({ "<x>": "passed" }, { steps: { "<x>": "pending" } });
  assert.doesNotMatch(escaped, /<x>/);
  assert.match(escaped, /&lt;x&gt;/);

  // 状态未变、失败类型变化也要显示。
  const kinds = panel._test.compareRows(
    { a: "failed" },
    { steps: { a: "failed" }, step_kinds: { a: "timeout" } },
    { a: "exit_nonzero" }
  );
  assert.match(kinds, /失败类型 timeout → exit_nonzero/);
});

test("mount fetches the list and renders cards", async () => {
  const { panel } = harness();
  const view = fakeRoot();
  const calls = [];
  await panel.mount(view, {
    get: async path => {
      calls.push(path);
      return listPayload;
    },
    esc,
  });
  await flush();
  assert.match(view.innerHTML, /新建剧本/);
  assert.ok(calls.includes("/playbooks"));
  assert.match(view.querySelector(".owo-pb-list").innerHTML, /代码评审/);
  assert.match(view.querySelector(".owo-pb-meta").textContent, /共 1 个剧本/);
});

test("refresh failure surfaces a readable error", async () => {
  const { panel } = harness();
  const view = fakeRoot();
  await panel.mount(view, {
    get: async () => {
      throw new Error("boom");
    },
    esc,
  });
  await flush();
  const error = view.querySelector(".owo-pb-error");
  assert.equal(error.hidden, false);
  assert.match(error.textContent, /剧本列表加载失败/);
});
