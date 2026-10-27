import assert from "node:assert/strict";
import { test } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const picker = require("../core/model-picker.js");

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
    id: "openai",
    base_url: "https://api.openai.com/v1",
    models: ["gpt-4.1", "gpt-4o"],
    chat_modalities: { text: true, image_input: true },
    image_models: ["gpt-image-1"],
    speech_models: ["tts-1"],
  },
  { id: "custom", base_url: "", models: [], chat_modalities: { text: true } },
];

function memoryStorage() {
  const map = new Map();
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
  };
}

test("选择持久化按会话隔离，坏数据返回 null", () => {
  const storage = memoryStorage();
  assert.equal(picker.readSelection(storage, "s1"), null);
  picker.writeSelection(storage, "s1", { providerId: "bigmodel", model: "glm-4.6" });
  assert.deepEqual(picker.readSelection(storage, "s1"), {
    providerId: "bigmodel",
    model: "glm-4.6",
    baseUrl: "",
  });
  assert.equal(picker.readSelection(storage, "s2"), null);
  storage.setItem(picker.storageKey("bad"), "{not json");
  assert.equal(picker.readSelection(storage, "bad"), null);
});

test("换提供商时旧模型回落到新提供商默认模型", () => {
  const first = picker.applySelection(PROVIDERS, null, { providerId: "bigmodel" });
  assert.deepEqual(first, {
    providerId: "bigmodel",
    model: "glm-5.3-flash",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
  });
  const switched = picker.applySelection(PROVIDERS, first, { providerId: "openai" });
  assert.equal(switched.model, "gpt-4.1", "不应把 glm 模型发给 openai");
  assert.equal(switched.baseUrl, "https://api.openai.com/v1");
});

test("同提供商内换模型保留用户选择，自定义地址优先", () => {
  const current = { providerId: "bigmodel", model: "glm-5.3-flash", baseUrl: "" };
  const same = picker.applySelection(PROVIDERS, current, { providerId: "bigmodel", model: "glm-4.6" });
  assert.equal(same.model, "glm-4.6");
  const custom = picker.applySelection(PROVIDERS, current, {
    providerId: "custom",
    model: "local-7b",
    baseUrl: "http://127.0.0.1:8000/v1",
  });
  assert.deepEqual(custom, { providerId: "custom", model: "local-7b", baseUrl: "http://127.0.0.1:8000/v1" });
});

test("胶囊文案与多模态能力", () => {
  assert.equal(picker.labelFor(null), "选择模型");
  assert.equal(picker.labelFor({ model: "gpt-4.1" }), "gpt-4.1");
  const long = "org/some-vendor/extremely-long-model-name-2026-09-30";
  assert.ok(picker.labelFor({ model: long }, 20).length <= 20);
  assert.ok(picker.labelFor({ model: long }, 20).endsWith("2026-09-30"));
  assert.equal(picker.canModality(PROVIDERS, { providerId: "bigmodel" }, "image_output"), true);
  assert.equal(picker.canModality(PROVIDERS, { providerId: "custom" }, "image_input"), false);
});
