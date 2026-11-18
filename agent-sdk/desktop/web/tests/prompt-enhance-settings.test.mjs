import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

// 最小 DOM / localStorage 桩：模块初始化只需要它们存在。
global.document = {
  readyState: "complete",
  addEventListener() {},
  removeEventListener() {},
  getElementById: () => null,
  querySelector: () => null,
  createElement: () => ({ setAttribute() {}, appendChild() {}, addEventListener() {}, classList: { add() {} } }),
};
const store = new Map();
global.localStorage = {
  getItem: (key) => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: (key) => store.delete(key),
};

const require = createRequire(import.meta.url);
require("../core/prompt-enhance-settings.js");
const api = globalThis.OwoPromptEnhanceSettings;
assert.ok(api, "OwoPromptEnhanceSettings 应在加载后暴露");
const { normalize, resolveWith } = api._test;

const CATALOG = {
  entries: [
    { id: "glm-5.2", label: "glm-5.2", custom: false },
    { id: "my-local-llm", label: "本地模型", custom: true },
  ],
  customModels: [
    { id: "my-local-llm", label: "本地模型", baseUrl: "http://127.0.0.1:11434/v1", apiFormat: "openai" },
  ],
};

test("normalize 只接受三种模式，follow 清空模型", () => {
  assert.deepEqual(normalize({ mode: "cloud", model: "  glm-5.2 " }), { mode: "cloud", model: "glm-5.2" });
  assert.deepEqual(normalize({ mode: "follow", model: "glm-5.2" }), { mode: "follow", model: "" });
  assert.deepEqual(normalize({ mode: "nope", model: "x" }), { mode: "follow", model: "" });
  assert.deepEqual(normalize(null), { mode: "follow", model: "" });
});

test("resolveWith：跟随当前模型", () => {
  assert.deepEqual(resolveWith({ mode: "follow" }, CATALOG), { mode: "follow", model: "", useConnection: false });
  // 选了模式但没选模型，同样按跟随处理。
  assert.deepEqual(resolveWith({ mode: "cloud", model: "" }, CATALOG), { mode: "follow", model: "", useConnection: false });
});

test("resolveWith：云端模型只覆盖 id，不带自定义连接", () => {
  assert.deepEqual(resolveWith({ mode: "cloud", model: "glm-5.2" }, CATALOG), {
    mode: "cloud",
    model: "glm-5.2",
    useConnection: false,
  });
});

test("resolveWith：本地模型带连接", () => {
  assert.deepEqual(resolveWith({ mode: "local", model: "my-local-llm" }, CATALOG), {
    mode: "local",
    model: "my-local-llm",
    useConnection: true,
  });
});

test("resolveWith：模型不可用或已删除时回退到跟随，不发送不存在的模型", () => {
  const fallback = { mode: "follow", model: "", useConnection: false };
  assert.deepEqual(resolveWith({ mode: "cloud", model: "gone-model" }, CATALOG), fallback);
  assert.deepEqual(resolveWith({ mode: "local", model: "gone-model" }, CATALOG), fallback);
  // 云端列表里出现、但模式是 local：本地模式只认自定义连接。
  assert.deepEqual(resolveWith({ mode: "local", model: "glm-5.2" }, CATALOG), fallback);
  // 自定义连接被禁用（disabled）也算不可用。
  assert.deepEqual(
    resolveWith({ mode: "local", model: "my-local-llm" }, { entries: [], customModels: [{ id: "my-local-llm", disabled: true }] }),
    fallback
  );
});

test("resolveWith：目录为空时一律回退", () => {
  assert.deepEqual(resolveWith({ mode: "cloud", model: "glm-5.2" }, null), {
    mode: "follow",
    model: "",
    useConnection: false,
  });
});

test("read / write 往返，损坏数据按跟随处理", () => {
  assert.deepEqual(api.read(), { mode: "follow", model: "" });
  api.write({ mode: "local", model: "my-local-llm" });
  assert.deepEqual(api.read(), { mode: "local", model: "my-local-llm" });
  store.set(api.KEY, "{not json");
  assert.deepEqual(api.read(), { mode: "follow", model: "" });
});
