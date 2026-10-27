import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");

function inputHarness(slash = false) {
  let handler;
  const calls = { sent: 0, command: 0, closed: 0, prevented: 0 };
  const match = app.match(/\$\("prompt"\)\.addEventListener\("keydown", \(event\) => \{[\s\S]*?\n\}\);/);
  assert.ok(match);
  runInNewContext(match[0], {
    $: () => ({ addEventListener: (_type, fn) => { handler = fn; } }),
    composerMenuEl: slash ? { dataset: { owner: "slash" } } : null,
    slashActiveIndex: 0,
    slashMatches: [{ name: "new" }],
    sendPrompt: () => calls.sent++,
    runSlashCommand: () => calls.command++,
    closeSlashMenu: () => calls.closed++,
    refreshSlashMenu() {},
    updateComposerHint() {},
  });
  return {
    calls,
    key: (overrides = {}) => handler({
      key: "Enter", shiftKey: false, isComposing: false, keyCode: 13,
      preventDefault: () => calls.prevented++, ...overrides,
    }),
  };
}

test("confirming Chinese IME candidates never sends a turn or executes a slash command", () => {
  for (const slash of [false, true]) {
    const h = inputHarness(slash);
    h.key({ isComposing: true });
    h.key({ keyCode: 229 });
    assert.deepEqual(h.calls, { sent: 0, command: 0, closed: 0, prevented: 0 });
  }
});

test("Enter sends once, Shift+Enter preserves a newline, and slash Enter executes only the command", () => {
  const h = inputHarness();
  h.key({ shiftKey: true });
  assert.equal(h.calls.sent, 0);
  assert.equal(h.calls.prevented, 0);
  h.key();
  assert.equal(h.calls.sent, 1);
  assert.equal(h.calls.prevented, 1);
  const slash = inputHarness(true);
  slash.key();
  assert.equal(slash.calls.command, 1);
  assert.equal(slash.calls.sent, 0);
});

test("a session's project context cannot be replaced by the new-session workspace target", () => {
  const elements = Object.fromEntries([
    "workspace", "composerProjectName", "sidebarWorkspaceName", "composerProjectBtn",
    "sidebarWorkspaceBtn", "emptyState", "emptyStateTitle", "emptyStateDescription", "emptyStateWorkspaceBtn",
  ].map(id => [id, { value: "D:\\next-project", textContent: "", classList: { toggle() {} } }]));
  const state = { sessionId: "current", sessionWorkspace: "C:\\projects\\active-project" };
  const sync = runInNewContext(app.match(/function syncProjectChip\(\) \{[\s\S]*?\n\}/)[0] + "\nsyncProjectChip;", {
    $: id => elements[id], state, currentSessionMode: () => "agent",
    document: { querySelector: () => null },
    EMPTY_STATE_COPY: { agent: { title: "开始", description: "目标" } },
  });
  sync();
  assert.equal(elements.composerProjectName.textContent, "active-project");
  assert.equal(elements.emptyStateTitle.textContent, "在 active-project 中开始构建");
  assert.equal(elements.workspace.value, "D:\\next-project", "showing the session never changes the next-session target");
  state.sessionId = null;
  sync();
  assert.equal(elements.composerProjectName.textContent, "next-project");
});

test("new Work conversations prefer the chosen next project and otherwise retain the current session's project", async () => {
  const domain = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
  const input = { value: "" };
  const state = { sessionWorkspace: "C:\\projects\\active", sessionMode: "agent" };
  const workspaces = [];
  const create = runInNewContext(domain.match(/async function newSession\(workspaceOverride\) \{[\s\S]*?\n\}/)[0] + "\nnewSession;", {
    $: () => input, state, currentSessionMode: () => "agent",
    localStorage: { setItem() {} },
    ModelRouting: { buildCreateSessionRequest: workspace => { workspaces.push(workspace); return { workspace }; } },
    api: async () => ({ id: "created" }), selectSession: async () => {},
    syncSessionWelcome() {}, showToast() {}, persistWorkspace: async () => true,
  });
  await create();
  input.value = "D:\\next-project";
  await create();
  await create("E:\\explicit-project");
  assert.deepEqual(workspaces, ["C:\\projects\\active", "D:\\next-project", "E:\\explicit-project"]);
});

test("the send control exposes Stop only for the current session and reports cancellation in flight", () => {
  let turn = null;
  const attributes = {};
  const button = { classList: { toggle() {} }, setAttribute: (name, value) => { attributes[name] = value; } };
  const update = runInNewContext(app.match(/function updateComposerRunning\(\) \{[\s\S]*?\n\}/)[0] + "\nupdateComposerRunning;", {
    $: () => button, currentTurn: () => turn, STOP_SVG: "stop", SEND_SVG: "send",
  });
  update();
  assert.equal(attributes["aria-label"], "发送");
  turn = {};
  update();
  assert.equal(attributes["aria-label"], "中断当前回合");
  assert.equal(button.disabled, false);
  turn.stopping = true;
  update();
  assert.equal(button.disabled, true);
  assert.equal(attributes["aria-busy"], "true");
  turn = null;
  update();
  assert.equal(attributes["aria-label"], "发送");
  assert.equal(attributes["aria-busy"], "false");
});
