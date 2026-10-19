import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { createRequire } from "node:module";
const nav = createRequire(import.meta.url)("../core/workbench-navigation.js");
const read = file => readFileSync(new URL("../" + file, import.meta.url), "utf8");

test("primary and disclosed navigation preserve every registered route exactly once", () => {
  const sections = nav.sections();
  assert.deepEqual(sections[0].entries.map(item => item.id), ["home","project-launcher","project-history","workswarm","evidence","action-center"]);
  const ids = sections.flatMap(group => group.entries.map(item => item.id));
  assert.equal(new Set(ids).size, 23);
  assert.deepEqual([...ids].sort(), nav.entries.map(item => item.id).sort());
  assert.deepEqual(nav.sections(["notes","permissions"]).flatMap(group => group.entries.map(item => item.id)), ["notes","permissions"]);
});

test("search reveals folded destinations and restores user disclosure after clearing", () => {
  const buttons = ["notes","permissions","eval"].map(id => ({ dataset: { panel: id }, hidden: false }));
  const sections = buttons.map((button,i) => ({ dataset: {}, open: i === 0, hidden: false, querySelectorAll: () => [button] }));
  const root = { querySelectorAll: selector => selector.startsWith("button") ? buttons : sections };
  const source = read("app-domain.js").match(/function filterPanelNavigation\(nav, query\) \{[\s\S]*?\n\}/)[0];
  const filter = runInNewContext(source + "\nfilterPanelNavigation;", { window: { OwoWorkbenchNavigation: nav }, PANEL_ORDER: buttons.map(b => b.dataset.panel), Set, String, Array });
  filter(root, "权限");
  assert.equal(sections[1].open, true);
  assert.equal(buttons[1].hidden, false);
  assert.equal(sections[0].hidden, true);
  filter(root, "无匹配任务");
  assert.ok(sections.every(section => section.hidden));
  filter(root, "");
  assert.ok(buttons.every(button => !button.hidden));
  assert.equal(sections[0].open, true);
  assert.equal(sections[1].open, false);
  assert.ok(sections.every(section => !section.hidden));
});

test("empty-session entrance survives administrative notices but yields to work, failures and running turns", () => {
  const state = { sessionId: "empty" };
  const hidden = new Set();
  const views = { empty: { querySelector: () => null }, worked: { querySelector: () => ({}) }, failed: { querySelector: () => ({}) } };
  let running = null;
  const empty = { classList: { toggle: (name,on) => on ? hidden.add(name) : hidden.delete(name) } };
  const messages = { querySelector: selector => views[selector.match(/data-sid="([^"]+)"/)[1]] };
  const source = read("app.js").match(/function syncSessionWelcome\(\) \{[\s\S]*?\n\}/)[0];
  const sync = runInNewContext(source + "\nsyncSessionWelcome;", { state, $: id => id === "emptyState" ? empty : messages, currentTurn: () => running, Boolean });
  sync(); assert.equal(hidden.has("hidden"), false);
  state.sessionId = "worked"; sync(); assert.equal(hidden.has("hidden"), true);
  state.sessionId = "empty"; sync(); assert.equal(hidden.has("hidden"), false);
  running = {}; sync(); assert.equal(hidden.has("hidden"), true);
  running = null; state.sessionId = "failed"; sync(); assert.equal(hidden.has("hidden"), true);
  state.sessionId = null; sync(); assert.equal(hidden.has("hidden"), false);
});
