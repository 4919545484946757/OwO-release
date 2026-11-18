// TaskApiClient 契约：所有任务读写都经同一客户端，路径、请求体与响应校验统一收口。
// 运行：node --test tests/task-api-client.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../core/task-api-client.js", import.meta.url), "utf8");

function load() {
  const window = {};
  vm.runInNewContext(source, { window, globalThis: window }, { filename: "task-api-client.js" });
  return window.OwoTaskApi;
}

function transport(reply) {
  const calls = [];
  const record = (method) => (path, body) => {
    calls.push({ method, path, body });
    return Promise.resolve(typeof reply === "function" ? reply(method, path, body) : reply);
  };
  return { calls, get: record("GET"), post: record("POST") };
}

const task = { task_id: "t-1", state: "queued" };
// vm 沙箱里的对象属于另一个 realm，比较请求体前转成本地结构。
const local = (value) => JSON.parse(JSON.stringify(value));

test("list/get/events use the unified /agent-tasks paths", async () => {
  const api = load().createTaskApiClient(
    transport((method, path) => {
      if (path.startsWith("/agent-tasks?")) return { tasks: [task] };
      if (path.includes("/events")) return { events: [{ sequence: 2 }, { sequence: 1 }] };
      return { task, approvals: [], artifacts: [], latest_checkpoint: null };
    }),
  );
  assert.deepEqual((await api.list({ state: "queued" })).tasks, [task]);
  const detail = await api.get("t-1");
  assert.equal(detail.task.task_id, "t-1");
  assert.deepEqual(detail.approvals, []);
  assert.deepEqual(detail.artifacts, []);
  assert.equal((await api.events("t-1", { after: 4, limit: 10 })).events.length, 2);
});

test("action methods send the server's documented bodies", async () => {
  const t = transport({ ok: true, task });
  const api = load().createTaskApiClient(t);
  await api.start("t-1");
  await api.pause("t-1");
  await api.resume("t-1");
  await api.cancel("t-1");
  await api.verify("t-1", { goal: { passed: true, detail: "ok" } });
  await api.approve("t-1", "a-9", "approved");
  await api.create({ user_goal: "目标" });

  const paths = t.calls.map(call => call.method + " " + call.path);
  assert.deepEqual(paths, [
    "POST /agent-tasks/t-1/start",
    "POST /agent-tasks/t-1/pause",
    "POST /agent-tasks/t-1/resume",
    "POST /agent-tasks/t-1/cancel",
    "POST /agent-tasks/t-1/verify",
    "POST /agent-tasks/t-1/approvals/a-9",
    "POST /agent-tasks",
  ]);
  assert.deepEqual(local(t.calls[2].body), {}, "resume 不需要请求体");
  assert.equal(t.calls[3].body.reason, "用户取消");
  assert.deepEqual(local(t.calls[4].body), { goal: { passed: true, detail: "ok" } });
  assert.deepEqual(local(t.calls[5].body), { decision: "approved", decided_by: "desktop" });
  assert.deepEqual(local(t.calls[6].body), { user_goal: "目标" });
});

test("incomplete payloads are rejected instead of rendered as empty tasks", async () => {
  const api = load().createTaskApiClient(transport(() => ({ ok: true })));
  await assert.rejects(() => api.list({}), /任务列表响应不完整/);
  await assert.rejects(() => api.get("t-1"), /任务响应不完整/);
  await assert.rejects(() => api.create({}), /任务响应不完整/);
  await assert.rejects(() => api.events("t-1"), /事件响应不完整/);
});

test("missing task id and approval id fail locally without a request", async () => {
  const t = transport({ task });
  const api = load().createTaskApiClient(t);
  assert.throws(() => api.get(""), /缺少任务 ID/);
  assert.throws(() => api.approve("t-1", "", "approved"), /缺少审批 ID/);
  assert.throws(() => load().createTaskApiClient({ get: async () => ({}) }), /需要 get\/post/);
  assert.equal(t.calls.length, 0, "本地校验失败不得发出请求");
});
