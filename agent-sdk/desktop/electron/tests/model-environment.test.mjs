import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import commands from "../src/main/shell-commands.js";
import imeTaskAccessModule from "../src/main/ime-task-access.js";

function actualCoreEnv(config, inherited = {}) {
  const source = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");
  const start = source.indexOf("function coreEnv(");
  const end = source.indexOf("// ---------- HTTP", start);
  assert.ok(start >= 0 && end > start);
  const context = {
    process: { env: inherited }, shellCommands: commands, imeTaskAccessModule,
    agentDataDir: () => "fixture-data", PAIRING_SECRET: "fixture-pairing", IME_ORIGIN_KEY: "fixture-origin",
  };
  vm.createContext(context);
  vm.runInContext(source.slice(start, end), context);
  return context.coreEnv(config, { OWO_DESKTOP_INSTANCE_ID: "fixture-instance" });
}

test("every explicit provider injects the endpoint and model shown in settings", () => {
  for (const provider of ["ollama", "bigmodel", "openai", "deepseek", "dashscope"]) {
    const model = { provider, base_url: "", name: "" };
    const status = commands.providerStatusValue(model, { envGet: () => "" });
    const env = actualCoreEnv({ model });
    assert.equal(env.OPENAI_BASE_URL, status.baseUrl, provider);
    assert.equal(env.OPENAI_MODEL, status.model, provider);
    assert.equal(env.OWO_PROVIDER, "openai");
    assert.equal(env.OWO_DESKTOP_INSTANCE_ID, "fixture-instance");
  }
});

test("explicit provider switch replaces inherited endpoint/model/native routing", () => {
  const env = actualCoreEnv({ model: { provider: "ollama", base_url: "", name: "" } }, {
    OPENAI_BASE_URL: "http://127.0.0.1:9999/v1", OPENAI_MODEL: "previous-model", OWO_PROVIDER: "anthropic",
  });
  assert.equal(env.OPENAI_BASE_URL, commands.PROVIDER_DEFAULTS.ollama.base_url);
  assert.equal(env.OPENAI_MODEL, commands.PROVIDER_DEFAULTS.ollama.model);
  assert.equal(env.OWO_PROVIDER, "openai");
});

test("timeout reaches the gateway variable and temperature zero is preserved", () => {
  const env = actualCoreEnv({ model: { provider: "bigmodel", timeout_secs: 17, temperature: 0 } });
  assert.equal(env.OWO_MODEL_REQUEST_TIMEOUT_SECS, "17");
  assert.equal(env.OWO_MODEL_TIMEOUT_SECS, "17");
  assert.equal(env.OWO_MODEL_TEMPERATURE, "0");
});

test("explicit parameter clear removes inherited values; omission preserves them", () => {
  const inherited = { OWO_MODEL_REQUEST_TIMEOUT_SECS: "9", OWO_MODEL_TIMEOUT_SECS: "9", OWO_MODEL_TEMPERATURE: "1", OWO_AGENT_COMPACTION: "1" };
  const cleared = actualCoreEnv({ model: { provider: "bigmodel", timeout_secs: null, temperature: null, compaction: null } }, inherited);
  for (const key of Object.keys(inherited)) assert.equal(cleared[key], undefined, key);
  const omitted = actualCoreEnv({ model: { provider: "bigmodel" } }, inherited);
  for (const key of Object.keys(inherited)) assert.equal(omitted[key], inherited[key], key);
});

test("local custom and externally configured local endpoints are ready without credentials", () => {
  const custom = { provider: "custom", base_url: "http://127.0.0.1:9911/v1", name: "fixture-model" };
  assert.equal(commands.providerStatusValue(custom, { envGet: () => "" }).ready, true);
  const inherited = { OPENAI_BASE_URL: "http://localhost:9911/v1", OPENAI_MODEL: "fixture-model" };
  const status = commands.providerStatusValue({ provider: "unset" }, { envGet: name => inherited[name] });
  const env = actualCoreEnv({ model: { provider: "unset" } }, inherited);
  assert.equal(status.ready, true);
  assert.equal(status.baseUrl, env.OPENAI_BASE_URL);
  assert.equal(status.model, env.OPENAI_MODEL);
  assert.equal(commands.providerStatusValue({ provider: "unset" }, { envGet: () => "" }).ready, false);
});

test("integer runtime budgets reject fractions instead of silently falling back in Rust", () => {
  for (const field of ["timeout_secs", "context_window", "keep_recent"]) {
    const patch = commands.applyModelConfigPatch({ version: 1, model: { provider: "bigmodel" } }, { [field]: 17.5 });
    assert.equal(patch.ok, false, field);
  }
  const patch = commands.applyModelConfigPatch({ version: 1, model: { provider: "bigmodel" } }, { temperature: 0.75 });
  assert.equal(patch.ok, true);
});
