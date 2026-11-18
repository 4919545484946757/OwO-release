// TaskEventStore 契约（Agent V2 Phase 5）：补拉 + SSE 单一事件序列。
// 运行：node --test tests/task-event-store.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../core/task-event-store.js", import.meta.url), "utf8");

function load() {
  const window = {};
  // vm 沙箱没有 DOM 定时器与 AbortController，按浏览器环境注入。
  const sandbox = { window, globalThis: window, AbortController, setTimeout, clearTimeout };
  vm.runInNewContext(source, sandbox, { filename: "task-event-store.js" });
  return window.OwoTaskEvents;
}

const sleep = (ms) => new Promise(resolve => setTimeout(resolve, ms));
// vm 沙箱里的对象/数组属于另一个 realm，比较前转成本地结构。
const local = (value) => JSON.parse(JSON.stringify(value));
const event = (sequence) => ({ sequence, event_type: "task.state.changed", payload: { sequence } });
const frame = (sequence) => ({
  event: "task-event",
  id: String(sequence),
  data: JSON.stringify(event(sequence)),
});

function clientWith(history) {
  const calls = [];
  return {
    calls,
    events: async (taskId, options) => {
      calls.push({ taskId, options });
      return { events: history.filter(item => item.sequence > (options.after || 0)) };
    },
  };
}

test("history backfill and live frames share one ordered sequence", async () => {
  const api = load();
  const client = clientWith([event(1), event(2)]);
  const paths = [];
  const states = [];
  const store = api.createTaskEventStore({
    client,
    reconnectBaseMs: 5,
    onChange: snapshot => states.push(snapshot.state),
    openStream: async (path, options) => {
      paths.push(path);
      options.onOpen();
      options.onEvent(frame(3));
      options.onEvent(frame(4));
      options.onEvent({ event: "done", data: "{}" });
    },
  });

  store.start("t-1");
  await sleep(20);
  const snapshot = store.snapshot();
  assert.equal(snapshot.taskId, "t-1");
  assert.deepEqual(local(snapshot.events.map(item => item.sequence)), [1, 2, 3, 4]);
  assert.equal(snapshot.cursor, 4);
  assert.equal(snapshot.state, "closed", "done 帧到达后必须停止重连");
  assert.equal(snapshot.error, null);
  assert.equal(paths[0], "/agent-tasks/t-1/events/stream?after=2", "补拉完成后按游标订阅");
  assert.deepEqual(local(client.calls[0]), { taskId: "t-1", options: { after: 0, limit: 200 } });
  assert.ok(states.includes("live"));
});

test("out-of-order and duplicate frames are re-ordered and deduplicated", async () => {
  const api = load();
  const client = clientWith([event(1), event(2)]);
  const store = api.createTaskEventStore({
    client,
    reconnectBaseMs: 5,
    openStream: async (path, options) => {
      options.onEvent(frame(2)); // 与补拉重复
      options.onEvent(frame(5));
      options.onEvent(frame(3)); // 乱序到达
      options.onEvent({ event: "done", data: "{}" });
    },
  });
  store.start("t-1");
  await sleep(20);
  assert.deepEqual(local(store.snapshot().events.map(item => item.sequence)), [1, 2, 3, 5]);
  assert.equal(store.snapshot().cursor, 5);
});

test("a broken stream degrades and reconnects from the last received sequence", async () => {
  const api = load();
  const client = clientWith([event(1), event(2)]);
  const paths = [];
  const seen = [];
  let attempts = 0;
  const store = api.createTaskEventStore({
    client,
    reconnectBaseMs: 5,
    maxBackoffMs: 20,
    onChange: snapshot => seen.push(snapshot.state),
    openStream: async (path, options) => {
      paths.push(path);
      attempts += 1;
      if (attempts === 1) throw new Error("connection reset");
      options.onEvent(frame(3));
      options.onEvent({ event: "done", data: "{}" });
    },
  });

  store.start("t-1");
  await sleep(80);
  assert.ok(seen.includes("degraded"), "断流必须显式进入重连态");
  assert.equal(attempts, 2, "必须重连一次");
  assert.equal(paths[1], "/agent-tasks/t-1/events/stream?after=2", "重连必须带最后序号");
  assert.deepEqual(local(store.snapshot().events.map(item => item.sequence)), [1, 2, 3]);
  assert.equal(store.snapshot().state, "closed");
});

test("stop cancels the stream and drops late frames", async () => {
  const api = load();
  const client = clientWith([]);
  let captured = null;
  const store = api.createTaskEventStore({
    client,
    reconnectBaseMs: 5,
    openStream: async (path, options) => {
      captured = options;
      await new Promise(resolve => {
        options.signal.addEventListener("abort", resolve);
      });
    },
  });

  store.start("t-1");
  await sleep(5);
  store.stop();
  captured.onEvent(frame(9));
  await sleep(5);
  const snapshot = store.snapshot();
  assert.equal(snapshot.state, "idle");
  assert.equal(snapshot.taskId, null);
  assert.deepEqual(local(snapshot.events), []);
});

test("missing dependencies fail loudly instead of silently dropping events", () => {
  const api = load();
  assert.throws(() => api.createTaskEventStore({}), /需要 TaskApiClient/);
  assert.throws(
    () => api.createTaskEventStore({ client: { events: async () => ({ events: [] }) } }),
    /需要 openStream/,
  );
});
