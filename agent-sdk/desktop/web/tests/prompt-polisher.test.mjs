import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const Polisher = require("../core/prompt-polisher.js");

const COMPLETE_ZH =
  "你是资深前端工程师。背景：现有 React 18 + Vite 项目，登录页在 360px 宽度下按钮溢出。" +
  "要求：只改 CSS，不引入新依赖。输出：改动前后的样式代码和文件路径。" +
  "验收：360px 下无横向滚动，现有测试通过。";

const COMPLETE_EN =
  "Write a product announcement for a new note-taking app. Audience: busy students. " +
  "Output: 3 short paragraphs. Acceptance: no hype, clear benefits.";

test("empty input stays empty and never fabricates a prompt", () => {
  const result = Polisher.optimize("   \n  ");
  assert.equal(result.status, "empty");
  assert.equal(result.prompt, "");
  assert.equal(result.scoreAfter, 0);
  assert.deepEqual(Array.from(result.notes), []);
});

test("too-short Chinese input returns a fill-in starter instead of padded prose", () => {
  const result = Polisher.optimize("帮我改改");
  assert.equal(result.status, "too_short");
  assert.equal(result.language, "zh");
  assert.match(result.prompt, /^我要完成：/);
  assert.match(result.prompt, /验收：/);
  assert.equal(result.prompt.includes("帮我改改"), false);
  assert.ok(result.notes.includes("未扩写，避免套话"));
  assert.ok(result.template);
});

test("too-short English input uses an English starter", () => {
  const result = Polisher.optimize("help me");
  assert.equal(result.status, "too_short");
  assert.equal(result.language, "en");
  assert.match(result.prompt, /^Goal: /);
  assert.doesNotMatch(result.prompt, /我要完成/);
});

test("Chinese prompt keeps its original sentence and adds Chinese-only scaffolding", () => {
  const original = "帮我写一封给客户的道歉邮件，说明发货延迟，语气诚恳，300字以内。";
  const result = Polisher.optimize(original);
  assert.equal(result.status, "ok");
  assert.equal(result.language, "zh");
  assert.ok(result.prompt.startsWith(original));
  assert.match(result.prompt, /## 执行要求/);
  assert.match(result.prompt, /## 输出/);
  assert.match(result.prompt, /## 验收/);
  assert.doesNotMatch(result.prompt, /## Requirements|## Output|## Acceptance/);
  assert.ok(result.scoreAfter > result.scoreBefore);
});

test("English prompt keeps its language and uses English scaffolding", () => {
  const original =
    "Write a short onboarding email for a project management tool aimed at small design studios.";
  const result = Polisher.optimize(original);
  assert.equal(result.status, "ok");
  assert.equal(result.language, "en");
  assert.ok(result.prompt.startsWith(original));
  assert.match(result.prompt, /## Requirements/);
  assert.doesNotMatch(result.prompt, /## 执行要求|## 输出|## 验收/);
});

test("a highly complete prompt is returned unchanged", () => {
  const result = Polisher.optimize(COMPLETE_ZH);
  assert.equal(result.status, "complete");
  assert.equal(result.prompt, COMPLETE_ZH);
  assert.ok(result.notes.includes("目标明确"));
  assert.ok(result.scoreBefore >= Polisher.COMPLETE_SCORE);
});

test("English complete prompt is also returned unchanged", () => {
  const result = Polisher.optimize(COMPLETE_EN);
  assert.equal(result.status, "complete");
  assert.equal(result.prompt, COMPLETE_EN);
});

test("force mode still offers a structured version of a complete prompt", () => {
  const result = Polisher.optimize(COMPLETE_ZH, { force: true });
  assert.equal(result.status, "ok");
  assert.ok(result.prompt.startsWith(COMPLETE_ZH));
  assert.notEqual(result.prompt, COMPLETE_ZH);
  assert.match(result.prompt, /## 执行要求/);
});

test("structured prompts are augmented instead of being rewritten", () => {
  const original = [
    "## 目标",
    "修复登录页在窄窗口下的按钮溢出。",
    "",
    "## 要求",
    "- 只改 CSS",
    "- 不引入新依赖",
    "",
    "## 输出",
    "- 给出改动前后的代码和文件路径",
  ].join("\n");
  const result = Polisher.optimize(original);
  assert.equal(result.status, "ok");
  assert.ok(result.prompt.startsWith(original));
  assert.match(result.prompt, /## 补充要求/);
  assert.equal((result.prompt.match(/## 目标/g) || []).length, 1);
  assert.equal((result.prompt.match(/## 输出/g) || []).length, 1);
  assert.match(result.prompt, /验收：/);
});

test("intent-specific output and acceptance are added for code tasks", () => {
  const result = Polisher.optimize("修复登录接口在空参数时返回 500 的报错。");
  assert.equal(result.intent, "code");
  assert.match(result.prompt, /验证命令/);
  assert.match(result.prompt, /边界情况/);
});

test("no domain facts are invented for a generic request", () => {
  const result = Polisher.optimize("帮我写一段产品介绍。");
  assert.equal(result.prompt.includes("React"), false);
  assert.equal(result.prompt.includes("Vite"), false);
  assert.equal(result.prompt.includes("360px"), false);
  assert.match(result.prompt, /不编造事实/);
});

test("numbers, paths and technical names survive optimization verbatim", () => {
  const original = "在 src/pages/login.tsx 中修复 360px 宽度下的溢出，保持 React 18 写法。";
  const result = Polisher.optimize(original);
  assert.ok(result.prompt.includes("src/pages/login.tsx"));
  assert.ok(result.prompt.includes("360px"));
  assert.ok(result.prompt.includes("React 18"));
});

test("re-optimizing an optimized prompt does not duplicate sections", () => {
  const first = Polisher.optimize("分析这份销售数据，找出下降原因。");
  const second = Polisher.optimize(first.prompt);
  assert.ok(second.status === "complete" || second.status === "ok");
  assert.equal((second.prompt.match(/## 执行要求/g) || []).length <= 1, true);
  assert.equal((second.prompt.match(/## 补充要求/g) || []).length <= 1, true);
});

test("analysis exposes the dimensions used by the UI score badge", () => {
  const empty = Polisher.analyze("");
  assert.equal(empty.empty, true);
  assert.equal(empty.tooShort, false);
  assert.deepEqual(Array.from(empty.missing), ["goal", "context", "constraints", "format", "criteria"]);

  const vague = Polisher.analyze("帮我看看这个");
  assert.equal(vague.tooShort, true);
  assert.equal(vague.complete, false);

  const complete = Polisher.analyze(COMPLETE_ZH);
  assert.equal(complete.complete, true);
  assert.equal(complete.present.goal, true);
  assert.equal(complete.present.format, true);
  assert.equal(complete.present.criteria, true);
});

test("scaffold is deterministic and language-matched", () => {
  assert.match(Polisher.scaffold("zh"), /我要完成：/);
  assert.match(Polisher.scaffold("en"), /^Goal: /);
  assert.equal(Polisher.scaffold("zh"), Polisher.scaffold("zh"));
});
