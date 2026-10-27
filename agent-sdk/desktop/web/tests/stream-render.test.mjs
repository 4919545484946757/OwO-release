import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../core/stream-render.js", import.meta.url), "utf8");
const sandbox = { window: {} };
vm.createContext(sandbox);
vm.runInContext(source, sandbox);
const { createStreamRenderer, DEFAULT_INTERVAL_MS } = sandbox.window.OwoStreamRender;

function fakeTimers() {
  let nextId = 1;
  const timers = new Map();
  return {
    schedule(fn, ms) {
      const id = nextId++;
      timers.set(id, { fn, ms });
      return id;
    },
    cancel(id) {
      timers.delete(id);
    },
    get size() {
      return timers.size;
    },
    runAll() {
      const queued = [...timers.values()];
      timers.clear();
      for (const { fn } of queued) fn();
    },
  };
}

function fakeBubble() {
  return { innerHTML: "", paints: 0 };
}

test("多个 delta 合并成一次渲染，取最后一次文本", () => {
  const timers = fakeTimers();
  const rendered = [];
  const painted = [];
  const renderer = createStreamRenderer(
    (text) => {
      rendered.push(text);
      return "<p>" + text + "</p>";
    },
    { schedule: timers.schedule, cancel: timers.cancel, onPaint: (bubble) => painted.push(bubble) }
  );
  const bubble = fakeBubble();
  renderer.update(bubble, "你");
  renderer.update(bubble, "你好");
  renderer.update(bubble, "你好，世界");
  assert.equal(timers.size, 1, "同一帧只排一次渲染");
  assert.equal(rendered.length, 0, "合并窗口内不渲染");
  timers.runAll();
  assert.deepEqual(rendered, ["你好，世界"]);
  assert.equal(bubble.innerHTML, "<p>你好，世界</p>");
  assert.equal(painted.length, 1, "每次真正落盘才重新绑定复制按钮");
  assert.equal(renderer.pending, false);
});

test("flush 立即渲染并取消挂起帧", () => {
  const timers = fakeTimers();
  const rendered = [];
  const renderer = createStreamRenderer((text) => {
    rendered.push(text);
    return text;
  }, { schedule: timers.schedule, cancel: timers.cancel });
  const bubble = fakeBubble();
  renderer.update(bubble, "第一段");
  renderer.update(bubble, "第二段");
  renderer.flush();
  assert.deepEqual(rendered, ["第二段"]);
  assert.equal(timers.size, 0, "flush 后不应留下定时器");
  assert.equal(bubble.innerHTML, "第二段");
  // 无新内容时重复 flush 不重复渲染。
  renderer.flush();
  assert.deepEqual(rendered, ["第二段"]);
});

test("reset 丢弃未落盘文本，避免旧帧覆盖 final", () => {
  const timers = fakeTimers();
  const rendered = [];
  const renderer = createStreamRenderer((text) => {
    rendered.push(text);
    return text;
  }, { schedule: timers.schedule, cancel: timers.cancel });
  const bubble = fakeBubble();
  renderer.update(bubble, "旧增量");
  renderer.reset();
  assert.equal(timers.size, 0);
  assert.equal(renderer.pending, false);
  renderer.flush();
  assert.deepEqual(rendered, []);
  assert.equal(bubble.innerHTML, "");
});

test("渲染抛错不会遗留脏标记（下一次 update 仍能工作）", () => {
  const timers = fakeTimers();
  let calls = 0;
  const renderer = createStreamRenderer(() => {
    calls += 1;
    if (calls === 1) throw new Error("boom");
    return "ok";
  }, { schedule: timers.schedule, cancel: timers.cancel });
  const bubble = fakeBubble();
  renderer.update(bubble, "a");
  assert.throws(() => timers.runAll(), /boom/);
  renderer.update(bubble, "b");
  timers.runAll();
  assert.equal(bubble.innerHTML, "ok");
});

test("默认间隔是有界的小值", () => {
  assert.equal(typeof DEFAULT_INTERVAL_MS, "number");
  assert.ok(DEFAULT_INTERVAL_MS >= 16 && DEFAULT_INTERVAL_MS <= 200);
});

test("app 的流式渲染接入合并器，终态与收尾都正确落盘", () => {
  const app = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  assert.match(html, /<script src="core\/stream-render\.js"><\/script>/);
  assert.match(app, /createStreamRenderer\(renderMarkdown, \{ onPaint: bindCopyButtons \}\)/);
  assert.match(app, /if \(answerRenderer\) \{\s*answerRenderer\.update\(bubble, assistantText\);/);
  assert.match(app, /if \(answerRenderer\) answerRenderer\.reset\(\);/);
  assert.match(app, /if \(answerRenderer\) answerRenderer\.flush\(\);/);
  // 恢复态（重放历史页）也走同一个合并器，并在中断时落盘。
  assert.match(app, /const renderer = window\.OwoStreamRender/);
  assert.match(app, /if \(renderer\) \{\s*renderer\.update\(bubble, text\);/);
  assert.match(app, /if \(renderer\) renderer\.reset\(\);/);
  assert.match(app, /if \(renderer\) renderer\.flush\(\);/);
});
