(function (root) {
  "use strict";
  let cache = [], problem = "", initialized = false;
  function legacyModels() { try { const value = JSON.parse(root.localStorage.getItem("owo.custom-models") || "[]"); return Array.isArray(value) ? value : []; } catch { return []; } }
  function apply(snapshot) { cache = Array.isArray(snapshot?.models) ? snapshot.models : []; problem = snapshot?.error || (snapshot?.encryptionAvailable === false ? "credentials/encryption_unavailable" : ""); initialized = true; return cache; }
  function bridge() { return root.owo?.customModels; }
  async function init() {
    const old = legacyModels();
    if (!bridge()) { cache = old.map(model => ({ ...model, migrationPending: true, hasKey: false })); initialized = true; problem = "credentials/desktop_required"; return cache; }
    try {
      apply(await bridge().list());
      if (old.length) {
        let keys; try { keys = JSON.parse(root.localStorage.getItem("owo.model-keys") || "{}"); } catch { throw Error("credentials/legacy_corrupt"); }
        const result = await bridge().migrate({ models: old, keys });
        apply(result);
        const ids = new Set(result.migratedIds || []);
        const current = legacyModels(), currentKeys = JSON.parse(root.localStorage.getItem("owo.model-keys") || "{}");
        const cleanup = new Set(current.filter(model => ids.has(model.id) &&
          JSON.stringify(model) === JSON.stringify(old.find(item => item.id === model.id)) &&
          currentKeys[model.id] === keys[model.id]).map(model => model.id));
        for (const id of cleanup) delete currentKeys[id];
        const kept = current.filter(model => !cleanup.has(model.id));
        if (kept.length) root.localStorage.setItem("owo.custom-models", JSON.stringify(kept)); else root.localStorage.removeItem("owo.custom-models");
        if (Object.keys(currentKeys).length) root.localStorage.setItem("owo.model-keys", JSON.stringify(currentKeys)); else root.localStorage.removeItem("owo.model-keys");
        if (result.conflictedIds?.length) problem = "credentials/migration_conflict";
      }
    } catch { problem = "credentials/migration_pending"; }
    for (const model of old) if (!cache.some(item => item.id === model.id)) cache.push({ ...model, migrationPending: true, hasKey: false });
    initialized = true; return cache;
  }
  async function upsert(model, apiKey, previousId) {
    if (!bridge()) throw Error("自定义模型凭据需要在桌面端配置");
    apply(await bridge().upsert({ ...model, apiKey, previousId })); return cache;
  }
  async function remove(id) { if (!bridge()) throw Error("此操作需要桌面端"); apply(await bridge().remove(id)); return cache; }
  async function probe(input) { if (!bridge()) throw Error("此操作需要桌面端"); return bridge().probe(input); }
  root.OwoCustomModels = Object.freeze({ init, models: () => cache.slice(), upsert, remove, probe, status: () => ({ initialized, problem, desktop: Boolean(bridge()) }) });
})(typeof window !== "undefined" ? window : globalThis);
