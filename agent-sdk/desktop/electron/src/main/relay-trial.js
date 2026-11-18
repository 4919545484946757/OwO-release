"use strict";

// 限时免费活动的共享试用密钥。
//
// 为什么放在主进程而不是 desktop/web：
//   `tests/preset-dropdown.test.mjs` 有一条硬约束——前端预设注册表里不得出现任何
//   形如 sk-… 的密钥。这条守卫的本意是「渲染层永远不接触凭据」：渲染层会进 devtools、
//   会被截图、会被 console 打印，密钥一旦进去就等同于公开。
//   所以试用密钥留在这里，渲染层只能请求「把中转站端点 + 模型 + 试用密钥写进本机
//   配置」这一个动作，命令回执里不含密钥本体。
//
// 这把 key 仍是**可被提取的共享凭据**（二进制里必然有），因此：
//   * 只能在中转站侧单独创建、限定分组、设额度上限，且随时可吊销；
//   * 不得复用到别的服务商；
//   * 活动结束：把 TRIAL_KEY 置空 + 吊销，preset 里的 promo 一并撤掉即可。
const TRIAL_KEY = "sk-sNru0Is9otkR0ByQekBbadkM5hacv5yzxQu29YlZ2ifT2qmC";

const TRIAL = Object.freeze({
  id: "owo_relay",
  label: "OwO 中转站（限时免费）",
  baseUrl: "https://owo.dafeiyu.homes/v1",
  model: "deepseek-v4.1-flash",
  siteUrl: "https://owo.dafeiyu.homes/keys",
  until: "2026-12-31",
  // 与 desktop/web/config/provider-presets.js 的 freeModels 同一份清单（测试对账）。
  models: Object.freeze([
    "deepseek-v4.1-flash",
    "hy4",
    "claude-opus-5.5",
    "claude-haiku-5.5",
    "gpt-4o-mini",
  ]),
});

function hasTrialKey() {
  return /^sk-[A-Za-z0-9]{8,}$/.test(TRIAL_KEY);
}

/// 回执里只给端点/模型/是否可用，绝不带 key。
function relayTrialStatus() {
  return {
    ok: true,
    id: TRIAL.id,
    label: TRIAL.label,
    baseUrl: TRIAL.baseUrl,
    model: TRIAL.model,
    siteUrl: TRIAL.siteUrl,
    until: TRIAL.until,
    models: [...TRIAL.models],
    hasTrialKey: hasTrialKey(),
  };
}

/// 生成 set_model_config 的参数（供主进程 read-modify-write）。
/// 模型必须落在活动清单内，避免渲染层拿试用 key 去打清单外的模型。
function relayTrialPatch(requestedModel) {
  if (!hasTrialKey()) {
    return { ok: false, error: "限时免费试用密钥未配置（活动已结束或未内置）" };
  }
  const wanted = String(requestedModel == null ? "" : requestedModel).trim();
  if (wanted && !TRIAL.models.includes(wanted)) {
    return { ok: false, error: `该模型不在限时免费清单内：${wanted}` };
  }
  return {
    ok: true,
    mode: "cloud",
    base_url: TRIAL.baseUrl,
    model: wanted || TRIAL.model,
    api_key: TRIAL_KEY,
    api_key_env: "OPENAI_API_KEY",
  };
}

module.exports = { TRIAL, hasTrialKey, relayTrialStatus, relayTrialPatch };
