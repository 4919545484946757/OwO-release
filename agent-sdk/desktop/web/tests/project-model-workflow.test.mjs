import test from "node:test";
import { runInNewContext } from "node:vm";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
const app = readFileSync(join(here, "..", "app.js"), "utf8");
const domain = readFileSync(join(here, "..", "app-domain.js"), "utf8");
const index = readFileSync(join(here, "..", "index.html"), "utf8");
const style = readFileSync(join(here, "..", "style.css"), "utf8");
const modelRouting = readFileSync(join(here, "..", "core", "model-routing.js"), "utf8");
const sessionStartSource = readFileSync(join(here, "..", "core", "session-start.js"), "utf8");
test("new sessions do not inherit previous model overrides and can restore default", () => {
  assert.match(domain, /buildCreateSessionRequest\(workspace,\s*state\.pendingModelOverride\)/);
  assert.match(app, /data-model-default/);
  assert.match(app, /sessionModelUpdateQueue\.enqueue\(sessionId, model\)/);
  assert.match(modelRouting, /createSessionModelUpdateQueue/);
  assert.match(modelRouting, /buildSessionModelRequest\(model\)/);
  assert.match(index, /core\/model-routing\.js/);
});
test("sidebar exposes project workspaces and Electron project creation", () => {
  assert.match(index, /id="sidebarWorkspaceBtn"/);
  assert.match(app, /create_project_workspace/);
  assert.match(app, /recentWorkspaces/);
});

test("empty workspace is shown consistently instead of a fake local project", () => {
  assert.match(index, /id="sidebarWorkspaceName">未选择项目</);
  assert.match(index, /id="composerProjectName">未选择项目</);
  assert.match(index, /当前工作区 <strong>未选择项目<\/strong>/);
  assert.match(app, /const currentName = current \?[^;]*: "未选择项目"/);
  assert.match(app, /const displayName = name \|\| "未选择项目"/);
});



test("model picker states the change scope and provides a searchable model list", () => {
  const helper = /function filterComposerModelMenu\(menu, query\) \{[\s\S]*?\n\}/.exec(app);
  const picker = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(helper);
  assert.ok(picker);
  assert.match(picker[0], /只影响当前会话/);
  assert.match(picker[0], /用于下一条新会话/);
  assert.match(picker[0], /data-model-search/);
  assert.match(picker[0], /filterComposerModelMenu\(menu, filter.value\)/);

  const choices = [
    { dataset: { modelSearch: "glm-5.3-flash BigModel" }, textContent: "", hidden: false },
    { dataset: { modelSearch: "qwen3.8-max Qwen" }, textContent: "", hidden: false },
  ];
  const group = { hidden: false, querySelectorAll: () => choices };
  const empty = { hidden: true };
  const filter = runInNewContext(helper[0] + "\nfilterComposerModelMenu;", {});
  const menu = { querySelectorAll: () => [group], querySelector: () => empty };
  assert.equal(filter(menu, "QWEN"), 1);
  assert.deepEqual(choices.map(choice => choice.hidden), [true, false]);
  assert.equal(group.hidden, false);
  assert.equal(empty.hidden, true);
  assert.equal(filter(menu, "missing"), 0);
  assert.deepEqual(choices.map(choice => choice.hidden), [true, true]);
  assert.equal(group.hidden, true);
  assert.equal(empty.hidden, false);
});

test("model picker supports keyboard traversal across filtered choices", () => {
  const helper = /function focusComposerModelChoice\(menu, key, currentTarget\) \{[\s\S]*?\n\}/.exec(app);
  const picker = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(helper);
  assert.ok(picker);
  assert.match(picker[0], /button\.addEventListener\("keydown"/);
  assert.match(picker[0], /focusComposerModelChoice\(menu, event\.key, event\.target\)/);
  const choices = [
    { hidden: false, disabled: false },
    { hidden: true, disabled: false },
    { hidden: false, disabled: false },
  ];
  const menu = { querySelectorAll: () => choices };
  const focus = runInNewContext(helper[0] + "\nfocusComposerModelChoice;", {});
  assert.equal(focus(menu, "ArrowDown", null), choices[0]);
  assert.equal(focus(menu, "ArrowUp", null), choices[2]);
  assert.equal(focus(menu, "ArrowDown", choices[0]), choices[2]);
  assert.equal(focus(menu, "ArrowUp", choices[2]), choices[0]);
  assert.equal(focus(menu, "Home", choices[2]), choices[0]);
  assert.equal(focus(menu, "End", choices[0]), choices[2]);
  assert.equal(focus({ querySelectorAll: () => [] }, "ArrowDown", null), null);
});

test("composer menu placement uses the final width and keeps a viewport gutter", () => {
  const match = /function composerMenuPosition\(triggerRect, menuRect, viewportWidth\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match);
  const position = runInNewContext(match[0] + "\ncomposerMenuPosition;", {});
  const result = position(
    { right: 664, top: 520, bottom: 540 },
    { width: 336, height: 320 },
    664,
  );
  assert.deepEqual({ left: result.left, right: 664 - result.left - 336 }, { left: 316, right: 12 });
});

test("narrow composer keeps per-session model switching reachable", () => {
  const mobile = style.slice(style.indexOf("@media (max-width: 700px)"));
  assert.match(mobile, /#modelChip\s*\{\s*display:\s*inline-flex/);
  assert.match(mobile, /#effortChip\s*\{\s*display:\s*none/);
});


test("model settings warn on mismatched current models and expose provider-compatible choices", () => {
  assert.match(index, /id="modelCompatibilityHint"/);
  assert.match(app, /providerModelOptionState\(/);
  assert.match(app, /syncProviderModels\(preset\)/);
  assert.match(modelRouting, /providerModelOptionState/);
});

test("blank prompt and model setup gates run before automatic session creation", () => {
  const body = /async function sendPrompt\(\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  assert.ok(body);
  assert.ok(body.indexOf('const prompt = $("prompt").value.trim()') < body.indexOf("promptSessionStart.run(() => newSession())"));
  assert.ok(body.indexOf("if (!prompt) return;") < body.indexOf("promptSessionStart.run(() => newSession())"));
  assert.ok(body.indexOf("if (modelGateMissing())") < body.indexOf("promptSessionStart.run(() => newSession())"));
  assert.match(body, /if \(promptInput\.value === promptValue\) promptInput\.value = ""/);
});

test("automatic first-send session creation is single-flight and can retry after failure", async () => {
  const context = {};
  runInNewContext(sessionStartSource, context);
  const gate = context.OwoSessionStart.createSessionStartGate();
  let creates = 0;
  let release;
  const pendingCreate = new Promise((resolve) => { release = resolve; });
  const first = gate.run(() => { creates += 1; return pendingCreate; });
  const second = gate.run(() => { creates += 1; return Promise.resolve("duplicate"); });
  assert.equal(first, second);
  await Promise.resolve();
  assert.equal(creates, 1);
  release("session-1");
  assert.equal(await first, "session-1");
  assert.equal(await gate.run(() => { creates += 1; return "session-2"; }), "session-2");
  assert.equal(creates, 2);

  let fail = true;
  await assert.rejects(gate.run(() => {
    if (fail) { fail = false; throw new Error("creation failed"); }
    return "recovered";
  }), /creation failed/);
  assert.equal(await gate.run(() => "recovered"), "recovered");
});

test("first send uses the single-flight automatic session gate", () => {
  assert.match(index, /core\/session-start\.js/);
  assert.match(domain, /const promptSessionStart = OwoSessionStart\.createSessionStartGate\(\)/);
  const body = /async function sendPrompt\(\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  assert.ok(body);
  assert.match(body, /await promptSessionStart\.run\(\(\) => newSession\(\)\)/);
});

test("model changes serialize by session and stale session responses cannot update the current chip", () => {
  assert.match(app, /createSessionModelUpdateQueue/);
  const body = /async function selectModel\(id\) \{([\s\S]*?)\n\}/.exec(app)?.[1];
  assert.ok(body);
  assert.match(body, /const sessionId = state\.sessionId/);
  assert.match(body, /sessionModelUpdateQueue\.enqueue\(sessionId, model\)/);
  assert.match(body, /!result\.latest \|\| state\.sessionId !== sessionId/);
});


test("stale session refreshes cannot overwrite current selection or panels", () => {
  assert.match(domain, /generation !== sessionListRefreshGeneration/);
  assert.match(domain, /selectionVersion === state\.selectionVersion/);
  const renderSession = /const renderSession = \(session, depth, groupWorkspace\) => \{([\s\S]*?)\n  \};/.exec(domain)?.[1];
  assert.ok(renderSession);
  assert.doesNotMatch(renderSession, /state\.sessionId\s*=\s*session\.id/);
  const contextRefresh = /async function refreshSessionContext\(sessionId\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  const diffRefresh = /async function refreshDiff\(sessionId\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  assert.match(contextRefresh, /sessionId !== state\.sessionId/);
  assert.match(diffRefresh, /sessionDiffRefreshGeneration/);
});


test("creating a project keeps the model explicitly selected for the next session", () => {
  const projectFlow = /async function createProjectWorkspace\(\) \{([\s\S]*?)\n\}/.exec(app)?.[1];
  assert.ok(projectFlow);
  assert.match(projectFlow, /await newSession\(\)/);
  assert.doesNotMatch(projectFlow, /state\.pendingModelOverride\s*=\s*null/);
});

test("workspace selection requires a desktop persistence result before updating the active workspace", () => {
  const persist = /createWorkspaceSelectionController\(\s*async \(path\) => \{([\s\S]*?)\n  \},/.exec(app)?.[1];
  assert.ok(persist);
  assert.match(persist, /requireWorkspacePersistence\(/);
  assert.match(persist, /未切换本地目录/);
});

test("workspace switches are serialized and new project activation is fenced", () => {
  const routing = readFileSync(join(here, "..", "core", "workspace-routing.js"), "utf8");
  const electronMain = readFileSync(join(here, "..", "..", "electron", "src", "main", "main.js"), "utf8");
  assert.match(app, /workspaceSelection\.select\(target, revision\)/);
  assert.match(app, /create_project_workspace", \{ name, activate: false \}/);
  assert.match(app, /if \(!workspaceSelection\.isCurrent\(revision\)\)/);
  assert.match(electronMain, /args && args\.activate === false/);
  assert.match(routing, /var task = tail\.catch/);
});

test("project folder failures preserve the completed stage in the user-facing message", () => {
  const projectFlow = /async function createProjectWorkspace\(\) \{([\s\S]*?)\n\}/.exec(app)?.[1];
  assert.ok(projectFlow);
  assert.match(projectFlow, /let phase = "create"/);
  assert.match(projectFlow, /phase = "activate"/);
  assert.match(projectFlow, /phase = "session"/);
  assert.match(projectFlow, /projectCreationFailureMessage\(phase, error\)/);
});

test("first-run empty state makes workspace selection explicit and restores the normal welcome after selection", () => {
  assert.match(index, /id="emptyStateWorkspaceBtn"[^>]*hidden/);
  assert.match(app, /emptyStateWorkspaceBtn.*openWorkspaceMenu\(\$\("composerProjectBtn"\)\)/);
  assert.match(app, /applyLocalPrefs\(\);\s*syncProjectChip\(\);\s*initGlobalStatusBar\(\)/);
  assert.match(app, /if \(workspaceInput\) workspaceInput\.value = workspaceConfigured \? workspacePath : \"\";/);
  const match = /function syncProjectChip\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match, "workspace synchronization must own the first-run empty-state copy");
  const classes = new Set();
  const elements = {
    workspace: { value: "" },
    emptyState: { classList: { toggle: (name, on) => on ? classes.add(name) : classes.delete(name) } },
    emptyStateTitle: { textContent: "" },
    emptyStateDescription: { textContent: "" },
    emptyStateWorkspaceBtn: { hidden: true },
  };
  const context = {
    state: { sessionId: null },
    $: (id) => elements[id] || null,
    document: { querySelector: () => null },
  };
  const sync = runInNewContext(match[0] + "\nsyncProjectChip;", context);
  sync();
  assert.equal(elements.emptyStateTitle.textContent, "先选择一个项目工作区");
  assert.match(elements.emptyStateDescription.textContent, /新任务需要一个工作目录/);
  assert.equal(elements.emptyStateWorkspaceBtn.hidden, false);
  assert.equal(classes.has("needs-workspace"), true);
  elements.workspace.value = "D:\\work\\project";
  sync();
  assert.equal(elements.emptyStateTitle.textContent, "你想完成什么？");
  assert.equal(elements.emptyStateWorkspaceBtn.hidden, true);
  assert.equal(classes.has("needs-workspace"), false);
});


test("first send without a workspace opens the project picker before the model gate", () => {
  const body = /async function sendPrompt\(\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  assert.ok(body);
  const workspaceGate = body.indexOf('if (!state.sessionId && !$("workspace").value.trim())');
  const modelGate = body.indexOf("if (modelGateMissing())");
  assert.ok(workspaceGate >= 0);
  assert.ok(modelGate > workspaceGate);
  assert.match(body, /openWorkspaceMenu\(\$\("composerProjectBtn"\)\)/);
  assert.match(body, /if \(!prompt\) return;/);
});
