// 限时免费试用密钥只存在主进程：这里钉住三件事——
//   ① 能生成写配置的参数；② 不越界（不许拿试用 key 去打清单外的模型）；
//   ③ 回执（relay_trial_status / 状态）里绝不出现密钥本体。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import relayTrial from "../src/main/relay-trial.js";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "..", "src", "main", "relay-trial.js"), "utf8");

test("试用密钥形如 sk-… 且活动清单与前端预设一致", () => {
  assert.match(source, /const TRIAL_KEY = "sk-[A-Za-z0-9]{8,}"/, "主进程应内置试用密钥");
  assert.equal(relayTrial.hasTrialKey(), true);
  assert.deepEqual([...relayTrial.TRIAL.models], [
    "deepseek-v4.1-flash",
    "hy4",
    "claude-opus-5.5",
    "claude-haiku-5.5",
    "gpt-4o-mini",
  ]);
  assert.equal(relayTrial.TRIAL.baseUrl, "https://owo.dafeiyu.homes/v1");
});

test("relayTrialStatus 回执里没有密钥本体", () => {
  const status = relayTrial.relayTrialStatus();
  const text = JSON.stringify(status);
  assert.equal(/sk-[A-Za-z0-9]{8,}/.test(text), false, "状态回执泄露了密钥");
  assert.equal(status.baseUrl, "https://owo.dafeiyu.homes/v1");
  assert.equal(status.hasTrialKey, true);
  assert.equal(status.models.length, 5);
});

test("relayTrialPatch 只允许活动清单内的模型，默认用推荐模型", () => {
  const fallback = relayTrial.relayTrialPatch();
  assert.equal(fallback.ok, true);
  assert.equal(fallback.mode, "cloud");
  assert.equal(fallback.model, "deepseek-v4.1-flash");
  assert.match(fallback.api_key, /^sk-/);
  assert.equal(fallback.api_key_env, "OPENAI_API_KEY");

  const picked = relayTrial.relayTrialPatch("hy4");
  assert.equal(picked.model, "hy4");

  const rejected = relayTrial.relayTrialPatch("workbuddy/hy4free");
  assert.equal(rejected.ok, false);
  assert.match(rejected.error, /不在限时免费清单内/);
});
