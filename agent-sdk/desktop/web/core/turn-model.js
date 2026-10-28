// 回合请求体与模型连接的合流：UI 只拿到"当前选择"，这里负责把它变成
// 服务端 turn 请求里的 model_connection（不覆盖调用方显式给的字段）。
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.OwoTurnModel = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  /**
   * @param {object} body        原始 turn 请求体（prompt/session 等）
   * @param {object|null} connection  buildModelConnection() 的产物或 null
   * @returns {object} 新的请求体；connection 为空时原样返回副本
   */
  function withModelConnection(body, connection) {
    const request = Object.assign({}, body || {});
    if (!connection || !connection.model) return request;
    request.model_connection = Object.assign({}, connection);
    return request;
  }

  /** 展示用：连接是否"非默认"（不同提供商/模型时 UI 需要提示"本回合已切换"）。 */
  function isCustomConnection(connection, defaults) {
    if (!connection) return false;
    const base = String(defaults && defaults.base_url ? defaults.base_url : "")
      .trim()
      .replace(/\/+$/, "")
      .toLowerCase();
    const model = String(defaults && defaults.model ? defaults.model : "").trim();
    const connBase = String(connection.base_url || "")
      .trim()
      .replace(/\/+$/, "")
      .toLowerCase();
    const connModel = String(connection.model || "").trim();
    return (base && connBase && base !== connBase) || (model && connModel && model !== connModel);
  }

  /** 从 /settings 的 provider 视图推断默认连接（用于"是否已切换"的对比）。 */
  function defaultConnectionFromSettings(settings) {
    if (!settings || typeof settings !== "object") return null;
    const provider = settings.provider || {};
    const model = String(provider.model || (settings.model_effective || settings.model) || "").trim();
    const baseUrl = String(provider.base_url || "").trim();
    if (!model && !baseUrl) return null;
    return { model, base_url: baseUrl };
  }

  return { withModelConnection, isCustomConnection, defaultConnectionFromSettings };
});
