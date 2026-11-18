import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/capabilities.panel.js", import.meta.url), "utf8");

function harness() {
  const window = { OwoPanels: {}, OwoApi: { get: async () => ({ capabilities: [] }) } };
  const document = { body: { classList: { contains: () => false } } };
  vm.runInNewContext(source, { window, document, globalThis: window, Promise, String }, { filename: "capabilities.panel.js" });
  const root = () => {
    const nodes = new Map();
    return {
      nodes,
      innerHTML: "",
      isConnected: true,
      querySelector(selector) {
        if (!nodes.has(selector)) nodes.set(selector, {
          innerHTML: "",
          textContent: "",
          hidden: false,
          listeners: {},
          addEventListener(type, callback) { this.listeners[type] = callback; },
        });
        return nodes.get(selector);
      },
    };
  };
  return { panel: window.OwoPanels.capabilities, root };
}

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const flush = () => new Promise(resolve => setTimeout(resolve, 0));
const catalog = name => ({
  count: 1,
  maturity: { stable: 1 },
  capabilities: [{ user_name: name, maturity: "stable", summary: "ok" }],
});

test("maturityDonut draws proportional arcs and legend counts", () => {
  const { panel } = harness();
  const html = panel._test.maturityDonut({ stable: 0, beta: 10, experimental: 2 }, 12);
  assert.match(html, /<svg viewBox="0 0 42 42"/);
  // beta 10/12 ≈ 83.3% 弧长。
  assert.match(html, /stroke="var\(--accent\)"/);
  assert.match(html, /试用 10/);
  assert.match(html, /实验 2/);
  assert.doesNotMatch(html, /稳定 0/); // 计数为 0 的分段不进图例。
  assert.match(html, /<text [^>]*>12<\/text>/); // 总数在环心。
  // 空数据不渲染。
  assert.equal(panel._test.maturityDonut({}, 0), "");
});

test("effectChip and kindMark render typed badges", () => {
  const { panel } = harness();
  assert.match(panel._test.effectChip("read"), /owo-cap-eff-read/);
  assert.match(panel._test.effectChip("execute"), /执行命令/);
  assert.match(panel._test.kindMark("ui"), /owo-cap-kind-ui/);
  assert.match(panel._test.kindMark("http"), /HTTP/);
  assert.match(panel._test.kindMark("cli"), /命令行/);
  // 未知 kind 不崩，原样显示。
  assert.match(panel._test.kindMark("other"), /other/);
});

test("unknown maturity labels are escaped before rendering into capability badges", () => {
  const { panel } = harness();
  const badge = panel._test.maturityBadge('<img src=x onerror=alert(1)>');
  assert.match(badge, /&lt;img/);
  assert.doesNotMatch(badge, /<img/);
});

test("latest capability refresh wins when responses arrive out of order", async () => {
  const { panel, root } = harness();
  const view = root();
  const first = deferred();
  const second = deferred();
  let requests = 0;
  panel.mount(view, { get: () => (++requests === 1 ? first.promise : second.promise) });
  panel.refresh(view);
  second.resolve(catalog("new result"));
  await flush();
  first.resolve(catalog("stale result"));
  await flush();
  assert.match(view.querySelector(".owo-cap-list").innerHTML, /new result/);
  assert.doesNotMatch(view.querySelector(".owo-cap-list").innerHTML, /stale result/);
});

test("disposing a capability page prevents a late request from painting detached content", async () => {
  const { panel, root } = harness();
  const view = root();
  const pending = deferred();
  panel.mount(view, { get: () => pending.promise });
  panel.dispose();
  view.isConnected = false;
  pending.resolve(catalog("late content"));
  await flush();
  assert.equal(view.querySelector(".owo-cap-list").innerHTML, "");
});

test("initial load failure replaces the loading label with a retryable error state", async () => {
  const { panel, root } = harness();
  const view = root();
  panel.mount(view, {
    get: async () => { throw new Error("offline"); },
    friendlyError: error => error.message,
  });
  await flush();
  assert.match(view.querySelector(".owo-cap-meta").textContent, /加载失败，可重试/);
  assert.equal(view.querySelector(".owo-cap-error").hidden, false);
  assert.match(view.querySelector(".owo-cap-error").textContent, /offline/);
});
