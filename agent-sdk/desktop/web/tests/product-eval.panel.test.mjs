import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/eval.panel.js", import.meta.url), "utf8");

function harness() {
  const nodes = new Map();
  const ensureNode = (selector) => {
    if (!nodes.has(selector)) {
      nodes.set(selector, {
        value: "",
        innerHTML: "",
        textContent: "",
        className: "",
        disabled: false,
        listeners: {},
        addEventListener(type, callback) { this.listeners[type] = callback; },
        querySelectorAll(selector) {
          if (selector !== ".owo-eval-item") return [];
          if (this._buttonsMarkup === this.innerHTML) return this._buttons;
          const buttons = [];
          const matcher = /<button[^>]*class="[^"]*owo-eval-item[^"]*"[^>]*data-file="([^"]*)"/g;
          let match;
          while ((match = matcher.exec(this.innerHTML))) {
            const file = match[1];
            buttons.push({
              listeners: {},
              getAttribute(name) { return name === "data-file" ? file : null; },
              addEventListener(type, callback) { this.listeners[type] = callback; },
            });
          }
          this._buttonsMarkup = this.innerHTML;
          this._buttons = buttons;
          return buttons;
        },
      });
    }
    return nodes.get(selector);
  };
  const section = {
    querySelector: ensureNode,
    querySelectorAll: () => [],
  };
  const root = {
    html: "",
    set innerHTML(value) { this.html = String(value); },
    get innerHTML() { return this.html; },
    querySelector: () => section,
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" } };
  const document = { contains: node => node === section };
  const context = { window, document, Promise, JSON, String, Number, Math, Date, Error, RegExp };
  vm.runInNewContext(source, context, { filename: "eval.panel.js" });
  return { panel: window.OwoPanels.eval, root, nodes, section };
}
const flush = () => new Promise(resolve => setTimeout(resolve, 0));

test("engineering eval panel registers as a lazy browser panel", () => {
  const { panel } = harness();
  assert.equal(panel.id, "eval");
  assert.equal(panel.title, "模型评测");
  assert.match(panel.nav(), /id="owo-eval-run"/);
  assert.match(panel.nav(), /id="owo-eval-list"/);
});

test("mount loads report history through injected transport and escapes untrusted report data", async () => {
  const { panel, root, nodes } = harness();
  const reads = [];
  panel.mount(root, {
    get: async path => {
      reads.push(path);
      return { reports: [{ file: "<img>", suite: "<script>", pass_rate: 0.5, passed: 1, total: 2, model: "<unsafe>" }] };
    },
    post: async () => ({}),
    esc: value => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;"),
    friendlyError: error => String(error),
  });
  await flush();
  assert.deepEqual(reads, ["/eval/gate/reports"]);
  assert.match(nodes.get("#owo-eval-list").innerHTML, /&lt;script&gt;/);
  assert.doesNotMatch(nodes.get("#owo-eval-list").innerHTML, /<script>/);
});

test("run action reports credential skips and restores the submit button", async () => {
  const { panel, root, nodes, section } = harness();
  const writes = [];
  const reads = [];
  panel.mount(root, {
    get: async path => { reads.push(path); return { reports: [] }; },
    post: async (path, body) => {
      writes.push({ path, body });
      return { skipped: true, reason: "未配置模型凭据" };
    },
    esc: value => String(value),
    friendlyError: error => String(error),
  });
  await flush();
  section.querySelector("#owo-eval-suite").value = "suite-v1";
  nodes.get("#owo-eval-run").listeners.click();
  await flush();
  assert.deepEqual(JSON.parse(JSON.stringify(writes)), [{ path: "/eval/gate/run", body: { suite: "suite-v1" } }]);
  assert.match(nodes.get("#owo-eval-status").textContent, /已跳过：未配置模型凭据/);
  assert.equal(nodes.get("#owo-eval-run").disabled, false);
  assert.equal(reads.filter(path => path === "/eval/gate/reports").length, 2);
});

test("history selection loads the clicked report instead of always loading latest", async () => {
  const { panel, root, nodes } = harness();
  const reads = [];
  panel.mount(root, {
    get: async path => {
      reads.push(path);
      if (path === "/eval/gate/reports") {
        return { reports: [
          { file: "20260202T000000Z.json", suite: "newer", pass_rate: 1, passed: 2, total: 2 },
          { file: "20260101T000000Z.json", suite: "older", pass_rate: 0.5, passed: 1, total: 2 },
        ] };
      }
      if (path.endsWith("20260101T000000Z.json")) {
        return { file: "20260101T000000Z.json", report: { suite: "older selected", pass_rate: 1, passed: 1, total: 1 } };
      }
      throw new Error("unexpected report request: " + path);
    },
    post: async () => ({}),
    esc: value => String(value),
    friendlyError: error => String(error),
  });
  await flush();
  const oldReport = nodes.get("#owo-eval-list").querySelectorAll(".owo-eval-item")[1];
  oldReport.listeners.click.call(oldReport);
  await flush();
  assert.equal(reads[1], "/eval/gate/report?file=20260101T000000Z.json");
  assert.match(nodes.get("#owo-eval-detail").innerHTML, /older selected/);
});

test("leaving and reopening while a run is pending restores the run button when it finishes", async () => {
  const { panel, root, nodes, section } = harness();
  let finishRun;
  const helpers = {
    get: async () => ({ reports: [] }),
    post: () => new Promise(resolve => { finishRun = resolve; }),
    esc: value => String(value),
    friendlyError: error => String(error),
  };
  panel.mount(root, helpers);
  await flush();
  nodes.get("#owo-eval-run").listeners.click();
  assert.equal(nodes.get("#owo-eval-run").disabled, true);
  panel.dispose();
  panel.mount(root, helpers);
  await flush();
  assert.equal(nodes.get("#owo-eval-run").disabled, true);
  finishRun({ skipped: true, reason: "not configured" });
  await flush();
  await flush();
  assert.equal(nodes.get("#owo-eval-run").disabled, false);
  assert.match(nodes.get("#owo-eval-status").textContent, /评测已结束/);
  assert.ok(section);
});


for (const [label,report,expected] of [
  ['zero cases',{total:0,passed:0,pass_rate:1,failures:[]},/未执行用例/],
  ['missing failure details',{total:2,passed:1,pass_rate:.5},/未提供失败明细/],
  ['partial outcome without details',{total:2,passed:1,pass_rate:.5,failures:[]},/有未通过用例/],
  ['inconsistent rate',{total:2,passed:1,pass_rate:1,failures:[]},/不一致/],
  ['missing counts',{pass_rate:1,failures:[]},/统计不完整/],
]) test('eval never reports all passed for '+label,async()=>{
  const {panel,root,nodes}=harness();
  panel.mount(root,{get:async path=>path.includes('report?')?{file:'case.json',report}:{reports:[]}});
  await panel._test.loadDetail('case.json');
  const html=nodes.get('#owo-eval-detail').innerHTML;
  assert.match(html,expected);assert.doesNotMatch(html,/全部用例通过/);
});

test('newer report selection wins when older request finishes last',async()=>{
  const {panel,root,nodes}=harness();let finishOld;
  panel.mount(root,{get:path=>path.includes('old.json')?new Promise(resolve=>finishOld=resolve):Promise.resolve(path.includes('report?')?{file:'new.json',report:{suite:'current selection',total:1,passed:1,pass_rate:1,failures:[]}}:{reports:[]})});
  const old=panel._test.loadDetail('old.json');await panel._test.loadDetail('new.json');
  finishOld({file:'old.json',report:{suite:'stale selection'}});await old;
  assert.match(nodes.get('#owo-eval-detail').innerHTML,/current selection/);assert.doesNotMatch(nodes.get('#owo-eval-detail').innerHTML,/stale selection/);
});
