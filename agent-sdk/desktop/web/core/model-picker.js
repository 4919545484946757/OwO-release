// 模型选择器的状态逻辑：会话级记忆 + 提供商/模型切换 + 展示文案。
// 纯函数、无 DOM，便于单测；UI 只负责渲染与调用。
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.OwoModelPicker = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  const STORAGE_PREFIX = "owo.model.";

  function storageKey(sessionId) {
    return STORAGE_PREFIX + String(sessionId || "default");
  }

  /** 读取会话上次选择（localStorage 可注入，测试用内存实现）。 */
  function readSelection(storage, sessionId) {
    try {
      const raw = storage && storage.getItem(storageKey(sessionId));
      if (!raw) return null;
      const parsed = JSON.parse(raw);
      if (!parsed || typeof parsed !== "object") return null;
      if (!parsed.providerId && !parsed.model) return null;
      return {
        providerId: String(parsed.providerId || "custom"),
        model: String(parsed.model || ""),
        baseUrl: String(parsed.baseUrl || ""),
      };
    } catch (error) {
      return null;
    }
  }

  function writeSelection(storage, sessionId, selection) {
    if (!storage || !selection) return null;
    const normalized = {
      providerId: String(selection.providerId || "custom"),
      model: String(selection.model || ""),
      baseUrl: String(selection.baseUrl || ""),
    };
    try {
      storage.setItem(storageKey(sessionId), JSON.stringify(normalized));
    } catch (error) {
      // 隐私模式/配额满：选择仍然在内存里生效，不阻断对话。
    }
    return normalized;
  }

  /**
   * 应用一次选择：
   * - 选预设且没填地址 → 用预设地址；
   * - 换提供商但沿用旧模型 → 回落到新提供商默认模型（避免把 glm 模型发给 openai）。
   */
  function applySelection(providers, current, next) {
    const catalog = providers || [];
    const preset = catalog.find((item) => String(item.id) === String(next.providerId)) || null;
    const providerChanged = !current || String(current.providerId) !== String(next.providerId);
    const presetModels = preset && Array.isArray(preset.models) ? preset.models : [];

    let model = String(next.model || "").trim();
    if (!model || (providerChanged && !presetModels.includes(model) && presetModels.length > 0)) {
      model = presetModels[0] || model;
    }
    const baseUrl =
      String(next.baseUrl || "").trim() || (preset ? String(preset.base_url || "") : "") ||
      String((current && current.baseUrl) || "");

    return {
      providerId: preset ? String(preset.id) : String(next.providerId || "custom"),
      model,
      baseUrl,
    };
  }

  /** 胶囊文案：模型名太长时保留尾部（版本号通常在后半段）。 */
  function labelFor(selection, maxLength) {
    const limit = Number(maxLength) || 28;
    const model = String((selection && selection.model) || "").trim();
    if (!model) return "选择模型";
    if (model.length <= limit) return model;
    return "…" + model.slice(model.length - (limit - 1));
  }

  /** 该选择是否具备某种多模态能力（无提供商信息时退化为 false）。 */
  function canModality(providers, selection, modality) {
    const preset = (providers || []).find(
      (item) => String(item.id) === String((selection && selection.providerId) || "")
    );
    if (!preset) return false;
    if (modality === "image_output") return (preset.image_models || []).length > 0;
    if (modality === "audio_output") return (preset.speech_models || []).length > 0;
    const chat = preset.chat_modalities || {};
    return Boolean(chat[modality]);
  }

  return { STORAGE_PREFIX, storageKey, readSelection, writeSelection, applySelection, labelFor, canModality };
});
