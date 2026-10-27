import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/about.panel.js", import.meta.url), "utf8");
function loadPanel() {
  const sandbox = { window: { OwoPanels: {} }, Promise, Date, navigator: { userAgent: "test" }, document: {} };
  vm.createContext(sandbox);
  vm.runInContext(source, sandbox, { filename: "about.panel.js" });
  return sandbox.window.OwoPanels.about;
}
function makeRoot() {
  const elements = new Map([
    ["owo-about-health", { textContent: "加载中…", innerHTML: "", addEventListener() {} }],
    ["owo-about-caps", { textContent: "", innerHTML: "正在读取能力状态…", addEventListener() {} }],
    ["owo-about-diag", { textContent: "正在读取诊断…", innerHTML: "", addEventListener() {} }],
  ]);
  const root = {
    innerHTML: "",
    querySelector(selector) {
      const key = selector.replace(/^#/, "");
      if (!elements.has(key)) elements.set(key, { textContent: "", innerHTML: "", addEventListener() {} });
      return elements.get(key);
    },
    element(id) { return this.querySelector("#" + id); },
  };
  for (const id of ["owo-about-health"]) {
    const el = root.element(id); let markup = el.innerHTML;
    Object.defineProperty(el, "innerHTML", { get() { return markup; }, set(value) { markup = value; this.textContent = value.replace(/<[^>]*>/g, " "); } });
  }
  return root;
}
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function responseFor(path, label) {
  if (path === "/health") return { version: label, healthy: true, auto_approve: false };
  if (path === "/server/status") return { workspace: label, model: "test-model", read_only: false };
  if (path === "/skills" || path === "/plugins") return [{ name: "one" }];
  if (path === "/mcp") return { connected: [], count: 0, servers: [] };
  if (path === "/automations") return { count: 0 };
  if (path === "/notes") return { notes: [] };
  if (path === "/sessions") return { sessions: [] };
  if (path === "/plugins/market") return { count: 0 };
  return { provider: label, ready: true };
}
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

test("离开帮助页后的旧响应不会覆盖重新打开页面的数据", async () => {
  const panel = loadPanel();
  const makeCalls = () => [];
  const firstCalls = makeCalls();
  const secondCalls = makeCalls();
  const helpersFor = (calls) => ({ baseUrl: "", get(path) { const request = deferred(); calls.push({ path, ...request }); return request.promise; } });
  const oldRoot = makeRoot();
  const newRoot = makeRoot();

  panel.mount(oldRoot, helpersFor(firstCalls));
  panel.dispose();
  panel.mount(newRoot, helpersFor(secondCalls));
  assert.equal(firstCalls.some((item) => item.path === "/usage/summary"), false, "不应发出未展示的用量请求");

  for (const request of firstCalls) request.resolve(responseFor(request.path, "旧页面"));
  await flush();
  assert.equal(newRoot.element("owo-about-health").textContent, "加载中…");
  assert.equal(newRoot.element("owo-about-caps").innerHTML, "正在读取能力状态…");

  for (const request of secondCalls) request.resolve(responseFor(request.path, "新页面"));
  await flush();
  assert.match(newRoot.element("owo-about-health").textContent, /新页面/);
  assert.doesNotMatch(newRoot.element("owo-about-health").textContent, /旧页面/);
});

test("能力请求失败显示不可用和重试提示，不伪装成零项", async () => {
  const panel = loadPanel();
  const root = makeRoot();
  const helpers = {
    baseUrl: "",
    get(path) {
      if (path === "/skills") return Promise.reject(new Error("offline"));
      return Promise.resolve(responseFor(path, "online"));
    },
  };
  panel.mount(root, helpers);
  await flush();
  const cards = root.element("owo-about-caps").innerHTML;
  assert.match(cards, /技能/);
  assert.match(cards, /不可用/);
  assert.match(cards, /读取失败；点击上方刷新重试/);
  assert.match(cards, /MCP 服务器/);
  assert.match(cards, /<b>0<\/b>/, "成功读取到的空列表仍然显示真实的 0");
});


test("MCP 服务器名称作为卡片文本转义，不能注入 HTML", async () => {
  const panel = loadPanel();
  const root = makeRoot();
  const helpers = {
    baseUrl: "",
    esc(value) {
      return String(value == null ? "" : value).replace(/[&<>"']/g, (char) => ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[char]);
    },
    get(path) {
      if (path === "/mcp") {
        return Promise.resolve({ connected: ['<img src=x onerror=alert(1)>'], count: 1, servers: [] });
      }
      return Promise.resolve(responseFor(path, "online"));
    },
  };
  panel.mount(root, helpers);
  await flush();

  const cards = root.element("owo-about-caps").innerHTML;
  assert.match(cards, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.doesNotMatch(cards, /<img src=x onerror=/);
});

test("未知响应形状显示未知而不是空列表", async () => {
  const panel = loadPanel();
  const root = makeRoot();
  const helpers = {
    baseUrl: "",
    get(path) {
      if (["/skills", "/plugins", "/mcp", "/automations", "/notes", "/sessions"].includes(path)) {
        return Promise.resolve({ unexpected: true });
      }
      return Promise.resolve(responseFor(path, "online"));
    },
  };
  panel.mount(root, helpers);
  await flush();
  const cards = root.element("owo-about-caps").innerHTML;
  assert.match(cards, /<b>未知<\/b>/);
  assert.match(cards, /服务未返回可识别的数量/);
  assert.match(cards, /服务未返回可识别的连接状态/);
});


test("OCR engine creation and model configuration do not imply inference readiness", async () => {
  const panel = loadPanel(), root = makeRoot();
  await panel.mount(root, { get: async path => {
    if (path === '/perception/ocr/status') return { engine_created:true, onnx_models_present:false };
    if (path === '/vision/status') return { provider:'ollama', model:'test-vlm', ollama_models:[] };
    if (path === '/server/status') return { storage:{read_only:true} };
    if (path === '/health') return { version:'0.1', healthy:true, stage:'ready', api_version:'0.7' };
    return responseFor(path, 'test');
  }});
  assert.match(root.element('owo-about-diag').textContent, /OCR：模型文件缺失/);
  assert.match(root.element('owo-about-diag').textContent, /视觉：推理就绪度未确认/);
  assert.match(root.element('owo-about-diag').textContent, /本地模型列表：空/);
  assert.doesNotMatch(root.element('owo-about-diag').textContent, /OCR：就绪|视觉：就绪/);
  assert.match(root.element('owo-about-health').textContent, /存储模式\s+只读/);
  assert.match(root.element('owo-about-health').textContent, /审批方式\s+未知/);
});

test("invalid count and malformed MCP connection list remain unknown", async () => {
  const panel = loadPanel(), root = makeRoot();
  await panel.mount(root, { get: async path => {
    if (path === '/skills') return { count:-1 };
    if (path === '/mcp') return { connected:'not-an-array' };
    return responseFor(path, 'test');
  }});
  assert.match(root.element('owo-about-caps').innerHTML, /技能<\/span><b>未知/);
  assert.match(root.element('owo-about-caps').innerHTML, /连接状态/);
});

test("copy failure exposes selectable diagnostic text without claiming success", async () => {
  const panel = loadPanel(), root = makeRoot();
  const handlers = {};
  root.element('owo-about-copy').addEventListener = (name, action) => { handlers[name] = action; };
  await panel.mount(root, { get: async path => responseFor(path, 'online') });
  handlers.click();
  assert.match(root.element('owo-about-copy-status').textContent, /手动复制/);
  assert.equal(root.element('owo-about-copy-fallback').hidden, false);
  assert.match(root.element('owo-about-copy-fallback').value, /OwO Agent 诊断信息\n/);
  assert.doesNotMatch(root.element('owo-about-copy-status').textContent, /已复制/);
});

test("schema read goes through authenticated helpers and stale handlers cannot request", async () => {
  const panel = loadPanel(), root = makeRoot(), events = {};
  root.element('owo-about-schemas').addEventListener = (name, action) => { events[name] = action; };
  let calls = 0;
  await panel.mount(root, { get: async path => { if(path === '/schemas') { calls++; return {schema:'<script>bad</script>'}; } return responseFor(path, 'online'); } });
  await events.click();
  assert.equal(calls, 1);
  assert.match(root.element('owo-about-schema-result').innerHTML, /&lt;script&gt;/);
  panel.dispose(); await events.click(); assert.equal(calls, 1);
});
