import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const domain = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../chatgpt.css", import.meta.url), "utf8");
const mode = readFileSync(new URL("../core/session-mode.js", import.meta.url), "utf8");

test("顶层只提供对话 / Work 两个模式，且切换控件在页面主区", () => {
  assert.match(html, /id="modeSwitch" class="mode-switch" role="tablist"/);
  const buttons = html.match(/data-mode-switch="/g) || [];
  assert.equal(buttons.length, 2, "模式只能有对话与 Work 两个选项");
  assert.match(html, /data-mode-switch="chat" aria-selected="true">对话</);
  assert.match(html, /data-mode-switch="agent" aria-selected="false">Work</);
  assert.match(mode, /label: "对话"/);
  assert.match(mode, /label: "Work"/);
  assert.match(app, /function refreshModeSwitch\(mode\)/);
  assert.match(app, /setSessionMode\(button\.dataset\.modeSwitch\)/);
  assert.match(css, /\.mode-switch \{/);
});

test("左上入口改成「新对话」，并直接新建会话而不是打开任务面板", () => {
  assert.match(html, /id="newChatTop" class="primary workbench-create-task"[\s\S]{0,200}<span>新对话<\/span>/);
  assert.doesNotMatch(html, /id="newChatTop"[^>]*data-workbench-panel/);
  assert.match(app, /\$\("newChatTop"\)\?\.addEventListener\("click", \(\) => \$\("newSession"\)\.click\(\)\)/);
});

test("侧栏改为 ChatGPT 式图标栏：核心功能保留，右侧专注工作区与会话", () => {
  const railCss = readFileSync(new URL("../sidebar-rail.css", import.meta.url), "utf8");
  assert.match(html, /chatgpt\.css[\s\S]{0,160}sidebar-rail\.css/, "图标栏样式必须最后加载");
  assert.match(html, /<nav class="codex-rail" aria-label="主导航">/);
  for (const panel of ["home", "project-history", "workswarm", "evidence", "action-center"]) {
    assert.match(html, new RegExp('data-workbench-panel="' + panel + '"'));
  }
  assert.match(html, /id="railNewChat"/);
  assert.match(html, /id="sidebarSearchBtn"[\s\S]{0,200}aria-label="搜索会话"/);
  assert.match(html, /<span class="codex-sessions-head">最近对话<\/span>/);
  assert.match(html, /<ul id="sessionList" class="list codex-sessions"><\/ul>/);
  assert.match(railCss, /#sidebar \{[\s\S]{0,80}flex-direction: row/);
  assert.match(railCss, /\.codex-rail \{[\s\S]{0,160}flex-direction: column/);
  assert.match(railCss, /body\.tools-open\.panel-open \.codex-rail \{[\s\S]{0,80}display: flex !important/);
  assert.doesNotMatch(html, /class="workbench-shortcuts"/, "功能入口不再堆在会话栏");
  assert.doesNotMatch(html, /class="codex-nav"/, "分类导航收进更多功能");
  assert.match(app, /\$\("railNewChat"\)\?\.addEventListener\("click", \(\) => \$\("newSession"\)\.click\(\)\)/);
  assert.match(domain, /#sidebar \.codex-rail \[data-workbench-panel\]/);
});

test("对话示例移到输入框下方，默认隐藏且点击只填入不发送", () => {
  assert.match(html, /id="chatStarters" class="chat-starters home-suggestions hidden"/);
  assert.match(html, /id="workStarters" class="chat-starters home-suggestions hidden"/);
  assert.equal((html.match(/data-chat-starter="/g) || []).length, 4);
  assert.equal((html.match(/data-work-starter="/g) || []).length, 3);
  assert.equal((html.match(/data-workflow-brief="/g) || []).length, 1);
  assert.match(css, /\.home-suggestions \{[\s\S]*order: 4;[\s\S]*grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/);
  assert.match(css, /body:has\(#emptyState\.hidden\) \.home-suggestions \{ display: none; \}/);
  assert.match(app, /const chip = event\.target\.closest\("\[data-chat-starter\]"\)/);
  assert.match(app, /input\.value = chip\.dataset\.chatStarter \|\| ""/);
  assert.match(app, /input\.dispatchEvent\(new Event\("input", \{ bubbles: true \}\)\)/);
  assert.doesNotMatch(app, /data-chat-starter[\s\S]{0,400}requestSubmit/);
  // Work 的「一句话生成工作流」直接打开画布并聚焦，不把长度有限的建议当任务发送。
  assert.match(app, /const workflow = event\.target\.closest\("\[data-workflow-brief\]"\)/);
  assert.match(app, /openPanelById\("custom-workflows"\)/);
  assert.match(app, /panel\.focusBrief\(brief\)/);
  assert.doesNotMatch(app, /data-workflow-brief[\s\S]{0,400}requestSubmit/);
});

test("模式切换驱动两套空状态与两套建议", () => {
  assert.match(app, /function syncChatStarters\(mode\)/);
  assert.match(app, /starters\.classList\.toggle\("hidden", !chat\)/);
  assert.match(app, /workStarters\.classList\.toggle\("hidden", chat\)/);
  assert.match(app, /title: "今天想聊点什么？"/);
  assert.match(app, /title: "今天要一起完成什么？"/);
  assert.match(app, /提问、解释、查资料。对话模式不会读写本机文件，也不会执行命令。/);
  assert.match(app, /if \(typeof syncChatStarters === "function"\) syncChatStarters\(mode\);/);
});

test("默认进入对话模式，刷新后不用先选工作区", () => {
  assert.match(app, /sessionMode: "chat"/);
  assert.match(app, /const MODE_PREF_KEY = "owo\.homeMode\.v2";/);
  // parse("") 会归一成 agent；没有原始偏好时必须保留默认 chat。
  assert.match(app, /const raw = localStorage\.getItem\(MODE_PREF_KEY\);/);
  assert.match(app, /const saved = raw && window\.OwoSessionMode/);
  assert.match(html, /<h2 id="emptyStateTitle">今天想聊点什么？<\/h2>/);
  assert.match(html, /提问、解释、查资料。对话模式不会读写本机文件，也不会执行命令。/);
});

test("对话不选工作区也能开始，Work 仍先要求目录", () => {
  assert.match(domain, /if \(!workspace && mode !== "chat"\) \{/);
  assert.match(domain, /const chatMode = typeof currentSessionMode === "function" && currentSessionMode\(\) === "chat";/);
  assert.match(domain, /!chatMode\) \{\s*openWorkspaceMenu/);
  assert.match(app, /const displayName = name \|\| \(chat \? "对话" : "未选择项目"\);/);
  assert.match(app, /const needsWorkspace = !value && !state\.sessionId && !chat;/);
  assert.match(app, /对话不使用本地工作区（可点击选择）/);
});

test("侧栏把默认 chat 分组标成对话，且不提供建 Work 会话的加号", () => {
  assert.match(domain, /groups\.set\(key, \{ workspace, roots: \[\], preseeded: false, chatOnly: true \}\)/);
  assert.match(domain, /const chatGroup = !group\.preseeded && group\.chatOnly === true/);
  assert.match(domain, /chatGroup \? "对话" : workspaceLabel\(group\.workspace\)/);
  assert.match(domain, /if \(group\.workspace && !chatGroup\) \{/);
  assert.match(domain, /对话会话（不使用本地工作区）/);
});

test("无工作区对话切 Work 时明确告知仍在隔离目录", () => {
  assert.match(app, /target === "chat"[\s\S]*?已切换到对话模式/);
  assert.match(app, /else if \(!\$\("workspace"\)\.value\.trim\(\)\)/);
  assert.match(app, /已切换到 Work 模式（隔离目录）/);
  assert.match(app, /文件工具只会在对话隔离目录内运行/);
  assert.match(app, /需要项目文件请新建会话并选择工作区/);
});

test("运行中切模式只影响下一回合，并明确当前回合仍用旧工具面", () => {
  assert.match(app, /const running = Boolean\(currentTurn\(\)\);/);
  assert.match(app, /已切换到对话模式（下一回合生效）/);
  assert.match(app, /当前回合仍按原模式执行，可能继续读写本地文件/);
  assert.match(app, /已切换到 Work 模式（隔离目录，下一回合生效）/);
  assert.match(app, /当前回合仍按对话模式执行，不会读写本地文件。/);
});

test("切换模式同步项目入口，对话不残留「未选择项目」", () => {
  const match = /function refreshComposerModeChip\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match, "refreshComposerModeChip 必须在场");
  const calls = [];
  const elements = {
    mediaBtn: { classList: { toggle: () => {} }, setAttribute: () => {}, title: "" },
    prompt: { placeholder: "" },
    modeChipText: { textContent: "" },
    modeChip: { classList: { toggle: () => {} } },
    modeNotice: { textContent: "", hidden: false },
  };
  const context = {
    $: (id) => elements[id] || null,
    currentSessionMode: () => "chat",
    syncChatStarters: () => calls.push("starters"),
    syncProjectChip: () => calls.push("project"),
    document: { querySelectorAll: () => [] },
    window: {
      OwoSessionMode: {
        label: () => "对话",
        notice: () => "对话模式不读写本机文件",
      },
    },
  };
  const refresh = runInNewContext(match[0] + "\nrefreshComposerModeChip;", context);
  refresh();
  // 项目 chip 必须跟空状态一起刷新；先 starters 后 project，保证项目入口是最终文案。
  assert.equal(calls.join(","), "starters,project");
  assert.equal(elements.modeChipText.textContent, "对话");
  assert.match(elements.prompt.placeholder, /问点什么/);
});
