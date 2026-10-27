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

test("工具类入口显示为「工具」，且普通模式不泄漏技术标识", () => {
  const { panel } = harness();
  const html = panel._test.card({
    user_name: "文生图",
    maturity: "beta",
    summary: "生成图片",
    required_effects: ["execute", "write"],
    advanced: true,
    entrypoints: [{ kind: "tool", target: "generate_image", label: "生成图片" }],
  }, false);
  assert.match(html, /<span class="owo-cap-kind">工具<\/span>/, "tool 入口未显示为工具");
  // 目标标识只放在 title 里供悬停查看，正文不渲染 <code> 技术入口。
  assert.doesNotMatch(html, /<code[^>]*>generate_image/, "普通模式不应内联技术标识");
  assert.match(html, /悬停查看技术入口/, "普通模式应提示可悬停查看");
  assert.match(html, /owo-cap-adv/, "高级能力应有标记");
  assert.match(html, /执行命令 · 写入文件/, "影响范围文案应可读");
});


test("capability filters do not reveal hidden advanced entries or make new requests", async () => {
  const {panel,root}=harness(); const view=root(); let requests=0;
  panel.mount(view,{get:async()=>{requests++; return {capabilities:[
    {id:"ordinary",user_name:"普通能力",maturity:"beta"},
    {id:"secret",user_name:"隐藏能力",advanced:true,maturity:"experimental"}
  ]};}});
  await flush();
  view.querySelector(".owo-cap-search").value="隐藏";
  view.querySelector(".owo-cap-search").listeners.input();
  assert.match(view.querySelector(".owo-cap-list").innerHTML,/没有匹配/);
  assert.doesNotMatch(view.querySelector(".owo-cap-list").innerHTML,/隐藏能力/);
  assert.equal(requests,1);
  view.querySelector(".owo-cap-search").value="";
  view.querySelector(".owo-cap-filter").value="stable";
  view.querySelector(".owo-cap-filter").listeners.change();
  assert.match(view.querySelector(".owo-cap-list").innerHTML,/没有匹配/);
});
test("invalid capability payload keeps the last valid catalog with an explicit error", async () => {
  const {panel,root}=harness(); const view=root(); let bad=false;
  panel.mount(view,{get:async()=>bad?{}:catalog("retained")}); await flush();
  bad=true; await panel.refresh(view);
  assert.match(view.querySelector(".owo-cap-list").innerHTML,/retained/);
  assert.match(view.querySelector(".owo-cap-meta").textContent,/更新失败/);
  assert.equal(view.querySelector(".owo-cap-error").hidden,false);
});
test("session capability labels its action as creating a new chat", async () => {
  const {panel,root}=harness(); const view=root();
  panel.mount(view,{get:async()=>({capabilities:[{id:"sessions.turns",user_name:"会话与回合",maturity:"beta"}]}),openChat:async()=>{}});
  await flush();
  assert.match(view.querySelector(".owo-cap-list").innerHTML,/新建对话/);
  assert.doesNotMatch(view.querySelector(".owo-cap-list").innerHTML,/打开对话/);
});
