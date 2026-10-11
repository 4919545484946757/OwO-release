// R4-2 §4.3「全局状态条」契约测试。
//
// §4.3 的原话是"所有状态点击后进入对应页面，不能只是不可交互的文字灯"——这条最容易
// 被做成一排 <span>，所以断言集中在三件事：
//   ① 五段齐全（后台/工作区/模型/权限/当前任务）且每段是真 <button>、可点、有 aria 文案；
//   ② 术语与 §3.4/§4.7 的稳定码一致（同一错误码在首启/断线/恢复失败说同一句话），
//      工作区段必须走掩码（不得回显完整本地绝对路径）；
//   ③ 零新增 HTTP：状态条在首屏就在，任何 fetch 都会挤爆 §8.2「首屏 ≤5 请求」口径，
//      模型段只能走壳 IPC。
// 另：重绘只改 textContent，不重建节点（否则 1s 定时器会把用户的 Tab 焦点抢走）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const viewSource = readFileSync(join(WEB, "views", "status-bar.view.js"), "utf8");
const indexHtml = readFileSync(join(WEB, "index.html"), "utf8");
const appSource = readFileSync(join(WEB, "app.js"), "utf8");
const cssSource = readFileSync(join(WEB, "style.css"), "utf8");

function makeElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    className: "",
    children: [],
    listeners: {},
    attrs: {},
    textContent: "",
    style: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    getAttribute(name) {
      return this.attrs[name] === undefined ? null : this.attrs[name];
    },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    addEventListener(type, handler) {
      (this.listeners[type] = this.listeners[type] || []).push(handler);
    },
    querySelector(selector) {
      const attr = /^\[data-owo-status="([^"]+)"\]$/.exec(selector);
      if (attr) {
        return this.children.find((child) => child.attrs["data-owo-status"] === attr[1]) || null;
      }
      const cls = /^\.([A-Za-z0-9_-]+)$/.exec(selector);
      if (cls) {
        return this.children.find((child) => String(child.className).split(/\s+/).includes(cls[1])) || null;
      }
      const id = /^#([A-Za-z0-9_-]+)$/.exec(selector);
      if (id) return this.children.find((child) => child.attrs.id === id[1]) || null;
      return null;
    },
    querySelectorAll() {
      return this.children.slice();
    },
    click() {
      (this.listeners.click || []).forEach((fn) => fn({ type: "click" }));
    },
  };
  return el;
}

function makeSandbox(extra) {
  const host = makeElement("div");
  const dispatched = [];
  const sandbox = Object.assign(
    {
      document: {
        visibilityState: "visible",
        documentElement: makeElement("html"),
        body: { classList: { add() {}, remove() {}, toggle() {} } },
        createElement: makeElement,
      },
      CustomEvent: function CustomEvent(type, init) {
        this.type = type;
        this.detail = init && init.detail;
      },
      dispatchEvent: (event) => dispatched.push(event),
      setInterval: () => 0,
      clearInterval: () => {},
      navigator: {},
    },
    extra || {},
  );
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(viewSource, sandbox, { filename: "status-bar.view.js" });
  sandbox.__dispatched = dispatched;
  sandbox.__host = host;
  return sandbox;
}

function mountedSandbox(options) {
  const sandbox = makeSandbox(options && options.sandboxExtras);
  sandbox.OwoWorkspaceDisplay = {
    alias: (root) => String(root).split(/[\\/]/).filter(Boolean).pop(),
    masked: (root) => String(root).replace(/^([A-Za-z]):[\\/]+Users[\\/][^\\/]+/i, "$1:\\…\\用户"),
  };
  const source = options && options.source ? options.source : {};
  sandbox.OwoStatusBar.mount(sandbox.__host, { getFacts: () => source });
  return sandbox;
}

test("五段齐全且顺序符合 §4.3，每段都是可点击按钮（不是文字灯）", () => {
  const sandbox = mountedSandbox();
  const keys = sandbox.__host.children.map((child) => child.attrs["data-owo-status"]);
  assert.deepEqual(keys, ["backend", "workspace", "model", "permission", "task"], "五段顺序必须对齐 §4.3");
  for (const child of sandbox.__host.children) {
    assert.equal(child.tagName, "BUTTON", `${child.attrs["data-owo-status"]} 必须是 button（键盘可达）`);
    assert.equal(child.type, "button", "显式 type=button，避免落进表单默认提交");
    assert.ok(child.listeners.click && child.listeners.click.length, "必须真的挂了点击处理器");
    assert.ok(
      child.children.some((c) => c.className === "owo-status-label"),
      "每段要有中文标签，不能只丢一个内部枚举值",
    );
  }
});

test("点击段落派发导航事件，目标页与 §4.3 一一对应", () => {
  const sandbox = mountedSandbox();
  const byKey = {};
  for (const child of sandbox.__host.children) byKey[child.attrs["data-owo-status"]] = child;
  byKey.workspace.click();
  byKey.task.click();
  const events = sandbox.__dispatched;
  assert.equal(events.length, 2, "两次点击应派发两次导航事件");
  assert.equal(events[0].type, "owo:statusbar-navigate");
  assert.equal(events[0].detail.key, "workspace");
  assert.equal(events[0].detail.target, "projects", "工作区段 → 工作区页");
  assert.equal(events[1].detail.target, "chat", "任务段 → 会话页");
});

test("稳定错误码 → 用户术语（§4.7 同码同术语，不泄漏内部枚举）", () => {
  const sandbox = makeSandbox();
  const bar = sandbox.OwoStatusBar;
  const cases = [
    ["core/handshake_timeout", "启动超时"],
    ["core/binary_missing", "不可用"],
    ["core/identity_mismatch", "不可用"],
    ["core/exited", "失败"],
    ["workspace/required", "未选工作区"],
    ["provider/not_configured", "模型未配置"],
    ["storage/not_writable", "存储不可写"],
    ["network/cloud_disabled", "已拒绝联网"],
  ];
  for (const [code, term] of cases) {
    const fact = bar.backendFromDiagnostics({ state: "failed", errorCode: code }, null);
    assert.equal(fact.text, term, `${code} 必须说「${term}」`);
    assert.equal(fact.tone, "bad", `${code} 属终态，必须标红`);
    assert.ok(!fact.text.includes("/"), `${code} 不得把内部码原样丢给用户`);
  }
});

test("后台四态可辨识：可用 / 正在启动 / 降级 / 失败（§4.3 列举的四态）", () => {
  const sandbox = makeSandbox();
  const bar = sandbox.OwoStatusBar;
  assert.deepEqual(
    JSON.parse(JSON.stringify(bar.backendFromDiagnostics({ state: "ready" }, "live"))),
    { text: "可用", tone: "ok" },
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(bar.backendFromDiagnostics({ state: "starting" }, null))),
    { text: "正在启动", tone: "warn" },
  );
  const degraded = bar.backendFromDiagnostics({ state: "ready" }, "degraded");
  assert.equal(degraded.tone, "warn", "事件流降级必须是黄，不得伪装成全绿");
  assert.match(degraded.text, /降级/);
  assert.equal(bar.backendFromDiagnostics({ state: "restarting", errorCode: "core/exited" }, null).text, "重启中");
});

test("浏览器预览与桌面诊断故障不再永久显示为检查中", () => {
  const browser = makeSandbox();
  const preview = browser.OwoStatusBar.backendFromDiagnostics(null, null);
  assert.equal(preview.text, "浏览器预览");
  assert.equal(preview.tone, "muted");
  assert.match(preview.detail, /没有 Electron 桌面诊断连接/);

  const desktop = makeSandbox({ __TAURI_INTERNALS__: { invoke() {} } });
  assert.equal(desktop.OwoStatusBar.backendFromDiagnostics(null, null).text, "诊断重试中");
  const failed = desktop.OwoStatusBar.backendFromDiagnostics(
    { state: "unknown", message: "IPC 暂不可用" }, null
  );
  assert.equal(failed.text, "诊断重试中");
  assert.equal(failed.tone, "warn");
  assert.match(failed.detail, /IPC 暂不可用/);
});

test("工作区段给名称 + 路径摘要，绝不回显完整本地绝对路径", () => {
  const privateRoot = "C:\\Users\\ovo\\Documents\\客户A\\项目X";
  const sandbox = makeSandbox();
  sandbox.OwoWorkspaceDisplay = {
    alias: (root) => String(root).split(/[\\/]/).filter(Boolean).pop(),
    masked: (root) => root.replace(/^C:[\\/]+Users[\\/][^\\/]+/i, "C:\\…\\用户"),
  };
  const fact = sandbox.OwoStatusBar.computeFacts({ workspaceRoot: privateRoot }, null)[1];
  assert.match(fact.text, /项目X/, "要给出可读名称（§4.3：名称）");
  assert.equal(fact.detail, "…\\客户A\\项目X", "摘要只到末两级");
  for (const forbidden of ["C:", "Users", "ovo", "Documents"]) {
    assert.ok(!(fact.text + fact.detail).includes(forbidden), `状态条不得出现 ${forbidden}`);
  }
  const empty = sandbox.OwoStatusBar.computeFacts({ workspaceRoot: "" }, null)[1];
  assert.equal(empty.text, "未选择");
  assert.equal(empty.tone, "warn", "没选工作区必须显眼，不能显示成正常态");
});

test("权限段待审批数量来自运行态；档位未回灌时说「点按查看」而不是猜", () => {
  const sandbox = makeSandbox();
  const bar = sandbox.OwoStatusBar;
  const before = bar.computeFacts({ pendingApproval: null }, null)[3];
  assert.equal(before.text, "点按查看");
  assert.equal(before.tone, "muted");
  const waiting = bar.computeFacts({ pendingApproval: "req-1" }, null)[3];
  assert.match(waiting.text, /待审批 1/);
  assert.equal(waiting.tone, "warn");
  bar.reportPermission({ profile: "工作区（workspace）" });
  const after = bar.computeFacts({ pendingApproval: null }, null)[3];
  assert.equal(after.text, "工作区（workspace）", "权限中心回灌后展示权威档位");
});

test("当前任务段：运行 / 等待审批 / 已取消 / 已完成 / 空闲各有区分", () => {
  const sandbox = makeSandbox();
  const bar = sandbox.OwoStatusBar;
  const task = (source) => bar.computeFacts(source, null)[4];
  assert.equal(task({ reading: true }).text, "运行中");
  assert.equal(task({ reading: true, pendingApproval: "r" }).text, "等待审批", "等待审批优先级高于运行中");
  assert.equal(task({ lastTurnOutcome: "cancelled" }).text, "已取消");
  assert.equal(task({ lastTurnOutcome: "failed" }).text, "上一轮失败");
  assert.equal(task({ lastTurnOutcome: "completed" }).text, "回答完成 · 未验证");
  assert.equal(task({}).text, "空闲");
});

async function flush() {
  // 壳 IPC 是跨 realm 的 thenable，微任务要过好几跳；用宏任务冲刷，别靠猜跳数。
  for (let i = 0; i < 4; i += 1) await new Promise((resolve) => setImmediate(resolve));
}

test("模型状态兼容对象形设置值，不显示 [object Object]", () => {
  const sandbox = makeSandbox({ __owoCoreDiagnostics: { state: "ready" } });
  sandbox.OwoStatusBar.reportModel({
    provider: { provider: "bigmodel" },
    model: { name: "glm-5.3-flash" },
  });
  const model = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.equal(model.text, "bigmodel · glm-5.3-flash");
  assert.doesNotMatch(model.text, /\[object Object\]/);
  sandbox.OwoStatusBar.reportModel({ provider: "[object Object]", model: "glm-5.3-flash" });
  assert.equal(sandbox.OwoStatusBar.computeFacts({}, null)[2].text, "glm-5.3-flash");
});

test("壳提供商状态含对象时仍显示可读模型名", async () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: { state: "ready" },
    __TAURI_INTERNALS__: {
      invoke: (command) => Promise.resolve(command === "get_provider_status"
        ? { provider: { id: "bigmodel" }, model: { name: "glm-5.3-flash" }, ready: true }
        : { workspace: "" }),
    },
  });
  sandbox.OwoWorkspaceDisplay = { alias: (root) => root, masked: (root) => root };
  sandbox.OwoStatusBar.mount(sandbox.__host, { getFacts: () => ({}) });
  await flush();
  const model = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.equal(model.text, "bigmodel · glm-5.3-flash");
  assert.doesNotMatch(model.text, /\[object Object\]/);
});

test("模型段两份真相不混着说：壳侧报未配置但 core 已就绪时不得标红下结论", async () => {
  // 真机截图抓到的现场：密钥经 sidecar 注入，壳的 get_provider_status 报
  // provider=unset/ready=false，而 core 正常 ready 并在用 glm-5.3-flash——
  // 状态条当时显示「模型 unset · 未配置」（黄），与同一条「后台 可用」自相矛盾。
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: { state: "ready" },
    __TAURI_INTERNALS__: {
      invoke: (command) =>
        Promise.resolve(command === "get_provider_status" ? { provider: "unset", ready: false, model: "" } : { workspace: "" }),
    },
  });
  sandbox.OwoWorkspaceDisplay = { alias: (r) => r, masked: (r) => r };
  sandbox.OwoStatusBar.mount(sandbox.__host, { getFacts: () => ({ workspaceRoot: "" }) });
  await flush();
  const model = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.notEqual(model.tone, "warn", "core 已就绪时不得把壳侧视图当结论标黄：" + model.text);
  assert.notEqual(model.tone, "bad");
  assert.match(model.text, /壳侧/, "必须说明这是壳侧视图：" + model.text);
  assert.match(model.detail, /不一致|为准/, "明细要指出两份视图不一致：" + model.detail);
  // 设置页从 core 水合到实际生效模型后回灌 → 模型段改说权威事实。
  sandbox.OwoStatusBar.reportModel({ provider: "GLM", model: "glm-5.3-flash", credential: "environment" });
  const after = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.equal(after.text, "GLM · glm-5.3-flash");
  assert.equal(after.tone, "ok");
  assert.match(after.detail, /core/, "回灌值必须标明来源是 core");
});

test("壳侧报未配置且 core 也没就绪时，模型段就该显眼（不是永远温和）", async () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: { state: "failed", errorCode: "provider/not_configured" },
    __TAURI_INTERNALS__: {
      invoke: (command) =>
        Promise.resolve(command === "get_provider_status" ? { provider: "unset", ready: false, model: "" } : { workspace: "" }),
    },
  });
  sandbox.OwoStatusBar.mount(sandbox.__host, { getFacts: () => ({}) });
  await flush();
  const model = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.equal(model.tone, "warn", "真未配置必须标黄，不能拿「壳侧视图」当挡箭牌：" + model.text);
  assert.match(model.text, /未配置/);
});

test("未显式选择但环境有凭据：模型段说清来源，既不谎称已选也不报未配置", async () => {
  // provider.rs 与 core 统一为「显式选择 > 环境凭据」后，ready=true + provider=unset
  // 是合法组合（内置端点兜底）。状态条必须写明凭据来源，否则用户以为从没配上。
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: { state: "ready" },
    __TAURI_INTERNALS__: {
      invoke: (command) =>
        Promise.resolve(
          command === "get_provider_status"
            ? { provider: "unset", ready: true, keyConfigured: true, model: "", baseUrl: "" }
            : { workspace: "" },
        ),
    },
  });
  sandbox.OwoWorkspaceDisplay = { alias: (r) => r, masked: (r) => r };
  sandbox.OwoStatusBar.mount(sandbox.__host, { getFacts: () => ({}) });
  await flush();
  const model = sandbox.OwoStatusBar.computeFacts({}, null)[2];
  assert.equal(model.tone, "ok", "可用就是可用：" + model.text);
  assert.match(model.text, /环境变量凭据|内置端点/, "必须标明凭据来源：" + model.text);
  assert.ok(!/未配置/.test(model.text), "不得再说未配置：" + model.text);
  assert.match(model.detail, /未显式选择|可在设置/, "明细要说明这是兜底路径：" + model.detail);
});

test("重绘只改文本不重建节点（保住键盘焦点）", () => {
  const sandbox = mountedSandbox({ source: { workspaceRoot: "", reading: false } });
  const firstNodes = sandbox.__host.children.slice();
  sandbox.OwoStatusBar.reportPermission({ profile: "只读" });
  sandbox.OwoStatusBar.repaint();
  assert.equal(sandbox.__host.children.length, 5, "重绘不得新增/替换段节点");
  firstNodes.forEach((node, index) => assert.equal(node, sandbox.__host.children[index], "同一段必须是同一 DOM 节点"));
  assert.match(sandbox.__host.children[3].children[1].textContent, /只读/);
});

test("状态条零新增 HTTP：模型/工作区段走壳 IPC（§8.2 首屏 ≤5 请求口径）", () => {
  assert.ok(!/fetch\(|XMLHttpRequest|OwoApi|apiClient/.test(viewSource), "视图内不得出现任何 HTTP 出口");
  assert.match(viewSource, /invokeShell\(owner, "get_provider_status"\)/, "模型段只能经壳命令取提供商状态");
  assert.match(viewSource, /invokeShell\(owner, "get_workspace"\)/, "工作区段必须壳侧水合（真机缺陷回归）");
  assert.match(viewSource, /PROVIDER_TTL_MS = 15000/, "IPC 取值必须带 TTL 缓存，不得每秒重查");
  assert.match(viewSource, /WORKSPACE_TTL_MS = 5000/, "工作区变化要更勤（换目录后 5s 内可见）");
  assert.match(viewSource, /if \(uiHidden\(\)\) return;[\s\S]{0,160}refreshProviderCache\(false\)/, "隐藏窗口不得继续刷新（§3.4）");
});

test("工作区段壳侧回灌：localStorage 为空时也必须显示已选工作区（R4 真机缺陷回归）", async () => {
  // 真机全新 WebView2 存储下 localStorage 没有 owo.workspace，只读前端 state 会
  // 在「壳其实带着工作区跑起来了」时显示「未选择」——28/31 那轮就是这么红的。
  const invoked = [];
  const sandbox = makeSandbox({
    __TAURI_INTERNALS__: {
      invoke(command) {
        invoked.push(command);
        if (command === "get_workspace") return Promise.resolve({ workspace: "D:\\work\\客户甲\\订单系统", configured: true, state: "ready" });
        return Promise.resolve({ ready: true, model: "glm-5.3-flash" });
      },
    },
  });
  sandbox.OwoWorkspaceDisplay = {
    alias: (root) => String(root).split(/[\\/]/).filter(Boolean).pop(),
    masked: (root) => root,
  };
  const host = sandbox.__host;
  sandbox.OwoStatusBar.mount(host, { getFacts: () => ({ workspaceRoot: "" }) });
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(invoked.sort(), ["get_provider_status", "get_workspace"], "两段都必须各自查一次壳（都是 IPC，不发 HTTP）");
  const ws = sandbox.OwoStatusBar.computeFacts({ workspaceRoot: "" }, null)[1];
  assert.equal(ws.text, "订单系统", "壳报的工作区必须显示出来，不能停在「未选择」");
  assert.equal(ws.tone, "ok");
  assert.equal(ws.detail, "…\\客户甲\\订单系统");
  // 撤销工作区后不得停留在旧值上（壳报空 = 状态条报空）。
  sandbox.OwoStatusBar.unmount();
  const sandbox2 = makeSandbox({
    __TAURI_INTERNALS__: {
      invoke(command) {
        if (command === "get_workspace") return Promise.resolve({ workspace: "C:\\Users\\23843", configured: false, state: "ready" });
        return Promise.resolve({ ready: false });
      },
    },
  });
  sandbox2.OwoWorkspaceDisplay = { alias: (r) => r, masked: (r) => r };
  sandbox2.OwoStatusBar.mount(sandbox2.__host, { getFacts: () => ({ workspaceRoot: "" }) });
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(sandbox2.OwoStatusBar.computeFacts({ workspaceRoot: "" }, null)[1].text, "未选择");
});

test("接线：容器与脚本就位、boot 早期挂载、导航事件有接收方", () => {
  assert.match(indexHtml, /<div id="globalStatusBar" class="global-status-bar"/, "主窗口顶部必须有状态条容器");
  assert.match(indexHtml, /<script src="views\/status-bar\.view\.js"><\/script>/, "视图脚本必须被引入");
  const boot = /async function boot\(\) \{([\s\S]{0,900}?)if \(await needsSetup\(\)\)/.exec(appSource);
  assert.ok(boot, "boot 早期段落应可定位");
  assert.match(boot[1], /initGlobalStatusBar\(\);/, "状态条必须在水合/引导分流之前就位（引导页也要能看出后台态）");
  assert.match(appSource, /addEventListener\("owo:statusbar-navigate"/, "app.js 必须接收状态条导航事件");
  assert.match(cssSource, /\.owo-status-item:focus-visible/, "键盘焦点样式必须存在（§4.10）");
  assert.match(cssSource, /\.owo-status-item \.owo-status-label \{ display: none; \}/, "860px 窄屏收起标签只留值");
});

test("selected custom model status uses its own endpoint instead of the cached default provider", () => {
  const sandbox=makeSandbox({__owoCoreDiagnostics:{state:"ready"}});
  sandbox.OwoStatusBar.reportModel({provider:"openai-compatible",model:"gpt-4o-mini"});
  const source={selectedModel:"custom-x",settings:{custom_models:[{id:"custom-x",apiFormat:"anthropic",baseUrl:"http://localhost:7777/v1"}]}};
  const selected=sandbox.OwoStatusBar.computeFacts(source,null)[2];
  assert.equal(selected.text,"Anthropic · custom-x");
  assert.match(selected.detail,/localhost:7777/);
  assert.doesNotMatch(selected.text,/gpt-4o-mini/);
  assert.equal(sandbox.OwoStatusBar.computeFacts({},null)[2].text,"openai-compatible · gpt-4o-mini");
});
