import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
const source = readFileSync(new URL("../core/ime-settings.js", import.meta.url), "utf8");
function setup({ desktop = true, failSave = false } = {}) {
  const ids = ["imeTasksEnabled", "imeTasksStatus", "imeConnectionStatus", "imeConnect", "imeDisconnect"];
  const nodes = Object.fromEntries(ids.map(id => [id, { checked: false, disabled: false, textContent: "", events: {}, addEventListener(name, handler) { this.events[name] = handler; } }]));
  let requested = false;
  const calls = [];
  const window = { document: { getElementById: id => nodes[id] || null } };
  if (desktop) window.owo = {
    imeTaskAccess: {
      status: async () => ({ requestedEnabled: requested, activeEnabled: false, restartRequired: requested }),
      setEnabled: async value => { calls.push(["set", value]); if (failSave) throw new Error("synthetic failure"); requested = value; return { requestedEnabled: requested, activeEnabled: false, restartRequired: requested }; },
    },
    imeConnection: { status: async () => ({ state: "disconnected", message: "尚未连接" }), start: async () => { calls.push(["connect"]); }, stop: async () => { calls.push(["disconnect"]); } },
  };
  vm.runInNewContext(source, { window, setInterval: () => 1, clearInterval: () => {} });
  const controller = window.OwoImeSettings.createTaskAccess();
  return { nodes, calls, controller };
}
test("unified settings work without the obsolete permission form or HTTP route", async () => {
  const { nodes, controller } = setup();
  await controller.refresh();
  assert.equal(nodes.imeTasksEnabled.checked, false);
  assert.equal(nodes.imeTasksEnabled.disabled, false);
  assert.match(nodes.imeTasksStatus.textContent, /已关闭/);
  assert.equal(nodes.imeConnectionStatus.textContent, "尚未连接");
});
test("checkbox saves only a boolean and reports pending restart without restarting", async () => {
  const { nodes, calls, controller } = setup();
  await controller.refresh();
  nodes.imeTasksEnabled.checked = true;
  await nodes.imeTasksEnabled.events.change();
  assert.deepEqual(calls, [["set", true]]);
  assert.equal(nodes.imeTasksEnabled.checked, true);
  assert.match(nodes.imeTasksStatus.textContent, /手动重启/);
});
test("failed save restores the stored checkbox and remains retryable", async () => {
  const { nodes, controller } = setup({ failSave: true });
  await controller.refresh();
  nodes.imeTasksEnabled.checked = true;
  await nodes.imeTasksEnabled.events.change();
  assert.equal(nodes.imeTasksEnabled.checked, false);
  assert.equal(nodes.imeTasksEnabled.disabled, false);
  assert.match(nodes.imeTasksStatus.textContent, /保存失败/);
});
test("browser surface has no host capability and cannot enable task admission", async () => {
  const { nodes, calls, controller } = setup({ desktop: false });
  await controller.refresh();
  assert.equal(nodes.imeTasksEnabled.disabled, true);
  assert.equal(nodes.imeConnect.disabled, true);
  assert.equal(nodes.imeDisconnect.disabled, true);
  await nodes.imeTasksEnabled.events.change();
  assert.deepEqual(calls, []);
});
test("connection actions stay separate from the task-admission choice", async () => {
  const { nodes, calls, controller } = setup();
  await controller.refresh();
  await nodes.imeConnect.events.click();
  await nodes.imeDisconnect.events.click();
  assert.deepEqual(calls, [["connect"], ["disconnect"]]);
  assert.equal(nodes.imeTasksEnabled.checked, false);
});
