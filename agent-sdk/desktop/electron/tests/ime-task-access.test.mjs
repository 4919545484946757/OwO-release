import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import vm from "node:vm";
const { enabled, environment, create } = createRequire(import.meta.url)("../src/main/ime-task-access.js");

test("task ingress is off unless the stored choice is exactly boolean true", () => {
  for (const value of [undefined, null, false, "true", 1, {}, []]) assert.equal(enabled({ ime_tasks_enabled: value }), false);
  assert.equal(enabled({ ime_tasks_enabled: true }), true);
  assert.equal(environment({}, { OWO_IME_TASKS_ENABLED: "true", KEEP: "unchanged" }).OWO_IME_TASKS_ENABLED, "false");
  assert.deepEqual(environment({ ime_tasks_enabled: true }, { KEEP: "unchanged" }), { KEEP: "unchanged", OWO_IME_TASKS_ENABLED: "true" });
});

test("changing the requested choice preserves configuration and waits for a new core", () => {
  const model = { provider: "unset", name: "unchanged" };
  let config = { model, unrelated: { preserve: true } };
  let running = { owned: true, ready: true, imeTasksEnabled: false };
  let writes = 0;
  const access = create({ readConfig: () => config, writeConfig: value => { config = value; writes++; }, runtime: () => running });
  assert.deepEqual(access.status(), { requestedEnabled: false, activeEnabled: false, restartRequired: false });
  assert.deepEqual(access.setEnabled(true), { requestedEnabled: true, activeEnabled: false, restartRequired: true });
  assert.equal(config.model, model);
  assert.deepEqual(config.unrelated, { preserve: true });
  assert.equal(running.imeTasksEnabled, false);
  running = { owned: true, ready: true, imeTasksEnabled: true };
  assert.equal(access.status().restartRequired, false);
  assert.deepEqual(access.setEnabled(false), { requestedEnabled: false, activeEnabled: true, restartRequired: true });
  assert.equal(writes, 2);
});

test("unowned, starting and older cores do not claim an effective switch", () => {
  for (const running of [null, { owned: false, ready: true, imeTasksEnabled: true }, { owned: true, ready: false, imeTasksEnabled: true }, { owned: true, ready: true }]) {
    const access = create({ readConfig: () => ({ ime_tasks_enabled: true }), runtime: () => running });
    assert.deepEqual(access.status(), { requestedEnabled: true, activeEnabled: null, restartRequired: true });
  }
});

test("renderer input cannot write a non-boolean value or bypass a failed save", () => {
  let writes = 0;
  const access = create({ readConfig: () => ({}), runtime: () => null, writeConfig: () => { writes++; throw new Error("disk unavailable"); } });
  for (const value of ["true", 1, null, {}, { enabled: true }]) assert.throws(() => access.setEnabled(value), /开关/);
  assert.equal(writes, 0);
  assert.throws(() => access.setEnabled(true), /disk unavailable/);
  assert.equal(access.status().requestedEnabled, false);
});

test("real core environment wiring overrides inherited and extra task opt-ins", () => {
  const source = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");
  const start = source.indexOf("function coreEnv(config, extra = {})");
  const end = source.indexOf("// ---------- HTTP", start);
  assert.ok(start > 0 && end > start);
  const context = {
    process: { env: { OWO_IME_TASKS_ENABLED: "true", KEEP: "original" } },
    modelVault: () => ({ environment: () => ({}) }),
    shellCommands: { applyModelEnvironment: env => env },
    agentDataDir: () => "synthetic-data",
    IME_ORIGIN_KEY: "synthetic-origin", PAIRING_SECRET: "synthetic-pairing",
    imeTaskAccessModule: { environment },
  };
  vm.createContext(context);
  vm.runInContext(source.slice(start, end), context);
  assert.equal(context.coreEnv({}, { OWO_IME_TASKS_ENABLED: "true" }).OWO_IME_TASKS_ENABLED, "false");
  assert.equal(context.coreEnv({ ime_tasks_enabled: true }, { OWO_IME_TASKS_ENABLED: "false" }).OWO_IME_TASKS_ENABLED, "true");
  assert.equal(context.coreEnv({}).KEEP, "original");
});

test("only the main workbench may change the task ingress preference through IPC", () => {
  const source = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");
  const start = source.indexOf('const imeTaskAccessModule = require("./ime-task-access.js");');
  const end = source.indexOf("// ---------- 配置读写", start);
  assert.ok(start > 0 && end > start);
  let config = {};
  let writes = 0;
  const sender = {};
  const handlers = new Map();
  const context = {
    require: name => { assert.equal(name, "./ime-task-access.js"); return { enabled, environment, create }; },
    readConfig: () => config,
    writeConfig: value => { config = value; writes++; },
    core: null,
    mainWindow: { webContents: sender, isDestroyed: () => false },
    ipcMain: { handle: (name, callback) => handlers.set(name, callback) },
  };
  vm.createContext(context);
  vm.runInContext(source.slice(start, end), context);
  const set = handlers.get("ime:tasks:set");
  assert.throws(() => set({ sender: {} }, true), /main_window_required/);
  assert.equal(writes, 0);
  assert.equal(set({ sender }, true).requestedEnabled, true);
  assert.equal(writes, 1);
  assert.throws(() => set({ sender }, "true"), /开关/);
  assert.equal(writes, 1);
  assert.deepEqual(Object.keys(handlers.get("ime:tasks:status")()).sort(), ["activeEnabled", "requestedEnabled", "restartRequired"]);
});
