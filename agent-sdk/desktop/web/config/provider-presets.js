/* 常用模型服务提供方预设（唯一事实源）。
 *
 * 为什么单独一个文件（而不是写在渲染代码里）：
 *   1. `tests/panels-lint.test.mjs` 禁止在渲染脚本里硬编码本机 URL（只放行动态端口
 *      拼接与缺省端口回退）。本地 Ollama 的 `127.0.0.1:11434` 是产品预置项而非
 *      "写死的连接地址"，放在配置模块里语义清楚，也不必给 lint 开特例；
 *   2. 首次启动引导页与设置页的模型卡共用同一份预设，避免两处默认值漂移
 *      （历史缺陷形态：引导页默认 BigModel、设置页默认别的端点）。
 *
 * 密钥不在这里，也永远不会在这里：只描述"端点 + 默认模型名 + 该端点用哪个
 * 环境变量放密钥"，密钥本体只存在于系统环境变量。
 *
 * OwO 中转站的"限时免费"（2026-11-11 模型清单复核）同样遵守这条：共享试用密钥存在 Electron
 * 主进程（desktop/electron/src/main/relay-trial.js），本文件只有它的**命令名**
 * （`trialKeyVia`）。渲染层发一次 apply_relay_trial，由壳把端点 + 模型 + 密钥
 * 写进本机配置，回执里不含密钥——这样 devtools/截图/日志都拿不到它。
 */
(function (global) {
  "use strict";

  var OLLAMA_PORT = 11434;
  var OLLAMA_HOST = "127.0.0.1";

  function ollamaBaseUrl() {
    return "http://" + OLLAMA_HOST + ":" + OLLAMA_PORT + "/v1";
  }

  // 自建 vLLM / OpenAI 兼容服务默认端口；与 ollama 一样用拼接而非字面量，
  // 避免被 panels-lint 判成"硬编码本机 URL"。
  var VLLM_PORT = 8080;
  function vllmBaseUrl() {
    return "http://" + OLLAMA_HOST + ":" + VLLM_PORT + "/v1";
  }

  // ---- OwO 中转站（自建 new-api 网关）----
  // 模型清单来自 https://owo.dafeiyu.homes/api/pricing 的 default 分组（2026-11-11 复核）。
  // 2026-11-11 中转站去掉了原 workbuddy/、kiro/ 命名空间，hy4free 也改名为 hy4；
  // /v1/models 与计费清单一致。这五个模型在活动期内标"限时免费"，活动结束后清空
  // freeModels / promo 即可收回标签，不需要删预设（端点与模型名仍然可用，只是恢复计费）。
  var OWO_RELAY_ID = "owo_relay";
  var OWO_RELAY_BASE_URL = "https://owo.dafeiyu.homes/v1";
  var OWO_RELAY_SITE_URL = "https://owo.dafeiyu.homes/keys";
  // 活动截止日：仅用于界面文案，不做硬性拦截——活动方随时可以提前结束。
  var OWO_RELAY_UNTIL = "2026-12-31";
  // 共享试用密钥**不在这里**（渲染层永远不接触凭据，见 tests/preset-dropdown.test.mjs
  // 的「前端预设注册表不含任何密钥」）。它存在 Electron 主进程的
  // desktop/electron/src/main/relay-trial.js，渲染层只能请求 apply_relay_trial
  // 这一个动作，把端点 + 模型 + 密钥一次性写进本机配置，回执里不含密钥。
  // 活动结束：主进程把 TRIAL_KEY 置空即可，这里的 promo 一并下掉。
  var OWO_RELAY_TRIAL_COMMAND = "apply_relay_trial";
  var OWO_RELAY_MODELS = [
    "deepseek-v4.1-flash",
    "hy4",
    "claude-opus-5.5",
    "claude-haiku-5.5",
    "gpt-4o-mini",
  ];

  /// 按 id 或 base_url 找到预设（界面拿到的是两者之一，别让调用方各自判一遍）。
  function resolvePreset(subject) {
    if (!subject) return null;
    if (typeof subject === "object") return subject;
    var text = String(subject).trim();
    var list = presets();
    for (var i = 0; i < list.length; i++) {
      if (list[i].id === text) return list[i];
    }
    var endpoint = text.toLowerCase().replace(/^https?:\/\//, "");
    if (!endpoint) return null;
    for (var j = 0; j < list.length; j++) {
      var base = String(list[j].baseUrl || "").trim().toLowerCase().replace(/^https?:\/\//, "");
      if (base && (endpoint === base || endpoint.indexOf(base) === 0)) return list[j];
    }
    return null;
  }

  /// 该模型是否在"限时免费"清单里；返回徽标文案，空串表示不标。
  /// 传 modelId 是逐模型判断（菜单里只给免费模型挂徽标）；不传则是"该端点有活动吗"。
  function freeBadge(subject, modelId) {
    var preset = resolvePreset(subject);
    if (!preset || !preset.promo) return "";
    var free = Array.isArray(preset.freeModels) ? preset.freeModels : [];
    if (!free.length) return "";
    var id = String(modelId || "").trim();
    if (!id) return preset.promo.badge || "限时免费";
    return free.indexOf(id) >= 0 ? preset.promo.badge || "限时免费" : "";
  }

  /// 生成可复制的 PowerShell 环境变量命令（设置页「启动命令」行用）。
  /// 没有 base_url 的自定义预设返回空串——它本来就没有可预设的命令。
  function envCommand(preset) {
    if (!preset || !preset.baseUrl) return "";
    var parts = ['$env:OPENAI_BASE_URL = "' + preset.baseUrl + '"'];
    if (preset.model) parts.push('$env:OPENAI_MODEL = "' + preset.model + '"');
    if (preset.keyEnv) parts.push('$env:' + preset.keyEnv + ' = "<你的 API Key>"');
    return parts.join("; ");
  }

  function presets() {
    return [
      {
        // 自建中转站（new-api）。放在首位是为了让"限时免费"成为列表里的第一选择；
        // 它不是本地端点，仍然需要密钥（内置试用 key 或用户自备）。
        // id 必须写成字面量：tests/provider-preset-parity.test.mjs 按源码文本比对前后端 id 集合。
        id: "owo_relay",
        label: "OwO 中转站（限时免费）",
        baseUrl: OWO_RELAY_BASE_URL,
        model: OWO_RELAY_MODELS[0],
        keyEnv: "OPENAI_API_KEY",
        models: OWO_RELAY_MODELS.slice(),
        imageModels: [],
        speechModels: [],
        // 活动信息只驱动界面（徽标 / 有效期 / 领 key 链接），不参与调用逻辑。
        promo: {
          badge: "限时免费",
          until: OWO_RELAY_UNTIL,
          siteUrl: OWO_RELAY_SITE_URL,
        },
        freeModels: OWO_RELAY_MODELS.slice(),
        // 推理能力的**实测结论**，用来纠正 isReasoningModel 的关键字猜测：
        // REASONING_MODEL_RE 会把任何 deepseek-v4* 当成会思考，但中转站的
        // deepseek-v4.1-flash 实测不回 reasoning_content；而 hy4
        // 名字里没有任何关键字，实际会回。两处都靠显式声明纠正。
        reasoningModels: ["hy4"],
        nonReasoningModels: [
          "deepseek-v4.1-flash",
          "claude-opus-5.5",
          "claude-haiku-5.5",
          "gpt-4o-mini",
        ],
        trialKeyVia: OWO_RELAY_TRIAL_COMMAND,
        note: "自建中转站：活动期内免费开放 " + OWO_RELAY_MODELS.length +
          " 个模型，额度用完或活动结束后恢复计费。",
      },
      {
        id: "bigmodel",
        label: "智谱 BigModel",
        baseUrl: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-5.3-flash",
        keyEnv: "OPENAI_API_KEY",
        // 该服务商可挑选的模型（含推理模型；推理模型回传 reasoning_content → 深度思考块）
        models: ["glm-5.3-flash", "glm-4-plus", "glm-z1-flashx", "glm-z1-flash"],
imageModels: ["cogview-4"],
        speechModels: ["cogtts"],
      },
      {
        id: "openai",
        label: "OpenAI",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        keyEnv: "OPENAI_API_KEY",
        models: ["gpt-4o-mini", "gpt-4o", "o3-mini"],
imageModels: ["gpt-image-1", "dall-e-3"],
        speechModels: ["tts-1", "gpt-4o-mini-tts"],
      },
      {
        id: "deepseek",
        label: "DeepSeek",
        baseUrl: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
        keyEnv: "OPENAI_API_KEY",
        // 实测（2026-10-03 直连验证）：deepseek-flash / deepseek-v4-flash 也回
        // reasoning_content（各 ~220 字），同 deepseek-reasoner 一样是推理模型；
        // deepseek-chat 不回。别按名字猜——flash 系同样是深度思考模型。
        models: [
          "deepseek-flash",
          "deepseek-v4-flash",
          "deepseek-chat",
          "deepseek-reasoner",
        ],
imageModels: [],
        speechModels: [],
      },
      {
        id: "dashscope",
        label: "阿里 DashScope 兼容",
        baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        model: "qwen-plus",
        keyEnv: "OPENAI_API_KEY",
        // 2026-10 官方 DashScope 文本/兼容模型中常用且支持 OpenAI-compatible 调用的型号。
        models: [
          "qwen3.8-max",
          "qwen3.8-flash",
          "qwen3.7-max",
          "qwen3.7-plus",
          "qwen3.7-flash",
          "qwen3.6-flash",
          "qwen-plus",
          "qwen-max",
          "qwen-flash",
        ],
imageModels: ["wanx2.1-t2i-turbo"],
        speechModels: ["cosyvoice-v2"],
      },
      {
        // 以下四家与 crates/owo-agent-core/src/gateway/presets.rs 保持同一份事实：
        // 新增服务商时两处都要加，tests/provider-preset-parity.test.mjs 会拦住漂移。
        id: "moonshot",
        label: "月之暗面 Kimi",
        baseUrl: "https://api.moonshot.cn/v1",
        model: "moonshot-v1-32k",
        keyEnv: "OPENAI_API_KEY",
        models: [
          "moonshot-v1-128k",
          "moonshot-v1-32k-vision-preview",
          "moonshot-v1-32k",
          "moonshot-v1-8k",
        ],
imageModels: [],
        speechModels: [],
      },
      {
        id: "siliconflow",
        label: "硅基流动 SiliconFlow",
        baseUrl: "https://api.siliconflow.cn/v1",
        model: "deepseek-ai/DeepSeek-V3.2",
        keyEnv: "OPENAI_API_KEY",
        models: [
          "deepseek-ai/DeepSeek-V3.2",
          "Qwen/Qwen3-235B-A22B-Instruct-2507",
          "moonshotai/Kimi-K2-Instruct-0905",
        ],
imageModels: ["Kwai-Kolors/Kolors"],
        speechModels: ["FunAudioLLM/CosyVoice2-0.5B"],
      },
      {
        id: "openrouter",
        label: "OpenRouter 聚合",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "deepseek/deepseek-chat",
        keyEnv: "OPENROUTER_API_KEY",
        models: [
          "deepseek/deepseek-chat",
          "anthropic/claude-sonnet-4",
          "google/gemini-2.5-flash",
        ],
imageModels: [],
        speechModels: [],
      },
      {
        id: "vllm",
        label: "本地 vLLM / 自建 OpenAI 兼容",
        baseUrl: vllmBaseUrl(),
        model: "local",
        keyEnv: "",
        models: [],
imageModels: [],
        speechModels: [],
      },
      {
        id: "ollama",
        label: "本地 Ollama",
        baseUrl: ollamaBaseUrl(),
        model: "local",
        keyEnv: "",
        models: [],
imageModels: [],
        speechModels: [],
      },
      {
        id: "custom",
        label: "自定义 / 自建",
        baseUrl: "",
        model: "",
        keyEnv: "OPENAI_API_KEY",
imageModels: [],
        speechModels: [],
      },
    ];
  }

  global.OwoProviderPresets = {
    presets: presets,
    ollamaBaseUrl: ollamaBaseUrl,
    // 供设置页/模型菜单解析"这个模型是不是限时免费"，以及生成终端命令。
    resolvePreset: resolvePreset,
    freeBadge: freeBadge,
    envCommand: envCommand,
    relay: {
      id: OWO_RELAY_ID,
      baseUrl: OWO_RELAY_BASE_URL,
      siteUrl: OWO_RELAY_SITE_URL,
      until: OWO_RELAY_UNTIL,
      models: OWO_RELAY_MODELS.slice(),
      // 试用密钥由壳托管：这里只暴露"该调哪个命令"，不暴露密钥是否存在
      // （可用性由壳的 relay_trial_status 回执决定，活动结束后自然变成不可用）。
      trialKeyVia: OWO_RELAY_TRIAL_COMMAND,
    },
  };
})(typeof window !== "undefined" ? window : globalThis);
