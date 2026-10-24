import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, "..");
const source = readFileSync(join(web, "core", "prompt-library.js"), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(source, sandbox);
const library = sandbox.window.OwoPromptLibrary;

const read = (...parts) => readFileSync(join(web, ...parts), "utf8");

test("提示词校验与后端上限一致", () => {
  assert.equal(library.validate("  ", "x").ok, false);
  assert.equal(library.validate("x", "   ").ok, false);
  // vm 里返回的对象原型与宿主不同，deepEqual 会失败，逐字段断言。
  const trimmed = library.validate(" 评审 ", " 先给结论 ");
  assert.equal(trimmed.ok, true);
  assert.equal(trimmed.name, "评审");
  assert.equal(trimmed.content, "先给结论");
  assert.equal(library.validate("a".repeat(61), "x").ok, false);
  assert.equal(library.validate("x", "a".repeat(20001)).ok, false);
  assert.equal(library.validate("x", "a".repeat(20000)).ok, true);
});

test("按正文反查预设，避免把整段文本塞进 chip", () => {
  const presets = [
    { id: "1", name: "严谨评审", content: "先给结论，再列证据。" },
    { id: "2", name: "闲聊", content: "轻松一点。" },
  ];
  assert.equal(library.findActive(presets, " 先给结论，再列证据。 ").id, "1");
  assert.equal(library.chipLabel(presets, "先给结论，再列证据。"), "严谨评审");
  assert.equal(library.chipLabel(presets, ""), "默认提示词");
  assert.equal(library.chipLabel(presets, "未保存的文本"), "自定义提示词");
  assert.equal(library.findActive(presets, ""), null);
});

test("应用提示词：空值转为 null 而不是空串", () => {
  assert.equal(library.applyBody("  ").prompt, null);
  assert.equal(library.applyBody("风格").prompt, "风格");
  assert.equal(library.createSessionPrompt(""), null);
  assert.equal(library.createSessionPrompt("风格"), "风格");
});

test("输入区提示词入口已接线，且服务端先行", () => {
  const html = read("index.html");
  assert.match(html, /id="promptChip"/);
  assert.match(html, /<script src="core\/prompt-library.js"><\/script>/);

  const app = read("app.js");
  assert.match(app, /function openPromptMenu\(\)/);
  assert.match(app, /function openPromptManager\(selectedId\)/);
  assert.match(app, /await api\("\/prompts"\)/, "未拉取提示词库");
  assert.match(
    app,
    /await api\(`\/session\/\$\{state\.sessionId\}\/system-prompt`/,
    "未调用应用接口",
  );
  // 失败必须先于本地显示：不能先改 state 再请求。
  const applyBlock = app.slice(app.indexOf("async function applySystemPrompt"));
  assert.ok(
    applyBlock.indexOf("await api(") < applyBlock.indexOf("state.sessionSystemPrompt = String("),
    "切换提示词必须服务端先行",
  );
  assert.match(app, /state\.pendingSystemPrompt = text;/, "未建会话时应记到下一次创建");

  const domain = read("app-domain.js");
  assert.match(domain, /state\.sessionSystemPrompt = String\(\(detail && detail\.system_prompt\)/, "切会话未同步提示词");
  assert.match(domain, /state\.pendingSystemPrompt\n/, "建会话未带提示词");

  const routing = read("core", "model-routing.js");
  assert.match(routing, /request\.system_prompt = prompt;/, "建会话请求未带 system_prompt");
});

test("提示词库界面不会把内容当成 HTML 注入", () => {
  const app = read("app.js");
  const menuBlock = app.slice(app.indexOf("function openPromptMenu"), app.indexOf("async function applySystemPrompt"));
  assert.match(menuBlock, /esc\(preset\.name\)/, "预设名必须转义");
  assert.match(menuBlock, /esc\(String\(preset\.content/, "预设正文摘要必须转义");
  const css = read("experience.css");
  assert.match(css, /\.prompt-manager \{/);
  // Hermes 式分享：导出复制 JSON，导入按同名覆盖。
  assert.match(app, /已复制提示词 JSON/);
  assert.match(app, /planImport\(text, promptPresetsCache\)/);
  assert.match(read("app.js"), /id="promptImport"/);
  assert.match(css, /\.prompt-manager-import \{/);
  assert.match(css, /\.prompt-list-item\.is-active/);
});

test("导入计划：同名更新而不是重复创建，超限/坏数据显式报错", () => {
  const library = sandbox.window.OwoPromptLibrary;
  const existing = [
    { id: "id-1", name: "严谨评审", content: "旧内容" },
    { id: "id-2", name: "苏格拉底", content: "提问式" },
  ];
  const text = JSON.stringify({
    version: 1,
    prompts: [
      { name: "严谨评审", content: "新内容" },
      { name: "学习教练", content: "先例子再定义" },
      { name: "学习教练", content: "重复条目被忽略" },
    ],
  });
  const plan = library.planImport(text, existing);
  assert.equal(plan.ok, true);
  assert.equal(plan.updated, 1);
  assert.equal(plan.created, 1);
  assert.equal(plan.entries.length, 2);
  assert.equal(plan.entries[0].id, "id-1");
  assert.equal(plan.entries[0].content, "新内容");
  assert.equal(plan.entries[1].id, "");

  // 裸数组也接受。
  assert.equal(library.planImport(JSON.stringify([{ name: "A", content: "a" }]), []).ok, true);
  // 非法输入有明确文案。
  assert.match(library.planImport("", []).error, /粘贴/);
  assert.match(library.planImport("{oops", []).error, /JSON/);
  assert.match(library.planImport('{"prompts":[]}', []).error, /没有可导入/);
  assert.match(library.planImport(JSON.stringify([{ name: "", content: "x" }]), []).error, /第 1 条/);
  // 上限：现有 50 条时不能再新增（更新仍允许）。
  const full = Array.from({ length: 50 }, (_, i) => ({ id: `i${i}`, name: `n${i}`, content: "c" }));
  assert.match(
    library.planImport(JSON.stringify([{ name: "新", content: "c" }]), full).error,
    /超出上限/,
  );
  assert.equal(
    library.planImport(JSON.stringify([{ name: "n0", content: "改" }]), full).ok,
    true,
  );
});

test("导出只含名称与内容，可被导入计划还原", () => {
  const library = sandbox.window.OwoPromptLibrary;
  const text = library.serialize([
    { id: "x", name: "严谨评审", content: "先结论", updated_at: "2026" },
    { name: "", content: "无效条目被跳过" },
  ]);
  const parsed = JSON.parse(text);
  assert.equal(parsed.version, 1);
  assert.equal(parsed.prompts.length, 1);
  assert.equal(parsed.prompts[0].name, "严谨评审");
  assert.equal(parsed.prompts[0].id, undefined);
  assert.equal(library.planImport(text, []).entries.length, 1);
});
