// Explicit desktop preference. Changing it never restarts or authorizes a task.
"use strict";
function enabled(config) { return config?.ime_tasks_enabled === true; }
function environment(config, base) {
  return { ...base, OWO_IME_TASKS_ENABLED: enabled(config) ? "true" : "false" };
}
function create({ readConfig, writeConfig, runtime }) {
  function status() {
    const requestedEnabled = enabled(readConfig());
    const core = runtime();
    const activeEnabled = core?.owned && core.ready && typeof core.imeTasksEnabled === "boolean"
      ? core.imeTasksEnabled : null;
    return {
      requestedEnabled, activeEnabled,
      restartRequired: activeEnabled === null ? requestedEnabled : requestedEnabled !== activeEnabled,
    };
  }
  function setEnabled(value) {
    if (typeof value !== "boolean") throw new Error("输入法任务开关必须为开或关");
    const current = readConfig();
    writeConfig({ ...current, ime_tasks_enabled: value });
    return status();
  }
  return { status, setEnabled };
}
module.exports = { enabled, environment, create };
