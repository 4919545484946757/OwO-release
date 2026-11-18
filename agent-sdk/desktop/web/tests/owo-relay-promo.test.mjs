// OwO 中转站「限时免费」接入的契约测试。
// 覆盖三件事：① 预设描述本身（端点 / 模型 / 活动字段）；② 界面要用的
// freeBadge / envCommand helper（后端渲染文案靠它，别让它们悄悄返回空）；
// ③ app.js 真的把这些 helper 接进了模型菜单与设置页——只加数据不接线是常见漏项。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, "..");
const source = readFileSync(join(web, "config", "provider-presets.js"), "utf8");
const appSource = readFileSync(join(web, "app.js"), "utf8");
const rustSource = readFileSync(
  join(web, "..", "..", "crates", "owo-agent-core", "src", "gateway", "presets.rs"),
  "utf8"
);

function relayRustModels() {
  const start = rustSource.indexOf('id: "owo_relay"');
  assert.ok(start > 0, "Rust 预设缺少 owo_relay");
  const next = rustSource.indexOf('id: "', start + 'id: "owo_relay"'.length);
  const block = rustSource.slice(start, next > 0 ? next : rustSource.length);
  const list = block.match(/models:\s*&\[([\s\S]*?)\]/);
  assert.ok(list, "Rust 的 owo_relay 缺少 models 列表");
  return [...list[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

function load() {
  const sandbox = { window: {} };
  vm.runInNewContext(source, sandbox);
  return sandbox.window.OwoProviderPresets;
}

const registry = load();
const relay = registry.presets().find((preset) => preset.id === "owo_relay");

test("中转站预设：端点为 /v1，模型与中转站 default 分组一致", () => {
  assert.ok(relay, "缺少 owo_relay 预设");
  assert.equal(relay.baseUrl, "https://owo.dafeiyu.homes/v1");
  assert.deepEqual(
    [...relay.models],
    [
      "deepseek-v4.1-flash",
      "hy4",
      "claude-opus-5.5",
      "claude-haiku-5.5",
      "gpt-4o-mini",
    ],
    "模型清单要与 /api/pricing 的 default 分组一致"
  );
  assert.deepEqual(
    relayRustModels(),
    [...relay.models],
    "Rust 预设必须使用同一组新模型名"
  );
  assert.ok(relay.models.includes(relay.model), "默认模型必须在清单里，否则选完不可用");
  assert.equal(relay.promo.badge, "限时免费");
  assert.match(relay.promo.siteUrl, /^https:\/\/owo\.dafeiyu\.homes\//);
});

test("freeBadge：只给活动模型挂徽标，其它服务商与未知模型不挂", () => {
  for (const model of relay.models) {
    assert.equal(registry.freeBadge("owo_relay", model), "限时免费", model + " 应标限时免费");
    assert.equal(registry.freeBadge(relay.baseUrl, model), "限时免费", "按 base_url 也要能判定");
  }
  assert.equal(registry.freeBadge("owo_relay", "workbuddy/hy4free"), "", "旧模型名不得继续标活动");
  assert.equal(registry.freeBadge("deepseek", "deepseek-chat"), "");
  assert.equal(registry.freeBadge("", ""), "");
  // 不传 modelId = 这个端点有没有活动
  assert.equal(registry.freeBadge(relay.baseUrl), "限时免费");
  assert.equal(registry.freeBadge(relay.baseUrl + "/chat/completions", relay.model), "限时免费");
});

test("resolvePreset：id 与带路径的 base_url 都能命中", () => {
  // vm 沙箱里创建的对象与外面不同 realm，按 id 断言而不是引用相等。
  assert.equal(registry.resolvePreset("owo_relay")?.id, "owo_relay");
  assert.equal(registry.resolvePreset("https://owo.dafeiyu.homes/v1")?.id, "owo_relay");
  assert.equal(registry.resolvePreset("OWO.DAFEIYU.HOMES/v1")?.id, "owo_relay");
  assert.equal(registry.resolvePreset("https://api.deepseek.com/v1").id, "deepseek");
  assert.equal(registry.resolvePreset(""), null);
});

test("envCommand：生成可直接粘贴的 PowerShell 命令", () => {
  const command = registry.envCommand(relay);
  assert.match(command, /OPENAI_BASE_URL = "https:\/\/owo\.dafeiyu\.homes\/v1"/);
  assert.match(command, new RegExp('OPENAI_MODEL = "' + relay.model.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + '"'));
  assert.match(command, /OPENAI_API_KEY = "<你的 API Key>"/);
  // 自定义预设没有 base_url，不该编出一条假命令。
  assert.equal(registry.envCommand({ id: "custom", baseUrl: "" }), "");
});

test("app.js 已把徽标接进模型菜单与设置页（不能只加数据不接线）", () => {
  assert.match(appSource, /freeBadgeMark\(item\)/, "模型菜单行需要渲染徽标");
  assert.match(appSource, /registry\.freeBadge\(preset, id\)/, "设置页模型下拉需要带限时免费后缀");
  assert.match(appSource, /renderPresetPromo\(preset\)/, "预设信息块需要渲染活动说明");
  assert.match(appSource, /usePresetTrialKey\(/, "活动说明块要能一键启用试用");
});

test("预设表里没有密钥：试用密钥只存在桌面壳主进程", () => {
  // 与 tests/preset-dropdown.test.mjs 的硬约束同源，这里再钉一次：渲染层不能有 sk-。
  assert.equal(/sk-[A-Za-z0-9]{8,}/.test(source), false, "预设表不得出现密钥");
  assert.equal(relay.trialKey, undefined, "预设不该再有明文字段 trialKey");
  assert.equal(relay.trialKeyVia, "apply_relay_trial", "只暴露壳命令名");
  assert.match(appSource, /usePresetTrialKey/, "设置页要能一键调用壳命令启用试用");
  assert.match(appSource, /relay_trial_status/, "活动结束后要能自动禁用试用按钮");
});

test("推理能力按实测声明，压过关键字猜测", () => {
  // hy4 名字不带关键字但实测回 reasoning_content；deepseek-v4.1-flash 命中正则却实测不回。
  assert.deepEqual([...relay.reasoningModels], ["hy4"]);
  assert.ok(relay.nonReasoningModels.includes("deepseek-v4.1-flash"));
  const overlap = [...relay.reasoningModels].filter((id) => relay.nonReasoningModels.includes(id));
  assert.deepEqual(overlap, [], "同一模型不能既声明会思考又声明不会");
  assert.match(appSource, /function declaredReasoning/, "app.js 必须按声明覆盖正则猜测");
});
