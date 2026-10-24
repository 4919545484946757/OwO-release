// 输入区的"模型胶囊 + 选择面板"渲染（纯字符串，不直接操作 DOM）：
// app.js 只需要把返回值插进容器，再在自己的事件委托里调用 onPick 逻辑。
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.OwoModelBar = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  function escapeHtml(value) {
    return String(value == null ? "" : value)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function capabilityBadges(preset) {
    if (!preset) return "";
    const chat = preset.chat_modalities || {};
    const badges = [];
    if (chat.image_input) badges.push("图入");
    if ((preset.image_models || []).length > 0) badges.push("图出");
    if (chat.audio_input) badges.push("音入");
    if ((preset.speech_models || []).length > 0) badges.push("TTS");
    return badges.map((text) => `<span class="mb-badge">${escapeHtml(text)}</span>`).join("");
  }

  /** 胶囊按钮（输入区右下角）。 */
  function capsuleHtml(selection, label, switched) {
    const text = escapeHtml(label || "选择模型");
    return (
      `<button type="button" class="mb-capsule${switched ? " mb-switched" : ""}"` +
      ` data-owo-model-bar="toggle" title="切换模型 / 提供商" aria-haspopup="dialog">` +
      `<span class="mb-dot"></span><span class="mb-label">${text}</span></button>`
    );
  }

  /** 选择面板：提供商分组 + 每个模型一行 + 自定义入口。 */
  function panelHtml(providers, selection) {
    const list = Array.isArray(providers) ? providers : [];
    const currentProvider = String((selection && selection.providerId) || "");
    const currentModel = String((selection && selection.model) || "");
    const groups = list
      .map((preset) => {
        const models = Array.isArray(preset.models) ? preset.models : [];
        const rows = models
          .map((model) => {
            const active = preset.id === currentProvider && model === currentModel;
            return (
              `<button type="button" class="mb-item${active ? " mb-active" : ""}"` +
              ` data-provider="${escapeHtml(preset.id)}" data-model="${escapeHtml(model)}">` +
              `<span class="mb-item-name">${escapeHtml(model)}</span>` +
              (active ? '<span class="mb-check">✓</span>' : "") +
              `</button>`
            );
          })
          .join("");
        const head =
          `<div class="mb-group"><div class="mb-group-head">` +
          `<span class="mb-group-name">${escapeHtml(preset.name || preset.id)}</span>` +
          capabilityBadges(preset) +
          `</div>${rows}` +
          (preset.note ? `<div class="mb-note">${escapeHtml(preset.note)}</div>` : "") +
          `</div>`;
        return models.length > 0 || preset.id === "custom" ? head : "";
      })
      .join("");
    const custom =
      `<div class="mb-custom"><div class="mb-group-head"><span class="mb-group-name">自定义</span></div>` +
      `<input class="mb-input" data-owo-model-bar="baseUrl" placeholder="Base URL（如 https://api.example.com/v1）" value="${escapeHtml(
        (selection && selection.baseUrl) || ""
      )}">` +
      `<input class="mb-input" data-owo-model-bar="model" placeholder="模型名（如 local-7b）" value="${escapeHtml(
        currentProvider === "custom" ? currentModel : ""
      )}">` +
      `<input class="mb-input" data-owo-model-bar="apiKey" type="password" placeholder="API Key（仅本机保存，可留空）">` +
      `<button type="button" class="mb-apply" data-owo-model-bar="applyCustom">使用该连接</button></div>`;
    return `<div class="mb-panel" data-owo-model-bar="panel" role="dialog" aria-label="选择模型与提供商">${groups}${custom}</div>`;
  }

  return { escapeHtml, capabilityBadges, capsuleHtml, panelHtml };
});
