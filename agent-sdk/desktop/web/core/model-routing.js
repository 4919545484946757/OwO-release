(function (root) {
  "use strict";
  function effectiveDefaultModel(settings) {
    const value = settings && settings.runtime && settings.runtime.model
      ? settings.runtime.model
      : settings && settings.model;
    return String(value || "").trim();
  }
  function buildCustomModelConnection(id, models, keys) {
    const model = (Array.isArray(models) ? models : []).find((item) => item && item.id === id);
    if (!model) return null;
    if (model.disabled) throw new Error("该自定义模型已移除，请选择其他模型");
    if (model.migrationPending) throw new Error("此模型尚未完成安全迁移，请在桌面端修复配置");
    const baseUrl = String(model.baseUrl || "").trim();
    let url;
    try { url = new URL(baseUrl); } catch (_) { throw new Error("请填写自定义模型的请求地址"); }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
      throw new Error("自定义模型地址必须为不含凭据、查询参数或片段的 HTTP(S) 地址");
    }
    const format = String(model.apiFormat || "openai");
    if (!["openai", "anthropic"].includes(format)) throw new Error("不支持的自定义模型接口格式");
    const request = {
      model: String(model.id).trim(), base_url: baseUrl, api_format: format,
      use_full_url: model.useFullUrl === true,
    };
    const key = String((keys && keys[model.id]) || "").trim();
    if (model.apiKeyEnv) {
      if (!/^OWO_CUSTOM_MODEL_SECRET_[a-fA-F0-9]{32}$/.test(model.apiKeyEnv)) throw new Error("无效的模型凭据引用");
      request.api_key_env = model.apiKeyEnv;
    } else if (key) request.api_key = key;
    if (model.temperature !== undefined) {
      const value = Number(model.temperature);
      if (!Number.isFinite(value) || value < 0 || value > 2) throw new Error("温度必须在 0 到 2 之间");
      request.temperature = value;
    }
    if (model.timeoutSecs !== undefined) {
      const value = Number(model.timeoutSecs);
      if (!Number.isInteger(value) || value < 1 || value > 3600) throw new Error("超时必须为 1 到 3600 秒");
      request.timeout_secs = value;
    }
    return request;
  }
  function buildSessionModelRequest(model) {
    const value = String(model || "").trim();
    return { model: value || null };
  }
  function buildCreateSessionRequest(workspace, explicitModel) {
    const request = { workspace: String(workspace || "").trim() };
    const model = String(explicitModel || "").trim();
    if (model) request.model = model;
    return request;
  }
  function createSessionModelUpdateQueue(update) {
    if (typeof update !== "function") throw new TypeError("update must be a function");
    const tails = new Map();
    const revisions = new Map();
    function enqueue(sessionId, model) {
      const id = String(sessionId || "").trim();
      if (!id) return Promise.reject(new Error("sessionId is required"));
      const revision = (revisions.get(id) || 0) + 1;
      revisions.set(id, revision);
      const previous = tails.get(id) || Promise.resolve();
      const task = previous.catch(() => undefined).then(() =>
        update(id, buildSessionModelRequest(model))
      );
      tails.set(id, task);
      return task.then(
        () => ({ revision, latest: revisions.get(id) === revision, error: null }),
        (error) => ({ revision, latest: revisions.get(id) === revision, error })
      ).finally(() => {
        if (tails.get(id) === task) tails.delete(id);
      });
    }
    return Object.freeze({ enqueue });
  }
  function normalizeProviderId(provider) {
    const id = String(provider || "").trim().toLowerCase();
    if (id === "qwen" || id === "aliyun" || id === "alibaba") return "dashscope";
    if (id === "zhipu" || id === "glm") return "bigmodel";
    return id;
  }
  function findProviderPreset(provider, presets, baseUrl) {
    const list = Array.isArray(presets) ? presets : [];
    const normalized = normalizeProviderId(provider);
    const byId = normalized && list.find((preset) => normalizeProviderId(preset && preset.id) === normalized);
    if (byId) return byId;
    const strip = (url) => String(url || "").trim().toLowerCase().replace(/^https?:\/\//, "");
    const endpoint = strip(baseUrl);
    if (!endpoint) return null;
    return list.find((preset) => {
      const candidate = strip(preset && preset.baseUrl);
      return candidate && endpoint.includes(candidate);
    }) || null;
  }
  function providerModelOptionState(model, supportedModels, currentModel, custom) {
    const supported = Array.isArray(supportedModels) ? supportedModels.map(String) : [];
    if (custom || supported.length === 0) return { hidden: false, disabled: false };
    const compatible = supported.includes(String(model || ""));
    const isCurrent = String(model || "") === String(currentModel || "");
    return { hidden: !compatible && !isCurrent, disabled: !compatible && isCurrent };
  }
  function providerModelCompatibility(model, supportedModels, custom) {
    const value = String(model || "").trim();
    const supported = Array.isArray(supportedModels) ? supportedModels.map(String) : [];
    if (!value || custom || supported.length === 0) return { known: false, compatible: true };
    return { known: true, compatible: supported.includes(value) };
  }
  const api = {
    effectiveDefaultModel,
    buildCustomModelConnection,
    buildSessionModelRequest,
    buildCreateSessionRequest,
    createSessionModelUpdateQueue,
    providerModelOptionState,
    providerModelCompatibility,
    normalizeProviderId,
    findProviderPreset,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.OwoModelRouting = api;
})(typeof window !== "undefined" ? window : globalThis);
