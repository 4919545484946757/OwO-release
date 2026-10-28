import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/command.panel.js", import.meta.url), "utf8");

function createHarness(helpers, windowOptions = {}) {
  const elements = new Map();
  function element() {
    return {
      value: "",
      files: [],
      style: {},
      disabled: false,
      textContent: "",
      innerHTML: "",
      handlers: {},
      addEventListener(name, callback) { this.handlers[name] = callback; },
      setAttribute(name, value) { this[name] = value; },
      removeAttribute(name) { delete this[name]; },
    };
  }
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, element());
      return elements.get(id);
    },
  };
  const root = {
    set innerHTML(_) {},
    querySelector(selector) {
      const id = selector.replace(/^#/, "");
      return document.getElementById(id);
    },
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" }, ...windowOptions };
  runInNewContext(source, { window, document, Promise, String, JSON, Number, isFinite, FileReader: class {} });
  const panel = window.OwoPanels.command;
  panel.mount(root, helpers);
  return {
    panel,
    document,
    elements,
    remount() { elements.clear(); panel.mount(root, helpers); },
  };
}

function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

test("command execution shares one submission lock and safely escapes array result text", async () => {
  const pending = deferred();
  const calls = [];
  const h = createHarness({
    get: async () => ({ audit: [] }),
    post(path, body) { calls.push([path, body]); return pending.promise; },
    esc(value) { return String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); },
    friendlyError: error => String(error),
    renderMarkdown: value => String(value),
    notify() {},
  });
  h.document.getElementById("owo-command-mode").value = "text";
  h.document.getElementById("owo-command-text").value = "inspect safely";
  const run = h.document.getElementById("owo-command-run");
  const first = run.handlers.click();
  const second = h.document.getElementById("owo-command-text").handlers.keydown({ key: "Enter" });
  await second;
  assert.equal(calls.length, 1);
  assert.equal(run.disabled, true);
  pending.resolve({ intent: "search", confidence: 0.9, text: "inspect safely", results: { matches: ["<img src=x onerror=alert(1)>"] } });
  await first;
  const rendered = h.document.getElementById("owo-command-results").innerHTML;
  assert.match(rendered, /&lt;img/);
  assert.doesNotMatch(rendered, /<img/);
  assert.equal(run.disabled, false);
});

test("pending command stays locked across panel disposal and remount", async () => {
  const pending = deferred();
  const calls = [];
  const h = createHarness({
    get: async () => ({ audit: [] }),
    post(path, body) { calls.push([path, body]); return pending.promise; },
    esc: value => String(value),
    friendlyError: error => String(error),
    renderMarkdown: value => String(value),
    notify() {},
  });
  h.document.getElementById("owo-command-mode").value = "text";
  h.document.getElementById("owo-command-text").value = "run once";
  const firstButton = h.document.getElementById("owo-command-run");
  const first = firstButton.handlers.click();
  h.panel.dispose();
  h.remount();
  const currentButton = h.document.getElementById("owo-command-run");
  assert.notEqual(currentButton, firstButton);
  assert.equal(currentButton.disabled, true, "重进页面时仍显示原命令正在执行");
  await currentButton.handlers.click();
  assert.equal(calls.length, 1, "旧请求结束前不能重复执行命令");

  pending.resolve({ intent: "text", confidence: 1, text: "run once", results: {} });
  await first;
  assert.equal(currentButton.disabled, false, "原命令结束后当前页按钮解锁");
});

test("command requests resolving after panel disposal cannot repaint results", async () => {
  const pending = deferred();
  const h = createHarness({
    get: async () => ({ audit: [] }),
    post: async () => pending.promise,
    esc: value => String(value),
    friendlyError: error => String(error),
    renderMarkdown: value => String(value),
    notify() {},
  });
  h.document.getElementById("owo-command-mode").value = "text";
  h.document.getElementById("owo-command-text").value = "old request";
  const run = h.document.getElementById("owo-command-run");
  const request = run.handlers.click();
  h.panel.dispose();
  pending.resolve({ intent: "old", confidence: 1, text: "old", results: { result: "stale" } });
  await request;
  assert.equal(h.document.getElementById("owo-command-results").innerHTML, "");
});

test("command panel initializes its default API helpers when none are injected", async () => {
  const requested = [];
  const h = createHarness(undefined, {
    OwoApi: {
      stream: async path => {
        requested.push(path);
        return { ok: true, status: 200, json: async () => ({ audit: [{ event: "ready", detail: "loaded" }] }) };
      },
    },
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(requested.includes("/command/audit"));
  assert.match(h.document.getElementById("owo-command-audit").textContent, /ready — loaded/);
});


test("OCR fills a reviewable draft without executing recognized text", async () => {
  const calls=[];
  const h=createHarness({get:async()=>({audit:[]}),post:async(path,body)=>{calls.push([path,body]);return {text:'桌面动作：点击确认'};},prompt:async()=> '10,20,300,80'});
  h.document.getElementById('owo-command-mode').value='region';
  await h.document.getElementById('owo-command-run').handlers.click();
  assert.deepEqual(calls.map(c=>c[0]),['/perception/ocr/region']);
  assert.equal(h.document.getElementById('owo-command-text').value,'桌面动作：点击确认');
  assert.equal(h.document.getElementById('owo-command-mode').value,'text');
  assert.match(h.document.getElementById('owo-command-status').textContent,/检查后再执行/);
});

test("leaving while the OCR region prompt is open prevents late capture submission", async()=>{
  const dialog=deferred(),writes=[];
  const h=createHarness({get:async()=>({audit:[]}),post:async(path)=>{writes.push(path);return {};},prompt:()=>dialog.promise});
  h.document.getElementById('owo-command-mode').value='region';
  const task=h.document.getElementById('owo-command-run').handlers.click();
  h.panel.dispose();dialog.resolve('1,2,30,40');await task;
  assert.deepEqual(writes,[]);
});

test("invalid audit data remains a visible failure instead of an empty audit",async()=>{
  const h=createHarness({get:async()=>({}),post:async()=>({})});await h.panel.refresh();
  assert.match(h.document.getElementById('owo-command-audit').textContent,/失败|不完整/);
});
