import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "../core/workbench-view.js"), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(source, sandbox, { filename: "workbench-view.js" });
const create = sandbox.window.OwoWorkbenchView.create;
const replaceRouteHash = sandbox.window.OwoWorkbenchView.replaceRouteHash;
const clearGroupFilters = sandbox.window.OwoWorkbenchView.clearGroupFilters;

function harness() {
  const classes = new Set();
  const body = {
    classList: {
      add(...names) { names.forEach((name) => classes.add(name)); },
      remove(...names) { names.forEach((name) => classes.delete(name)); },
      contains(name) { return classes.has(name); },
      toggle(name, force) {
        if (force) classes.add(name);
        else classes.delete(name);
        return Boolean(force);
      },
    },
  };
  const attributes = {};
  const toggleButton = {
    textContent: "",
    setAttribute(name, value) { attributes[name] = value; },
  };
  let clearCount = 0;
  let scrollCount = 0;
  const view = create({
    body,
    toggleButton,
    clearToolGroups() { clearCount += 1; },
    scrollSettings() { scrollCount += 1; },
  });
  return { view, body, toggleButton, attributes, cleared: () => clearCount, scrolled: () => scrollCount };
}

test("工具与设置切换维持互斥视图并更新可访问控件状态", () => {
  const h = harness();
  h.view.showTools(true);
  assert.equal(h.body.classList.contains("tools-open"), true);
  assert.equal(h.body.classList.contains("settings-open"), false);
  assert.equal(h.attributes["aria-expanded"], "true");

  h.view.showSettings(true);
  assert.equal(h.body.classList.contains("show-tools"), true);
  assert.equal(h.body.classList.contains("tools-open"), false);
  assert.equal(h.body.classList.contains("settings-open"), true);
  assert.equal(h.toggleButton.textContent, "收起工具与设置");
  assert.equal(h.scrolled(), 1);

  h.view.showSettings(false);
  assert.equal(h.body.classList.contains("show-tools"), false);
  assert.equal(h.body.classList.contains("tools-open"), false);
  assert.equal(h.body.classList.contains("settings-open"), false);
  assert.equal(h.attributes["aria-expanded"], "false");
  assert.equal(h.cleared(), 1);
});

test("工具按钮可从工具视图和设置页回到会话，再从会话重新打开工具", () => {
  const h = harness();
  h.view.toggleTools();
  assert.equal(h.body.classList.contains("tools-open"), true);
  h.view.toggleTools();
  assert.equal(h.body.classList.contains("tools-open"), false);
  assert.equal(h.body.classList.contains("settings-open"), false);
  h.view.showSettings(true);
  h.view.toggleTools();
  assert.equal(h.body.classList.contains("settings-open"), false);
  assert.equal(h.body.classList.contains("show-tools"), false);
  h.view.toggleTools();
  assert.equal(h.body.classList.contains("tools-open"), true);
});


test("打开扩展面板时清除侧栏分组筛选并恢复全部导航", () => {
  const classes = new Set(["tools-group-automation"]);
  const makeButton = (dataset, active) => ({
    dataset,
    classList: {
      remove(name) { if (name === "active") active = false; },
      toggle(name, force) { if (name === "active") active = Boolean(force); },
      contains(name) { return name === "active" && active; },
    },
    active: () => active,
  });
  const groupButton = makeButton({ codexGroup: "automation" }, true);
  const allButton = makeButton({ jump: "all" }, false);
  const automationButton = makeButton({ jump: "automation" }, true);
  const body = {
    classList: {
      remove(name) { classes.delete(name); },
      contains(name) { return classes.has(name); },
    },
    querySelectorAll(selector) {
      if (selector === "[data-codex-group]") return [groupButton];
      if (selector === "[data-jump]") return [allButton, automationButton];
      return [];
    },
  };

  clearGroupFilters(body);
  assert.equal(classes.has("tools-group-automation"), false);
  assert.equal(groupButton.active(), false);
  assert.equal(allButton.active(), true);
  assert.equal(automationButton.active(), false);
});

test("有效初始深链进入指定页，无效 hash 回到会话页", () => {
  const resolve = sandbox.window.OwoWorkbenchView.resolveInitialRoute;
  const panel = resolve("#about", ["about", "team"]);
  assert.equal(panel.kind, "panel");
  assert.equal(panel.id, "about");
  assert.equal(resolve("#settings", ["about", "team"]).kind, "settings");
  assert.equal(resolve("#stale-panel", ["about", "team"]).kind, "chat");
  assert.equal(resolve("", ["about", "team"]).kind, "chat");
});

test("route hash follows chat, settings, and panel transitions while preserving path and query", () => {
  const location = { pathname: "/workbench/index.html", search: "?mode=local", hash: "#fleet" };
  const writes = [];
  const history = {
    replaceState(_state, _title, url) {
      writes.push(url);
      const parsed = new URL(url, "http://127.0.0.1");
      location.pathname = parsed.pathname;
      location.search = parsed.search;
      location.hash = parsed.hash;
    },
  };
  assert.equal(replaceRouteHash(location, history, null), true);
  assert.equal(location.pathname + location.search + location.hash, "/workbench/index.html?mode=local");
  assert.equal(replaceRouteHash(location, history, "settings"), true);
  assert.equal(location.hash, "#settings");
  assert.equal(replaceRouteHash(location, history, "#team"), true);
  assert.equal(location.hash, "#team");
  assert.equal(replaceRouteHash(location, history, "team"), false);
  assert.equal(writes.length, 3);
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const domain = readFileSync(join(here, "../app-domain.js"), "utf8");
  assert.match(app, /replaceRoute: \(route\) => window\.OwoWorkbenchView\.replaceRouteHash/);
  assert.match(domain, /if \(writeHash\) workbenchView\.setRoute\(id\)/);
  const openPanel = /function openPanelById\(id\) \{([\s\S]*?)\n\}/.exec(domain)?.[1];
  assert.ok(openPanel, "panel route handler must exist");
  assert.match(openPanel, /clearGroupFilters\(document\.body\)/);
  assert.doesNotMatch(openPanel, /refreshPluginSkillOverview/, "focused pages must not hydrate a hidden tools overview");
});
