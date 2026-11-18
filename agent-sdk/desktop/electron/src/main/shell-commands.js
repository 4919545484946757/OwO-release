"use strict";

// ADR-003：Tauri 兼容命令桥的**纯逻辑层**。
//
// 背景（为什么要这层）：web 工作台（desktop/web）对壳的调用面是
// `window.__TAURI_INTERNALS__.invoke(name, args)`，散落在 api-client.js、
// folder-picker.js、settings-panel.view.js、setup-guide.view.js、
// service-error.view.js、status-bar.view.js 六处共 14 个命令。Tauri 壳移除后
// 这个全局对象消失，会让「模型配置保存」「原生目录选择器」「诊断页操作」全部
// 静默降级成"非桌面环境"——UI 长得一模一样，功能却丢了一半。
//
// 对策：主进程按**同名同形状**实现这 14 个命令，并在 preload 里原样暴露该全局
// 对象。desktop/web 一行不改 —— 这正是"UI 与功能一模一样"的实现方式。
//
// 本文件刻意不 import electron：语义（默认值、掩码、读改写、状态形状）全部可
// 由 node --test 直接断言（见 tests/shell-commands.test.mjs）。

const supervision = require("./core-supervision.js");

// 与旧 Tauri provider.rs 的 ProviderMode::default_base_url / default_model 逐字一致。
const PROVIDER_DEFAULTS = {
  bigmodel: { base_url: "https://open.bigmodel.cn/api/paas/v4", model: "glm-5.3-flash" },
  openai: { base_url: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  deepseek: { base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  dashscope: { base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus" },
  ollama: { base_url: "http://127.0.0.1:11434/v1", model: "local" },
  custom: { base_url: "", model: "" },
  unset: { base_url: "", model: "" },
};

const DEFAULT_KEY_ENV = "OPENAI_API_KEY";
const LEGACY_KEY_ENV = "DASHSCOPE_API_KEY";

// 旧壳的 14 个命令名。加命令必须同步这里（测试据此对账 web 侧的实际调用）。
const SHELL_COMMANDS = [
  "apply_relay_trial",
  "choose_data_directory",
  "choose_project_directory",
  "clear_workspace",
  "create_project_workspace",
  "desktop_pairing",
  "get_core_connection",
  "get_core_state",
  "get_provider_status",
  "get_workspace",
  "open_core_logs",
  "open_project_directory",
  "relay_trial_status",
  "reload_model_config",
  "retry_core_start",
  "reveal_model_config",
  "set_model_config",
  "set_provider",
  "set_workspace",
];

function validateProjectFolderName(raw) {
  const name = String(raw == null ? "" : raw).trim();
  if (!name) return { ok: false, error: "项目文件夹名称不能为空" };
  if (name === "." || name === ".." || /[<>:"|?*]/.test(name) ||
      name.includes("/") || name.includes("\\") || /[\x00-\x1f]/.test(name) ||
      /[. ]$/.test(name)) {
    return { ok: false, error: "项目名称不能包含路径分隔符、Windows 保留字符，且不能以点或空格结尾" };
  }
  const stem = name.split(".")[0].toLowerCase();
  if (["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5",
       "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4",
       "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"].includes(stem)) {
    return { ok: false, error: "该名称是 Windows 保留设备名，请换一个名称" };
  }
  return { ok: true, name };
}
function canonicalProvider(model) {
  const raw = model && typeof model.provider === "string" ? model.provider : "";
  return supervision.normalizeProvider(raw) || "unset";
}

function effectiveBaseUrl(model) {
  const explicit = model && typeof model.base_url === "string" ? model.base_url.trim() : "";
  if (explicit) return explicit;
  return PROVIDER_DEFAULTS[canonicalProvider(model)].base_url;
}

function effectiveModel(model) {
  // 历史字段别名：文件里可能写作 model（provider.rs 的 serde alias）。
  const explicit = model && typeof model.name === "string" ? model.name.trim() : "";
  if (explicit) return explicit;
  return PROVIDER_DEFAULTS[canonicalProvider(model)].model;
}

function keyEnvName(model) {
  const raw = model && typeof model.api_key_env === "string" ? model.api_key_env.trim() : "";
  return raw || DEFAULT_KEY_ENV;
}

// 掩码展示：sk-1234…cdef（前后各 4 位；长度不足则整体星号）。密钥本体永不外传。
function maskKey(key) {
  const text = String(key == null ? "" : key).trim();
  if (text.length <= 8) return "*".repeat(Math.max(text.length, 4));
  return `${text.slice(0, 4)}…${text.slice(-4)}`;
}

// 解析凭据（单一实现，等价于 provider.rs::resolve_api_key）。
// `envGet(name)` 由调用方注入，纯函数不直接读 process.env。
function resolveApiKey(model, envGet) {
  const get = typeof envGet === "function" ? envGet : () => "";
  const fileKey = model && typeof model.api_key === "string" ? model.api_key.trim() : "";
  if (fileKey) return { key: fileKey, source: "config_file" };

  const envName = keyEnvName(model);
  const fromNamed = String(get(envName) || "").trim();
  if (fromNamed) {
    return { key: fromNamed, source: envName === DEFAULT_KEY_ENV ? "environment" : "config_env" };
  }
  if (envName !== DEFAULT_KEY_ENV) {
    const fromDefault = String(get(DEFAULT_KEY_ENV) || "").trim();
    if (fromDefault) return { key: fromDefault, source: "environment" };
  }
  if (!model || model.api_key_env === undefined || model.api_key_env === null) {
    const fromLegacy = String(get(LEGACY_KEY_ENV) || "").trim();
    if (fromLegacy) return { key: fromLegacy, source: "legacy_env" };
  }
  return { key: "", source: "none" };
}

// get_provider_status 的返回形状（等价于 provider.rs::provider_status）。
// 注意 keyMasked 只给掩码，keyConfigured 只给布尔——前端拿不到可用凭据。

function isLocalModelEndpoint(baseUrl) {
  try {
    const host = new URL(baseUrl).hostname.toLowerCase();
    return host === "localhost" || host === "[::1]" || host === "::1" ||
      /^127(?:\.\d{1,3}){3}$/.test(host);
  } catch (_) { return false; }
}

// Presentation and child-process injection must resolve the same configuration.
function resolveModelConfig(model, envGet = () => "") {
  const source = model && typeof model === "object" ? model : {};
  const provider = canonicalProvider(source);
  const envValue = (name) => String(envGet(name) || "").trim();
  const baseUrl = provider === "unset"
    ? envValue("OPENAI_BASE_URL") || PROVIDER_DEFAULTS.bigmodel.base_url
    : effectiveBaseUrl(source);
  const name = provider === "unset"
    ? envValue("OPENAI_MODEL") || PROVIDER_DEFAULTS.bigmodel.model
    : effectiveModel(source);
  const credential = resolveApiKey(source, envGet);
  const local = isLocalModelEndpoint(baseUrl);
  return { provider, baseUrl, model: name, credential, local, ready: local || Boolean(credential.key) };
}

function applyModelEnvironment(env, model) {
  const target = env && typeof env === "object" ? env : {};
  const source = model && typeof model === "object" ? model : {};
  const resolved = resolveModelConfig(source, (name) => target[name]);
  if (resolved.provider !== "unset") {
    target.OPENAI_BASE_URL = resolved.baseUrl;
    target.OPENAI_MODEL = resolved.model;
    // A UI selection of an OpenAI-compatible provider replaces inherited native routing.
    target.OWO_PROVIDER = "openai";
  }
  const numbers = [
    ["context_window", ["OWO_MODEL_CONTEXT_WINDOW"], 1, Infinity],
    ["temperature", ["OWO_MODEL_TEMPERATURE"], 0, 2],
    ["timeout_secs", ["OWO_MODEL_TIMEOUT_SECS", "OWO_MODEL_REQUEST_TIMEOUT_SECS"], 1, 3600],
    ["keep_recent", ["OWO_AGENT_KEEP_RECENT"], 1, Infinity],
  ];
  for (const [field, names, min, max] of numbers) {
    // Missing = inherit. Explicit null/blank = clear and use the core default.
    if (!Object.prototype.hasOwnProperty.call(source, field)) continue;
    const raw = source[field], value = Number(raw);
    const valid = raw != null && String(raw).trim() !== "" && Number.isFinite(value) && (field === "temperature" || Number.isInteger(value)) && value >= min && value <= max;
    for (const name of names) {
      if (valid) target[name] = String(value);
      else delete target[name];
    }
  }
  if (Object.prototype.hasOwnProperty.call(source, "compaction")) {
    if (source.compaction === true) target.OWO_AGENT_COMPACTION = "1";
    else if (source.compaction === false) target.OWO_AGENT_COMPACTION = "0";
    else delete target.OWO_AGENT_COMPACTION;
  }
  applyModelOutputEnv(target, source);
  if (resolved.credential.key) target.OPENAI_API_KEY = resolved.credential.key;
  return target;
}

function providerStatusValue(model, options = {}) {
  const source = model && typeof model === "object" ? model : {};
  const effective = resolveModelConfig(source, options.envGet);
  const provider = effective.provider;
  const resolved = effective.credential;
  const ready = effective.ready;
  return {
    provider,
    deferred: source.deferred === true,
    baseUrl: effective.baseUrl,
    model: effective.model,
    keyConfigured: Boolean(resolved.key),
    keySource: resolved.source,
    keyMasked: resolved.key ? maskKey(resolved.key) : "",
    keyEnv: keyEnvName(source),
    ready,
    configPath: options.configPath || "",
    models: Array.isArray(source.models) ? source.models : [],
    contextWindow: numOrNull(source.context_window),
    maxOutputTokens: numOrNull(source.max_output_tokens) || supervision.DEFAULT_MODEL_OUTPUT_TOKENS,
    modelOutputTokens: normalizeModelOutputTokens(source.model_output_tokens || {}),
    temperature: numOrNull(source.temperature),
    timeoutSecs: numOrNull(source.timeout_secs),
    keepRecent: numOrNull(source.keep_recent),
    compaction: typeof source.compaction === "boolean" ? source.compaction : null,
  };
}

function numOrNull(value) {
  if (value === null || value === undefined || value === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function normalizeModelOutputTokens(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const normalized = {};
  for (const [rawName, rawLimit] of Object.entries(value)) {
    const name = String(rawName).trim();
    const limit = Number(rawLimit);
    if (name && Number.isInteger(limit) && limit >= 1 && limit <= supervision.MAX_MODEL_OUTPUT_TOKENS) normalized[name] = limit;
  }
  return normalized;
}

function applyModelOutputEnv(env, model) {
  const target = env && typeof env === "object" ? env : {};
  const source = model && typeof model === "object" ? model : {};
  delete target.OWO_MODEL_MAX_OUTPUT_TOKENS;
  delete target.OWO_MODEL_OUTPUT_TOKENS_BY_MODEL;
  const configuredDefault = Number(source.max_output_tokens);
  target.OWO_MODEL_MAX_OUTPUT_TOKENS = String(
    Number.isInteger(configuredDefault) && configuredDefault >= 1 && configuredDefault <= supervision.MAX_MODEL_OUTPUT_TOKENS
      ? configuredDefault
      : supervision.DEFAULT_MODEL_OUTPUT_TOKENS,
  );
  target.OWO_MODEL_OUTPUT_TOKENS_BY_MODEL = JSON.stringify(normalizeModelOutputTokens(source.model_output_tokens));
  return target;
}

// ---- 数值/布尔字段解析（空串、0、非法值 = 清除，回到核心默认） ----

function parsePositive(raw) {
  if (raw === null || raw === undefined) return { present: false };
  const parsed = Number(String(raw).trim());
  if (!Number.isFinite(parsed) || parsed <= 0) return { present: true, value: null };
  return { present: true, value: parsed };
}

function parseTemperature(raw) {
  if (raw === null || raw === undefined) return { present: false };
  const parsed = Number(String(raw).trim());
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 2) return { present: true, value: null };
  return { present: true, value: parsed };
}

function parseCompaction(raw) {
  if (raw === null || raw === undefined) return { present: false };
  const key = String(raw).trim().toLowerCase();
  if (["1", "true", "yes", "on"].includes(key)) return { present: true, value: true };
  if (["0", "false", "no", "off"].includes(key)) return { present: true, value: false };
  if (typeof raw === "boolean") return { present: true, value: raw };
  return { present: true, value: null };
}

function parseModelList(raw) {
  if (!Array.isArray(raw)) return { present: false };
  const cleaned = [];
  for (const item of raw) {
    const name = String(item == null ? "" : item).trim();
    if (name && !cleaned.includes(name)) cleaned.push(name);
  }
  return { present: true, value: cleaned };
}

// set_model_config 的读改写（等价于 commands.rs::set_model_config 的参数语义）：
// 未传（undefined）= 保持原样；空串 = 显式清除。密钥字段同理，避免前端
// "没传字段"把用户已存的密钥抹掉。
function applyModelConfigPatch(config, args) {
  const input = args && typeof args === "object" ? args : {};
  const model = { ...(config && config.model ? config.model : {}) };

  if (typeof input.mode === "string") {
    const canonical = supervision.normalizeProvider(input.mode);
    if (!canonical) return { ok: false, error: `未知模型提供方：${input.mode}` };
    model.provider = canonical;
    // Keep the explicit setup decision in the durable shell config. WebView
    // localStorage is origin-scoped and changes when the core chooses a port.
    model.deferred = canonical === "unset";
  }
  if (input.base_url !== undefined) {
    const text = String(input.base_url == null ? "" : input.base_url).trim();
    model.base_url = text || null;
  }
  if (input.model !== undefined || input.name !== undefined) {
    const raw = input.model !== undefined ? input.model : input.name;
    const text = String(raw == null ? "" : raw).trim();
    model.name = text || null;
  }
  if (input.api_key !== undefined) {
    const text = String(input.api_key == null ? "" : input.api_key).trim();
    model.api_key = text || null;
  }
  if (input.api_key_env !== undefined) {
    const text = String(input.api_key_env == null ? "" : input.api_key_env).trim();
    model.api_key_env = text || null;
  }

  const numericFields = {
    context_window: input.context_window,
    max_output_tokens: input.max_output_tokens,
    timeout_secs: input.timeout_secs,
    keep_recent: input.keep_recent,
  };
  for (const [field, raw] of Object.entries(numericFields)) {
    if (raw === undefined) continue;
    model[field] = parsePositive(raw).value;
  }
  if (input.temperature !== undefined) {
    model.temperature = parseTemperature(input.temperature).value;
  }
  if (input.compaction !== undefined) {
    model.compaction = parseCompaction(input.compaction).value;
  }
  if (input.model_output_tokens !== undefined) {
    if (!input.model_output_tokens || typeof input.model_output_tokens !== "object" || Array.isArray(input.model_output_tokens)) {
      return { ok: false, error: "model_output_tokens 必须是模型名到 token 上限的对象" };
    }
    const normalized = {};
    for (const [rawName, rawLimit] of Object.entries(input.model_output_tokens)) {
      const name = String(rawName).trim();
      if (!name || rawLimit === null || rawLimit === "") continue;
      const limit = Number(rawLimit);
      if (!Number.isInteger(limit) || limit < 1 || limit > supervision.MAX_MODEL_OUTPUT_TOKENS) {
        return { ok: false, error: `模型 ${name} 的输出上限必须是 1–${supervision.MAX_MODEL_OUTPUT_TOKENS.toLocaleString("en-US")} 的整数` };
      }
      normalized[name] = limit;
    }
    model.model_output_tokens = normalized;
  }
  const list = parseModelList(input.models);
  if (list.present) model.models = list.value;

  const next = { ...(config && typeof config === "object" ? config : {}), version: 1, model };
  const verdict = supervision.validateConfig(next, {});
  if (!verdict.ok) return { ok: false, error: verdict.errors.join("；"), config: next };
  return { ok: true, config: next };
}

module.exports = {
  PROVIDER_DEFAULTS,
  DEFAULT_KEY_ENV,
  SHELL_COMMANDS,
  validateProjectFolderName,
  canonicalProvider,
  effectiveBaseUrl,
  effectiveModel,
  keyEnvName,
  maskKey,
  resolveApiKey,
  providerStatusValue,
  applyModelOutputEnv,
  applyModelEnvironment,
  resolveModelConfig,
  applyModelConfigPatch,
  parsePositive,
  parseTemperature,
  parseCompaction,
  parseModelList,
};
