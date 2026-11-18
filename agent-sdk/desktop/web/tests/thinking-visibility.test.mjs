import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// 仓库里部分文件是 CRLF：断言按逻辑行匹配，先归一化行尾再读。
const read = (name) => readFileSync(new URL(`../${name}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const app = read("app.js");
const domain = read("app-domain.js");
const sse = read("core/turn-sse.js");
const experience = read("experience.css");
const finalLayer = read("frontend-visual-system.css");

const sliceFn = (source, header) => {
  const start = source.indexOf(header);
  assert.ok(start >= 0, `缺少 ${header}`);
  const end = source.indexOf("\n}\n", start);
  return source.slice(start, end < 0 ? source.length : end);
};

test("thinking text is never silently truncated", () => {
  // 旧实现有 30000 字硬上限，超出部分只计数不渲染，用户看到的是半句思考。
  assert.doesNotMatch(app, /THINKING_RENDER_LIMIT/);
  assert.doesNotMatch(app, /仅展示前/);
  assert.doesNotMatch(app, /chars > THINKING/);
  assert.match(app, /const THINKING_FLUSH_MS = 120;/);
  // 批量落 DOM 仍然保留（每个增量一次重排会拖垮长回合）。
  assert.match(app, /block\.body\.appendChild\(document\.createTextNode\(block\.pending\)\)/);
});

test("replayed reasoning replaces the block instead of appending twice", () => {
  const push = sliceFn(app, "function pushReasoning(delta, options)");
  assert.match(push, /options\.replace/);
  assert.match(push, /block\.body\.textContent = delta;/);
  assert.match(domain, /pushReasoning\(payload\.delta \|\| "", \{ replace: Boolean\(meta && meta\.replay\) \}\)/);
  assert.match(sse, /onEvent\(type, payload, \{ id: hasSeq \? sequence : null, replay \}\)/);
});

test("a turn keeps one thinking block and pauses it while tools run", () => {
  const tool = sliceFn(app, "function pushToolUse(payload)");
  assert.match(tool, /pauseThinking\(\)/);
  assert.doesNotMatch(tool, /finishThinking\(\)/);

  const pause = sliceFn(app, "function pauseThinking()");
  assert.match(pause, /工具执行中/);
  assert.match(pause, /flushThinkingPending/);
  // 实时思考块默认展开，结束后由 finishThinking 自动收起。
  assert.match(app, /el\.open = true;/);
  assert.match(sliceFn(app, "function finishThinking()"), /if \(block\.autoOpened && !block\.manual\) block\.el\.open = false;/);
});

test("history replay renders persisted reasoning before the answer", () => {
  assert.match(app, /function addHistoricalThinking\(text\)/);
  assert.match(domain, /message\.reasoning_replay && typeof message\.reasoning_replay\.content === "string"/);
  assert.match(domain, /if \(replayText && \(message\.content \|\| calls\.length\)\) addHistoricalThinking\(replayText\);/);
  // 思考必须排在工具轨迹之前：先想、再调工具、再回答。
  const body = domain.slice(domain.indexOf("message.role === \"assistant\""));
  assert.ok(
    body.indexOf("addHistoricalThinking(replayText)") < body.indexOf("appendHistoryToolSteps(calls"),
    "历史思考块应先于工具步骤出现",
  );
});

test("tool steps report real execution state, not rebuild time", () => {
  const step = sliceFn(app, "function createToolStep(payload = {})");
  assert.match(step, /const historical = Boolean\(payload\.historical\)/);
  assert.match(step, /historical \? \(ok \? "" : "失败"\)/);
  assert.match(app, /historical: true,/);
});

test("the agent avatar is a single mark, not a two-dot glyph", () => {
  const avatar = experience.match(/--agent-avatar: url\("data:image\/svg\+xml,([\s\S]*?)"\);/);
  assert.ok(avatar, "缺少 --agent-avatar 定义");
  // 24px 下两个圆点会被读成「99」；头像只保留一个居中标记。
  assert.doesNotMatch(avatar[1], /<circle/);
  assert.match(avatar[1], /viewBox='0 0 24 24'/);
  assert.match(finalLayer, /#messages \.thinking-block \.thinking-body \{/);
  assert.match(finalLayer, /font-variant-numeric: tabular-nums/);
});
