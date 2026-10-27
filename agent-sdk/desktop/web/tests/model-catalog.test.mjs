import assert from "node:assert/strict";
import { test } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const catalog = require("../core/model-catalog.js");

const PROVIDERS = [
  {
    id: "bigmodel",
    base_url: "https://open.bigmodel.cn/api/paas/v4",
    models: ["glm-5.3-flash", "glm-4.6"],
    chat_modalities: { text: true, image_input: true },
    image_models: ["cogview-4"],
    speech_models: ["cogtts"],
  },
  {
    id: "deepseek",
    base_url: "https://api.deepseek.com/v1",
    models: ["deepseek-chat"],
    chat_modalities: { text: true, image_input: false },
    image_models: [],
    speech_models: [],
  },
  { id: "custom", base_url: "", models: [], chat_modalities: { text: true } },
];

test("detect 按域名识别提供商，忽略大小写与尾斜杠", () => {
  assert.equal(catalog.detect(PROVIDERS, "HTTPS://API.DeepSeek.com/v1/").id, "deepseek");
  assert.equal(catalog.detect(PROVIDERS, "https://open.bigmodel.cn/api/paas/v4").id, "bigmodel");
  assert.equal(catalog.detect(PROVIDERS, "http://localhost:8000/v1"), null);
  assert.equal(catalog.detect(PROVIDERS, ""), null);
});

test("modelsFor 支持自定义模型名并去重", () => {
  assert.deepEqual(catalog.modelsFor(PROVIDERS, "deepseek"), ["deepseek-chat"]);
  assert.deepEqual(catalog.modelsFor(PROVIDERS, "deepseek", "my-tuned"), ["my-tuned", "deepseek-chat"]);
  assert.deepEqual(catalog.modelsFor(PROVIDERS, "deepseek", "deepseek-chat"), ["deepseek-chat"]);
});

test("supports 区分视觉输入与图像/语音输出", () => {
  assert.equal(catalog.supports(PROVIDERS, "bigmodel", "image_input"), true);
  assert.equal(catalog.supports(PROVIDERS, "bigmodel", "image_output"), true);
  assert.equal(catalog.supports(PROVIDERS, "bigmodel", "audio_output"), true);
  assert.equal(catalog.supports(PROVIDERS, "deepseek", "image_input"), false);
  assert.equal(catalog.supports(PROVIDERS, "deepseek", "image_output"), false);
  assert.equal(catalog.supports(PROVIDERS, "nope", "text"), false);
});

test("buildProviderPayload 自动补全地址与默认模型", () => {
  assert.deepEqual(catalog.buildProviderPayload(PROVIDERS, { providerId: "deepseek", apiKey: "k" }), {
    provider: "deepseek",
    base_url: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    api_key: "k",
  });
  assert.deepEqual(
    catalog.buildProviderPayload(PROVIDERS, {
      providerId: "custom",
      baseUrl: "http://127.0.0.1:8000/v1",
      model: "local-7b",
      apiKey: "",
    }),
    { provider: "custom", base_url: "http://127.0.0.1:8000/v1", model: "local-7b", api_key: "" },
  );
});

test("load 兼容 {providers:[]} 与裸数组，失败返回空表", async () => {
  const ok = await catalog.load(async () => ({
    ok: true,
    json: async () => ({ providers: PROVIDERS }),
  }));
  assert.equal(ok.length, 3);
  const bare = await catalog.load(async () => ({ ok: true, json: async () => PROVIDERS }));
  assert.equal(bare.length, 3);
  const failed = await catalog.load(async () => ({ ok: false, json: async () => ({}) }));
  assert.deepEqual(failed, []);
});

test("buildModelConnection 产出服务端 CustomModelConnection 形状", () => {
  const withPreset = catalog.buildModelConnection(PROVIDERS, { providerId: "bigmodel", apiKey: "sk-1" });
  assert.deepEqual(withPreset, {
    model: "glm-5.3-flash",
    base_url: "https://open.bigmodel.cn/api/paas/v4",
    api_format: "openai",
    api_key: "sk-1",
  });
  const keyless = catalog.buildModelConnection(PROVIDERS, {
    providerId: "custom",
    baseUrl: "http://127.0.0.1:8000/v1",
    model: "local-7b",
  });
  assert.equal(keyless.api_key, undefined, "空 Key 不应下发字段");
  assert.equal(keyless.base_url, "http://127.0.0.1:8000/v1");
  assert.equal(keyless.model, "local-7b");
});

test("presetToForm 生成表单初值（含空 Key 允许与 apiFormat）", () => {
  const local = PROVIDERS.find((item) => item.id === "custom");
  assert.deepEqual(catalog.presetToForm(local), {
    providerId: "custom",
    baseUrl: "",
    model: "",
    apiFormat: "openai",
    allowsEmptyKey: false,
    name: "custom",
  });
  const glm = PROVIDERS[0];
  assert.equal(catalog.presetToForm(glm).model, "glm-5.3-flash");
  assert.equal(catalog.presetToForm(glm).baseUrl, "https://open.bigmodel.cn/api/paas/v4");
  assert.equal(catalog.presetToForm(null), null);
});
