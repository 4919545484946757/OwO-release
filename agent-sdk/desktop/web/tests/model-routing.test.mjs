import test from "node:test";
import assert from "node:assert/strict";
import routing from "../core/model-routing.js";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
test("runtime model wins over stale saved-model value", () => {
  assert.equal(routing.effectiveDefaultModel({model:"qwen3.8-max",runtime:{model:"glm-5.3-flash"}}),"glm-5.3-flash");
  assert.equal(routing.effectiveDefaultModel({model:"deepseek-chat"}),"deepseek-chat");
  assert.equal(routing.effectiveDefaultModel({}),"");
});
test("empty session model clears the override", () => {
  assert.deepEqual(routing.buildSessionModelRequest("  "),{model:null});
  assert.deepEqual(routing.buildSessionModelRequest("glm-5.3-flash"),{model:"glm-5.3-flash"});
});
test("new sessions inherit no previous override", () => {
  assert.deepEqual(routing.buildCreateSessionRequest("D:\\work\\app",null),{workspace:"D:\\work\\app"});
  assert.deepEqual(routing.buildCreateSessionRequest("D:\\work\\app","glm-5.3-flash"),{workspace:"D:\\work\\app",model:"glm-5.3-flash"});
});


test("provider aliases resolve environment-only Qwen runtime to the DashScope model catalog", () => {
  const dashscope = { id: "dashscope", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", models: ["qwen3.8-max"] };
  assert.equal(routing.normalizeProviderId("qwen"), "dashscope");
  assert.equal(routing.normalizeProviderId(" QWEN "), "dashscope");
  assert.equal(routing.findProviderPreset("qwen", [dashscope], ""), dashscope);
  assert.equal(routing.findProviderPreset("openai-compatible", [dashscope], dashscope.baseUrl), dashscope);
  assert.equal(routing.findProviderPreset("unknown", [dashscope], ""), null);
});

test("DashScope catalog includes currently documented Qwen 3.8 models", () => {
  const source = readFileSync(new URL("../config/provider-presets.js", import.meta.url), "utf8");
  assert.match(source, /"qwen3\.8-max"/);
  assert.match(source, /"qwen3\.8-flash"/);
});

test("known provider model lists hide incompatible choices but preserve an invalid current model with warning state", () => {
  assert.deepEqual(routing.providerModelOptionState("glm-5.3-flash", ["glm-5.3-flash"], "glm-5.3-flash"), {
    hidden: false, disabled: false,
  });
  assert.deepEqual(routing.providerModelOptionState("qwen3.8-max", ["glm-5.3-flash"], "glm-5.3-flash"), {
    hidden: true, disabled: false,
  });
  assert.deepEqual(routing.providerModelOptionState("qwen3.8-max", ["glm-5.3-flash"], "qwen3.8-max"), {
    hidden: false, disabled: true,
  });
  assert.deepEqual(routing.providerModelOptionState("custom-id", ["glm-5.3-flash"], "custom-id", true), {
    hidden: false, disabled: false,
  });
  assert.deepEqual(routing.providerModelOptionState("ollama-local", [], "ollama-local"), {
    hidden: false, disabled: false,
  });
});


test("settings model list adds supported options and marks only the incompatible current choice", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function syncProviderModels\(presetOverride\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match, "syncProviderModels must remain testable as a bounded function");
  const option = (value) => ({ value, dataset: {}, hidden: false, disabled: false, textContent: value });
  const select = {
    value: "qwen3.8-max",
    options: [option("qwen3.8-max"), option("qwen3.7-max"), option("glm-5.2")],
    querySelector(selector) {
      const value = selector.match(/option\[value="(.*)"\]/)?.[1];
      return this.options.find((item) => item.value === value) || null;
    },
    appendChild(item) { this.options.push(item); },
  };
  const hint = { hidden: true, textContent: "" };
  const context = {
    state: { settings: { runtime: { provider: "bigmodel" }, provider: { base_url: "https://open.bigmodel.cn/api/paas/v4" } } },
    window: { OwoProviderPresets: { presets: () => [{ id: "bigmodel", baseUrl: "https://open.bigmodel.cn/api/paas/v4", models: ["glm-5.3-flash", "glm-z1-flash"] }] } },
    ModelRouting: routing,
    CSS: { escape: (value) => value },
    document: { createElement: () => option("") },
    isReasoningModel: () => false,
    getDefaultModel: () => select.value,
    $: (id) => id === "settingsModel" ? select : id === "modelCompatibilityHint" ? hint : null,
  };
  const sync = runInNewContext(match[0] + "\nsyncProviderModels;", context);
  sync();
  const byId = (id) => select.options.find((item) => item.value === id);
  assert.equal(byId("glm-5.3-flash").hidden, false);
  assert.equal(byId("qwen3.7-max").hidden, true);
  assert.equal(byId("qwen3.8-max").hidden, false);
  assert.equal(byId("qwen3.8-max").disabled, true);
  assert.equal(hint.hidden, false);
  select.value = "glm-5.3-flash";
  sync();
  assert.equal(byId("qwen3.8-max").hidden, true);
  assert.equal(byId("glm-5.3-flash").disabled, false);
  assert.equal(hint.hidden, true);
});

test("Qwen runtime alias filters composer/settings choices even when endpoint came from environment", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function syncProviderModels\(presetOverride\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match);
  const composerMenu = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(composerMenu);
  assert.match(composerMenu[0], /ModelRouting\.findProviderPreset/, "会话模型菜单与设置页必须共用服务商别名解析");
  const option = (value) => ({ value, dataset: {}, hidden: false, disabled: false, textContent: value });
  const select = {
    value: "qwen3.8-max",
    options: [option("qwen3.8-max"), option("deepseek-v4-pro-0813"), option("glm-5.2")],
    querySelector(selector) {
      const value = selector.match(/option\[value="(.*)"\]/)?.[1];
      return this.options.find((item) => item.value === value) || null;
    },
    appendChild(item) { this.options.push(item); },
  };
  const context = {
    state: { settings: { runtime: { provider: "qwen" }, provider: {} } },
    window: { OwoProviderPresets: { presets: () => [{ id: "dashscope", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", models: ["qwen3.8-max", "qwen3.8-flash"] }] } },
    ModelRouting: routing,
    CSS: { escape: (value) => value },
    document: { createElement: () => option("") },
    isReasoningModel: () => false,
    getDefaultModel: () => select.value,
    $: (id) => id === "settingsModel" ? select : null,
  };
  runInNewContext(match[0] + "\nsyncProviderModels();", context);
  const byId = (id) => select.options.find((item) => item.value === id);
  assert.equal(byId("qwen3.8-max").hidden, false);
  assert.equal(byId("qwen3.8-max").disabled, false);
  assert.equal(byId("qwen3.8-flash").hidden, false);
  assert.equal(byId("deepseek-v4-pro-0813").hidden, true);
  assert.equal(byId("glm-5.2").hidden, true);
});

test("same-session model updates are serialized and only latest selection reports current", async () => {
  let releaseFirst;
  const requests = [];
  const queue = routing.createSessionModelUpdateQueue((sessionId, request) => {
    requests.push({ sessionId, request });
    if (request.model === "model-a") {
      return new Promise((resolve) => { releaseFirst = resolve; });
    }
    return Promise.resolve();
  });

  const first = queue.enqueue("session-1", "model-a");
  await Promise.resolve();
  const second = queue.enqueue("session-1", "model-b");
  await Promise.resolve();
  assert.equal(requests.length, 1, "第二次变更等待第一次结束");
  releaseFirst();
  const [oldChoice, latestChoice] = await Promise.all([first, second]);

  assert.deepEqual(requests, [
    { sessionId: "session-1", request: { model: "model-a" } },
    { sessionId: "session-1", request: { model: "model-b" } },
  ]);
  assert.equal(oldChoice.latest, false);
  assert.equal(latestChoice.latest, true);
});

test("failed older model choice does not block newer choice or surface as current", async () => {
  const requests = [];
  const queue = routing.createSessionModelUpdateQueue((sessionId, request) => {
    requests.push(request.model);
    if (request.model === "bad") return Promise.reject(new Error("model rejected"));
    return Promise.resolve();
  });
  const oldChoice = queue.enqueue("session-2", "bad");
  const latestChoice = queue.enqueue("session-2", "good");
  const [oldResult, latestResult] = await Promise.all([oldChoice, latestChoice]);
  assert.deepEqual(requests, ["bad", "good"]);
  assert.equal(oldResult.latest, false);
  assert.match(oldResult.error.message, /model rejected/);
  assert.equal(latestResult.latest, true);
  assert.equal(latestResult.error, null);
});

test("provider compatibility distinguishes unsupported choices from custom endpoints and models", () => {
  assert.deepEqual(routing.providerModelCompatibility("qwen3.8-max", ["glm-5.3-flash"]), { known: true, compatible: false });
  assert.deepEqual(routing.providerModelCompatibility("glm-5.3-flash", ["glm-5.3-flash"]), { known: true, compatible: true });
  assert.deepEqual(routing.providerModelCompatibility("local-model", ["glm-5.3-flash"], true), { known: false, compatible: true });
  assert.deepEqual(routing.providerModelCompatibility("local-model", []), { known: false, compatible: true });
});

test("composer menu labels unsupported current model separately instead of as provider-supported", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(match);
  const option = (value, custom = false) => ({ value, textContent: value, dataset: { custom: custom ? "1" : "" } });
  const options = [option("qwen3.8-max"), option("glm-5.3-flash"), option("my-local-model", true)];
  let menuHtml = "";
  const context = {
    state: { sessionId: "s1", selectedModel: "qwen3.8-max", settings: { runtime: { provider: "bigmodel" }, provider: {} } },
    window: { OwoProviderPresets: { presets: () => [{ id: "bigmodel", label: "BigModel", models: ["glm-5.3-flash"] }] } },
    ModelRouting: routing,
    $: (id) => id === "settingsModel" ? { options } : id === "modelChip" ? {} : null,
    getComposerModel: () => "qwen3.8-max",
    getDefaultModel: () => "glm-5.3-flash",
    esc: (value) => String(value),
    escapeAttribute: (value) => String(value),
    openComposerMenu: (_trigger, html) => { menuHtml = html; },
    closeComposerMenu() {},
    selectModel() {},
    setSettingsPageVisible() {},
    setSettingsTab() {},
    composerMenuEl: null,
  };
  runInNewContext(match[0] + "\nopenModelMenu();", context);
  assert.match(menuHtml, /当前模型与服务商不匹配/);
  assert.match(menuHtml, /当前服务商可用模型/);
  assert.match(menuHtml, /glm-5.3-flash/);
  assert.match(menuHtml, /当前会话模型（不兼容）/);
  assert.match(menuHtml, /qwen3.8-max/);
  assert.match(menuHtml, /自定义模型/);
});

test("custom selection carries its own connection instead of only the model id", () => {
  const models = [{ id: "custom-x", baseUrl: "http://127.0.0.1:9876/complete",
    apiFormat: "openai", useFullUrl: true, temperature: 0, timeoutSecs: 9 }];
  assert.deepEqual(routing.buildCustomModelConnection("custom-x", models, { "custom-x": "fixture-key" }), {
    model: "custom-x", base_url: "http://127.0.0.1:9876/complete", api_format: "openai",
    use_full_url: true, api_key: "fixture-key", temperature: 0, timeout_secs: 9,
  });
  assert.equal(routing.buildCustomModelConnection("ordinary", models, {}), null);
  assert.equal(routing.buildCustomModelConnection("alt", [{id:"alt",baseUrl:"http://localhost:7777/v1",apiFormat:"anthropic"}], {}).api_format, "anthropic");
  assert.equal(models[0].api_key, undefined);
});
test("invalid custom configuration fails before submitting a turn", () => {
  for (const patch of [{baseUrl:""}, {baseUrl:"file:///tmp"}, {baseUrl:"https://user:secret@example.com"},
    {baseUrl:"https://example.com/?api_key=secret"}, {temperature:NaN}, {timeoutSecs:0}, {apiFormat:"unknown"}]) {
    assert.throws(() => routing.buildCustomModelConnection("x", [{id:"x",baseUrl:"https://example.com/v1", ...patch}], {}));
  }
});

test("image declaration is bound to endpoint, model, protocol and URL mode", () => {
  const binding={model:"vision",base_url:"https://EXAMPLE.com:443/v1/",api_format:"openai",image_input:true};
  assert.equal(routing.imageInputDeclaration({...binding,base_url:"https://example.com/v1"},[binding]),"supported");
  for(const change of [{model:"other"},{base_url:"https://example.com/V1"},{api_format:"anthropic"},{use_full_url:true}]) {
    assert.equal(routing.imageInputDeclaration({...binding,...change},[binding]),"unknown");
  }
  assert.equal(routing.imageInputDeclaration(binding,[{...binding,image_input:false}]),"unsupported");
  assert.equal(routing.imageInputDeclaration(binding,[]),"unknown");
  assert.match(routing.imageInputLabel("supported"),/声明支持.*未实测/);
});
test("custom connections carry exact declarations and reject stale declarations", () => {
  const model={id:"vision",baseUrl:"https://example.com/v1",apiFormat:"anthropic",inputCapabilities:{model:"vision",base_url:"https://example.com/v1",api_format:"anthropic",image_input:true}};
  assert.deepEqual(routing.buildCustomModelConnection("vision",[model],{}).input_capabilities,model.inputCapabilities);
  assert.throws(()=>routing.buildCustomModelConnection("changed",[{...model,id:"changed"}],{}),/声明.*不匹配/);
  assert.equal(routing.buildCustomModelConnection("unknown",[{id:"unknown",baseUrl:model.baseUrl}],{}).input_capabilities,undefined);
});
