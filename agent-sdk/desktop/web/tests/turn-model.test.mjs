import assert from "node:assert/strict";
import { test } from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const turnModel = require("../core/turn-model.js");

test("withModelConnection 注入 model_connection 且不改动原对象", () => {
  const body = { prompt: "hi", session_id: "s1" };
  const out = turnModel.withModelConnection(body, {
    model: "gpt-4.1",
    base_url: "https://api.openai.com/v1",
    api_format: "openai",
  });
  assert.equal(out.prompt, "hi");
  assert.deepEqual(out.model_connection, {
    model: "gpt-4.1",
    base_url: "https://api.openai.com/v1",
    api_format: "openai",
  });
  assert.equal(body.model_connection, undefined, "不应污染入参");
});

test("没有连接或连接缺模型时不下发字段", () => {
  assert.deepEqual(turnModel.withModelConnection({ prompt: "x" }, null), { prompt: "x" });
  assert.deepEqual(turnModel.withModelConnection({ prompt: "x" }, { base_url: "u" }), { prompt: "x" });
});

test("isCustomConnection 忽略地址尾斜杠与大小写", () => {
  const defaults = { model: "glm-5.3-flash", base_url: "https://Open.BigModel.cn/api/paas/v4/" };
  assert.equal(
    turnModel.isCustomConnection({ model: "glm-5.3-flash", base_url: "https://open.bigmodel.cn/api/paas/v4" }, defaults),
    false
  );
  assert.equal(turnModel.isCustomConnection({ model: "gpt-4.1", base_url: "https://api.openai.com/v1" }, defaults), true);
  assert.equal(turnModel.isCustomConnection(null, defaults), false);
});

test("defaultConnectionFromSettings 兼容 provider 段与扁平字段", () => {
  assert.deepEqual(
    turnModel.defaultConnectionFromSettings({
      provider: { model: "glm-4.6", base_url: "https://open.bigmodel.cn/api/paas/v4" },
    }),
    { model: "glm-4.6", base_url: "https://open.bigmodel.cn/api/paas/v4" }
  );
  assert.deepEqual(turnModel.defaultConnectionFromSettings({ model: "m" }), { model: "m", base_url: "" });
  assert.equal(turnModel.defaultConnectionFromSettings(null), null);
});
