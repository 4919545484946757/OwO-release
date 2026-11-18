// R4-5 §3.4「provider 未配置」引导门的取样契约（真机矩阵三条红的根因收口）。
//
// 事实背景（不是推测，是 r3-failure-matrix 抓出来的）：R3-C 门禁重列 sidecar 之后，
// core 在**没有任何凭据**时不再以 provider/not_configured 退出，而是按 §3.4 的
// "正常运行"继续 ready，把稳定码推迟到模型调用时返回。于是 `needsSetup()` 的
// 一次性快照落在 starting 态 → 判"不需要引导" → 用户直接进主界面，
// 每次模型调用都挂。§3.4 规定这一类的可操作终态是**模型配置引导**，
// 所以引导判据必须等壳侧出终态后再取一次。
//
// 同时不能把 R3-BUG-23 的教训改回去：ready 之后复查壳的 provider 配置面，
// 会把"密钥只在环境里、没在壳里显式选过"的健康启动顶进引导页，
// 并且因为 boot() 提前 return 打乱首屏计数。这一版能安全复查的前提是
// provider.rs 已改成与 core 同口径（显式选择 > 环境凭据）——那条口径由
// 壳侧单测 `status_reflects_mode_and_key_presence` 守着，本文件守 UI 侧取样。
//
// 取"从 app.js 里抽出真实函数体在沙箱执行"的方式：needsSetup 的缺陷全在
// 取样时序上，读源码断言抓不住"什么时候会返回 false"这种行为。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const appSource = readFileSync(join(WEB, "app.js"), "utf8");
const setupGuideSource = readFileSync(join(WEB, "views", "setup-guide.view.js"), "utf8");
const preloadSource = readFileSync(join(WEB, "..", "electron", "src", "main", "preload.js"), "utf8");

/** 按大括号配平抽出顶层函数源码（含声明），用于在沙箱里真实执行。 */
function extractTopLevelFunction(source, name) {
  const start = source.indexOf(`async function ${name}(`);
  assert.ok(start >= 0, `app.js 里找不到 async function ${name}`);
  let depth = 0;
  let seenOpen = false;
  for (let i = start; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") {
      depth += 1;
      seenOpen = true;
    } else if (ch === "}") {
      depth -= 1;
      if (seenOpen && depth === 0) return source.slice(start, i + 1);
    }
  }
  throw new Error(`函数 ${name} 大括号不配平`);
}

const NEEDS_SETUP_SOURCE = extractTopLevelFunction(appSource, "needsSetup");

function makeHarness(options) {
  const opts = options || {};
  const invoked = [];
  const httpCalls = [];
  const states = (opts.states || []).slice();
  const sandbox = {
    SETUP_GATE_SETTLE_MS: opts.settleMs == null ? 600 : opts.settleMs,
    SETUP_GATE_TICK_MS: opts.tickMs == null ? 20 : opts.tickMs,
    localStorage: {
      getItem: key => key === "owo.setup.provider-deferred" && opts.providerDeferred ? "1" : null,
      setItem: () => {},
      removeItem: () => {},
    },
    setTimeout,
    Date,
    Promise,
    console,
    __workspaceInput: { value: "" },
    $: (id) => id === "workspace" ? sandbox.__workspaceInput : null,
    http: { request: () => httpCalls.push("http") },
    window: {
      __TAURI_INTERNALS__: {
        invoke: (command) => {
          invoked.push(command);
          if (command === "get_provider_status") return Promise.resolve(opts.providerStatus || { ready: true });
          if (command === "get_workspace") return Promise.resolve(opts.workspace || { workspace: "C:\projects\demo", configured: true });
          return Promise.resolve(null);
        },
      },
      OwoApiClient: { tauriInvokeOwner: (w) => w.__TAURI_INTERNALS__ },
    },
    apiClient: {
      ensureCoreConnection: () => {
        httpCalls.push("connection");
        const next = states.length > 1 ? states.shift() : states[0];
        return Promise.resolve(next === undefined ? null : next);
      },
    },
  };
  sandbox.window.apiClient = sandbox.apiClient;
  sandbox.window.OwoApi = sandbox.apiClient;
  vm.createContext(sandbox);
  vm.runInContext(`${NEEDS_SETUP_SOURCE}\nglobalThis.__run = needsSetup;`, sandbox, { filename: "needsSetup-extract.js" });
  sandbox.invoked = invoked;
  sandbox.httpCalls = httpCalls;
  return sandbox;
}

test("过渡态不再一次性判否：starting→ready 且壳说提供商未就绪时，必须进引导页", async () => {
  const sandbox = makeHarness({
    states: [
      { state: "starting" },
      { state: "starting" },
      { state: "ready" },
    ],
    providerStatus: { provider: "unset", ready: false, keyConfigured: false },
  });
  const started = Date.now();
  const result = await sandbox.__run();
  assert.equal(result, true, "§3.4：core 正常运行但无凭据 = 模型配置引导，不是主界面");
  assert.ok(sandbox.invoked.includes("get_provider_status"), "终态后才复查壳侧提供商视图");
  assert.ok(Date.now() - started >= 20, "必须真的等过至少一个取样周期，而不是拿首帧快照下结论");
  assert.ok(httpCallsLength(sandbox) >= 3, "过渡态要重新取壳快照");
});

function httpCallsLength(sandbox) {
  return sandbox.httpCalls.length;
}

test("有环境凭据的健康启动不得被顶进引导页（R3-BUG-23 前提）", async () => {
  const sandbox = makeHarness({
    states: [{ state: "ready" }],
    // provider.rs 新口径：未显式选择但环境有 key → ready:true（与 core 同源）。
    providerStatus: { provider: "unset", ready: true, keyConfigured: true },
  });
  assert.equal(await sandbox.__run(), false, "两份视图同口径后，ready+可用必须放行主界面");
});

test("真故障终态不被引导页盖掉归因：failed 直接放行错误卡，且不复查提供商", async () => {
  const sandbox = makeHarness({
    states: [{ state: "failed", errorCode: "core/exited", message: "后台意外退出" }],
  });
  assert.equal(await sandbox.__run(), false, "core/exited 是故障，不是未配置提供商");
  assert.ok(!sandbox.invoked.includes("get_provider_status"), "故障终态不得再问 get_provider_status 制造第二归因");
  assert.equal(sandbox.httpCalls.filter(call => call === "http").length, 0, "设置门探测不得调用 HTTP");
});

test("未选工作区不再强制引导页；稳定码 provider/not_configured 仍直接进引导", async () => {
  const workspace = makeHarness({ states: [{ state: "no_workspace" }] });
  assert.equal(await workspace.__run(), false, "无工作区允许进入主界面，由顶部入口/空状态引导");
  assert.deepEqual(workspace.invoked, [], "no_workspace 不必再查提供商（省一次 IPC）");

  const code = makeHarness({
    states: [{ state: "failed", errorCode: "provider/not_configured" }],
  });
  assert.equal(await code.__run(), true, "壳/core 已上报该稳定码时立即引导（R3-BUG-19 路径保留）");
});


test("workspace 为空时放行主界面；已明确稍后配置模型时同样放行", async () => {
  const missingWorkspace = makeHarness({ states: [{ state: "ready" }], workspace: { workspace: "" } });
  assert.equal(await missingWorkspace.__run(), false, "没有项目工作区时进入主界面的空状态，不再顶到首次配置");

  const fallbackOnly = makeHarness({ states: [{ state: "ready" }], workspace: { workspace: "C:\\Users\\23843", configured: false } });
  assert.equal(await fallbackOnly.__run(), false, "核心启动回退目录不能冒充用户项目工作区");
  assert.equal(fallbackOnly.__workspaceInput.value, "", "未配置时应清空旧 UI 选择");

  const deferred = makeHarness({ states: [{ state: "ready" }], workspace: { workspace: "C:\\projects\\demo" }, providerStatus: { ready: false }, providerDeferred: true });
  assert.equal(await deferred.__run(), false, "用户明确选择稍后配置后可以进入主界面");
});

test("取不到终态时不得无限等待：超时按不进引导处理", async () => {
  const sandbox = makeHarness({ states: [{ state: "starting" }], settleMs: 120, tickMs: 20 });
  const started = Date.now();
  const result = await sandbox.__run();
  assert.equal(result, false, "永远过渡态 = 事实未定，交给后续 readiness/错误卡链路");
  assert.ok(Date.now() - started < 2000, `必须有界返回（实际 ${Date.now() - started}ms）`);
});

test("非壳环境（纯浏览器 dev）不判引导；引导门取样全程零 HTTP", async () => {
  const sandbox = makeHarness({ states: [{ state: "ready" }] });
  delete sandbox.window.__TAURI_INTERNALS__;
  assert.equal(await sandbox.__run(), false, "无 Tauri IPC 时不得凭猜测进引导页");

  // 源码面守卫：函数体内不得出现 api()/fetch——引导门发生在首屏之前，
  // 任何 HTTP 都会污染 §8.2 的首屏请求口径。
  assert.ok(!/\bapi\(|fetch\(|XMLHttpRequest/.test(NEEDS_SETUP_SOURCE), `needsSetup 只能走 IPC：\n${NEEDS_SETUP_SOURCE}`);
  assert.match(appSource, /const SETUP_GATE_SETTLE_MS = 6000;/, "等待窗口必须显式常量（≤ §3.4 的 10s 可操作时限）");
});


test("稍后配置和目录选择都不依赖易失的浏览器状态", () => {
  assert.match(appSource, /Boolean\(provider && provider\.deferred\)/);
  assert.match(setupGuideSource, /displayAlias: false/);
});

test("设置门脚本接线：桌面先加载原生目录选择器与配置视图，后加载 app 主入口", () => {
  const html = readFileSync(join(WEB, "index.html"), "utf8");
  assert.match(html, /<main id="setupRoot"[^>]*hidden/);
  const picker = html.indexOf('<script src="core/folder-picker.js"></script>');
  const guide = html.indexOf('<script src="views/setup-guide.view.js"></script>');
  const app = html.indexOf('<script src="app.js"></script>');
  assert.ok(picker >= 0 && guide > picker && app > guide, "设置视图依赖必须先于 app 加载");
});


test("工作台桌宠显隐同时写核心并立即通知桌面壳", () => {
  assert.match(appSource, /window\.owo\.setPetVisible\(next\)/);
  assert.match(preloadSource, /setPetVisible: visible => ipcRenderer\.invoke\("pet:visible", Boolean\(visible\)\)/);
});

test("明确配置的 shell 工作区会回灌输入框，即使 WebView 本地存储为空", async () => {
  const selected = makeHarness({ states: [{ state: "ready" }], workspace: { workspace: "D:\\work\\demo", configured: true } });
  assert.equal(await selected.__run(), false);
  assert.equal(selected.__workspaceInput.value, "D:\\work\\demo");
});
