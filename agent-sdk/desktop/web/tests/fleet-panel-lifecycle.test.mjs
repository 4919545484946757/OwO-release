import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../panels/fleet.panel.js", import.meta.url), "utf8");

function harness(helpers) {
  const controls = new Map();
  const views = new Map();
  function element() {
    return {
      value: "",
      checked: false,
      handlers: {},
      innerHTML: "",
      textContent: "",
      disabled: false,
      addEventListener(name, callback) { this.handlers[name] = callback; },
    };
  }
  const section = { querySelector(selector) {
    if (controls.has(selector)) return controls.get(selector);
    if (!views.has(selector)) views.set(selector, element());
    return views.get(selector);
  } };
  const root = {
    set innerHTML(_) {},
    querySelector(selector) {
      if (selector === ".owo-fleet-panel") return section;
      if (!controls.has(selector)) controls.set(selector, element());
      return controls.get(selector);
    },
  };
  const window = { OwoPanels: {}, location: { origin: "http://localhost" } };
  const document = { contains(node) { return node === section; } };
  runInNewContext(source, { window, document, Promise, JSON, String, encodeURIComponent, navigator: { platform: "Win32", userAgent: "x86_64" } });
  const panel = window.OwoPanels.fleet;
  panel.mount(root, helpers);
  return { panel, controls, views, section, root };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test("the latest Fleet task action wins when task details and events return out of order", async () => {
  const detail = deferred();
  const events = deferred();
  const h = harness({
    get(path) {
      if (path === "/fleet/nodes") return Promise.resolve({ nodes: [] });
      if (path === "/fleet/tasks/task-42") return detail.promise;
      if (path === "/fleet/tasks/task-42/events?format=json") return events.promise;
      throw new Error("unexpected path: " + path);
    },
    post: async () => ({}),
    esc: value => String(value),
    friendlyError: error => String(error),
  });
  if (!h.controls.has(".owo-fleet-task-get-id")) h.controls.set(".owo-fleet-task-get-id", { value: "" });
  h.controls.get(".owo-fleet-task-get-id").value = "task-42";
  h.controls.get(".owo-fleet-task-get").handlers.click();
  h.controls.get(".owo-fleet-task-events").handlers.click();

  events.resolve([{ kind: "progress", payload: { message: "latest event response" } }]);
  await Promise.resolve();
  await Promise.resolve();
  detail.resolve({ task_id: "task-42", status: "pending", events: [{ kind: "result", payload: { message: "stale task response" } }] });
  await Promise.resolve();
  await Promise.resolve();
  const rendered = h.views.get(".owo-fleet-task-view").innerHTML;
  assert.match(rendered, /latest event response/);
  assert.doesNotMatch(rendered, /stale task response/);
});

test("Fleet invalidates in-flight requests on disposal", async () => {
  const pending = deferred();
  const h = harness({
    get(path) {
      if (path === "/fleet/nodes") return Promise.resolve({ nodes: [] });
      if (path === "/fleet/tasks/task-42") return pending.promise;
      throw new Error("unexpected path: " + path);
    },
    post: async () => ({}),
    esc: value => String(value),
    friendlyError: error => String(error),
  });
  if (!h.controls.has(".owo-fleet-task-get-id")) h.controls.set(".owo-fleet-task-get-id", { value: "" });
  h.controls.get(".owo-fleet-task-get-id").value = "task-42";
  h.controls.get(".owo-fleet-task-get").handlers.click();
  if (!h.views.has(".owo-fleet-task-view")) h.views.set(".owo-fleet-task-view", { innerHTML: "" });
  h.panel.dispose();
  pending.resolve({ task_id: "task-42", status: "succeeded" });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(h.views.get(".owo-fleet-task-view").innerHTML, "");
});

test("Fleet fallback transport turns non-success HTTP responses into visible errors", async () => {
  const window = { OwoPanels: {}, location: { origin: "http://localhost" }, OwoApi: {
    stream: async () => ({ ok: false, status: 503, json: async () => ({ message: "service unavailable" }) }),
  } };
  const controls = new Map();
  const views = new Map();
  const section = { querySelector(selector) {
    if (!views.has(selector)) views.set(selector, { innerHTML: "" });
    return views.get(selector);
  } };
  const root = {
    set innerHTML(_) {},
    querySelector(selector) {
      if (selector === ".owo-fleet-panel") return section;
      if (!controls.has(selector)) controls.set(selector, { value: "", checked: false, addEventListener(_name, callback) { this.callback = callback; } });
      return controls.get(selector);
    },
  };
  const document = { contains: node => node === section };
  runInNewContext(source, { window, document, Promise, JSON, String, encodeURIComponent, navigator: { platform: "Win32", userAgent: "x86_64" } });
  window.OwoPanels.fleet.mount(root);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(views.get(".owo-fleet-nodes").innerHTML, /service unavailable/);
});

test("Fleet write responses from a previous mount cannot update the remounted panel", async () => {
  const operations = [
    {
      name: "register",
      selector: ".owo-fleet-node-result",
      button: ".owo-fleet-node-register",
      values: [[".owo-fleet-node-id", "node-a"], [".owo-fleet-node-worker", "worker-a"], [".owo-fleet-node-os", "windows"], [".owo-fleet-node-arch", "x86_64"]],
      response: { node_id: "node-a", status: { healthy: true }, lease_epoch: 1 },
    },
    {
      name: "submit",
      selector: ".owo-fleet-task-submit-result",
      button: ".owo-fleet-task-submit",
      values: [[".owo-fleet-task-id", "task-1"], [".owo-fleet-task-worker", "node-a"], [".owo-fleet-task-input", "{}"]],
      response: { task_id: "task-1", status: "pending", idempotency_key: "key-1" },
    },
    {
      name: "approval",
      selector: ".owo-fleet-approval-result",
      button: ".owo-fleet-approval-respond",
      values: [[".owo-fleet-approval-id", "task-1"], [".owo-fleet-approval-by", "owner"]],
      response: { task_id: "task-1", decision: "approved", status: "succeeded" },
    },
  ];

  for (const operation of operations) {
    for (const outcome of ["resolve", "reject"]) {
      const pending = deferred();
      const helpers = {
        get: path => path === "/fleet/nodes" ? Promise.resolve({ nodes: [] }) : Promise.resolve({task_id:"task-1", status:"awaiting_approval", events:[], approval:{decided:false, summary:"Review test approval", impact_preview:"No tool execution", evidence:[{kind:"test",summary:"UI lifecycle fixture"}]}}),
        post: () => pending.promise,
        esc: value => String(value),
        friendlyError: error => String(error),
      };
      const h = harness(helpers);
      for (const [selector, value] of operation.values) {
        h.controls.set(selector, { value, checked: false, handlers: {} });
      }
      if (operation.name === "submit") h.controls.set(".owo-fleet-task-approval", { checked: false });
      if (operation.name === "approval") h.controls.set(".owo-fleet-approval-decision", { value: "approve" });
      if (operation.name === "approval") await h.controls.get(".owo-fleet-approval-load").handlers.click();
      h.controls.get(operation.button).handlers.click();
      await Promise.resolve();
      await Promise.resolve();

      h.panel.dispose();
      h.panel.mount(h.root, helpers);
      const result = h.views.get(operation.selector) || { innerHTML: "" };
      h.views.set(operation.selector, result);
      result.innerHTML = "new mount content";

      if (outcome === "resolve") pending.resolve(operation.response);
      else pending.reject(new Error("old request failure"));
      await new Promise(resolve => setTimeout(resolve, 0));
      assert.equal(result.innerHTML, "new mount content", operation.name + " " + outcome);
    }
  }
});

test("Fleet write actions ignore duplicate clicks and retain their lock across remount", async () => {
  const operations = [
    {
      name: "register",
      button: ".owo-fleet-node-register",
      values: [[".owo-fleet-node-id", "node-a"], [".owo-fleet-node-worker", "worker-a"], [".owo-fleet-node-os", "windows"], [".owo-fleet-node-arch", "x86_64"]],
      response: { node_id: "node-a", status: { healthy: true }, lease_epoch: 1 },
      busyLabel: "登记中…",
      idleLabel: "登记设备",
    },
    {
      name: "submit",
      button: ".owo-fleet-task-submit",
      values: [[".owo-fleet-task-id", "task-1"], [".owo-fleet-task-worker", "node-a"], [".owo-fleet-task-input", "{}"]],
      response: { task_id: "task-1", status: "pending", idempotency_key: "key-1" },
      busyLabel: "提交中…",
      idleLabel: "提交任务",
    },
    {
      name: "approval",
      button: ".owo-fleet-approval-respond",
      values: [[".owo-fleet-approval-id", "task-1"], [".owo-fleet-approval-by", "owner"]],
      response: { task_id: "task-1", decision: "approved", status: "succeeded" },
      busyLabel: "处理中…",
      idleLabel: "提交裁决",
    },
  ];

  for (const operation of operations) {
    const pending = deferred();
    let posts = 0;
    const helpers = {
      get: path => path === "/fleet/nodes" ? Promise.resolve({ nodes: [] }) : Promise.resolve({task_id:"task-1", status:"awaiting_approval", events:[], approval:{decided:false, summary:"Review test approval", impact_preview:"No tool execution", evidence:[{kind:"test",summary:"UI lifecycle fixture"}]}}),
      post: () => { posts += 1; return pending.promise; },
      esc: value => String(value),
      friendlyError: error => String(error),
    };
    const h = harness(helpers);
    for (const [selector, value] of operation.values) {
      h.controls.set(selector, { value, checked: false, handlers: {}, disabled: false, textContent: "" });
    }
    if (operation.name === "submit") h.controls.set(".owo-fleet-task-approval", { checked: false });
    if (operation.name === "approval") h.controls.set(".owo-fleet-approval-decision", { value: "approve" });

    if (operation.name === "approval") await h.controls.get(".owo-fleet-approval-load").handlers.click();
    const button = h.controls.get(operation.button);
    button.handlers.click();
    button.handlers.click();
    await Promise.resolve(); await Promise.resolve();
    assert.equal(posts, 1, operation.name + " sends only one request");
    assert.equal(button.disabled, true);
    assert.equal(button.textContent, operation.busyLabel);

    h.panel.dispose();
    h.panel.mount(h.root, helpers);
    assert.equal(button.disabled, true, operation.name + " remains locked after remount");
    assert.equal(button.textContent, operation.busyLabel);

    pending.resolve(operation.response);
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(button.disabled, operation.name === "approval", operation.name + " requires a fresh approval snapshot after remount");
    assert.equal(button.textContent, operation.idleLabel);
  }
});

test("Fleet synchronous transport errors are shown and release the write lock", async () => {
  const h = harness({
    get: path => path === "/fleet/nodes" ? Promise.resolve({ nodes: [] }) : Promise.resolve({task_id:"task-1", status:"awaiting_approval", events:[], approval:{decided:false, summary:"Review test approval", impact_preview:"No tool execution", evidence:[{kind:"test",summary:"UI lifecycle fixture"}]}}),
    post: () => { throw new Error("synchronous transport failure"); },
    esc: value => String(value),
    friendlyError: error => String(error),
  });
  h.controls.set(".owo-fleet-task-id", { value: "task-1" });
  h.controls.set(".owo-fleet-task-worker", { value: "node-a" });
  h.controls.set(".owo-fleet-task-input", { value: "{}" });
  h.controls.set(".owo-fleet-task-approval", { checked: false });
  const button = h.controls.get(".owo-fleet-task-submit");
  button.handlers.click();
  assert.equal(button.disabled, true);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "提交任务");
  assert.match(h.views.get(".owo-fleet-task-submit-result").innerHTML, /synchronous transport failure/);
});


test("register requires actual node platform and never invents shell capability or local trust", async () => {
  const calls = [];
  const h = harness({get:async()=>({nodes:[]}),post:async(path,body)=>{calls.push({path,body});return {node_id:'safe-node',status:{healthy:true}};}});
  h.controls.get('.owo-fleet-node-id').value='safe-node';
  await h.controls.get('.owo-fleet-node-register').handlers.click(); assert.equal(calls.length,0);
  h.controls.get('.owo-fleet-node-os').value='linux'; h.controls.get('.owo-fleet-node-arch').value='aarch64';
  await h.controls.get('.owo-fleet-node-register').handlers.click();
  assert.equal(calls.length,1); assert.deepEqual(Array.from(calls[0].body.card.actions),[]);
  assert.equal(calls[0].body.card.os,'linux');assert.equal(calls[0].body.card.arch,'aarch64');assert.equal(calls[0].body.card.trust,'untrusted');assert.equal(calls[0].body.card.egress,'none');
});

test("approval must be loaded and explicitly chosen; cancel and approval share a task lock", async () => {
  const pending=deferred(), posts=[];
  const task={task_id:'task-1',status:'awaiting_approval',events:[],approval:{decided:false,summary:'Review',evidence:[]}};
  const h=harness({get:async path=>path==='/fleet/nodes'?{nodes:[]}:task,post:(path)=>{posts.push(path);return pending.promise;}});
  h.controls.get('.owo-fleet-approval-id').value='task-1';
  await h.controls.get('.owo-fleet-approval-respond').handlers.click();assert.equal(posts.length,0);
  await h.controls.get('.owo-fleet-approval-load').handlers.click();
  await h.controls.get('.owo-fleet-approval-respond').handlers.click();assert.equal(posts.length,0);
  h.controls.get('.owo-fleet-task-get-id').value='task-1';await h.controls.get('.owo-fleet-task-get').handlers.click();
  h.controls.get('.owo-fleet-approval-decision').value='reject';
  const responding=h.controls.get('.owo-fleet-approval-respond').handlers.click();
  await Promise.resolve();await Promise.resolve();
  h.controls.get('.owo-fleet-task-cancel').handlers.click();assert.equal(posts.length,1);assert.match(posts[0],/approvals/);
  pending.resolve({task_id:'task-1',decision:'rejected',status:'cancelled'});await responding;
});

test("cancel rechecks status, is single flight across remount, and does not claim node acknowledgement", async () => {
  const pending=deferred(); let posts=0;
  const helpers={get:async path=>path==='/fleet/nodes'?{nodes:[]}:{task_id:'task-1',status:'running',events:[]},post:()=>{posts++;return pending.promise;}};
  const h=harness(helpers);h.controls.get('.owo-fleet-task-get-id').value='task-1';
  await h.controls.get('.owo-fleet-task-cancel').handlers.click();assert.equal(posts,0);
  await h.controls.get('.owo-fleet-task-get').handlers.click();
  const cancelling=h.controls.get('.owo-fleet-task-cancel').handlers.click();await Promise.resolve();await Promise.resolve();
  h.controls.get('.owo-fleet-task-cancel').handlers.click();assert.equal(posts,1);
  h.panel.dispose();h.panel.mount(h.root,helpers);assert.equal(h.controls.get('.owo-fleet-task-cancel').disabled,true);
  pending.resolve({task_id:'task-1',status:'cancelled'});await cancelling;
  assert.equal(posts,1);assert.doesNotMatch(h.views.get('.owo-fleet-task-view').innerHTML,/任务已取消/);
});

test("malformed lists and missing health counters are visible instead of empty or failed", async () => {
  let nodes = {bad:true};const h=harness({get:async()=>nodes,post:async()=>({})});
  await new Promise(resolve=>setTimeout(resolve,0));assert.match(h.views.get('.owo-fleet-nodes').innerHTML,/设备列表无效/);
  nodes={nodes:[{id:'metadata-only'}]};await h.panel.refresh();await new Promise(resolve=>setTimeout(resolve,0));
  assert.match(h.views.get('.owo-fleet-nodes').innerHTML,/健康状态未知/);assert.match(h.views.get('.owo-fleet-nodes').innerHTML,/心跳 未知/);
  assert.doesNotMatch(h.views.get('.owo-fleet-nodes').innerHTML,/>异常</);
});


test('approve stays blocked when current material is incomplete; rejection remains available',async()=>{
 let posts=0;
 const h=harness({get:async path=>path==='/fleet/nodes'?{nodes:[]}:{task_id:'task-1',status:'awaiting_approval',events:[],approval:{decided:false,evidence:[]}},post:async()=>{posts++;return {};}});
 h.controls.get('.owo-fleet-approval-id').value='task-1';await h.controls.get('.owo-fleet-approval-load').handlers.click();
 h.controls.get('.owo-fleet-approval-decision').value='approve';h.controls.get('.owo-fleet-approval-decision').handlers.change();
 assert.equal(h.controls.get('.owo-fleet-approval-respond').disabled,true);
 await h.controls.get('.owo-fleet-approval-respond').handlers.click();assert.equal(posts,0);
 assert.match(h.views.get('.owo-fleet-approval-context').innerHTML,/影响预览：未提供/);
 h.controls.get('.owo-fleet-approval-decision').value='reject';h.controls.get('.owo-fleet-approval-decision').handlers.change();
 assert.equal(h.controls.get('.owo-fleet-approval-respond').disabled,false);
});


test('late cancellation cannot overwrite a newer event snapshot',async()=>{
 const pending=deferred();
 const h=harness({get:async path=>path==='/fleet/nodes'?{nodes:[]}:path.includes('/events?')?[{kind:'progress',payload:{message:'newer snapshot'}}]:{task_id:'task-1',status:'running',events:[]},post:()=>pending.promise});
 h.controls.get('.owo-fleet-task-get-id').value='task-1';await h.controls.get('.owo-fleet-task-get').handlers.click();
 const cancelling=h.controls.get('.owo-fleet-task-cancel').handlers.click();await Promise.resolve();await Promise.resolve();
 await h.controls.get('.owo-fleet-task-events').handlers.click();
 pending.resolve({task_id:'task-1',status:'cancelled'});await cancelling;
 assert.match(h.views.get('.owo-fleet-task-view').innerHTML,/newer snapshot/);
 assert.doesNotMatch(h.views.get('.owo-fleet-task-view').innerHTML,/控制面返回/);
});
