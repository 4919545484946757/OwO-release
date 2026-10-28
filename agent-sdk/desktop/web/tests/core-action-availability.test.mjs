import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../core/core-action-availability.js", import.meta.url), "utf8");
const index = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const panelSources = Object.fromEntries([
  "automations", "notes", "goal", "workflow", "memory", "eval", "command",
  "plugin-market", "fleet", "product-comparison", "team", "action-center",
  "project-history", "project-launcher", "workswarm",
].map((name) => [name, readFileSync(new URL("../panels/" + name + ".panel.js", import.meta.url), "utf8")]));
const workSwarmRender = readFileSync(new URL("../panels/workswarm/render.js", import.meta.url), "utf8");
const settingsView = readFileSync(new URL("../views/settings-panel.view.js", import.meta.url), "utf8");
const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const setupGuideView = readFileSync(new URL("../views/setup-guide.view.js", import.meta.url), "utf8");
const serviceErrorView = readFileSync(new URL("../views/service-error.view.js", import.meta.url), "utf8");

function control(disabled = false, title = null) {
  const attrs = new Map();
  if (title != null) attrs.set("title", title);
  return {
    disabled,
    attrs,
    getAttribute(name) { return attrs.has(name) ? attrs.get(name) : null; },
    setAttribute(name, value) { attrs.set(name, String(value)); },
    removeAttribute(name) { attrs.delete(name); },
    matches(selector) {
      return (selector === "[data-core-action]" && attrs.has("data-core-action")) ||
        (selector === "[data-core-action-hint]" && attrs.has("data-core-action-hint"));
    },
    querySelectorAll() { return []; },
  };
}

function harness(controls, hints = [], documentExtras = {}) {
  let observerCallback = null;
  const document = Object.assign({
    documentElement: {},
    querySelectorAll(selector) {
      if (selector === "[data-core-action]") return controls;
      if (selector === "[data-core-action-hint]") return hints;
      assert.fail("unexpected selector: " + selector);
    },
  }, documentExtras);
  class MutationObserver {
    constructor(callback) { observerCallback = callback; }
    observe() {}
  }
  const window = { document, MutationObserver };
  runInNewContext(source, { window, document, WeakMap, Boolean, String });
  return { availability: window.OwoCoreActionAvailability, added(node) {
    if (node.matches("[data-core-action]")) controls.push(node);
    if (node.matches("[data-core-action-hint]")) hints.push(node);
    observerCallback([{ addedNodes: [node] }]);
  } };
}

test("core mutations start disabled and unlock only after authenticated readiness", () => {
  const controls = [control(), control(true, "原本禁用")];
  const { availability } = harness(controls);
  assert.deepEqual(controls.map((item) => item.disabled), [true, true]);
  assert.deepEqual(controls.map((item) => item.attrs.get("aria-disabled")), ["true", "true"]);
  assert.ok(controls.every((item) => item.attrs.get("title") === "连接并授权本地核心后可使用"));

  availability.update(true);
  assert.deepEqual(controls.map((item) => item.disabled), [false, true]);
  assert.equal(controls[0].attrs.has("title"), false);
  assert.equal(controls[0].attrs.get("aria-disabled"), "false");
  assert.equal(controls[1].attrs.get("title"), "原本禁用");
});

test("core mutations relock after disconnect and remain reversible on recovery", () => {
  const button = control();
  const { availability } = harness([button]);
  availability.update(true);
  availability.update(false);
  assert.equal(button.disabled, true);
  assert.equal(button.attrs.get("aria-disabled"), "true");
  availability.update(true);
  assert.equal(button.disabled, false);
});


test("busy actions stay locked through availability updates and release to current state", () => {
  const button = control();
  const hint = { hidden: false };
  const { availability } = harness([button], [hint]);
  assert.equal(availability.isAvailable(), false);
  assert.equal(hint.hidden, false);
  availability.update(true);
  assert.equal(hint.hidden, true);
  button.attrs.set("data-core-busy", "true");
  availability.update(false);
  availability.update(true);
  assert.equal(button.disabled, true);
  button.attrs.delete("data-core-busy");
  availability.update(true);
  assert.equal(button.disabled, false);
});


test("dynamic panels get a generic readiness note when they have no specific hint", () => {
  const panelHints = [];
  const action = control();
  const panel = {
    firstChild: { tagName: "STYLE" },
    querySelector(selector) {
      return selector === "[data-core-action-hint]" ? panelHints[0] || null : null;
    },
    querySelectorAll(selector) {
      if (selector === "[data-core-action]") return [action];
      if (selector === "[data-core-action-hint]") return panelHints;
      return [];
    },
    insertBefore(node) {
      panelHints.unshift(node);
      this.firstChild = node;
    },
  };
  action.closest = selector => selector === "[data-panel]" ? panel : null;
  const { availability } = harness([action], panelHints, {
    createElement(tag) {
      assert.equal(tag, "p");
      return {
        attrs: new Map(),
        textContent: "",
        setAttribute(name, value) { this.attrs.set(name, value); },
      };
    },
  });
  assert.equal(panelHints.length, 1);
  assert.equal(panelHints[0].textContent, "连接并授权本地核心后可使用此页面的写入与管理操作。");
  assert.equal(panelHints[0].hidden, false);
  availability.update(true);
  assert.equal(panelHints[0].hidden, true);
  availability.update(false);
  assert.equal(panelHints[0].hidden, false);
});

test("the shared gate API marks late controls and holds/release busy state", () => {
  const mounted = harness([]);
  const button = control();
  mounted.availability.update(true);
  mounted.availability.mark(button);
  assert.equal(button.disabled, false);
  mounted.availability.setBusy(button, true);
  assert.equal(button.disabled, true);
  mounted.availability.update(false);
  mounted.availability.update(true);
  assert.equal(button.disabled, true);
  mounted.availability.setBusy(button, false);
  assert.equal(button.disabled, false);
});

test("dynamically mounted actions inherit offline state and busy controls can recover", () => {
  const mounted = harness([]);
  const button = control(false);
  button.attrs.set("data-core-action", "");
  button.attrs.set("data-core-busy", "true");
  mounted.added(button);
  assert.equal(button.disabled, true);
  mounted.availability.update(true);
  assert.equal(button.disabled, true);
  button.attrs.delete("data-core-busy");
  mounted.availability.update(true);
  assert.equal(button.disabled, false);
});

test("backend write actions across automation, intelligence, and plugins opt into the shared gate", () => {
  for (const id of [
    "learnStart", "learnPause", "learnResume", "learnStop", "learnClear",
    "evalRunBtn", "subagentRunBtn", "agentsSaveBtn",
  ]) {
    const tag = index.match(new RegExp('<button[^>]*id="' + id + '"[^>]*>'))?.[0] || "";
    assert.ok(tag.includes("data-core-action"), id + " should be gated");
  }
  assert.equal((index.match(/data-core-action-hint/g) || []).length, 13);
  assert.match(index, /连接并授权本地核心后可查看会话轨迹；服务状态恢复后点击“刷新列表”加载/);
  assert.match(index, /连接并授权本地核心后才能更改云端模型的数据出境设置/);
  assert.match(index, /主动建议和记忆检索需要连接并授权本地核心/);
  assert.match(index, /技能导入、项目模板写入和备份恢复需要连接并授权本地核心/);
  assert.match(index, /数据备份、导出和恢复需要本地核心连接/);
  assert.match(index, /连接并授权本地核心后可添加、连接或管理 MCP 服务器/);
  assert.match(index, /白名单会减少逐次审批，请只添加你信任的应用/);
  assert.match(index, /连接并授权本地核心后才能录制、清空样本或沉淀技能包/);
  assert.match(index, /连接并授权本地核心后可创建任务；实际执行前仍需任务级审批/);
  for (const id of [
    "settingsModel", "modelOutputModel", "modelOutputLimit", "modelOutputSaveBtn",
    "modelOutputResetBtn", "providerBaseUrl", "providerApiKey", "providerSaveBtn", "presetApplyModelBtn",
    "egressToggle", "prefProactive", "recallBtn", "skillImportBtn", "agentsTemplateBtn",
    "storageRestoreBtn", "storageBackupBtn", "storageExportBtn", "storageClearBtn",
  ]) {
    const tag = index.match(new RegExp("<(?:input|select|button)[^>]*id=\"" + id + "\"[^>]*>"))?.[0] || "";
    assert.ok(tag.includes("data-core-action"), id + " must be unavailable until the local core is authenticated");
  }
  assert.match(index, /连接并授权本地核心后，才能切换默认模型/);
  assert.match(appSource, /updatePresetApplyAvailability\(\)/, "preset save must follow the current authenticated state");
  assert.ok(index.includes("尚未运行</div>"), "offline empty-state copy must not prompt a disabled action");
  for (const name of ["notes", "goal", "workflow", "memory", "eval", "command", "plugin-market", "fleet", "product-comparison", "team", "action-center", "project-launcher", "workswarm"]) {
    assert.ok(panelSources[name].includes("data-core-action"), name + " panel writes should use the shared gate");
  }
  assert.ok(!panelSources["project-history"].includes("H.post("), "history goal reuse must not create a run before launcher review");
  assert.match(source, /ensurePanelHints\(controls\)/, "dynamic feature panels need a shared offline explanation");
  assert.match(source, /可使用此页面的写入与管理操作/, "shared offline explanation must name the unavailable actions");
  assert.match(panelSources.notes, /class="owo-notes-search"[^>]*data-core-action/, "note search must be unavailable when its core API is disconnected");
  assert.ok(panelSources.automations.includes('setAttribute("data-core-action", "true")'), "generated automation controls should use the shared gate");
  assert.ok(workSwarmRender.includes("data-core-action"), "generated WorkSwarm delivery actions should use the shared gate");
  assert.ok(settingsView.includes("markCoreAction(testButton)"), "provider test should use the shared gate");
  assert.ok(settingsView.includes("markCoreAction(sessionApply)"), "session model changes should use the shared gate");
  assert.ok(settingsView.includes("markCoreAction(sessionClear)"), "session model reset should use the shared gate");
  assert.ok(setupGuideView.includes("markCoreAction(testBtn)"), "setup guide provider test should use the shared gate");
  assert.ok(serviceErrorView.includes('data-core-action="true"'), "service error provider test should use the shared gate");
  for (const id of ["sinkForm", "computerTaskForm", "mcpForm", "whitelistForm"]) {
    const start = index.indexOf('<form id="' + id + '"');
    const end = index.indexOf("</form>", start);
    assert.ok(start >= 0 && end > start, id + " should exist");
    assert.ok(index.slice(start, end).includes("data-core-action"), id + " submit should be gated");
  }
});

test("feature readiness changes release a previously unavailable human action without bypassing the core gate", () => {
  const button = control(true, "当前没有未完成的人节点任务");
  const { availability } = harness([button]);
  button.attrs.set("data-core-action", "true");
  availability.update(true);
  assert.equal(button.disabled, true);
  availability.setDisabled(button, false, "");
  availability.update(true);
  assert.equal(button.disabled, false);
  assert.equal(button.attrs.get("aria-disabled"), "false");
  availability.update(false);
  availability.setDisabled(button, false, "");
  assert.equal(button.disabled, true);
  assert.equal(button.attrs.get("title"), "连接并授权本地核心后可使用");
  availability.update(true);
  assert.equal(button.disabled, false);
  availability.setDisabled(button, true, "团队已进入终态");
  availability.update(false);
  availability.update(true);
  assert.equal(button.disabled, true);
  assert.equal(button.attrs.get("title"), "团队已进入终态");
});

test("a feature transition cannot release an in-flight submission", () => {
  const button = control();
  const { availability } = harness([button]);
  availability.update(true);
  availability.setBusy(button, true);
  availability.setDisabled(button, false);
  assert.equal(button.disabled, true);
  availability.setDisabled(button, true, "团队已进入终态");
  availability.setBusy(button, false);
  assert.equal(button.disabled, true);
});
