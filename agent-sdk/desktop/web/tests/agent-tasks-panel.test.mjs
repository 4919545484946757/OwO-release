// 任务运行面板契约：状态展示、动作可见性、错误载荷与生命周期隔离。
// 运行：node --test tests/agent-tasks-panel.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../panels/agent-tasks.panel.js", import.meta.url), "utf8");
const apiClientSource = readFileSync(new URL("../core/task-api-client.js", import.meta.url), "utf8");
const eventStoreSource = readFileSync(new URL("../core/task-event-store.js", import.meta.url), "utf8");
const workspaceClientSource = readFileSync(new URL("../core/workspace-api-client.js", import.meta.url), "utf8");

function harness(schedule = setTimeout) {
  const window = { OwoPanels: {} };
  // 面板依赖事件存储的断线重连：vm 沙箱需按浏览器环境提供定时器与 AbortController。
  const context = { window, globalThis: window, AbortController, setTimeout: schedule, clearTimeout };
  // 面板只经 OwoTaskApi / OwoTaskEvents 访问任务接口，按 index.html 的加载顺序注入。
  vm.runInNewContext(apiClientSource, context, { filename: "task-api-client.js" });
  vm.runInNewContext(eventStoreSource, context, { filename: "task-event-store.js" });
  vm.runInNewContext(workspaceClientSource, context, { filename: "workspace-api-client.js" });
  vm.runInNewContext(source, context, { filename: "agent-tasks.panel.js" });
  const root = () => {
    const nodes = new Map();
    return {
      innerHTML: "",
      isConnected: true,
      dataset: {},
      listeners: {},
      addEventListener(type, callback) { this.listeners[type] = callback; },
      querySelectorAll() { return []; },
      querySelector(selector) {
        if (!nodes.has(selector)) {
          nodes.set(selector, {
            innerHTML: "",
            textContent: "",
            hidden: false,
            value: "",
            listeners: {},
            addEventListener(type, callback) { this.listeners[type] = callback; },
          });
        }
        return nodes.get(selector);
      },
    };
  };
  return { panel: window.OwoPanels["agent-tasks"], root };
}

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

const task = (overrides = {}) => Object.assign({
  task_id: "t-1",
  title: "修复登录按钮",
  user_goal: "修复登录按钮并验证",
  state: "running",
  execution_strategy: "single_agent",
  current_attempt: 1,
  created_at: "2026-11-03T01:02:03.456Z",
  updated_at: "2026-11-03T01:05:00.000Z",
  budget: { max_attempts: 3, max_repairs: 2, max_tool_calls: 200 },
}, overrides);

test("badge escapes unknown state text instead of injecting markup", () => {
  const { panel } = harness();
  const html = panel._test.stateBadge('<img src=x onerror=alert(1)>');
  assert.match(html, /&lt;img/);
  assert.doesNotMatch(html, /<img/);
});

test("action visibility follows the server state machine", () => {
  const { panel } = harness();
  const ids = state => [...panel._test.actionsFor(state).map(action => action.id)];
  assert.deepEqual(ids("queued"), ["start", "cancel"]);
  assert.deepEqual(ids("paused"), ["resume", "verify-open", "cancel"]);
  assert.deepEqual(ids("running"), ["pause", "verify-open", "cancel"]);
  // 后端提交执行事实后停在 verifying：这里可以继续做独立验证。
  assert.deepEqual(ids("verifying"), ["pause", "verify-open", "cancel"]);
  assert.deepEqual(ids("succeeded"), []);
  assert.deepEqual(ids("failed"), []);
  // 排队中的任务不能直接验证：服务端状态机只允许 running/verifying/repairing/paused → verifying。
  assert.ok(!ids("queued").includes("verify-open"));
});

test("task list rejects an incomplete payload instead of rendering it", () => {
  const { panel } = harness();
  assert.throws(() => panel._test.listHtml({ ok: true }, ""), /任务列表响应不完整/);
  assert.throws(() => panel._test.listHtml({ tasks: [null] }, ""), /无效条目/);
});

test("task card and detail escape user goal and show the unified state", () => {
  const { panel } = harness();
  const malicious = task({ title: "<b>粗体</b>", user_goal: "目标</p><script>alert(1)</script>" });
  const card = panel._test.taskCard(malicious);
  assert.doesNotMatch(card, /<b>粗体<\/b>/);
  assert.match(card, /&lt;b&gt;/);
  const detail = panel._test.detailHtml(
    { task: malicious, approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.doesNotMatch(detail, /<script>/);
  assert.match(detail, /执行中/);
  assert.match(detail, /单人执行/);
});

test("detail never claims success without verification state", () => {
  const { panel } = harness();
  const detail = panel._test.detailHtml(
    { task: task({ state: "verifying" }), approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.match(detail, /验证中/);
  assert.doesNotMatch(detail, /owo-at-tone-ok/);
});

test("paused cloud task explains that resume resubmits and reruns commands", () => {
  const { panel } = harness();
  const detail = panel._test.detailHtml(
    { task: task({ state: "paused", execution_strategy: "cloud_remote" }), approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.match(detail, /class="owo-at-hint"/);
  assert.match(detail, /重新提交一次云端任务/);
  assert.match(detail, /命令会再执行一遍/);
});

test("paused workflow task warns that resume starts a new run", () => {
  const { panel } = harness();
  const detail = panel._test.detailHtml(
    { task: task({ state: "paused", execution_strategy: "workflow" }), approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.match(detail, /class="owo-at-hint"/);
  assert.match(detail, /新建一次运行/);
  assert.match(detail, /追加型写入会再执行一遍/);
});

test("paused work swarm task says resume continues the same run", () => {
  const { panel } = harness();
  const detail = panel._test.detailHtml(
    { task: task({ state: "paused", execution_strategy: "work_swarm" }), approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.match(detail, /沿用同一次团队运行/);
  assert.match(detail, /不会重跑/);
});

test("resume hint only appears for paused tasks whose resume reruns work", () => {
  const { panel } = harness();
  assert.equal(panel._test.resumeHint(task({ state: "running", execution_strategy: "cloud_remote" })), "");
  assert.equal(panel._test.resumeHint(task({ state: "paused", execution_strategy: "single_agent" })), "");
  assert.equal(panel._test.resumeHint(task({ state: "paused", execution_strategy: "computer_use" })), "");
  const detail = panel._test.detailHtml(
    { task: task({ state: "running", execution_strategy: "cloud_remote" }), approvals: [], latest_checkpoint: null, artifacts: [] },
    { events: [] },
    false,
  );
  assert.doesNotMatch(detail, /owo-at-hint/);
});

test("mount renders the server list and keeps the attention count", async () => {
  const { panel, root } = harness();
  const view = root();
  panel.mount(view, {
    get: async () => ({
      tasks: [
        task({ task_id: "a", state: "running" }),
        task({ task_id: "b", state: "awaiting_approval", title: "等待审批" }),
      ],
    }),
  });
  await flush();
  assert.match(view.querySelector(".owo-at-list").innerHTML, /修复登录按钮/);
  assert.match(view.querySelector(".owo-at-list").innerHTML, /等待审批/);
  assert.match(view.querySelector(".owo-at-meta").textContent, /2 个任务/);
  assert.match(view.querySelector(".owo-at-meta").textContent, /1 个需要处理/);
});

test("list load failure is explicit and retryable", async () => {
  const { panel, root } = harness();
  const view = root();
  panel.mount(view, {
    get: async () => { throw new Error("offline"); },
    friendlyError: error => error.message,
  });
  await flush();
  assert.match(view.querySelector(".owo-at-meta").textContent, /加载失败，可重试/);
  assert.equal(view.querySelector(".owo-at-notice").hidden, false);
  assert.match(view.querySelector(".owo-at-notice").textContent, /offline/);
});

test("disposing the page prevents late list responses from painting", async () => {
  const { panel, root } = harness();
  const view = root();
  const pending = deferred();
  panel.mount(view, { get: () => pending.promise });
  panel.dispose();
  view.isConnected = false;
  pending.resolve({ tasks: [task({ title: "迟到结果" })] });
  await flush();
  assert.equal(view.querySelector(".owo-at-list").innerHTML, "");
});

test("latest refresh wins when responses arrive out of order", async () => {
  const { panel, root } = harness();
  const view = root();
  const first = deferred();
  const second = deferred();
  let requests = 0;
  panel.mount(view, { get: () => (++requests === 1 ? first.promise : second.promise) });
  panel.refresh(view);
  second.resolve({ tasks: [task({ title: "新结果" })] });
  await flush();
  first.resolve({ tasks: [task({ title: "旧结果" })] });
  await flush();
  assert.match(view.querySelector(".owo-at-list").innerHTML, /新结果/);
  assert.doesNotMatch(view.querySelector(".owo-at-list").innerHTML, /旧结果/);
});

test("opening a running task subscribes to the SSE stream and settles on the terminal state", async () => {
  const { panel, root } = harness();
  const view = root();
  const running = task({ state: "running" });
  const done = task({ state: "succeeded", title: "修复登录按钮" });
  let detailCalls = 0;
  const get = async (path) => {
    if (path === "/agent-tasks") return { tasks: [running] };
    if (path.includes("/events")) return { events: [{ sequence: 1, event_type: "task.created", payload: {} }] };
    detailCalls += 1;
    return {
      task: detailCalls === 1 ? running : done,
      approvals: [],
      latest_checkpoint: null,
      artifacts: [],
    };
  };
  const frames = [];
  panel.mount(view, {
    get,
    openStream: async (path, options) => {
      frames.push(path);
      options.onOpen();
      options.onEvent({ event: "task-event", id: "2", data: JSON.stringify({ sequence: 2, event_type: "task.backend.started", payload: {} }) });
      options.onEvent({ event: "done", data: "{}" });
    },
  });
  await flush();

  view.listeners.click({
    target: {
      closest: () => ({
        getAttribute: (name) => (name === "data-at" ? "open" : name === "data-task" ? "t-1" : null),
      }),
    },
  });
  await flush();
  await flush();
  await flush();

  assert.deepEqual(frames, ["/agent-tasks/t-1/events/stream?after=1"], "实时订阅必须从补拉后的游标开始");
  assert.match(view.querySelector(".owo-at-event-log").innerHTML, /#2/, "实时事件必须追加到事件日志");
  assert.equal(view.querySelector(".owo-at-live").textContent, "已结束", "终态帧后必须停止实时标记");
  assert.match(view.querySelector(".owo-at-detail").innerHTML, /已完成/, "终态后必须重读详情显示成功");
  assert.equal(detailCalls, 2, "终态只重读一次详情，避免循环");
});

for (const streamReconnects of [true, false]) {
  test(`a failed live detail refresh preserves the view and retries ${streamReconnects ? "after reconnection" : "on a quiet stream"}`, async () => {
    let retryDetail;
    const { panel, root } = harness((callback, delay) => {
      if (delay === 3000) retryDetail = callback;
      else return setTimeout(callback, delay);
    });
    const view = root();
    let current = task({ state: "running" });
    let detailCalls = 0;
    let disconnect;
    const streams = [];
    panel.mount(view, {
      get: async (path) => {
        if (path === "/agent-tasks") return { tasks: [current] };
        if (path.includes("/events")) return { events: [] };
        detailCalls += 1;
        if (detailCalls === 2) throw new TypeError("Failed to fetch");
        return { task: current, approvals: [], latest_checkpoint: null, artifacts: [] };
      },
      openStream: async (_path, options) => {
        streams.push(options);
        options.onOpen();
        await new Promise((resolve) => {
          disconnect = resolve;
          options.signal.addEventListener("abort", resolve, { once: true });
        });
      },
    });
    await flush();
    view.listeners.click({
      target: {
        closest: () => ({
          getAttribute: (name) => (name === "data-at" ? "open" : name === "data-task" ? "t-1" : null),
        }),
      },
    });
    await flush();
    current = task({ state: "verifying" });
    const event = {
      event: "task-event",
      data: JSON.stringify({ sequence: 2, event_type: "task.verification.started", payload: { from: "running", to: "verifying" } }),
    };
    streams[0].onEvent(event);
    await flush();
    assert.match(view.querySelector(".owo-at-detail").innerHTML, /执行中/);
    assert.equal(streams[0].signal.aborted, false);
    if (streamReconnects) {
      disconnect();
      await new Promise((resolve) => setTimeout(resolve, 900));
    } else {
      assert.equal(typeof retryDetail, "function");
      retryDetail();
    }
    await flush();
    assert.match(view.querySelector(".owo-at-detail").innerHTML, /验证中/);
    streams.at(-1).onEvent(event);
    await flush();
    assert.equal(detailCalls, 3);
    panel.dispose();
  });
}

test("cloud execution is labelled as remote-command or simulated, never a full remote agent", () => {
  const { panel } = harness();
  const real = panel._test.environmentBadge(task({ execution_strategy: "cloud_remote" }), { events: [] });
  assert.match(real, /远端命令执行/);
  assert.match(real, /不是完整远端 Agent/);
  const mock = panel._test.environmentBadge(task({ execution_strategy: "cloud_remote" }), {
    events: [{ sequence: 1, event_type: "task.cloud.submitted", payload: { transport: "mock" } }],
  });
  assert.match(mock, /模拟执行/);
  assert.equal(panel._test.environmentBadge(task({ execution_strategy: "single_agent" }), { events: [] }), "");
});

test("workspace diff review only exposes actions the lease state allows", () => {
  const { panel } = harness();
  const lease = { workspace_id: "ws-1", kind: "git_worktree", state: "prepared", path: "D:/ws/owo-ws-1", base_commit: "abcdef123456" };
  const diff = {
    empty: false,
    diff: { patch: "diff --git\n+<script>alert(1)</script>", files: [{ path: "a.txt", status: "M", insertions: 2, deletions: 1 }] },
  };
  const prepared = panel._test.workspaceHtml(lease, diff, null);
  assert.match(prepared, /应用改动/);
  assert.match(prepared, /拒绝改动/);
  assert.match(prepared, /a\.txt/);
  assert.match(prepared, /\+2 \/ -1/);
  assert.doesNotMatch(prepared, /<script>/);
  assert.match(prepared, /&lt;script&gt;/);

  const applied = panel._test.workspaceHtml(Object.assign({}, lease, { state: "applied" }), diff, null);
  assert.match(applied, /回滚改动/);
  assert.doesNotMatch(applied, /应用改动/);

  const readonly = panel._test.workspaceHtml(Object.assign({}, lease, { kind: "current_workspace" }), diff, null);
  assert.match(readonly, /只读审阅/);
  assert.doesNotMatch(readonly, /ws-apply/);

  const empty = panel._test.workspaceHtml(lease, { empty: true, diff: { patch: "", files: [] } }, null);
  assert.match(empty, /还没有改动/);
  assert.match(empty, /释放工作区/);

  assert.match(panel._test.workspaceHtml(null, null, new Error("boom")), /工作区读取失败：boom/);
});

test("opening a task with workspace_ref loads its lease and diff", async () => {
  const { panel, root } = harness();
  const view = root();
  const calls = [];
  const lease = {
    workspace_id: "ws-9", task_id: "t-1", kind: "git_worktree", state: "prepared",
    path: "D:/ws/owo-ws-9", base_commit: "abcdef123456",
  };
  panel.mount(view, {
    get: async (path) => {
      calls.push(path);
      if (path === "/agent-tasks/t-1") {
        return {
          task: task({ workspace_ref: lease.path, state: "succeeded" }),
          approvals: [], latest_checkpoint: null, artifacts: [],
        };
      }
      if (path.indexOf("/agent-tasks/t-1/events") === 0) return { events: [] };
      if (path === "/workspaces") return { ok: true, workspaces: [lease] };
      if (path === "/workspaces/ws-9/diff") {
        return { ok: true, diff: { workspace_id: "ws-9", state: "prepared", patch: "diff --git\n+ok", files: [] }, empty: false };
      }
      return { tasks: [] };
    },
    post: async () => ({}),
    del: async () => ({}),
  });
  await flush();
  view.listeners.click({
    target: {
      closest: () => ({
        getAttribute: (name) => (name === "data-at" ? "open" : name === "data-task" ? "t-1" : null),
      }),
    },
  });
  await flush();
  await flush();
  await flush();

  assert.ok(calls.indexOf("/workspaces") >= 0, "必须读取工作区租约");
  assert.ok(calls.indexOf("/workspaces/ws-9/diff") >= 0, "必须读取 Diff");
  const html = view.querySelector(".owo-at-workspace").innerHTML;
  assert.match(html, /应用改动/);
  assert.match(html, /\+ok/);
});

test("trusted task references open unified details without starting or approving anything", async () => {
  const {panel,root}=harness();const view=root();const calls=[];const writes=[];
  const id="11111111-1111-4111-8111-111111111111";
  panel.mount(view,{get:async path=>{
    calls.push(path);
    if(path.startsWith("/agent-tasks?"))return {tasks:[]};
    if(path.endsWith("/events?limit=100"))return {events:[]};
    if(path.endsWith("/artifacts"))return {artifacts:[]};
    if(path.includes("/approvals"))return {approvals:[]};
    if(path==="/agent-tasks/"+id)return {task:task({task_id:id,state:"queued"})};
    return {tasks:[],events:[],artifacts:[],approvals:[]};
  },post:async(path,body)=>writes.push([path,body])});
  await panel.open(id);
  assert.ok(calls.includes("/agent-tasks/"+id));
  assert.deepEqual(writes,[]);
  await assert.rejects(panel.open("../session/x"));
  panel.dispose();
});
