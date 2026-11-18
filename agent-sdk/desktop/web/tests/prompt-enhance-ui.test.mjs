import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

// 最小 DOM 桩：模块初始化只需要 document 存在（addEventListener / readyState）。
global.document = {
  readyState: "complete",
  addEventListener() {},
  removeEventListener() {},
  getElementById: () => null,
  querySelector: () => null,
  createElement: () => ({ setAttribute() {}, appendChild() {}, addEventListener() {}, classList: { add() {} } }),
};

const require = createRequire(import.meta.url);
require("../core/prompt-polish-ui.js");
const api = globalThis.OwoPromptEnhance;
assert.ok(api, "OwoPromptEnhance 应在加载后暴露");
const { buildRequestBody, legacyResultShape, errorMessageOf, currentModelId, enhanceTarget } = api._test;

test("buildRequestBody 发送真实模型 id 而非显示名，并携带已知上下文", () => {
  global.state = { sessionWorkspace: "D:/demo/project-x" };
  global.getComposerModel = () => "deepseek-chat";
  global.currentSessionMode = () => "agent";
  delete global.ModelRouting;
  delete global.loadCustomModels;
  const body = buildRequestBody("帮我优化这个页面");
  assert.equal(body.text, "帮我优化这个页面");
  assert.equal(body.model, "deepseek-chat");
  assert.equal(body.context.workspace, "D:/demo/project-x");
  assert.equal(body.context.project, "project-x");
  assert.equal(body.context.session_mode, "agent");
  assert.equal(body.model_connection, undefined);
});

test("buildRequestBody 无已知上下文时不编造 context 字段", () => {
  global.state = {};
  global.getComposerModel = () => "";
  global.currentSessionMode = () => "";
  const body = buildRequestBody("hi");
  assert.equal(body.text, "hi");
  assert.equal(body.context, undefined);
  assert.equal(body.model, undefined);
});

test("buildRequestBody 为自定义模型附上与回合一致的连接对象", () => {
  global.state = { sessionWorkspace: "D:/w" };
  global.getComposerModel = () => "my-local-llm";
  global.currentSessionMode = () => "chat";
  global.ModelRouting = {
    buildCustomModelConnection(id, models, keys) {
      assert.equal(keys && typeof keys, "object");
      const found = models.some((item) => item.id === id);
      if (!found) return null;
      return { model: id, base_url: "http://127.0.0.1:11434/v1", api_format: "openai", use_full_url: false };
    },
  };
  global.loadCustomModels = () => [{ id: "my-local-llm", baseUrl: "http://127.0.0.1:11434/v1", apiFormat: "openai" }];
  global.customModelKeys = () => ({});
  const body = buildRequestBody("本地模型试试");
  assert.equal(body.model_connection.model, "my-local-llm");
  assert.equal(body.model_connection.base_url, "http://127.0.0.1:11434/v1");
  // 清理桩，避免影响其他用例。
  delete global.ModelRouting;
  delete global.loadCustomModels;
  delete global.customModelKeys;
});

test("buildRequestBody 澄清答案原样透传", () => {
  global.state = {};
  global.getComposerModel = () => "";
  global.currentSessionMode = () => "";
  const body = buildRequestBody("帮我做一个网站", [{ question: "给谁看？", answer: "团队内部" }]);
  assert.deepEqual(body.clarifications, [{ question: "给谁看？", answer: "团队内部" }]);
});

test("legacyResultShape 把壳层 IPC 结果映射为 ready 结构", () => {
  const mapped = legacyResultShape({ ok: true, text: "  改写后的提示词  ", model: "deepseek-chat" });
  assert.equal(mapped.status, "ready");
  assert.equal(mapped.enhanced_prompt, "改写后的提示词");
  assert.deepEqual(mapped.clarifying_questions, []);
  // 失败 / 空文本不能伪装成成功结果。
  assert.equal(legacyResultShape({ ok: false, error: "prompt/timeout" }), null);
  assert.equal(legacyResultShape({ ok: true, text: "   " }), null);
  assert.equal(legacyResultShape(null), null);
});

test("errorMessageOf 去掉状态码前缀并限长", () => {
  assert.equal(errorMessageOf(new Error("502: prompt_enhance/provider_failed: boom")), "prompt_enhance/provider_failed: boom");
  assert.equal(errorMessageOf("prompt/timeout"), "prompt/timeout");
  assert.equal(errorMessageOf(null), "未知错误");
  const long = "x".repeat(300);
  assert.ok(errorMessageOf(new Error(long)).length <= 160);
});

test("设置页指定云端模型时覆盖 model，且不携带自定义连接", () => {
  global.state = {};
  global.getComposerModel = () => "deepseek-chat";
  global.currentSessionMode = () => "";
  global.ModelRouting = {
    buildCustomModelConnection() {
      throw new Error("cloud 模式不应构建自定义连接");
    },
  };
  global.loadCustomModels = () => [];
  global.OwoPromptEnhanceSettings = { resolve: () => ({ mode: "cloud", model: "glm-5.2", useConnection: false }) };
  const body = buildRequestBody("帮我改个函数");
  assert.equal(body.model, "glm-5.2");
  assert.equal(body.model_connection, undefined);
  delete global.OwoPromptEnhanceSettings;
  delete global.ModelRouting;
  delete global.loadCustomModels;
});

test("设置页指定本地模型时使用该模型并带上连接", () => {
  global.state = {};
  global.getComposerModel = () => "deepseek-chat";
  global.currentSessionMode = () => "";
  global.ModelRouting = {
    buildCustomModelConnection(id) {
      return { model: id, base_url: "http://127.0.0.1:11434/v1", api_format: "openai", use_full_url: false };
    },
  };
  global.loadCustomModels = () => [{ id: "my-local-llm" }];
  global.customModelKeys = () => ({});
  global.OwoPromptEnhanceSettings = { resolve: () => ({ mode: "local", model: "my-local-llm", useConnection: true }) };
  const body = buildRequestBody("本地试试");
  assert.equal(body.model, "my-local-llm");
  assert.equal(body.model_connection.model, "my-local-llm");
  delete global.OwoPromptEnhanceSettings;
  delete global.ModelRouting;
  delete global.loadCustomModels;
  delete global.customModelKeys;
});

test("设置解析回退到 follow 时仍使用当前对话模型", () => {
  global.state = {};
  global.getComposerModel = () => "deepseek-chat";
  global.currentSessionMode = () => "";
  global.OwoPromptEnhanceSettings = { resolve: () => ({ mode: "follow", model: "", useConnection: false }) };
  const body = buildRequestBody("hi");
  assert.equal(body.model, "deepseek-chat");
  assert.equal(enhanceTarget().mode, "follow");
  delete global.OwoPromptEnhanceSettings;
});

test("currentModelId 在宿主函数缺失时安全返回空串", () => {
  delete global.getComposerModel;
  assert.equal(currentModelId(), "");
  global.getComposerModel = () => "glm-5.3-flash";
  assert.equal(currentModelId(), "glm-5.3-flash");
  delete global.getComposerModel;
});
