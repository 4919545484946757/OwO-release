/* §4.3 全局状态条：主窗口顶部五段（后台 / 工作区 / 模型 / 权限 / 当前任务）。
 *
 * 设计约束（都是实测踩出来的，改动前请一并读）：
 * 1) **零 HTTP**：状态条在首屏就在，任何 fetch 都会挤爆 §8.2「首次可输入前 ≤5 请求」
 *    口径。所以这里只读三类既有事实——壳 IPC 快照（window.__owoCoreDiagnostics）、
 *    前端运行态（app.js 的 state）、以及页面自己上报的缓存（report*）。
 *    模型段用壳命令 get_provider_status（IPC，不计 HTTP），带 TTL 缓存。
 * 2) **可点击**（§4.3：不能只是文字灯）：每段都是真按钮，键盘可聚焦可回车，
 *    点击派发 owo:statusbar-navigate 由 app.js 决定去哪个页面。
 * 3) **重绘不重建**：只改 textContent/tone，不 innerHTML 覆盖，避免用户 Tab 焦点
 *    被 1 秒定时器抢走（§4.10 键盘可达性）。
 * 4) **隐藏期不刷**：uiHidden() 时跳过重绘与 IPC（§3.4 隐藏窗口零业务请求）。
 */
(function (global) {
  "use strict";

  const SEGMENTS = [
    { key: "backend", label: "服务", target: "settings", hint: "核心服务状态 · 点击进入设置与诊断" },
    { key: "workspace", label: "工作区", target: "projects", hint: "当前项目与读写范围 · 点击进入工作区" },
    { key: "model", label: "模型", target: "model", hint: "切换模型、设置自定义接口地址与模型名 · 点击进入模型页" },
    { key: "permission", label: "权限", target: "permissions", hint: "权限档位与待审批 · 点击进入权限中心" },
    { key: "task", label: "任务", target: "chat", hint: "任务进展 · 点击进入会话" },
  ];

  const PROVIDER_TTL_MS = 15000;
  // 工作区走 IPC（零 HTTP），可以比提供商更勤：引导页里换目录后 5s 内状态条就该跟上。
  const WORKSPACE_TTL_MS = 5000;
  const REFRESH_MS = 1000;

  const cache = {
    provider: { at: 0, value: null, pending: false },
    workspace: { at: 0, root: "", pending: false },
    permission: { profile: null, pendingApprovals: null, grants: null },
    // 设置页从 core 水合到的实际生效模型（唯一权威源），回灌给模型段。
    model: null,
  };

  function backendFromDiagnostics(diag, streamState) {
    const hasDesktopBridge = !!(
      (global.__TAURI__ && global.__TAURI__.core && typeof global.__TAURI__.core.invoke === "function") ||
      (global.__TAURI_INTERNALS__ && typeof global.__TAURI_INTERNALS__.invoke === "function")
    );
    if (!diag && !hasDesktopBridge) {
      return {
        text: "浏览器预览",
        tone: "muted",
        detail: "此页面没有 Electron 桌面诊断连接；页眉显示的服务状态反映 API 连通性",
      };
    }
    const info = diag || {};
    const state = String(info.state || "unknown");
    const code = info.errorCode ? String(info.errorCode) : null;
    if (state === "ready") {
      // 「可用」与「降级」的差别只在事件流：SSE 掉到 degraded 时界面仍可用但会滞后。
      if (streamState === "degraded") {
        return { text: "可用（事件流降级）", tone: "warn" };
      }
      return { text: "可用", tone: "ok" };
    }
    if (code) return { text: terminalBackendText(state, code), tone: "bad" };
    if (state === "starting" || state === "restarting") return { text: "正在启动", tone: "warn" };
    if (state === "no_workspace") return { text: "未选工作区", tone: "warn" };
    if (state === "stopped") return { text: "已停止", tone: "muted" };
    return { text: "诊断重试中", tone: "warn", detail: info.message || "桌面服务状态暂未返回" };
  }

  // 稳定码 → 用户术语（与 §4.7「相同错误码相同术语」同源，不在此页另起口径）。
  function terminalBackendText(state, code) {
    if (state === "restarting") return "重启中";
    if (code.indexOf("handshake") >= 0) return "启动超时";
    if (code.indexOf("binary_missing") >= 0 || code.indexOf("identity") >= 0) return "不可用";
    if (code.indexOf("workspace") >= 0) return "未选工作区";
    if (code.indexOf("provider") >= 0) return "模型未配置";
    if (code.indexOf("storage") >= 0) return "存储不可写";
    if (code.indexOf("cloud_disabled") >= 0) return "已拒绝联网";
    if (code.indexOf("exited") >= 0 || code.indexOf("hang") >= 0) return "失败";
    return "失败";
  }

  /** 路径摘要：只给末两级目录（§4.3「路径摘要」），账户目录与盘符都不出现。 */
  function pathSummary(root) {
    const parts = String(root || "").split(/[\\/]+/).filter(Boolean);
    if (!parts.length) return "";
    if (parts.length <= 2) return parts.join("\\");
    return "…" + "\\" + parts.slice(-2).join("\\");
  }

  function workspaceFromFacts(source) {
    // 真机实测（R4 验收）：全新 WebView2 存储下 localStorage 里没有 owo.workspace，
    // 只靠前端 state.workspaceRoot 会让工作区段在"壳其实已经带着工作区启动"时显示
    // 「未选择」——壳的 get_workspace 才是权威来源（IPC，零 HTTP）。
    const root = String((source && source.workspaceRoot) || (cache.workspace && cache.workspace.root) || "");
    const display = global.OwoWorkspaceDisplay;
    if (!root) return { text: "未选择", tone: "warn", detail: "尚未选择项目目录" };
    const alias = display ? display.alias(root) : pathSummary(root).split("\\").pop();
    const writable = cache.permission.profile
      ? profileWritable(cache.permission.profile)
      : null;
    const mode = writable === null ? "" : writable ? " · 可写" : " · 只读";
    // 摘要=末两级目录：够用户分辨同名项目，又不把账户目录/盘符带到 title 里。
    return { text: alias + mode, tone: writable === false ? "warn" : "ok", detail: pathSummary(root) };
  }

  function profileWritable(profile) {
    const text = String(profile || "").toLowerCase();
    if (text.indexOf("read_only") >= 0 || text.indexOf("只读") >= 0) return false;
    if (text.indexOf("workspace") >= 0 || text.indexOf("auto") >= 0 || text.indexOf("full") >= 0) return true;
    return null;
  }

  function modelLabel(value, keys) {
    if (typeof value === "string" || typeof value === "number") {
      const text = String(value).trim();
      return text === "[object Object]" || text === "undefined" || text === "null" ? "" : text;
    }
    if (!value || typeof value !== "object") return "";
    for (const key of keys) {
      const text = modelLabel(value[key], keys);
      if (text) return text;
    }
    return "";
  }

  function modelFromCache(source) {
    const state = source || {};
    const settings = state.settings || {};
    const selected = String(state.selectedModel || state.pendingModelOverride || settings.model || "");
    const custom = (Array.isArray(settings.custom_models) ? settings.custom_models : []).find(item => item && item.id === selected);
    if (selected) {
      const actual = String(state.lastExecutionModel || "未知");
      const defaultModel = String(settings.model || modelLabel(cache.model?.model, ["model", "id", "name"]) || "未知");
      const endpoint = custom ? String(custom.baseUrl || "") : "";
      return { text: custom ? (custom.apiFormat === "anthropic" ? "Anthropic" : "OpenAI 兼容") + " · " + selected : selected, secondary: "上次实际：" + actual, tone: "muted", detail: "默认模型：" + defaultModel + "；本次请求：" + selected + "；上次实际执行：" + actual + (endpoint ? "；自定义端点：" + endpoint : "") };
    }
    if (custom) {
      return {
        text: (custom.apiFormat === "anthropic" ? "Anthropic" : "OpenAI 兼容") + " · " + custom.id,
        tone: "muted",
        detail: "当前自定义模型请求地址：" + String(custom.baseUrl || ""),
      };
    }
    // 两份真相不能混着说（R3-B 缺陷 23 的同族问题，真机截图抓到）：
    // 密钥由壳注入 sidecar 环境时，壳自己的 get_provider_status 仍会报
    // provider=unset/ready=false，而 core 实际已就绪并在用 glm-5.3-flash。
    // 所以：① 设置页水合到的 core 事实优先回灌；② 只有壳视图时不得把
    // 「未配置」当结论标红，必须说明这是壳侧视图。
    if (cache.model) {
      const info = cache.model;
      const provider = modelLabel(info.provider, ["provider", "id", "name"]);
      const model = modelLabel(info.model, ["model", "id", "name"]);
      const label = [provider, model].filter(Boolean).join(" · ") || "已配置";
      return { text: label, tone: "ok", detail: "来源：core 实际生效配置（设置页水合）" };
    }
    const entry = cache.provider.value;
    if (!entry) return { text: "读取中…", tone: "muted" };
    if (entry.error) return { text: "未读取", tone: "warn", detail: entry.error };
    const model = modelLabel(entry.model, ["model", "id", "name"]) || "未设置";
    const provider = modelLabel(entry.provider, ["provider", "id", "name"]) || "未知";
    const coreReady = String((global.__owoCoreDiagnostics || {}).state || "") === "ready";
    // 未显式选择提供商但环境里有凭据：壳与 core 现在同一口径判"可用"（provider.rs
    // 的"显式选择 > 环境凭据"），这里必须说清是**内置端点兜底**，而不是谎称用户选过。
    if (entry.ready !== false && provider === "unset") {
      return {
        text: "环境变量凭据 · 内置端点",
        tone: "ok",
        detail: "未显式选择提供商：core 正在用 OPENAI_API_KEY + 内置云端端点工作，可在设置里明确选择",
      };
    }
    if (entry.ready === false && coreReady) {
      return {
        text: provider + " · 壳侧未配置",
        tone: "muted",
        detail: "core 已就绪并在服务；壳侧配置视图与 core 不一致，实际生效模型见设置页",
      };
    }
    if (entry.ready === false) return { text: provider + " · 未配置", tone: "warn" };
    return { text: provider + " · " + model, tone: "ok" };
  }

  function permissionFromCache(source) {
    // 待审批数量以前端运行态为准（审批条就是唯一真相），权限中心可回灌权威值。
    let pending = cache.permission.pendingApprovals;
    if (pending == null) pending = source && source.pendingApproval ? 1 : 0;
    const profile = cache.permission.profile;
    const profileText = profile ? String(profile) : "点按查看";
    const pendingText = pending ? " · 待审批 " + pending : "";
    return {
      text: profileText + pendingText,
      tone: pending ? "warn" : "muted",
      detail: profile ? "当前档位：" + profile : "权限中心会给出实际范围与撤销入口",
    };
  }

  function taskFromFacts(source) {
    const state = source || {};
    if (state.stopping) return { text: "正在停止", tone: "warn", detail: "等待宿主终态，已产生的变更可能保留" };
    if (state.pendingApproval) return { text: "等待审批", tone: "warn" };
    if (state.reading) return { text: "运行中", tone: "ok" };
    const outcome = String(state.lastTurnOutcome || "");
    if (outcome === "cancelled") return { text: "已取消", tone: "muted" };
    if (outcome === "interrupted") return { text: "终态未确认", tone: "warn" };
    if (outcome === "failed") return { text: state.lastCompletionRecord?.status === "blocked" || state.lastCompletionStatus === "blocked" ? "存在阻断" : "上一轮失败", tone: "bad" };
    if (outcome === "completed") {
      const record = state.lastCompletionRecord;
      const status = record?.status || state.lastCompletionStatus;
      if (record?.status === "accepted") return { text: "验收通过", tone: "ok", detail: "宿主完成记录；回执 " + (Array.isArray(record.evidence_receipt_ids) ? record.evidence_receipt_ids.length : 0) + " 项" };
      if (status === "candidate") return { text: "候选待验收", tone: "warn" };
      if (status === "blocked") return { text: "存在阻断", tone: "bad" };
      if (status === "aborted") return { text: "已取消", tone: "muted" };
      return { text: "回答完成 · 未验证", tone: "muted", detail: status === "accepted" ? "宿主状态为 accepted，但验收记录未返回" : "回答结束不代表产物已验收" };
    }
    return { text: "空闲", tone: "muted" };
  }

  function computeFacts(source, streamState) {
    const facts = {
      backend: backendFromDiagnostics(global.__owoCoreDiagnostics, streamState),
      workspace: workspaceFromFacts(source),
      model: modelFromCache(source),
      permission: permissionFromCache(source),
      task: taskFromFacts(source),
    };
    return SEGMENTS.map((segment) =>
      Object.assign({}, segment, facts[segment.key] || { text: "—", tone: "muted" }),
    );
  }

  function invokeOwner() {
    const internal = global.__TAURI_INTERNALS__;
    const publicCore = global.__TAURI__ && global.__TAURI__.core;
    const owner = publicCore && typeof publicCore.invoke === "function" ? publicCore : internal;
    return owner && typeof owner.invoke === "function" ? owner : null;
  }

  function invokeShell(owner, command) {
    return Promise.resolve(owner.invoke(command, {}));
  }

  function refreshProviderCache(force) {
    if (cache.provider.pending) return;
    const age = Date.now() - cache.provider.at;
    if (!force && cache.provider.value && age < PROVIDER_TTL_MS) return;
    const owner = invokeOwner();
    if (!owner) {
      cache.provider.at = Date.now();
      cache.provider.value = { error: "非桌面壳（浏览器直连时模型段由设置页提供）" };
      return;
    }
    cache.provider.pending = true;
    invokeShell(owner, "get_provider_status").then(
      (result) => {
        cache.provider.pending = false;
        cache.provider.at = Date.now();
        cache.provider.value = result && typeof result === "object" ? result : { error: "壳未返回提供商状态" };
        repaint();
      },
      (error) => {
        cache.provider.pending = false;
        cache.provider.at = Date.now();
        cache.provider.value = { error: String((error && error.message) || error) };
        repaint();
      },
    );
  }

  /** 工作区段壳侧水合：get_workspace 是权威源，localStorage 只是用户刚改过的即时反馈。 */
  function refreshWorkspaceCache(force) {
    if (cache.workspace.pending) return;
    const age = Date.now() - cache.workspace.at;
    if (!force && age < WORKSPACE_TTL_MS) return;
    const owner = invokeOwner();
    if (!owner) {
      cache.workspace.at = Date.now();
      return;
    }
    cache.workspace.pending = true;
    invokeShell(owner, "get_workspace").then(
      (result) => {
        cache.workspace.pending = false;
        cache.workspace.at = Date.now();
        const root = result && result.configured === false ? "" : result && typeof result.workspace === "string" ? result.workspace : "";
        // 壳报空也要清缓存：引导页里用户撤销/换目录后，状态条不得停留在旧值上。
        cache.workspace.root = root;
        repaint();
      },
      () => {
        cache.workspace.pending = false;
        cache.workspace.at = Date.now();
        repaint();
      },
    );
  }

  function uiHidden() {
    if (typeof global.document === "undefined" || !global.document.documentElement) return false;
    if (global.document.visibilityState === "hidden") return true;
    if (typeof global.owoWindowHidden === "function") return Boolean(global.owoWindowHidden());
    return Boolean(global.__owoWindowHidden);
  }

  let host = null;
  let source = null;
  let onRefresh = null;
  let timer = 0;
  let lastSerialized = "";

  function paint(list) {
    const serialized = list.map((item) => item.key + ":" + item.text + ":" + item.tone + ":" + (item.secondary || "") + ":" + (item.detail || "")).join("|");
    if (serialized === lastSerialized) return;
    lastSerialized = serialized;
    for (const item of list) {
      const node = host.querySelector('[data-owo-status="' + item.key + '"]');
      if (!node) continue;
      const value = node.querySelector(".owo-status-value");
      if (value && value.textContent !== item.text) value.textContent = item.text;
      let secondary = node.querySelector(".owo-status-secondary");
      if (!secondary && item.key === "model") { secondary = global.document.createElement("span"); secondary.className = "owo-status-secondary"; node.appendChild(secondary); }
      if (secondary) { secondary.textContent = item.secondary || ""; secondary.hidden = !item.secondary; }
      node.setAttribute("data-tone", item.tone);
      node.setAttribute("data-target", item.target);
      node.title = item.label + "：" + item.text + "（" + (item.detail ? item.detail + " · " : "") + item.hint + "）";
      node.setAttribute("aria-label", node.title);
    }
  }

  function repaint() {
    if (!host || uiHidden()) return;
    const facts = computeFacts(source ? source() : {}, streamState());
    paint(facts);
    if (onRefresh) onRefresh(facts);
  }

  function streamState() {
    try {
      const snap = typeof global.owoInvalidatorState === "function" ? global.owoInvalidatorState() : null;
      return snap && snap.state ? String(snap.state) : null;
    } catch (error) {
      return null;
    }
  }

  function build() {
    host.innerHTML = "";
    for (const segment of SEGMENTS) {
      const button = global.document.createElement("button");
      button.type = "button";
      button.className = "owo-status-item";
      button.setAttribute("data-owo-status", segment.key);
      button.setAttribute("data-tone", "muted");
      const label = global.document.createElement("span");
      label.className = "owo-status-label";
      label.textContent = segment.label;
      const value = global.document.createElement("span");
      value.className = "owo-status-value";
      value.textContent = "—";
      button.appendChild(label);
      button.appendChild(value);
      button.addEventListener("click", () => {
        if (segment.key === "model" || segment.key === "backend") refreshProviderCache(true);
        if (segment.key === "workspace") refreshWorkspaceCache(true);
        const detail = { key: segment.key, target: segment.target };
        if (typeof global.dispatchEvent === "function" && typeof global.CustomEvent === "function") {
          global.dispatchEvent(new global.CustomEvent("owo:statusbar-navigate", { detail }));
        }
        repaint();
      });
      host.appendChild(button);
    }
  }

  /**
   * 挂载状态条。
   * @param {Element} root   #globalStatusBar 容器
   * @param {Object} options { getFacts: () => ({workspaceRoot, reading, pendingApproval, lastTurnOutcome}) }
   */
  function mount(root, options) {
    if (!root || typeof global.document === "undefined") return;
    host = root;
    source = (options && options.getFacts) || null;
    onRefresh = (options && options.onRefresh) || null;
    build();
    if (!timer && typeof global.setInterval === "function") {
      timer = global.setInterval(() => {
        if (uiHidden()) return;
        refreshProviderCache(false);
        refreshWorkspaceCache(false);
        repaint();
      }, REFRESH_MS);
    }
    refreshProviderCache(true);
    refreshWorkspaceCache(true);
    repaint();
    if (global.document && global.document.body) global.document.body.classList.add("has-global-status-bar");
  }

  function unmount() {
    if (timer && typeof global.clearInterval === "function") global.clearInterval(timer);
    timer = 0;
    host = null;
    source = null;
    onRefresh = null;
    // 缓存必须一起清：验收脚本会在同一进程里连跑多个场景（不同私有工作区），
    // 留着上一轮的工作区/提供商结果就是"看起来成功"的假绿。
    cache.provider = { at: 0, value: null, pending: false };
    cache.workspace = { at: 0, root: "", pending: false };
    cache.permission = { profile: null, pendingApprovals: null, grants: null };
    cache.model = null;
    lastSerialized = "";
  }

  /** 权限中心/设置页把权威档位回灌给状态条（零额外请求）。 */
  function reportPermission(info) {
    cache.permission = Object.assign({}, cache.permission, info || {});
    repaint();
  }

  /**
   * 设置页把 core 实际生效的提供商/模型回灌给模型段（§4.3「provider、模型、连接状态」
   * 的唯一权威源；壳侧 get_provider_status 只是进程配置视图，两者不一致时以 core 为准）。
   */
  function reportModel(info) {
    cache.model = info && typeof info === "object" ? info : null;
    repaint();
  }

  /** 真机验收（§4.10）只读访问器：不改变任何状态。 */
  function snapshotFacts() {
    return computeFacts(source ? source() : {}, streamState());
  }

  global.OwoStatusBar = {
    SEGMENTS: SEGMENTS,
    backendFromDiagnostics: backendFromDiagnostics,
    terminalBackendText: terminalBackendText,
    profileWritable: profileWritable,
    computeFacts: computeFacts,
    reportPermission: reportPermission,
    reportModel: reportModel,
    mount: mount,
    unmount: unmount,
    repaint: repaint,
    snapshotFacts: snapshotFacts,
  };
  global.__owoStatusBarFacts = snapshotFacts;
})(typeof window !== "undefined" ? window : globalThis);
