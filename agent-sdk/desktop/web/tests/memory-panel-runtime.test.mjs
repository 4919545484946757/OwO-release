import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/memory.panel.js", import.meta.url), "utf8");

function harness(helpers, windowOptions = {}) {
  const elements = new Map();
  function element() {
    return {
      value: "", textContent: "", innerHTML: "", disabled: false, handlers: {},
      addEventListener(name, callback) { this.handlers[name] = callback; },
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
    querySelector(selector) { return document.getElementById(selector.replace(/^#/, "")); },
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" }, ...windowOptions };
  runInNewContext(source, { window, document, Promise, String, JSON, Number, encodeURIComponent, Object });
  const panel = window.OwoPanels.memory;
  panel.mount(root, helpers);
  return { panel, document, elements, root };
}

function deferred() {
  let resolve;
  const promise = new Promise(res => { resolve = res; });
  return { promise, resolve };
}

test("memory panel initializes its default API helpers when none are injected", async () => {
  const requested = [];
  const h = harness(undefined, {
    OwoApi: {
      stream: async path => {
        requested.push(path);
        return { ok: true, status: 200, json: async () => ({ buckets: [], entities: [], links: [], entries: [] }) };
      },
    },
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.ok(requested.includes("/memory/graph/timeline"));
  assert.ok(requested.includes("/memory/graph/entities?limit=30"));
  assert.ok(requested.includes("/memory/graph/links"));
  assert.ok(requested.includes("/memory/graph/entries?limit=50"));
});

test("memory recall keeps the newest query and ignores late responses after disposal", async () => {
  const pending = new Map();
  const h = harness({
    get(path) {
      if (path.includes("/recall?")) {
        const request = deferred();
        pending.set(path, request);
        return request.promise;
      }
      return Promise.resolve({ buckets: [], entities: [], links: [], entries: [] });
    },
    post: async () => ({}), esc: value => String(value), friendlyError: error => String(error),
  });
  const input = h.document.getElementById("owo-memory-recall");
  const box = h.document.getElementById("owo-memory-recall-box");
  input.value = "first";
  const first = h.panel._test.doRecall();
  input.value = "second";
  const second = h.panel._test.doRecall();
  pending.get("/memory/graph/recall?q=second&top_k=5").resolve({ count: 1, hits: [{ app_id: "b", ts: "now", summary: "new query result" }] });
  await second;
  pending.get("/memory/graph/recall?q=first&top_k=5").resolve({ count: 1, hits: [{ app_id: "a", ts: "then", summary: "old query result" }] });
  await first;
  assert.match(box.innerHTML, /new query result/);
  assert.doesNotMatch(box.innerHTML, /old query result/);

  input.value = "after-leave";
  const afterLeave = h.panel._test.doRecall();
  h.panel.dispose();
  pending.get("/memory/graph/recall?q=after-leave&top_k=5").resolve({ count: 1, hits: [{ summary: "detached result" }] });
  await afterLeave;
  assert.doesNotMatch(box.innerHTML, /detached result/);
});

test("manual memory relation rejects incomplete input and suppresses duplicate submissions", async () => {
  const pending = deferred();
  const posts = [];
  const h = harness({
    get: async () => ({ buckets: [], entities: [], links: [], entries: [] }),
    post(path, body) { posts.push([path, body]); return pending.promise; },
    esc: value => String(value), friendlyError: error => String(error), notify() {},
  });
  const a = h.document.getElementById("owo-memory-rel-a");
  const b = h.document.getElementById("owo-memory-rel-b");
  const relation = h.document.getElementById("owo-memory-rel-r");
  const button = h.document.getElementById("owo-memory-rel-add");
  await button.handlers.click();
  assert.equal(posts.length, 0);
  a.value = "Ada"; b.value = "Project"; relation.value = "owns";
  const first = button.handlers.click();
  const second = button.handlers.click();
  await second;
  assert.equal(posts.length, 1);
  assert.equal(button.disabled, true);
  pending.resolve({});
  await first;
  assert.equal(button.disabled, false);
  assert.equal(a.value, "");
});


test("memory entries apply app and inclusive date filters", async () => {
  const requested = [];
  const h = harness({
    get: async path => {
      requested.push(path);
      return { entries: [] };
    },
    post: async () => ({}), esc: value => String(value), friendlyError: error => String(error),
  });
  const app = h.document.getElementById("owo-memory-app");
  const from = h.document.getElementById("owo-memory-from");
  const to = h.document.getElementById("owo-memory-to");
  app.value = "qq work";
  from.value = "2026-10-01";
  to.value = "2026-10-09";
  await h.panel._test.loadEntries();
  assert.equal(
    requested.at(-1),
    "/memory/graph/entries?app=qq%20work&from=2026-10-01&to=2026-10-09&limit=50",
  );
});

test("memory entries reject reversed or malformed date filters without a request", async () => {
  const requested = [];
  const h = harness({
    get: async path => {
      requested.push(path);
      return { entries: [] };
    },
    post: async () => ({}), esc: value => String(value), friendlyError: error => String(error),
  });
  requested.length = 0;
  h.document.getElementById("owo-memory-from").value = "2026-10-10";
  h.document.getElementById("owo-memory-to").value = "2026-10-09";
  await h.panel._test.loadEntries();
  assert.equal(requested.length, 0);
  assert.equal(h.document.getElementById("owo-memory-entries").textContent, "开始日期不能晚于结束日期");

  h.document.getElementById("owo-memory-from").value = "not-a-date";
  h.document.getElementById("owo-memory-to").value = "";
  await h.panel._test.loadEntries();
  assert.equal(requested.length, 0);
  assert.equal(h.document.getElementById("owo-memory-entries").textContent, "日期格式无效，请重新选择日期");
});

test("blank recall makes no request and malformed hits remain an error", async () => {
  const calls=[];
  const h=harness({get:async path=>{calls.push(path);return {buckets:[],entities:[],links:[],entries:[]};},esc:String,friendlyError:String});
  calls.length=0;
  await h.panel._test.doRecall();
  assert.equal(calls.length,0);
  assert.match(h.document.getElementById("owo-memory-recall-box").innerHTML,/输入关键词/);
  h.document.getElementById("owo-memory-recall").value="query";
  await h.panel._test.doRecall();
  assert.match(h.document.getElementById("owo-memory-recall-box").innerHTML,/响应格式不完整/);
  assert.doesNotMatch(h.document.getElementById("owo-memory-recall-box").innerHTML,/未找到相关记忆/);
});

test("HTTP-success error payloads do not become empty memory lists", async () => {
  const h=harness({get:async()=>({error:"memory lock unavailable",buckets:[],entities:[],links:[],entries:[]}),esc:String,friendlyError:String});
  await h.panel.refresh();
  for(const name of ["timeline","entities","relations","entries"])
    assert.match(h.document.getElementById("owo-memory-"+name).innerHTML,/memory lock unavailable/);
});

test("same-core remount retains draft and prevents duplicate pending relation writes", async () => {
  const pending=deferred(); let posts=0;
  const helpers={baseUrl:"local",coreInstanceId:"one",get:async()=>({buckets:[],entities:[],links:[],entries:[]}),post(){posts++;return pending.promise;},esc:String,friendlyError:String};
  const h=harness(helpers);
  for(const [name,value] of [["rel-a","Ada"],["rel-r","owns"],["rel-b","Project"],["recall","draft query"]])
    h.document.getElementById("owo-memory-"+name).value=value;
  const write=h.panel._test.addRelation();
  h.panel.dispose();h.panel.mount(h.root,helpers);
  assert.equal(h.document.getElementById("owo-memory-recall").value,"draft query");
  await h.panel._test.addRelation();
  assert.equal(posts,1);
  assert.equal(h.document.getElementById("owo-memory-rel-add").disabled,true);
  pending.resolve({});await write;
  assert.equal(h.document.getElementById("owo-memory-rel-add").disabled,false);
});

test("successful relation submission preserves values typed while it was pending", async () => {
  const pending=deferred();
  const h=harness({get:async()=>({buckets:[],entities:[],links:[],entries:[]}),post:()=>pending.promise,esc:String,friendlyError:String});
  h.document.getElementById("owo-memory-rel-a").value="first";
  h.document.getElementById("owo-memory-rel-r").value="owns";
  h.document.getElementById("owo-memory-rel-b").value="target";
  const write=h.panel._test.addRelation();
  h.document.getElementById("owo-memory-rel-a").value="next";
  pending.resolve({});await write;
  assert.equal(h.document.getElementById("owo-memory-rel-a").value,"next");
  assert.equal(h.document.getElementById("owo-memory-rel-b").value,"");
});
