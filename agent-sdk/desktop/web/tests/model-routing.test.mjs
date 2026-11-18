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
    loadCustomModels: () => [],
    refreshComposerModelChip: () => {},
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
  const composerCatalog = /function composerModelCatalog\(\) \{[\s\S]*?\n\}/.exec(app);
  assert.ok(composerCatalog);
  assert.match(composerCatalog[0], /ModelRouting\.findProviderPreset/, "会话模型菜单与设置页必须共用服务商别名解析（含自定义连接端点）");
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
    loadCustomModels: () => [],
    refreshComposerModelChip: () => {},
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

test("composer menu flags the unsupported current model and keeps details collapsed", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  const catalogSource = /function composerModelCatalog\(\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(match && catalogSource);
  let menuHtml = "";
  const context = {
    state: { sessionId: "s1", selectedModel: "qwen3.8-max", settings: { runtime: { provider: "bigmodel" }, provider: {} } },
    window: { OwoProviderPresets: { presets: () => [{ id: "bigmodel", label: "BigModel", models: ["glm-5.3-flash"] }] } },
    ModelRouting: routing,
    EFFORT_MODES: [
      { id: "", label: "默认", desc: "跟随模型自身默认" },
      { id: "high", label: "高", desc: "尽量深入思考" },
    ],
    getEffort: () => "",
    loadCustomModels: () => [{ id: "my-local-model", label: "本地模型", baseUrl: "http://127.0.0.1:9999/v1", disabled: false }],
    providerGateMissing: () => false,
    $: (id) => id === "modelChip" ? {} : null,
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
  runInNewContext(catalogSource + "\n" + match[0] + "\nopenModelMenu();", context);
  // 当前模型不在可用目录里：给出可读提示，但不把它当成一个可选项再列一遍。
  assert.match(menuHtml, /当前模型不可用/);
  assert.match(menuHtml, /没有可用连接/);
  assert.match(menuHtml, /qwen3\.8-max/);
  assert.doesNotMatch(menuHtml, /data-model="qwen3\.8-max"/);
  assert.match(menuHtml, /glm-5\.3-flash/);
  assert.match(menuHtml, /my-local-model/);
  assert.match(menuHtml, /BigModel/);
  assert.match(menuHtml, /独立连接/);
  assert.match(menuHtml, /思考强度/);
  assert.match(menuHtml, /data-effort/);
  assert.match(menuHtml, /data-effort-segment/);
  assert.match(menuHtml, /data-model-default/);
  assert.match(menuHtml, /composer-model-browser/);
  assert.match(menuHtml, /data-model-info aria-expanded="false"/);
  assert.match(menuHtml, /data-model-effort-panel[^>]* hidden>/);
  assert.match(menuHtml, /data-model-back/);
  assert.match(menuHtml, /data-model-use hidden/);
  assert.match(menuHtml, /data-model-preview/);
  assert.match(menuHtml, /data-model-reasoning-feature/);
  assert.match(menuHtml, /data-model-image-feature/);
  assert.match(menuHtml, /可用功能/);
  assert.doesNotMatch(menuHtml, /composer-model-arrow/);
  assert.doesNotMatch(menuHtml, /悬停或聚焦模型/);
  assert.doesNotMatch(menuHtml, /aria-haspopup/);
  // 不足 6 个模型时不渲染搜索框，列表直接露出。
  assert.doesNotMatch(menuHtml, /data-model-search-input/);
});

test("custom model connection routes provider compatibility through its own endpoint", () => {
  const presets = [
    { id: "openai", baseUrl: "https://api.openai.com/v1", models: ["gpt-4o-mini", "gpt-4o", "o3-mini"] },
    { id: "deepseek", baseUrl: "https://api.deepseek.com/v1", models: ["deepseek-flash", "deepseek-chat", "deepseek-reasoner"] },
  ];
  assert.equal(routing.findProviderPreset("", presets, "https://api.deepseek.com/")?.id, "deepseek");
  assert.equal(
    routing.findModelProviderPreset(
      "deepseek-flash",
      [{ id: "deepseek-flash", baseUrl: "https://api.deepseek.com/" }],
      "openai-compatible", presets, "https://api.openai.com/v1"
    )?.id,
    "deepseek"
  );
  // 运行时 provider 还停在别的服务商（bigmodel）时，自定义连接自己的端点仍然优先。
  assert.equal(
    routing.findModelProviderPreset(
      "deepseek-flash",
      [{ id: "deepseek-flash", baseUrl: "https://api.deepseek.com/" }],
      "bigmodel", presets, "https://open.bigmodel.cn/api/paas/v4"
    )?.id,
    "deepseek"
  );
  assert.equal(routing.findModelProviderPreset("other", [], "openai-compatible", presets, "https://api.openai.com/v1")?.id, "openai");
});

test("composer menu keeps the catalog clean when a custom connection drives the current model", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  const catalogSource = /function composerModelCatalog\(\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(match && catalogSource);
  let menuHtml = "";
  const context = {
    state: { sessionId: null, pendingModelOverride: null, settings: { runtime: { provider: "openai-compatible" }, provider: { base_url: "https://api.openai.com/v1" } } },
    window: { OwoProviderPresets: { presets: () => [
      { id: "openai", label: "OpenAI", baseUrl: "https://api.openai.com/v1", models: ["gpt-4o-mini", "gpt-4o", "o3-mini"] },
      { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", models: ["deepseek-flash", "deepseek-chat", "deepseek-reasoner"] },
    ] } },
    ModelRouting: routing,
    EFFORT_MODES: [{ id: "", label: "默认", desc: "跟随模型自身默认" }],
    getEffort: () => "",
    loadCustomModels: () => [{ id: "deepseek-flash", label: "deepseek", baseUrl: "https://api.deepseek.com/", apiFormat: "openai" }],
    // 只配了 deepseek-flash 一条独立连接，服务商本身未就绪。
    providerGateMissing: () => true,
    $: (id) => id === "modelChip" ? {} : null,
    getComposerModel: () => "deepseek-flash",
    getDefaultModel: () => "deepseek-flash",
    esc: (value) => String(value),
    escapeAttribute: (value) => String(value),
    openComposerMenu: (_trigger, html) => { menuHtml = html; },
    closeComposerMenu() {},
    selectModel() {},
    setSettingsPageVisible() {},
    setSettingsTab() {},
    composerMenuEl: null,
  };
  runInNewContext(catalogSource + "\n" + match[0] + "\nopenModelMenu();", context);
  assert.doesNotMatch(menuHtml, /不匹配/);
  // 目录里没有连接过的模型（DeepSeek 预设的其它型号、OpenAI 的型号）都不该出现。
  assert.doesNotMatch(menuHtml, /gpt-4o/);
  assert.doesNotMatch(menuHtml, /deepseek-chat/);
  assert.match(menuHtml, /data-model="deepseek-flash"/);
  // 唯一可用模型就是默认模型时，不再重复一条「默认 · deepseek-flash」。
  assert.doesNotMatch(menuHtml, /data-model-default/);
});

test("model menu keeps a way back when the current model has no connection", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const match = /function openModelMenu\(\) \{[\s\S]*?\n\}/.exec(app);
  const catalogSource = /function composerModelCatalog\(\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(match && catalogSource);
  let menuHtml = "";
  const context = {
    state: {
      sessionId: "s1",
      selectedModel: "deepseek-chat",
      settings: {
        runtime: { provider: "openai-compatible", provider_ready: false },
        provider: { base_url: "https://api.openai.com/v1" },
      },
    },
    window: { OwoProviderPresets: { presets: () => [
      { id: "openai", label: "OpenAI", baseUrl: "https://api.openai.com/v1", models: ["gpt-4o-mini"] },
      { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", models: ["deepseek-flash", "deepseek-chat"] },
    ] } },
    ModelRouting: routing,
    EFFORT_MODES: [{ id: "", label: "默认", desc: "跟随模型自身默认" }],
    getEffort: () => "",
    loadCustomModels: () => [{ id: "deepseek-flash", label: "deepseek", baseUrl: "https://api.deepseek.com/", apiFormat: "openai" }],
    providerGateMissing: () => true,
    $: (id) => id === "modelChip" ? {} : null,
    getComposerModel: () => "deepseek-chat",
    getDefaultModel: () => "deepseek-flash",
    esc: (value) => String(value),
    escapeAttribute: (value) => String(value),
    openComposerMenu: (_trigger, html) => { menuHtml = html; },
    closeComposerMenu() {},
    selectModel() {},
    setSettingsPageVisible() {},
    setSettingsTab() {},
    composerMenuEl: null,
  };
  runInNewContext(catalogSource + "\n" + match[0] + "\nopenModelMenu();", context);
  // 当前模型没有连接时，菜单仍打开并给出切回可用模型的入口。
  assert.match(menuHtml, /当前模型不可用/);
  assert.match(menuHtml, /deepseek-chat 没有可用连接/);
  assert.doesNotMatch(menuHtml, /data-model="deepseek-chat"/);
  assert.match(menuHtml, /data-model="deepseek-flash"/);
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
