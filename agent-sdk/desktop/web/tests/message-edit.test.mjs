import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const domain = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../experience.css", import.meta.url), "utf8");
const baseCss = readFileSync(new URL("../style.css", import.meta.url), "utf8");

test("历史用户消息带上 rewind 所需的原始索引", () => {
  assert.match(
    domain,
    /for \(let messageIndex = 0; messageIndex < history\.length; messageIndex \+= 1\) \{\s*const message = history\[messageIndex\];/
  );
  assert.match(
    domain,
    /const bubble = addMessage\(message\.role === "user" \? "user" : "assistant", message\.content\);/
  );
  assert.match(
    domain,
    /attachUserMessageActions\(bubble, \{ sessionId: id, messageIndex, content: storedContent \}\)/
  );
  // keep 是「保留多少条」，即该用户消息在 history 中的下标。
  assert.match(domain, /body: JSON\.stringify\(\{ keep \}\)/);
});

test("编辑消息使用多行弹窗并转义原文", () => {
  assert.match(app, /function askMultiline\(\{ title = "编辑消息"/);
  assert.match(app, /<textarea id="modalMultilineInput" rows="8"/);
  assert.match(app, /\$\{esc\(value\)\}<\/textarea>/);
  assert.match(app, /onClose: \(\) => \{\s*if \(!done\) resolve\(null\);\s*\}/);
  // 空内容提交要像 promptModal 一样给出浏览器校验提示，而不是静默不响应。
  assert.match(app, /input\.setCustomValidity\("请输入内容。"\);/);
  assert.match(app, /input\.reportValidity\(\);/);
  assert.match(app, /addEventListener\("input", \(\) => input\.setCustomValidity\(""\)\)/);
  // 回车在 textarea 里必须是换行，不能被当成提交。
  assert.doesNotMatch(
    app,
    /askMultiline[\s\S]{0,4000}?keydown[\s\S]{0,200}?Enter/
  );
});

test("编辑重发先 rewind、重载历史，再发送新内容", () => {
  assert.match(domain, /function attachUserMessageActions\(node, \{ sessionId, messageIndex, content \}\)/);
  assert.match(domain, /if \(state\.sessionId !== sessionId\) \{\s*showToast\("请先切回该会话再编辑消息"\);/);
  assert.match(domain, /if \(currentTurn\(\)\) \{\s*showToast\("此会话仍有任务在运行，结束后再编辑"\);/);
  // 回退/重载/发送收敛到共用入口，避免编辑与重新生成各写一套。
  assert.match(domain, /await rewindAndResend\(sessionId, messageIndex, next\);/);
  assert.match(domain, /api\(`\/session\/\$\{sessionId\}\/rewind`, \{/);
  assert.match(domain, /await selectSession\(sessionId, true\)/);
  assert.match(domain, /\$\("prompt"\)\.value = content;/);
  assert.match(domain, /await sendPrompt\(\)/);
  // 失败不能只留在控制台，要显式进入时间线。
  assert.match(domain, /addEventChip\("system", "编辑重发失败", detail\)/);
});

test("编辑前说明会回退对话与文件变更，并保护输入区草稿", () => {
  assert.match(domain, /title: "回退并重发"/);
  assert.match(domain, /将回退到这条消息之前，撤销它之后回合产生的文件变更。/);
  assert.match(domain, /输入区现有草稿会被替换。/);
  assert.match(domain, /kind: "danger"/);
  assert.match(domain, /const next = await askMultiline\(\{/);
});

test("用户消息提供复制操作，操作区在气泡内右对齐", () => {
  assert.match(domain, /copy\.textContent = "复制";/);
  assert.match(domain, /copy\.title = "复制这条消息（Markdown 原文）";/);
  assert.match(domain, /edit\.textContent = "编辑";/);
  assert.match(domain, /actions\.append\(copy, edit\)/);
  assert.match(css, /\.msg\.user \.msg-actions \{ justify-content: flex-end; \}/);
  // 触屏无 hover：操作入口不能永远停在 opacity: 0。
  assert.match(css, /@media \(hover: none\) \{\s*\.msg \.msg-actions \{ opacity: 1; \}/);
  assert.match(baseCss, /#modalBody textarea \{/);
  assert.match(baseCss, /resize: vertical;/);
});

test("回退后没有用户消息时清掉旧任务缓存，避免重试已删除回合", () => {
  assert.match(
    domain,
    /if \(previousInput\) state\.lastSessionPrompts\.set\(id, String\(previousInput\.content\)\);\s*else state\.lastSessionPrompts\.delete\(id\);/
  );
});

test("最近一条正常回答提供重新生成，并复用 rewind 重发", () => {
  assert.match(domain, /let lastUserTurn = null;\s*let lastAssistantBubble = null;/);
  assert.match(domain, /lastUserTurn = \{ messageIndex, content: storedContent \};/);
  assert.match(domain, /lastAssistantBubble = bubble;/);
  // 带工具调用的 assistant 不是可重发终点，避免在工具链中间重开回合。
  assert.match(
    domain,
    /!\(Array\.isArray\(message\.tool_calls\) && message\.tool_calls\.length\)/
  );
  assert.match(
    domain,
    /if \(lastRole === "assistant" && lastAssistantBubble && lastUserTurn && !currentTurn\(\)\) \{\s*attachRegenerateAction\(lastAssistantBubble, id, lastUserTurn\);/
  );
  assert.match(domain, /function attachRegenerateAction\(node, sessionId, userMessage\)/);
  assert.match(domain, /button\.textContent = "重新生成";/);
  assert.match(domain, /rewindAndResend\(sessionId, userMessage\.messageIndex, userMessage\.content\)/);
  // 失败占位不能把按钮挂到更早的旧回答上。
  assert.match(domain, /addHistoricalTurnFailure\(storedContent\);\s*\/\/[^\n]*\n\s*lastAssistantBubble = null;/);
});

test("编辑与重新生成共用一个回退入口，并在会话被切走后停止发送", () => {
  assert.match(domain, /async function rewindAndResend\(sessionId, keep, content\)/);
  assert.match(domain, /if \(state\.sessionId !== sessionId\) \{\s*showToast\("会话已切换，未执行回退", "error"\);/);
  assert.match(domain, /showToast\("会话已切换；回退已生效，请回到该会话重发", "error"\)/);
  assert.match(domain, /await selectSession\(sessionId, true\);/);
  assert.match(domain, /await sendPrompt\(\);/);
});
