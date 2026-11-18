// ADR-003：Tauri 兼容桥纯逻辑层的契约测试。
//
// 锁定的是"web 工作台对壳的 14 个命令"的形状与语义——这些形状是 desktop/web 的
// 隐式契约（api-client.js / settings-panel.view.js / setup-guide.view.js /
// service-error.view.js / folder-picker.js / status-bar.view.js），改错一个字段名
// 前端就会静默降级，界面上却看不出来。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const commands = (await import(new URL("../src/main/shell-commands.js", import.meta.url))).default;

const {
  PROVIDER_DEFAULTS,
  SHELL_COMMANDS,
  canonicalProvider,
  effectiveBaseUrl,
  effectiveModel,
  keyEnvName,
  maskKey,
  resolveApiKey,
  providerStatusValue,
  applyModelOutputEnv,
  applyModelConfigPatch,
  validateProjectFolderName,
} = commands;

const webRoot = join(here, "..", "..", "web");
const readWeb = (relative) => readFileSync(join(webRoot, relative), "utf8");

test("命令面与 web 实际调用完全对账（不缺不漏）", () => {
  const files = readdirSync(join(webRoot, "core"))
    .map((name) => `core/${name}`)
    .concat(readdirSync(join(webRoot, "views")).map((name) => `views/${name}`))
    .concat(readdirSync(webRoot).filter((name) => name.endsWith(".js")).map((name) => name))
    .concat(readdirSync(join(webRoot, "panels")).map((name) => `panels/${name}`));

  const invoked = new Set();
  for (const file of files) {
    let text;
    try {
      text = readWeb(file);
    } catch (_) {
      continue;
    }
    for (const match of text.matchAll(/invoke\(\s*"([a-z_0-9]+)"/g)) invoked.add(match[1]);
    for (const match of text.matchAll(/invoke\(\s*'([a-z_0-9]+)'/g)) invoked.add(match[1]);
    // folder-picker.js 用常量 PICK_COMMAND
    for (const match of text.matchAll(/PICK_COMMAND\s*=\s*"([a-z_0-9]+)"/g)) invoked.add(match[1]);
  }

  for (const name of invoked) {
    assert.ok(SHELL_COMMANDS.includes(name), `web 调用了壳命令「${name}」，但桥未实现`);
  }
  // 反向：桥实现但 web 已不再调用的命令只提示不失败（diagnostics 类命令可能滞后）。
  // Electron 增加了本机新建项目文件夹能力；限时免费试用新增 apply_relay_trial /
  // relay_trial_status 两条（试用密钥只走主进程，渲染层不接触明文）。
  assert.equal(SHELL_COMMANDS.length, 19, "命令面包含兼容命令、项目目录命令与清除工作区");
  assert.ok(SHELL_COMMANDS.includes("desktop_pairing"));
  assert.ok(SHELL_COMMANDS.includes("open_project_directory"), "侧栏项目菜单需要在文件管理器中打开目录");
});

test("project folder names reject traversal, invalid characters and reserved device names", () => {
  assert.deepEqual(validateProjectFolderName("我的新项目"), { ok: true, name: "我的新项目" });
  for (const name of ["", ".", "..", "a/b", "a\\b", "bad:name", "name.", "CON", "NUL.txt"]) {
    assert.equal(validateProjectFolderName(name).ok, false, name);
  }
});

test("provider 默认值与旧 Tauri provider.rs 逐字一致", () => {
  assert.equal(PROVIDER_DEFAULTS.bigmodel.base_url, "https://open.bigmodel.cn/api/paas/v4");
  assert.equal(PROVIDER_DEFAULTS.bigmodel.model, "glm-5.3-flash");
  assert.equal(PROVIDER_DEFAULTS.ollama.base_url, "http://127.0.0.1:11434/v1");
  assert.equal(canonicalProvider({ provider: "zhipu" }), "bigmodel");
  assert.equal(canonicalProvider({ provider: "cloud" }), "bigmodel");
  assert.equal(canonicalProvider({ provider: "乱写" }), "unset");
});

test("有效端点/模型：显式配置优先，缺失回落提供商默认", () => {
  assert.equal(effectiveBaseUrl({ provider: "bigmodel", base_url: "  " }), PROVIDER_DEFAULTS.bigmodel.base_url);
  assert.equal(effectiveBaseUrl({ provider: "bigmodel", base_url: " https://x/v1 " }), "https://x/v1");
  assert.equal(effectiveModel({ provider: "deepseek" }), "deepseek-chat");
  assert.equal(effectiveModel({ provider: "deepseek", name: "deepseek-reasoner" }), "deepseek-reasoner");
});

test("凭据解析：文件 > 指定环境变量 > 默认环境变量 > 历史变量，掩码不泄漏本体", () => {
  const env = { OPENAI_API_KEY: "sk-default-1234", MY_KEY: "sk-named-5678", DASHSCOPE_API_KEY: "sk-legacy-9" };
  const envGet = (name) => env[name] || "";

  assert.deepEqual(resolveApiKey({ api_key: "sk-file" }, envGet), { key: "sk-file", source: "config_file" });
  assert.equal(resolveApiKey({ api_key_env: "MY_KEY" }, envGet).source, "config_env");
  assert.equal(resolveApiKey({ api_key_env: "OPENAI_API_KEY" }, envGet).source, "environment");
  // 未指定 api_key_env 时才回落到历史变量（与 provider.rs 同一顺序）
  assert.equal(resolveApiKey({}, envGet).source, "environment");
  assert.equal(resolveApiKey({ api_key_env: "OTHER" }, envGet).source, "environment");
  assert.deepEqual(resolveApiKey({ api_key_env: "OTHER" }, () => ""), { key: "", source: "none" });

  const masked = maskKey("sk-1234567890abcdef");
  assert.ok(!masked.includes("2345"), "掩码不得泄漏中段");
  assert.equal(maskKey("abc"), "****");
});

test("get_provider_status 形状：字段齐全、密钥只给掩码与布尔", () => {
  const status = providerStatusValue(
    { provider: "bigmodel", api_key: "sk-secret-key-9999", models: ["glm-5.3-flash"] },
    { envGet: () => "", configPath: "C:\\cfg\\config.json" },
  );
  for (const field of [
    "provider", "deferred", "baseUrl", "model", "keyConfigured", "keySource", "keyMasked",
    "keyEnv", "ready", "configPath", "models", "contextWindow", "maxOutputTokens",
    "temperature", "timeoutSecs", "keepRecent", "compaction", "modelOutputTokens",
  ]) {
    assert.ok(Object.prototype.hasOwnProperty.call(status, field), `缺字段 ${field}`);
  }
  assert.equal(status.keyConfigured, true);
  assert.equal(status.maxOutputTokens, 32000);
  assert.deepEqual(status.modelOutputTokens, {});
  assert.equal(status.keySource, "config_file");
  assert.ok(status.keyMasked.includes("…"));
  assert.ok(!JSON.stringify(status).includes("sk-secret-key-9999"), "明文密钥绝不能出现在状态里");
  assert.equal(status.ready, true);
  assert.equal(keyEnvName({}), "OPENAI_API_KEY");
});

test("本地端点（ollama）不需要凭据也算 ready；unset 必须有凭据", () => {
  const ollama = providerStatusValue({ provider: "ollama" }, { envGet: () => "" });
  assert.equal(ollama.ready, true);
  assert.equal(ollama.keyConfigured, false);
  const unset = providerStatusValue({ provider: "unset" }, { envGet: () => "" });
  assert.equal(unset.ready, false);
});

test("稍后配置模型的决定写入持久配置并在状态中保留", () => {
  const deferred = applyModelConfigPatch({ model: { provider: "unset", deferred: false } }, { mode: "unset" });
  assert.equal(deferred.ok, true);
  assert.equal(deferred.config.model.deferred, true);
  assert.equal(providerStatusValue(deferred.config.model, { envGet: () => "" }).deferred, true);
  const selected = applyModelConfigPatch(deferred.config, { mode: "bigmodel" });
  assert.equal(selected.ok, true);
  assert.equal(selected.config.model.deferred, false);
});

test("set_model_config：未传字段保持原样（不得抹掉用户已存的密钥）", () => {
  const before = { version: 1, model: { provider: "bigmodel", api_key: "sk-keep", timeout_secs: 120 } };
  const patched = applyModelConfigPatch(before, { mode: "openai" });
  assert.equal(patched.ok, true);
  assert.equal(patched.config.model.api_key, "sk-keep", "没传 api_key 时不得清除");
  assert.equal(patched.config.model.timeout_secs, 120, "没传可调参数时不得清除");
  assert.equal(patched.config.model.provider, "openai");
});

test("set_model_config：显式空串/0 = 清除，非法值也按清除处理", () => {
  const before = { version: 1, model: { provider: "bigmodel", api_key: "sk-x", timeout_secs: 120, temperature: 0.7 } };
  const cleared = applyModelConfigPatch(before, { api_key: "", timeout_secs: "0", temperature: "abc" });
  assert.equal(cleared.ok, true);
  assert.equal(cleared.config.model.api_key, null);
  assert.equal(cleared.config.model.timeout_secs, null);
  assert.equal(cleared.config.model.temperature, null);
});

test("set_model_config：未知 provider 拒绝；越界数值按旧壳语义「清除」而非拒绝", () => {
  assert.equal(applyModelConfigPatch({}, { mode: "不存在的厂商" }).ok, false);
  // 对齐 commands.rs：temperature 9 → parse 成功但 0..=2 过滤失败 → None（清除）。
  // 因此这里断言"被清成空"而不是"被拒绝"，否则就与旧壳行为漂移了。
  const patched = applyModelConfigPatch({ model: { provider: "bigmodel", temperature: 0.5 } }, { temperature: 9 });
  assert.equal(patched.ok, true);
  assert.equal(patched.config.model.temperature, null);
});

test("model output env clears inherited limits, sets 32k default and forwards per-model overrides", () => {
  const env = {
    OWO_MODEL_MAX_OUTPUT_TOKENS: "4096",
    OWO_MODEL_OUTPUT_TOKENS_BY_MODEL: "stale",
  };
  applyModelOutputEnv(env, { model_output_tokens: { "glm-5.3-flash": 24000, "qwen3.8-max": 64000, bad: 1000001 } });
  assert.equal(env.OWO_MODEL_MAX_OUTPUT_TOKENS, "32000");
  assert.deepEqual(JSON.parse(env.OWO_MODEL_OUTPUT_TOKENS_BY_MODEL), { "glm-5.3-flash": 24000, "qwen3.8-max": 64000 });
  applyModelOutputEnv(env, { max_output_tokens: 16000, model_output_tokens: {} });
  assert.equal(env.OWO_MODEL_MAX_OUTPUT_TOKENS, "16000");
  assert.deepEqual(JSON.parse(env.OWO_MODEL_OUTPUT_TOKENS_BY_MODEL), {});
});

test("get_provider_status：每模型输出预算只回传合法整数，不暴露其他配置", () => {
  const status = providerStatusValue({
    provider: "bigmodel",
    model_output_tokens: { "glm-5.3-flash": 24000, "qwen3.8-max": 64000, bad: 1000001, "": 1200, other: "8192" },
  });
  assert.deepEqual(status.modelOutputTokens, { "glm-5.3-flash": 24000, "qwen3.8-max": 64000, other: 8192 });
});

test("set_model_config：按模型输出上限可保存、校验并清除", () => {
  const patched = applyModelConfigPatch(
    { model: { provider: "bigmodel", max_output_tokens: 32000, model_output_tokens: { "glm-5.3-flash": 4096 } } },
    { model_output_tokens: { " glm-5.3-flash ": 24000, "qwen3.8-max": 64000 } },
  );
  assert.equal(patched.ok, true);
  assert.deepEqual(patched.config.model.model_output_tokens, { "glm-5.3-flash": 24000, "qwen3.8-max": 64000 });
  assert.equal(applyModelConfigPatch({ model: { provider: "bigmodel" } }, { model_output_tokens: { x: 64000 } }).ok, true);
  assert.equal(applyModelConfigPatch({ model: {} }, { model_output_tokens: { x: 1000001 } }).ok, false);
  const cleared = applyModelConfigPatch({ model: {} }, { model_output_tokens: {} });
  assert.deepEqual(cleared.config.model.model_output_tokens, {});
});

test("set_model_config：可调参数与模型清单按契约落位", () => {
  const patched = applyModelConfigPatch(
    { model: { provider: "custom" } },
    {
      context_window: "128000",
      max_output_tokens: "8192",
      model_output_tokens: { "model-a": "16000" },
      temperature: "0.3",
      timeout_secs: "90",
      keep_recent: "40",
      compaction: "true",
      models: ["a", "a", " b ", ""],
    },
  );
  assert.equal(patched.ok, true);
  assert.equal(patched.config.model.context_window, 128000);
  assert.equal(patched.config.model.max_output_tokens, 8192);
  assert.deepEqual(patched.config.model.model_output_tokens, { "model-a": 16000 });
  assert.equal(patched.config.model.temperature, 0.3);
  assert.equal(patched.config.model.timeout_secs, 90);
  assert.equal(patched.config.model.keep_recent, 40);
  assert.equal(patched.config.model.compaction, true);
  assert.deepEqual(patched.config.model.models, ["a", "b"], "清单要去重、去空、去空白");
});
