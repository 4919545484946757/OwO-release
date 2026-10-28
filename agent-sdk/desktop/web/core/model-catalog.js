// 提供商预设的浏览器侧目录：拉取 /providers/presets、缓存、按地址识别、按能力筛选。
// 纯函数 + 可注入 fetch，便于 node 单测；不直接操作 DOM。
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  root.OwoModelCatalog = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  const CACHE_TTL_MS = 5 * 60 * 1000;

  function normalize(baseUrl) {
    return String(baseUrl || "").trim().replace(/\/+$/, "").toLowerCase();
  }

  function hostOf(baseUrl) {
    const value = normalize(baseUrl);
    if (!value) return "";
    const withoutScheme = value.replace(/^[a-z]+:\/\//, "");
    return withoutScheme.split("/")[0];
  }

  function byId(providers, id) {
    const needle = String(id || "").trim().toLowerCase();
    return (providers || []).find((item) => String(item.id).toLowerCase() === needle) || null;
  }

  /** 用户只填地址时反查提供商（精确前缀 > 同域名）。 */
  function detect(providers, baseUrl) {
    const target = normalize(baseUrl);
    if (!target) return null;
    const list = (providers || []).filter((item) => normalize(item.base_url));
    const exact = list.find((item) => target === normalize(item.base_url));
    if (exact) return exact;
    const host = hostOf(target);
    return (
      list.find((item) => {
        const presetHost = hostOf(item.base_url);
        return host && presetHost && (host === presetHost || host.endsWith("." + presetHost));
      }) || null
    );
  }

  /** 预设可用模型；输入自定义模型名时透传（个性化端点常见）。 */
  function modelsFor(providers, providerId, customModel) {
    const preset = byId(providers, providerId);
    const models = preset && Array.isArray(preset.models) ? preset.models.slice() : [];
    const extra = String(customModel || "").trim();
    if (extra && !models.includes(extra)) models.unshift(extra);
    return models;
  }

  /** 能力查询：text / image_input / image_output / audio_input / audio_output。 */
  function supports(providers, providerId, modality) {
    const preset = byId(providers, providerId);
    if (!preset) return false;
    if (modality === "image_output") return (preset.image_models || []).length > 0;
    if (modality === "audio_output") return (preset.speech_models || []).length > 0;
    const chat = preset.chat_modalities || {};
    return Boolean(chat[modality]);
  }

  /** 表单 → 请求体：选预设时自动补 base_url，自定义时原样。 */
  function buildProviderPayload(providers, form) {
    const preset = byId(providers, form && form.providerId);
    const baseUrl = String((form && form.baseUrl) || "").trim() || (preset ? preset.base_url : "");
    const model =
      String((form && form.model) || "").trim() || (preset ? (preset.models || [])[0] || "" : "");
    return {
      provider: preset ? preset.id : "custom",
      base_url: baseUrl,
      model,
      api_key: String((form && form.apiKey) || "").trim(),
    };
  }

  /** 预设 → 表单初值（桌面端"添加模型"表单直接套用）。 */
  function presetToForm(preset) {
    if (!preset) return null;
    return {
      providerId: String(preset.id || "custom"),
      baseUrl: String(preset.base_url || ""),
      model: (preset.models || [])[0] || "",
      apiFormat: "openai",
      allowsEmptyKey: Boolean(preset.allows_empty_key),
      name: String(preset.name || preset.id || ""),
    };
  }

  /** 构建回合级自定义连接（对应服务端 protocol::CustomModelConnection）。 */
  function buildModelConnection(providers, form) {
    const preset = byId(providers, form && form.providerId);
    const baseUrl =
      String((form && form.baseUrl) || "").trim() || (preset ? preset.base_url : "");
    const model =
      String((form && form.model) || "").trim() || (preset ? (preset.models || [])[0] || "" : "");
    const key = String((form && form.apiKey) || "").trim();
    const connection = { model, base_url: baseUrl, api_format: "openai" };
    if (key) connection.api_key = key;
    return connection;
  }

  async function load(fetchImpl, basePath) {
    const fetcher = fetchImpl || (typeof fetch === "function" ? fetch : null);
    if (!fetcher) return [];
    const response = await fetcher(`${basePath || ""}/providers/presets`, {
      headers: { accept: "application/json" },
    });
    if (!response || !response.ok) return [];
    const body = await response.json();
    const providers = Array.isArray(body) ? body : body.providers;
    return Array.isArray(providers) ? providers : [];
  }

  return {
    CACHE_TTL_MS,
    normalize,
    hostOf,
    byId,
    detect,
    modelsFor,
    supports,
    buildProviderPayload,
    buildModelConnection,
    presetToForm,
    load,
  };
});
