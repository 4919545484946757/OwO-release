import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import fs from "node:fs";
const scope = {};
vm.runInNewContext(fs.readFileSync(new URL("../core/settings-search.js", import.meta.url), "utf8"), scope);
const { search, collect } = scope.OwoSettingsSearch;

test("设置检索支持多词交集、大小写与全角输入", () => {
  const entries = [
    { category: "配置", label: "API 密钥", description: "由核心加密保存" },
    { category: "常规", label: "云端模型", description: "数据出境" },
    { category: "配置", label: "API 端点", description: "OpenAI 兼容地址" }
  ];
  assert.equal(search(entries, "配置 ＡＰＩ").length, 2);
  assert.equal(search(entries, "api 密钥")[0], entries[0]);
  assert.equal(search(entries, "云端 加密").length, 0);
  assert.equal(search(entries, "  ").length, 0);
});

test("设置搜索索引不读取表单值、密钥或运行时状态", () => {
  const row = { querySelector(selector) {
    if (selector === ".settings-row-label strong, .settings-block-head strong, td") return { textContent: "API 密钥" };
    if (selector === ".settings-row-label span, td + td") return { textContent: "由核心加密保存" };
    throw new Error("不允许读取字段: " + selector);
  }, value: "secret-value-must-not-be-indexed" };
  const tab = {
    dataset: { settingsTab: "models" },
    querySelector(selector) {
      return { textContent: selector.endsWith("strong") ? "配置" : "服务商与端点" };
    },
    querySelectorAll() { return [row]; }
  };
  const entries = collect({ querySelectorAll() { return [tab]; } });
  assert.equal(entries.length, 2);
  assert.equal(search(entries, "secret-value").length, 0);
  assert.equal(search(entries, "加密")[0].target, row);
  assert.equal(search(entries, "服务商")[0].target, tab);
});

test("无结果和特殊字符按普通文本处理", () => {
  const entries = [{ category: "个性化", label: "<script>", description: "原样文字" }];
  assert.equal(search(entries, "<script>")[0], entries[0]);
  assert.equal(search(entries, "不存在的设置").length, 0);
});
