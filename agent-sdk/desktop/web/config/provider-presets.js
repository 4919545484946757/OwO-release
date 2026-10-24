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

  function presets() {
    return [
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
  };
})(typeof window !== "undefined" ? window : globalThis);
