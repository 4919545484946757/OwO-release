import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { createHash } from "node:crypto";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const css = readFileSync(new URL("../frontend-visual-system.css", import.meta.url), "utf8");
const ws = readFileSync(new URL("../panels/workswarm.panel.js", import.meta.url), "utf8");
const launcher = readFileSync(new URL("../panels/project-launcher.panel.js", import.meta.url), "utf8");
const capture = readFileSync(new URL("../../../docs/qa/evidence/frontend-optimization-20261112/capture.mjs", import.meta.url), "utf8");
const rawPrefix = html.slice(0, html.indexOf('  <aside id="right"'));
const prefix = rawPrefix.replace('  <link rel="stylesheet" href="task-details.css?v=20261101-restore">\n', "");

test("visual layer loads after the frozen home prefix and before app scripts", () => {
  assert.match(html, /<link rel="stylesheet" href="frontend-visual-system\.css\?v=20261118-1">/);
  assert.ok(html.indexOf('<link rel="stylesheet" href="frontend-visual-system.css') > html.indexOf('  <aside id="right"'));
  assert.ok(html.indexOf('<link rel="stylesheet" href="frontend-visual-system.css') < html.indexOf('<script src="app.js"'));
  assert.equal(createHash("sha256").update(prefix).digest("hex"), "9a47eb62418b955c787ac7785965d6aed6933adca97bf54e9031ee0e1c8d4293");
});

test("one workspace wordmark remains while duplicate and redundant chrome stays hidden", () => {
  const wordmark = new URL("../assets/owo-wordmark-clean.png", import.meta.url);
  assert.ok(existsSync(wordmark));
  assert.match(html, /class="menubar-brand menubar-wordmark"/);
  assert.match(html, /src="assets\/owo-wordmark-clean\.png\?v=20261113-2"[^>]*width="82"/);
  assert.match(html, /class="codex-workspace-wordmark-img"[^>]*width="96"/);
  assert.match(css, /#menubar #sidebarCollapseBtn,[\s\S]*?#menubar \.menubar-side > \.menubar-brand\.menubar-wordmark \{\s*display: none !important;/);
  assert.match(css, /#sidebar #sidebarBrandBtn,[\s\S]*?display: none !important;/);
  assert.match(css, /#sidebar \.codex-workspace-wordmark-img \{\s*width: 68px !important;/);
  assert.match(css, /#menubar \.menubar-right #connectionDetails,[\s\S]*?#menubarTheme \{\s*display: none !important;/);
  assert.match(css, /#emptyState \.codex-empty-logo,[\s\S]*?display: none !important;/);
  // 2026-11-15 字体层级：分组 14/600、项目 15/500、会话 14/400、时间 12/400。
  assert.match(css, /#sidebar \.codex-sessions \.owo-session-title strong \{\s*font-size: 14px !important;/);
  assert.match(css, /#sidebar \.codex-sidebar > #sidebarNewChat\.workbench-create-task \{[\s\S]*?font-size: 15px !important;/);
  assert.match(css, /#sidebar \.codex-sessions \.codex-group-label \{\s*font-size: 15px !important;/);
  assert.match(css, /--font-ui: Inter, "Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", system-ui, sans-serif;/);
  assert.match(css, /--text-primary: #202124;\s*--text-secondary: #5F6670;\s*--text-tertiary: #9299A3;\s*--text-disabled: #B5BAC2;/);
  assert.match(css, /\.codex-group-label \{[^}]*font-size: 15px !important;[^}]*font-weight: 500 !important;/);
  assert.match(css, /\.owo-session-title \.sub \{[^}]*font-size: 12px !important;/);
  // 模型与推理合并为单一控件：模型名始终可见，独立「推理」chip 不再占位。
  assert.doesNotMatch(css, /#modelChip\s*\{\s*width: 36px/);
  assert.doesNotMatch(css, /#modelChip #modelChipText,[\s\S]*?display: none !important;/);
  assert.match(html, /id="effortChip"[^>]*hidden/);
  assert.match(css, /#chatForm#chatForm #effortChip \{\s*display: none !important;/);
  assert.match(html, /id="modelChip"[^>]*data-tip="选择模型与思考强度"/);
});

test("enhance prompt is a one-click in-place rewrite with real model calls", () => {
  const ui = readFileSync(new URL("../core/prompt-polish-ui.js", import.meta.url), "utf8");
  const shell = readFileSync(new URL("../../electron/src/main/prompt-enhancer.js", import.meta.url), "utf8");
  assert.match(html, /id="promptPolishBtn"[^>]*data-prompt-polish="prompt"[^>]*data-tip="增强提示词"/);
  assert.doesNotMatch(html, /id="promptPolishBtn"[^>]*aria-haspopup="dialog"/);
  assert.match(html, /class="prompt-polish-mini"[^>]*data-prompt-polish="subagentPrompt"/);
  assert.match(launcher, /data-prompt-polish="pl-objective"/);
  // 主路径是核心服务的结构化增强端点；本地固定模板补全器不再加载。
  assert.match(html, /<script src="core\/prompt-polish-ui\.js"><\/script>/);
  assert.doesNotMatch(html, /<script src="core\/prompt-polisher\.js"><\/script>/);
  assert.match(ui, /OwoApi\.request\("\/prompt\/enhance"/);
  assert.doesNotMatch(ui, /OwoPromptPolisher/);
  // 点击直接回填 / 再点还原，不存在重型弹窗、编辑器或撤回胶囊。
  assert.match(ui, /fill\(target, enhanced, entry\)/);
  assert.match(ui, /fill\(target, entry\.original, entry\)/);
  // 旧核心（404）才回落壳层 IPC——同样是真实模型调用，不是模板拼接。
  assert.match(ui, /Number\(error\.status\) !== 404/);
  assert.match(ui, /global\.owo\.enhancePrompt/);
  assert.doesNotMatch(ui, /createPanel|pp-editor|prompt-polish-pop/);
  assert.doesNotMatch(css, /\.prompt-polish-pop \{/);
  assert.doesNotMatch(css, /\.pp-editor \{/);
  assert.doesNotMatch(css, /\.prompt-polish-undo \{/);
  assert.match(css, /#promptPolishBtn\[data-enhance="on"\]/);
  // 澄清问答：needs_clarification 在输入框下方展开轻量面板，不替换原文、不自动发送。
  assert.match(ui, /needs_clarification/);
  assert.match(ui, /renderClarifyPanel/);
  assert.match(ui, /原始输入保持不变；答案只用于本次增强/);
  assert.match(css, /\.prompt-enhance-panel \{/);
  // 增强期间用户改动草稿 → 结果只做预览，不覆盖新输入。
  assert.match(ui, /target\.value !== baseline/);
  assert.match(ui, /renderPreviewPanel/);
  // 兼容壳层仍是"保留原意与语言、只输出提示词正文"的真实改写。
  assert.match(shell, /必须使用与原文相同的语言/);
  assert.match(shell, /直接输出改写后的提示词正文/);
});

test("hover reveals content directly without an extra trigger button", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(app, /悬浮约 90ms 直接显示相关内容/);
  assert.match(app, /\}, 90\);/);
  assert.match(app, /window\.OwoTooltip = \{/);
  assert.match(app, /attributeFilter: \["title", "data-tip"\]/);
  assert.doesNotMatch(app, /bindHoverMenu/);
  assert.match(css, /\.owo-tooltip\[data-side="above"\]/);
  assert.match(html, /id="attachmentBtn" data-tip="添加附件"/);
  assert.match(html, /id="sendBtn" class="codex-send" data-tip="发送"/);
});

test("spec token bridge stays scoped and honors both themes", () => {
  // 2026-11-17 面板字阶硬规格（12/13/14/16/20）：页标题 22→20、分组 17→16、正文 15→14，
  // 行高随字阶同步；其余断言不变。
  assert.match(css, /--fs-page-title: 20px;/);
  assert.match(css, /--lh-page-title: 28px;/);
  assert.match(css, /--fs-section-title: 16px;/);
  assert.match(css, /--fs-body: 14px;/);
  assert.match(css, /--fs-control: 14px;/);
  assert.match(css, /--fs-caption: 13px;/);
  assert.match(css, /--rail-w: 55px;/);
  assert.match(css, /grid-template-columns: 55px minmax\(0, 1fr\) !important;/);
  assert.match(css, /width: 42px !important;\s*height: 42px !important;/);
  assert.match(css, /#sidebar #sessionList\.codex-sessions \{\s*overflow-x: hidden !important;/);
  assert.match(css, /--focus-ring: #8A9AAE;/);
  assert.match(css, /--state-success: #23865B;/);
  assert.match(css, /--shadow-pop: 0 8px 28px rgba\(16, 24, 23, \.10\);/);
  assert.match(css, /body\.dark-theme #panelRoot#panelRoot/);
  assert.doesNotMatch(css, /^body\s*\{[^}]*--surface:/m);
  assert.match(css, /prefers-reduced-motion: reduce/);
  assert.match(css, /@media \(min-width: 1600px\)/);
  assert.match(css, /--align-composer-w: clamp\(820px, 52vw, 1040px\)/);
  assert.match(css, /body\.empty-conversation:not\(\.tools-open\):not\(\.settings-open\):not\(\.panel-open\)/);
  assert.match(css, /#chatForm#chatForm #prompt#prompt#prompt[\s\S]*min-height: 56px !important/);
  assert.match(css, /max-width: var\(--align-composer-w\) !important/);

  assert.match(css, /\.composer-menu\.composer-menu-model \{[^}]*width: min\(320px,/);
  assert.match(css, /\.composer-model-details \{[^}]*position: absolute;/);
  assert.match(css, /\.composer-menu-model\[data-detail-side="inline"\] \.composer-model-details \{[^}]*position: static;/);
  assert.match(css, /#chatForm#chatForm#chatForm,[\s\S]*?border-radius: 22px !important/);
  assert.match(css, /#sendBtn\.codex-send[\s\S]*?border-radius: 50% !important/);
  // 底部工具栏与输入框同宽、贴合下沿，与输入框合成一张卡片，背景保持浅中性灰。
  assert.match(css, /#composerContextTray\.composer-context-tray \{[^}]*width: min\(var\(--align-composer-w\), calc\(100% - 64px\)\) !important;[^}]*max-width: var\(--align-composer-w\) !important;[^}]*margin: 0 auto !important;[^}]*background: var\(--home-tray-bg\) !important;[^}]*border: 1px solid var\(--align-line\) !important;[^}]*border-radius: 0 0 22px 22px !important;[^}]*box-shadow: none !important;/);
  assert.doesNotMatch(css, /composer-context-tray[\s\S]*?border-radius: 15px !important/);
  assert.match(css, /#composerContextTray \.context-tray-item:focus-visible \{/);
  assert.match(css, /#composerContextTray \.context-tray-item:is\(:disabled, \[aria-disabled="true"\]\)/);
  assert.match(css, /prompt-tools > #accessChip \{[\s\S]*?min-width: max-content !important/);
  assert.match(css, /body:not\(\.empty-conversation\)[\s\S]*?#effortChip \{[\s\S]*?display: none !important/);

  assert.match(css, /#workbenchPanelTitle#workbenchPanelTitle \{[^}]*font: 600 var\(--fs-page-title\) \/ var\(--lh-page-title\) var\(--font-display\) !important;/);
  assert.match(css, /strong:not\(\[class\]\) \{[^}]*font: 600 var\(--fs-page-title\) \/ var\(--lh-page-title\) var\(--font-display\) !important;/);
  assert.match(css, /#workbenchPanelTitle#workbenchPanelTitle \{[^}]*font: 600 var\(--fs-page-title\) \/ var\(--lh-page-title\) var\(--font-display\) !important;/);
  assert.match(css, /strong:not\(\[class\]\) \{[^}]*font: 600 var\(--fs-page-title\) \/ var\(--lh-page-title\) var\(--font-display\) !important;/);
});

test("team table, role cards and attachment states preserve real DOM contracts", () => {
  assert.match(ws, /<div class="hint">暂无团队/);
  assert.match(ws, /<div class="owo-ws-tablewrap"><table class="owo-ws-table">/);
  assert.match(ws, /data-team="'\s*\+\s*esc\(x\.team_id\)/);
  assert.match(css, /#ws-list-body \{ max-width: 100%; overflow-x: auto;/);
  assert.match(css, /\.owo-ws-table\.owo-ws-table \{ min-width: 900px;/);
  assert.match(css, /\[data-task-inputs\] fieldset/);
});

test("before/after evidence harness keeps fixed conditions and source identity", () => {
  assert.match(capture, /before|after/);
  assert.match(capture, /noFakeData: true/);
  assert.match(capture, /frozenHomePrefixSha256/);
  assert.match(capture, /PANELS = \[/);
  assert.match(capture, /SETTINGS = \[/);
  assert.match(capture, /deviceScaleFactor/);
});

