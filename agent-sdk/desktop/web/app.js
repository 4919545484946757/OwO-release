// OwO Agent 工作台（v0.4 P1 桌面壳，纯静态，直连本地 HTTP API + SSE）
"use strict";

// 侧栏项目分组的折叠状态（存工作区原文，localStorage 持久化）。
// 刻意放在模块级而非 state：它只影响列表渲染，不参与任何业务判定。
const collapsedGroups = new Set(
  (() => {
    try {
      const raw = JSON.parse(localStorage.getItem("owo.collapsedGroups") || "[]");
      return Array.isArray(raw) ? raw.map(String) : [];
    } catch (_) {
      return [];
    }
  })()
);

const state = {
  sessionId: null,
  pendingApproval: null,
  // 跨会话审批队列：request_id → { tool, reason, sessionId }。
  // 多对话并行时每个会话都可能挂起等待审批，单值卡会互相覆盖/不可见。
  pendingApprovals: new Map(),
  // 历史自动放行记录（FIFO），仅供旧回合的步骤展示。用于给对应的工具步骤
  // 打一个「自动放行」小徽标，而不是逐条往对话流里塞系统消息。
  autoAllowed: [],
  // 仍在运行的回合：sessionId → { controller, startedAt }。切走会话不打断任务，
  // 切回时恢复该会话的本地实时视图。
  activeTurns: new Map(),
  // 当前回合待回答的提问（ask_user：回合挂起等待用户答复）
  pendingQuestion: null,
  reading: false,
  attachments: [],
  abortController: null,
  selectionVersion: 0,
  selectedModel: "",
  pendingModelOverride: null,
  modelOutputTokens: {},
  defaultModelOutputTokens: window.OwoModelOutputBudget.DEFAULT,
  // 流式渲染：粘性滚动 + 当前回合的思考块/工具分组句柄
  autoScroll: true,
  thinking: null,
  toolRun: null,
  // 当前回合统计（工具次数/模型调用轮次/耗时），供回合汇报卡使用
  turn: null,
  // 本回合是否已记录过"该模型会推理"（每回合一次，避免高频读 localStorage）
  reasoningNoted: false,
  // 运行状态条（转圈 + 阶段文案 + 计时）：让用户明确感知 agent 正在工作
  runStartedAt: 0,
  runTicker: null,
  lastTurnOutcome: "",
  completedTurnFacts: new Map(),
  lastSessionPrompts: new Map(),
};

const $ = (id) => document.getElementById(id);

// Markdown/LaTeX 渲染器唯一来源：core/markdown.js（须在 app.js 之前加载）。
const {
  escapeHtml,
  escapeAttribute,
  safeMarkdownHref,
  texToReadable,
  extractTex,
  restoreTex,
  inlineMarkdown,
  splitMarkdownTableRow,
  isMarkdownTableSeparator,
  markdownTableAlignment,
  renderMarkdown,
} = window.OwoMarkdown || {};
const ModelRouting = window.OwoModelRouting;
const sessionModelUpdateQueue = ModelRouting.createSessionModelUpdateQueue((sessionId, request) =>
  api("/session/" + encodeURIComponent(sessionId) + "/model", {
    method: "POST",
    body: JSON.stringify(request),
  })
);
function getDefaultModel() {
  const saved = String((state.settings && state.settings.model) || "").trim();
  if (loadCustomModels().some((item) => item.id === saved)) return saved;
  return ModelRouting
    ? ModelRouting.effectiveDefaultModel(state.settings || {})
    : String((state.settings && state.settings.model) || "").trim();
}
function getComposerModel() {
  return state.sessionId
    ? String(state.selectedModel || getDefaultModel())
    : String(state.pendingModelOverride || getDefaultModel());
}
function refreshComposerModelChip() {
  const text = $("modelChipText");
  if (text) text.textContent = getComposerModel() || "默认模型";
  updateModelChipMeta();
  window.OwoStatusBar?.repaint();
}
// 由壳注入核心服务地址；经核心服务同源托管时为空字符串。
// 桌面壳下会在拿到 get_core_connection 后改写成壳的真实端口（壳用 --port 0 随机分配）。
let API_BASE = (window.OWO_API_BASE || "").replace(/\/+$/, "");

// ---------- 桌面壳能力（Electron / Tauri 通用） ----------
//
// 核心服务对「领 token」有两道门（crates/owo-agent-server/src/auth_token.rs）：
//   * pairing_gate_allows —— 需 x-owo-desktop-pairing（壳注入核心的共享秘密）；
//   * instance_gate_allows —— 需 x-owo-desktop-instance（本次桌面实例身份）。
// 两者任一缺失都返回 403 auth/pairing_required | auth/instance_mismatch，
// 表现为界面「服务未连接」+「列表加载失败：Failed to fetch」。
// 浏览器直接开（无壳）时两道门都放行，裸请求即可；桌面壳下必须先把两头补齐。
//
// 认证头的实现**只在 core/api-client.js 一处**（契约测试「业务脚本不绕过统一 API
// 客户端」硬性要求全站仅它可 fetch），下面通过 OwoApi 单例复用，不再各自实现。

// 取壳给出的连接信息（pairing + 实例 id + 可选注入 token）。
//
// 认证头的**唯一实现**在 core/api-client.js —— 契约测试「业务脚本不绕过统一 API 客户端」
// 规定全站只有它可以直接 fetch。本函数只做一件事：把壳的真实端口同步到 API_BASE
// （壳用 --port 0 随机分配，页面若停在旧端口会全线 401）。
//
// 关键：**未就绪时不缓存**。窗口刷新瞬间核心可能仍是 starting，此时返回 null；
// 若把 null 缓存下来，此后永远拿不到 pairing → /auth/token 恒 403 → 界面"服务未连接"。
// 症状是"首次打开正常、刷新后必坏"，极难定位。
async function shellConnection() {
  const client = window.OwoApi;
  if (!client || typeof client.ensureCoreConnection !== "function") return null;
  const connection = await client.ensureCoreConnection();
  if (connection && connection.state === "ready" && connection.port) {
    const url = "http://127.0.0.1:" + connection.port;
    if (API_BASE !== url) {
      API_BASE = url;
      window.OWO_API_BASE = url;
      if (client.baseUrl !== url) client.baseUrl = url;
    }
  }
  return connection;
}

// 领 token：桌面壳下由 ApiClient 补齐 pairing / 实例头（它内部已实现），
// 并在壳已注入 token 时直接复用；浏览器模式则是裸请求。
async function requestApiToken() {
  await shellConnection();
  const client = window.OwoApi;
  const token = await client.bootstrapToken(false);
  return { token, injected: Boolean(client.injectedToken) };
}

// First-use setup gate: only the desktop shell can authoritatively report workspace
// and provider state. Keep this probe IPC-only and bounded so it never adds HTTP to
// cold-start hydration or mistakes a transient "starting" snapshot for missing setup.
const SETUP_GATE_SETTLE_MS = 6000;
const SETUP_GATE_TICK_MS = 200;
async function needsSetup() {
  const apiClient = window.OwoApi;
  const bridge = window.OwoApiClient;
  const owner = bridge && typeof bridge.tauriInvokeOwner === "function"
    ? bridge.tauriInvokeOwner(window)
    : null;
  if (!owner || !apiClient || typeof apiClient.ensureCoreConnection !== "function") return false;

  const deadline = Date.now() + SETUP_GATE_SETTLE_MS;
  let connection = null;
  do {
    const remaining = Math.max(1, deadline - Date.now());
    connection = await Promise.race([
      apiClient.ensureCoreConnection(),
      new Promise((resolve) => setTimeout(() => resolve(null), remaining)),
    ]);
    if (connection && !['starting', 'restarting'].includes(connection.state)) break;
    if (Date.now() >= deadline) return false;
    await new Promise((resolve) => setTimeout(resolve, Math.min(SETUP_GATE_TICK_MS, deadline - Date.now())));
  } while (Date.now() < deadline);

  const errorCode = connection && connection.errorCode;
  if (connection && connection.state === "no_workspace") {
    window.__owoSetupDiagnostics = { workspaceConfigured: false, providerReady: false, errorCode };
    return true;
  }
  if (!connection || !["ready", "failed"].includes(connection.state)) return false;

  try {
    const workspace = await owner.invoke.call(owner, "get_workspace");
    const workspacePath = workspace && (workspace.workspace || workspace.path);
    const hasWorkspacePath = typeof workspacePath === "string" && workspacePath.trim().length > 0;
    const workspaceConfigured = workspace && typeof workspace.configured === "boolean"
      ? workspace.configured && hasWorkspacePath
      : hasWorkspacePath;
    // Electron 的工作区由 shell 持久化；端口变化会改变 WebView origin，不能把
    // localStorage 当作跨重启的权威值。未配置时也清掉旧 origin 下残留的假选择。
    const workspaceInput = $("workspace");
    if (workspaceInput) workspaceInput.value = workspaceConfigured ? workspacePath : "";
    try { syncProjectChip(); } catch (_) { /* optional UI sync must not affect setup gating */ }
    try {
      if (workspaceConfigured) localStorage.setItem("owo.workspace", workspacePath);
      else localStorage.removeItem("owo.workspace");
    } catch (_) { /* storage can be disabled in browser/test contexts */ }
    if (!workspaceConfigured) {
      window.__owoSetupDiagnostics = { workspaceConfigured: false, providerReady: false, errorCode };
      return true;
    }
    if (connection.state === "failed" && errorCode !== "provider/not_configured") return false;
    if (connection.state === "failed") {
      window.__owoSetupDiagnostics = { workspaceConfigured: true, providerReady: false, errorCode };
      return true;
    }

    const provider = await owner.invoke.call(owner, "get_provider_status");
    const providerReady = Boolean(provider && provider.ready);
    let deferredProvider = false;
    try {
      deferredProvider = !providerReady && localStorage.getItem("owo.setup.provider-deferred") === "1";
      if (providerReady) localStorage.removeItem("owo.setup.provider-deferred");
    } catch (_) { /* storage can be disabled in browser/test contexts */ }
    window.__owoSetupDiagnostics = {
      workspaceConfigured,
      providerReady: providerReady || deferredProvider,
      errorCode: providerReady ? null : "provider/not_configured",
    };
    return !workspaceConfigured || (!providerReady && !deferredProvider);
  } catch (_) {
    // Unknown shell state belongs to readiness/error handling; do not infer that a
    // healthy provider is missing from a failed diagnostic query.
    return false;
  }
}

function renderSetupGuide() {
  const root = document.getElementById("setupRoot");
  if (!root) return false;

  root.hidden = false;
  document.body.classList.add("setup-required");

  const showFallback = (message) => {
    root.replaceChildren();
    const card = document.createElement("section");
    card.className = "setup-card service-error";
    card.setAttribute("role", "alert");
    const title = document.createElement("strong");
    title.textContent = "首次配置暂时无法显示";
    const detail = document.createElement("p");
    detail.textContent = message;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "primary";
    retry.textContent = "重新加载";
    retry.addEventListener("click", () => window.location.reload());
    card.append(title, detail, retry);
    root.appendChild(card);
  };

  if (typeof window.renderOwoSetupGuide !== "function") {
    showFallback("配置页面组件未加载，请重新加载工作台。");
    return false;
  }

  try {
    window.renderOwoSetupGuide(root, window.__owoSetupDiagnostics || {}, () => {
      window.OwoApi.resetCoreConnection();
      window.location.reload();
    });
    return true;
  } catch (_) {
    showFallback("配置页面组件启动失败，请重新加载工作台。");
    return false;
  }
}

let recognition = null;
let listening = false;
let localRecorder = null;

function initSpeech() {
  const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SpeechRecognition) return;
  recognition = new SpeechRecognition();
  recognition.lang = "zh-CN";
  recognition.continuous = false;
  recognition.interimResults = false;
  recognition.onresult = (event) => {
    const text = event.results[0][0].transcript;
    const prompt = $("prompt");
    prompt.value = (prompt.value ? prompt.value + " " : "") + text;
  };
  recognition.onend = () => {
    listening = false;
    setMicRecording(false);
  };
}

function encodeWav(samples, sampleRate) {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const writeString = (offset, str) => {
    for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i));
  };
  writeString(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  writeString(8, "WAVE");
  writeString(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeString(36, "data");
  view.setUint32(40, samples.length * 2, true);
  let offset = 44;
  for (const sample of samples) {
    const clamped = Math.max(-1, Math.min(1, sample));
    view.setInt16(offset, clamped * 0x7fff, true);
    offset += 2;
  }
  return new Blob([buffer], { type: "audio/wav" });
}

// 本地优先语音输入：麦克风 → 16k WAV → /stt/transcribe（SenseVoice-Small）。
// 本地 STT 不可用时回退到系统 Web Speech。
async function startLocalRecording() {
  const AudioCtx = window.AudioContext || window.webkitAudioContext;
  if (!AudioCtx) return false;
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (_) {
    return false;
  }
  const context = new AudioCtx({ sampleRate: 16000 });
  const source = context.createMediaStreamSource(stream);
  const processor = context.createScriptProcessor(4096, 1, 1);
  const chunks = [];
  source.connect(processor);
  processor.connect(context.destination);
  processor.onaudioprocess = (event) => {
    chunks.push(new Float32Array(event.inputBuffer.getChannelData(0)));
  };
  localRecorder = {
    stop: async () => {
      source.disconnect();
      processor.disconnect();
      stream.getTracks().forEach((track) => track.stop());
      const sampleRate = context.sampleRate;
      await context.close();
      const samples = [];
      for (const chunk of chunks) samples.push(...chunk);
      return { blob: encodeWav(samples, sampleRate), sampleCount: samples.length };
    },
  };
  return true;
}

// ---------- R7 X03：本地 API bearer token（/auth/token 公开引导配对） ----------

let connectionUnavailableUntil = 0;
let authorizationUnavailable = false;
let shellBackgroundHidden = false;
let shellHydrated = false;
let invalidator = null;
let invalidationKey = "";
let workbenchRefresh = null;

function uiHidden() {
  return shellBackgroundHidden || document.hidden || document.visibilityState === "hidden";
}

window.owoSetBackground = function (hidden) {
  shellBackgroundHidden = Boolean(hidden);
  document.body.classList.toggle("desktop-background", shellBackgroundHidden);
  window.OwoInvalidation?.setShellBackground(shellBackgroundHidden);
  if (!shellBackgroundHidden) {
    invalidator?.onVisibility();
    serviceWatch.onVisibilityChange();
    workbenchRefresh?.wake();
  }
};
window.owoSetShellBackground = window.owoSetBackground;

function markConnectionUnavailable(error) {
  const status = Number(error && error.status);
  window.OwoCoreActionAvailability?.update(false);
  if (typeof updatePresetApplyAvailability === "function") updatePresetApplyAvailability();
  if (status === 401 || status === 403) {
    // A token/pairing rejection proves the HTTP service answered. Keep reachability
    // green, but hold a distinct authorization warning until an authenticated call works.
    authorizationUnavailable = true;
    markConnectionReady(false);
    return;
  }
  authorizationUnavailable = false;
  stopInvalidation();
  // 启动中的桌面壳会在同一时刻加载二十多个面板。短暂断连时只允许
  // 一次探测，避免每个面板都向 /auth/token 发请求并刷满控制台。
  connectionUnavailableUntil = Date.now() + 5000;
  const health = $("health");
  if (health) {
    health.textContent = "本地服务未连接";
    health.style.color = "var(--yellow)";
  }
  const bar = $("menubarHealth");
  if (bar) {
    bar.textContent = "服务未连接";
    bar.style.color = "var(--yellow)";
    bar.style.background = "var(--yellow-soft)";
    bar.style.borderColor = "transparent";
  }
  const summary = $("connectionSummary");
  if (summary) {
    summary.textContent = "本地服务未连接";
    summary.classList.remove("auth-unavailable");
    summary.classList.add("offline");
  }
  // 交给就绪探针低频确认：真断连才亮横幅，单次抖动不打扰用户。
  serviceWatch.notifyOffline();
}

function markConnectionReady(authenticated) {
  if (authenticated !== false) authorizationUnavailable = false;
  connectionUnavailableUntil = 0;
  const authFailed = authorizationUnavailable;
  window.OwoCoreActionAvailability?.update(authenticated !== false && !authFailed);
  if (typeof updatePresetApplyAvailability === "function") updatePresetApplyAvailability();
  const health = $("health");
  if (health) {
    health.textContent = authFailed ? "本地服务在线，桌面授权失败" : "本地服务已连接";
    health.style.color = authFailed ? "var(--yellow)" : "var(--green)";
  }
  const bar = $("menubarHealth");
  if (bar) {
    bar.textContent = authFailed ? "授权异常" : "服务已连接";
    bar.style.color = authFailed ? "var(--yellow)" : "var(--green)";
    bar.style.background = authFailed ? "var(--yellow-soft)" : "var(--green-soft)";
    bar.style.borderColor = "transparent";
  }
  const summary = $("connectionSummary");
  if (summary) {
    summary.textContent = authFailed ? "服务在线 · 桌面授权失败" : "服务已连接";
    summary.classList.remove("offline");
    if (authFailed) summary.classList.add("auth-unavailable");
    else summary.classList.remove("auth-unavailable");
  }
  serviceWatch.markOnline();
  if (shellHydrated && !authFailed) startInvalidation();
}

// ---------- 服务就绪探针与故障横幅 ----------
// 启动时以 100→800ms 指数退避在约 10s 窗口内静默等待核心服务（/health 为公开接口，
// 不经过 token 引导）；窗口耗尽或运行中断连后切到低频重试（3s→6s→12s，封顶 15s）
// 并显示非阻塞横幅。恢复后横幅自动消失，并沿用既有 markConnection* 文案通道。
// 不做秒级轮询：页面隐藏时停发请求，转回可见时立即补一次探测。

const serviceWatch = (() => {
  const STARTUP_DEADLINE_MS = 10000;
  const RETRY_BASE_MS = 3000;
  const RETRY_MAX_MS = 15000;
  let phase = "idle"; // idle | startup | offline | online
  let attempts = 0;
  let startupDeadline = 0;
  let startupDelay = 100;
  let retryDelay = RETRY_BASE_MS;
  let timer = null;
  let probeInFlight = null;
  let probeEpoch = 0;

  async function probeOnce() {
    const response = await window.OwoApi.request("/health", { public: true, cache: "no-store", responseType: "response" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json().catch(() => null);
    if (!data || data.healthy !== true) throw new Error("核心服务未报告 healthy");
    return data;
  }

  function clearTimer() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function showBanner() {
    const text = $("serviceBannerText");
    const bridge = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
    if (text) {
      text.textContent = typeof bridge === "function"
        ? "本地核心未连接，正在后台低频重试。服务启动后会自动恢复；也可点“立即重试”。"
        : "浏览器预览未连接本地核心。完整会话请在 OwO Agent Electron 工作台中使用；服务已启动时可点“立即重试”。";
    }
    const banner = $("serviceBanner");
    if (banner) banner.classList.remove("hidden");
  }

  function hideBanner() {
    const banner = $("serviceBanner");
    if (banner) banner.classList.add("hidden");
  }

  function schedule(delay) {
    clearTimer();
    if (uiHidden()) return; // 浏览器隐藏或桌面壳收起时暂停探测
    timer = setTimeout(tick, delay);
  }

  function markOnline() {
    probeEpoch += 1; // Fence any health probe that started before a successful API request.
    phase = "online";
    attempts = 0;
    retryDelay = RETRY_BASE_MS;
    clearTimer();
    hideBanner();
  }

  async function tick() {
    timer = null;
    if (probeInFlight) return probeInFlight;
    attempts += 1;
    const owner = probeEpoch;
    const run = (async () => {
      try {
        await probeOnce();
        if (owner !== probeEpoch) return phase === "online";
        markConnectionReady(false); // 公开健康探测只证明服务可达，不代表桌面授权已通过
        return true;
      } catch (error) {
        // An ordinary API request may have proved the service healthy while this probe
        // was in flight. Ignore its older failure instead of taking the app offline again.
        if (owner !== probeEpoch) return phase === "online";
        markConnectionUnavailable();
        if (phase === "startup" && Date.now() < startupDeadline) {
          // 启动窗口内：静默快速重试，暂不打扰用户。
          const delay = startupDelay;
          startupDelay = Math.min(startupDelay * 2, 800);
          schedule(delay);
          return false;
        }
        phase = "offline";
        showBanner();
        schedule(retryDelay);
        retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
        return false;
      }
    })();
    probeInFlight = run;
    try {
      return await run;
    } finally {
      if (probeInFlight === run) probeInFlight = null;
    }
  }

  function start() {
    if (phase === "online") return Promise.resolve(true);
    if (timer) return Promise.resolve(false);
    phase = "startup";
    attempts = 0;
    startupDelay = 100;
    startupDeadline = Date.now() + STARTUP_DEADLINE_MS;
    return tick();
  }

  function notifyOffline() {
    if (phase === "startup" || phase === "offline") return; // 已在探测或重试
    probeEpoch += 1;
    phase = "offline";
    retryDelay = RETRY_BASE_MS;
    schedule(RETRY_BASE_MS); // 先确认再亮横幅，避免单次抖动误报
  }

  const retryBtn = $("serviceBannerRetry");
  if (retryBtn) {
    retryBtn.addEventListener("click", () => {
      clearTimer();
      tick();
    });
  }
  function onVisibilityChange() {
    if (!uiHidden() && phase !== "online" && !timer) tick();
  }
  document.addEventListener("visibilitychange", onVisibilityChange);

  return { start, notifyOffline, markOnline, onVisibilityChange };
})();

async function ensureApiToken() {
  try {
    const result = await requestApiToken();
    markConnectionReady();
    return result.token;
  } catch (error) {
    markConnectionUnavailable(error);
    throw error;
  }
}

// One authentication/connection lifecycle for chat, panels, uploads and downloads.
async function api(path, options = {}) {
  try {
    const result = await window.OwoApi.request(path, options);
    markConnectionReady();
    return result;
  } catch (error) {
    if (!error.status) markConnectionUnavailable(error);
    throw error;
  }
}

async function apiRaw(path, options = {}) {
  try {
    const response = await window.OwoApi.request(path, { ...options, responseType: "response" });
    markConnectionReady();
    return response;
  } catch (error) {
    if (!error.status) markConnectionUnavailable(error);
    throw error;
  }
}

// 统一友好错误：404/405/5xx 提示"服务接口不可用"，其余透传原错误。
// resource=true 时（契约资源型 404 路径），404/405 提示"资源不存在"而非接口不可用。
function friendlyError(error, options = {}) {
  const msg = String((error && error.message) || error || "");
  const match = msg.match(/^(\d{3}):/);
  const status = match ? Number(match[1]) : 0;
  if (status === 404 || status === 405 || status >= 500) {
    return options.resource ? `资源不存在（HTTP ${status}）` : `服务接口不可用（HTTP ${status}）`;
  }
  return msg || "未知错误";
}

// ---------- 粘性滚动：用户上滑后不被强行拉底 ----------

function messagesAtBottom() {
  const box = $("messages");
  if (!box) return true;
  return box.scrollHeight - box.scrollTop - box.clientHeight < 40;
}

// 仅在「跟随底部」时滚动；force 用于用户主动点回到底部。
function followScroll(force = false) {
  const box = $("messages");
  if (!box) return;
  if (!force && !state.autoScroll) return;
  box.scrollTop = box.scrollHeight;
}

function updateScrollBottomBtn() {
  const button = $("scrollBottomBtn");
  if (!button) return;
  button.classList.toggle("hidden", state.autoScroll);
}

function initChatScroll() {
  const box = $("messages");
  if (box) {
    box.addEventListener("scroll", () => {
      state.autoScroll = messagesAtBottom();
      updateScrollBottomBtn();
    });
  }
  const button = $("scrollBottomBtn");
  if (button) {
    button.addEventListener("click", () => {
      state.autoScroll = true;
      followScroll(true);
      updateScrollBottomBtn();
    });
  }
}

// ---------- 思考流与工具计数：思考过程常显，工具调用只汇报次数 ----------

// ---------- 每会话独立消息视图：多对话并行时各自的输出留在自己的容器里 ----------
// 流写入目标是「发起回合的会话」（writeTargetSid）：切走会话任务继续跑且不串台；
// 未结束回合的消息在服务端 commit 前只存在于本地视图，切回时原样恢复。
let writeTargetSid = null;

function sessionView(sessionId) {
  const sid = sessionId || state.sessionId;
  if (!sid) return $("messages");
  let view = $("messages").querySelector(`.session-view[data-sid="${sid}"]`);
  if (!view) {
    view = document.createElement("div");
    view.className = "session-view";
    view.dataset.sid = sid;
    $("messages").appendChild(view);
  }
  return view;
}

function writeBox() {
  return sessionView(writeTargetSid || state.sessionId);
}

function showSessionView(sessionId) {
  $("messages").querySelectorAll(".session-view").forEach((view) => {
    view.style.display = view.dataset.sid === sessionId ? "" : "none";
  });
}

function syncSessionWelcome() {
  const empty = $("emptyState");
  if (!empty) return;
  const view = state.sessionId ? $("messages").querySelector(`.session-view[data-sid="${state.sessionId}"]`) : null;
  const work = view?.querySelector(".msg.user,.msg.assistant,.msg.error,.event-chip,.tool-steps,.thinking-block,.turn-summary,.historical-completion");
  empty.classList.toggle("hidden", Boolean(currentTurn() || work));
}

function newMessageBlock(node) {
  writeBox().appendChild(node);
  followScroll();
  const empty = $("emptyState");
  if (empty) empty.classList.add("hidden");
  return node;
}

// 思考块渲染约束：实测单回合推理可超 10 万字，整段灌 DOM 会拖垮页面、
// 也让用户误以为「只有思考没有结果」。超出上限的部分只计数不渲染；
// 增量按 120ms 批量落 DOM，避免每个 delta 一次重排 + 滚动。
const THINKING_RENDER_LIMIT = 30000;
const THINKING_FLUSH_MS = 120;

// 思考过程块：**默认折叠**——它是过程不是结论，展开着会把回答挤到屏幕外
// （实测单回合推理可超 10 万字）。摘要行给出发起/字数线索，点开才看内容。
function ensureThinking() {
  if (state.thinking) return state.thinking;
  const el = document.createElement("details");
  el.className = "thinking-block";
  const summary = document.createElement("summary");
  summary.innerHTML =
    '<span class="thinking-label">思考过程</span>' +
    '<span class="thinking-hint">思考中…</span>' +
    '<span class="thinking-peek hidden"></span>' +
    '<span class="thinking-chev" aria-hidden="true">⌄</span>';
  const body = document.createElement("div");
  body.className = "thinking-body";
  el.append(summary, body);
  el.classList.add("is-live");
  newMessageBlock(el);
  state.thinking = { el, summary, body, chars: 0, pending: "", flushTimer: null };
  return state.thinking;
}

function pushReasoning(delta) {
  if (!delta) return;
  // 运行时学习：这个模型真的会推理事后才知道（deepseek-flash 名字上看不出来）。
  // 只在每个回合记一次，避免每个增量都读 localStorage。
  if (!state.reasoningNoted) {
    state.reasoningNoted = true;
    rememberReasoningModel(
      state.selectedModel || getDefaultModel()
    );
  }
  // 推理与工具调用会交替出现（think → act → think → act）。**不**在这里另起一段
  // 工具时间线：那会让一个几十步的回合变成几十枚一行胶囊。工具步骤持续归入同一段，
  // 段落位置由 pushToolUse 保持在最末（见那里的说明）。
  const block = ensureThinking();
  block.chars += delta.length;
  if (block.chars > THINKING_RENDER_LIMIT) return;
  block.pending += delta;
  if (block.flushTimer) return;
  block.flushTimer = setTimeout(() => {
    block.flushTimer = null;
    if (!block.pending) return;
    block.body.appendChild(document.createTextNode(block.pending));
    block.pending = "";
    updateThinkingPeek(block);
    followScroll();
  }, THINKING_FLUSH_MS);
}

/// 折叠状态下给一截"正在想什么"的实时预览：深度思考的价值在于能看到模型在想什么，
/// 全折叠等于看不见；一行截断的尾巴既不打断阅读又能感知进度。
function updateThinkingPeek(block) {
  if (!block || !block.summary) return;
  const peek = block.summary.querySelector(".thinking-peek");
  if (!peek) return;
  const text = (block.body && block.body.textContent) || "";
  const tail = text.replace(/\s+/g, " ").trim().slice(-56);
  peek.textContent = tail ? `「${tail}」` : "";
  peek.classList.toggle("hidden", !tail);
}

// 思考结束：不再自动折叠，只把提示更新为字数（点摘要可手动折叠；
// 超限时注明仅展示前 N 字，避免用户误以为思考被截断丢失）。
function finishThinking() {
  const block = state.thinking;
  if (!block) return;
  if (block.flushTimer) {
    clearTimeout(block.flushTimer);
    block.flushTimer = null;
  }
  if (block.pending) {
    block.body.appendChild(document.createTextNode(block.pending));
    block.pending = "";
  }
  block.el.classList.remove("is-live");
  const hint = block.summary.querySelector(".thinking-hint");
  if (hint) {
    hint.textContent =
      block.chars > THINKING_RENDER_LIMIT
        ? `（${block.chars} 字，仅展示前 ${THINKING_RENDER_LIMIT} 字）`
        : `（${block.chars} 字）`;
  }
  // 思考结束：撤掉实时预览（点开折叠块看全文即可）。
  const peek = block.summary.querySelector(".thinking-peek");
  if (peek) {
    peek.textContent = "";
    peek.classList.add("hidden");
  }
  state.thinking = null;
}

// ---------- 工具步骤时间线：每次调用一枚可展开的 chip（对标 Codex 逐步可追溯） ----------
// 旧实现只汇报「已执行 N 个工具」，用户无法复核 agent 到底做了什么。现在每步落一枚 chip：
// 工具名 + 一行入参摘要 + 状态点，展开可见完整入参与结果预览（服务端 preview）或失败原因。
// 历史回放复用同一套渲染，不再出现「已折叠未显示」。

const TOOL_LABELS = {
  read_file: "读取文件",
  write_file: "写入文件",
  edit_file: "编辑文件",
  multi_edit: "批量编辑",
  list_dir: "列出目录",
  search_files: "搜索文件",
  grep: "搜索内容",
  run_command: "运行命令",
  explore: "探索代码库",
  subagent: "子代理",
  use_skill: "调用技能",
  ask_user: "询问用户",
  web_search: "网络搜索",
  web_fetch: "抓取网页",
  git_status: "仓库状态",
  git_diff: "查看改动",
  git_log: "提交历史",
  fan_out_subagents: "并行子代理",
  update_plan: "更新计划",
  clipboard_read: "读剪贴板",
  clipboard_write: "写剪贴板",
  screen_ocr: "屏幕识别",
  screenshot: "截屏",
};

function toolLabel(tool) {
  const name = String(tool || "").trim();
  if (!name) return "工具";
  if (TOOL_LABELS[name]) return TOOL_LABELS[name];
  // 审查 N5：MCP 工具注册名是命名空间 `mcp__{server}__{tool}`（防外部 server 顶替
  // 内置工具名）。展示层去掉命名空间前缀，按「MCP · 服务器 · 工具」分层，避免
  // 审批卡/时间线出现 `mcp__owo_plugin_translate__translate` 这种噪声。
  if (name.startsWith("mcp__")) {
    const rest = name.slice(5);
    const split = rest.indexOf("__");
    if (split > 0) {
      const server = rest.slice(0, split);
      const remote = rest.slice(split + 2);
      return remote && remote !== server ? `MCP · ${server} · ${remote}` : `MCP · ${server}`;
    }
    return `MCP · ${rest}`;
  }
  // 旧命名（历史会话里仍会出现，保持可读）。
  if (name.startsWith("mcp_")) return `MCP · ${name.slice(4)}`;
  if (name.startsWith("owo_plugin_")) return `插件 · ${name.slice(11)}`;
  return name;
}

/// 入参摘要：一行说清这一步在做什么（Codex 的 chip 也是这个信息密度）。
function toolStepSummary(tool, args) {
  const input = args && typeof args === "object" ? args : {};
  const pick = (...keys) => {
    for (const key of keys) {
      const value = input[key];
      if (typeof value === "string" && value.trim()) return value.trim();
    }
    return "";
  };
  const joined = (...parts) => parts.filter(Boolean).join(" · ");
  switch (tool) {
    case "run_command":
      return pick("command", "cmd", "script");
    case "read_file":
    case "write_file":
    case "edit_file":
    case "multi_edit":
      return pick("path", "file_path");
    case "list_dir":
      return pick("path", "dir");
    case "search_files":
    case "grep":
      return joined(pick("pattern", "query"), pick("path", "dir"));
    case "explore":
    case "subagent":
      return pick("task", "prompt", "query");
    case "use_skill":
      return pick("name", "skill");
    case "ask_user":
      return pick("question", "prompt");
    case "web_search":
      return pick("query");
    case "web_fetch":
      return pick("url", "urls");
    case "git_status":
      return pick("path");
    case "git_diff":
      return joined(pick("path"), input.staged ? "暂存区" : "");
    case "git_log":
      return joined(pick("path"), input.limit ? `最近 ${input.limit} 条` : "");
    case "fan_out_subagents": {
      const tasks = Array.isArray(input.tasks) ? input.tasks.length : 0;
      return tasks ? `${tasks} 个并行子任务` : "";
    }
    default: {
      const first = Object.values(input).find(
        (value) => typeof value === "string" && value.trim()
      );
      return first ? String(first).trim() : "";
    }
  }
}

const TOOL_PREVIEW_RENDER_LIMIT = 4000;

/// 结果预览：历史记录里的工具输出可能到 5 万字，落 DOM 前必须截断。
function clipToolPreview(text) {
  const raw = String(text == null ? "" : text);
  if (raw.length <= TOOL_PREVIEW_RENDER_LIMIT) return raw;
  return `${raw.slice(0, TOOL_PREVIEW_RENDER_LIMIT)}\n…[预览已截断，完整内容见会话记录]`;
}

/// 可点击打开的路径：只有文件/目录类工具给「打开」语义（命令、搜索模式不在此列）。
const FILE_TOOLS = new Set([
  "read_file",
  "write_file",
  "edit_file",
  "multi_edit",
  "list_dir",
]);

function toolStepPath(tool, args) {
  if (!FILE_TOOLS.has(String(tool || ""))) return "";
  const input = args && typeof args === "object" ? args : {};
  const value = input.path || input.file_path || input.dir || "";
  return typeof value === "string" ? value.trim() : "";
}

/// 点击路径 → 交本机服务在工作区内打开（越界路径服务端直接 403，不执行任何命令）。
/// 打开方式取本机偏好「默认文件打开方式」；所选程序不可用时后端回落系统默认并如实回报。
async function openWorkspacePath(path, line) {
  const target = String(path || "").trim();
  if (!target) return;
  const preferred = loadLocalPrefs().fileOpener || "system";
  try {
    const result = await api("/fs/open", {
      method: "POST",
      body: JSON.stringify({ path: target, line: line || null, opener: preferred }),
    });
    const used = (result && result.opener) || preferred;
    const labels = { system: "系统默认程序", notepad: "记事本", vscode: "VS Code" };
    const fallback = used === preferred ? "" : "（所选程序不可用，已回落系统默认）";
    showToast(`已用${labels[used] || used}打开：${target}${fallback}`, "ok");
  } catch (error) {
    showToast(`打开失败：${friendlyError(error, { resource: true })}`, "error");
  }
}

/// 单枚步骤 chip：details 承载「摘要一行 + 展开入参/结果」。
function createToolStep(payload = {}) {
  const row = document.createElement("details");
  row.className = "tool-step is-running";
  if (payload.id) row.dataset.toolId = String(payload.id);
  const summaryText = toolStepSummary(payload.tool, payload.args);
  const filePath = toolStepPath(payload.tool, payload.args);
  const summary = document.createElement("summary");
  const dot = document.createElement("span");
  dot.className = "tool-step-dot";
  dot.setAttribute("aria-hidden", "true");
  const label = document.createElement("span");
  label.className = "tool-step-label";
  label.textContent = toolLabel(payload.tool);
  summary.append(dot, label);
  if (summaryText) {
    if (filePath) {
      // 路径可点：直接交给编辑器打开。点击不冒泡，避免顺带把 chip 展开/收起。
      const link = document.createElement("button");
      link.type = "button";
      link.className = "tool-step-text path-link";
      link.dataset.path = filePath;
      link.title = `在编辑器中打开：${filePath}`;
      link.textContent = summaryText;
      link.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        openWorkspacePath(filePath);
      });
      summary.appendChild(link);
    } else {
      const text = document.createElement("span");
      text.className = "tool-step-text";
      text.title = summaryText;
      text.textContent = summaryText;
      summary.appendChild(text);
    }
  }
  const stateEl = document.createElement("span");
  stateEl.className = "tool-step-state";
  stateEl.textContent = "进行中";
  const chev = document.createElement("span");
  chev.className = "tool-step-chev";
  chev.setAttribute("aria-hidden", "true");
  chev.textContent = "⌄";
  summary.append(stateEl, chev);
  const body = document.createElement("div");
  body.className = "tool-step-body";
  const argsText =
    payload.args && typeof payload.args === "object" && Object.keys(payload.args).length
      ? JSON.stringify(payload.args, null, 2)
      : "";
  if (argsText) {
    const section = document.createElement("div");
    section.className = "tool-step-section";
    const caption = document.createElement("span");
    caption.className = "tool-step-caption";
    caption.textContent = "入参";
    const pre = document.createElement("pre");
    pre.textContent = argsText;
    section.append(caption, pre);
    body.appendChild(section);
  }
  const result = document.createElement("div");
  result.className = "tool-step-section tool-step-result hidden";
  const resultCaption = document.createElement("span");
  resultCaption.className = "tool-step-caption";
  resultCaption.textContent = "结果";
  const resultPre = document.createElement("pre");
  result.append(resultCaption, resultPre);
  body.appendChild(result);
  row.append(summary, body);
  // 结算：成功写结果预览（服务端 preview），失败写原因并自动展开（失败必须一眼可见）。
  // 成功的步骤不再逐条写「完成」——绿点本身就是完成，一个回合几十条"完成"只是噪音。
  const settle = (outcome = {}) => {
    const ok = outcome.ok !== false;
    row.classList.remove("is-running");
    row.classList.add(ok ? "is-ok" : "is-fail");
    stateEl.textContent = ok ? "" : "失败";
    const text = ok ? clipToolPreview(outcome.preview) : String(outcome.error || "未知原因");
    if (text) {
      resultPre.textContent = text;
      result.classList.remove("hidden");
      if (!ok) row.open = true;
    } else {
      result.remove();
    }
    if (!argsText && !text) body.remove();
    return ok;
  };
  return { row, body, result, resultPre, state: stateEl, chev, settle };
}

/// 时间线外壳：整段折叠成一行摘要（默认收起）。
/// 一个回合跑几十步时，逐条平铺会让过程比结论长十倍；过程按需展开即可。
function createToolRunShell() {
  const el = document.createElement("details");
  el.className = "tool-steps";
  const head = document.createElement("summary");
  head.className = "tool-steps-head";
  const icon = document.createElement("span");
  icon.className = "tool-steps-icon";
  icon.setAttribute("aria-hidden", "true");
  const text = document.createElement("span");
  text.className = "tool-steps-text";
  const chev = document.createElement("span");
  chev.className = "tool-steps-chev";
  chev.setAttribute("aria-hidden", "true");
  chev.textContent = "⌄";
  head.append(icon, text, chev);
  const list = document.createElement("div");
  list.className = "tool-step-list";
  el.append(head, list);
  return { el, head, headText: text, list, groups: [] };
}

/// 把一步挂进时间线：**连续同类合并成一组**（「读取文件 ×12」），落单的直接平铺。
/// 只出现一次就套一层折叠是纯噪音，所以升级成组推迟到第二次同类调用时。
function attachToolStep(run, step, label) {
  const last = run.groups[run.groups.length - 1];
  if (!last || last.label !== label) {
    run.list.appendChild(step.row);
    const group = { label, count: 1, first: step, wrap: null, pending: 0, failed: 0 };
    run.groups.push(group);
    return group;
  }
  last.count += 1;
  if (!last.wrap) {
    const wrap = document.createElement("details");
    wrap.className = "tool-group";
    const sum = document.createElement("summary");
    sum.className = "tool-group-head";
    const name = document.createElement("span");
    name.className = "tool-group-name";
    name.textContent = label;
    const count = document.createElement("span");
    count.className = "tool-group-count";
    const stateText = document.createElement("span");
    stateText.className = "tool-group-state";
    const chev = document.createElement("span");
    chev.className = "tool-step-chev";
    chev.setAttribute("aria-hidden", "true");
    chev.textContent = "⌄";
    sum.append(name, count, stateText, chev);
    const body = document.createElement("div");
    body.className = "tool-group-body";
    wrap.append(sum, body);
    run.list.insertBefore(wrap, last.first.row);
    run.list.removeChild(last.first.row);
    body.appendChild(last.first.row);
    last.wrap = wrap;
    last.body = body;
    last.countEl = count;
    last.stateEl = stateText;
  }
  last.body.appendChild(step.row);
  return last;
}

function refreshToolGroup(group) {
  if (!group || !group.wrap) return;
  group.countEl.textContent = `×${group.count}`;
  const parts = [];
  if (group.pending > 0) parts.push(`${group.pending} 个进行中`);
  if (group.failed > 0) parts.push(`${group.failed} 个失败`);
  group.stateEl.textContent = parts.join(" · ");
  group.wrap.classList.toggle("has-failure", group.failed > 0);
}

function ensureToolRun() {
  if (state.toolRun) return state.toolRun;
  const shell = createToolRunShell();
  newMessageBlock(shell.el);
  state.toolRun = {
    ...shell,
    total: 0,
    running: 0,
    failed: 0,
    steps: new Map(),
  };
  updateToolRun(state.toolRun);
  return state.toolRun;
}

function updateToolRun(run) {
  if (!run) return;
  // 只跑了一步就直接报工具名（"已执行 1 个工具" 重复出现等于没说）。
  const single =
    run.total === 1 && run.groups.length === 1 && run.groups[0].count === 1
      ? run.groups[0].label
      : "";
  const parts = [single || `已执行 ${run.total} 个工具`];
  if (run.running > 0) parts.push(`${run.running} 个进行中`);
  if (run.failed > 0) parts.push(`${run.failed} 个失败`);
  const text = parts.join(" · ");
  if (run.headText) run.headText.textContent = text;
  else run.head.textContent = `⚙ ${text}`;
  run.head.classList.toggle("has-failure", run.failed > 0);
}

function pushToolUse(payload) {
  finishThinking();
  const run = ensureToolRun();
  run.total += 1;
  run.running += 1;
  const step = createToolStep(payload);
  // 自动放行标记：把「已自动允许…」从逐条系统消息降级成 chip 上的一个小徽标。
  if (takeAutoAllowed(payload.tool)) {
    const badge = document.createElement("span");
    badge.className = "tool-step-auto";
    badge.textContent = "自动放行";
    badge.title = "当前访问级别下该工具无需逐次确认";
    step.row.querySelector("summary").insertBefore(badge, step.chev);
  }
  const group = attachToolStep(run, step, toolLabel(payload.tool));
  step.group = group;
  group.pending += 1;
  refreshToolGroup(group);
  if (payload.id) run.steps.set(String(payload.id), step);
  if (state.turn) {
    state.turn.tools += 1;
    // 记录本回合改动的文件（diff 接口是会话累计，汇报卡按回合口径过滤）。
    const tool = payload.tool || "";
    const path = payload.args && (payload.args.path || payload.args.file_path);
    if (path && (tool === "write_file" || tool === "edit_file")) {
      state.turn.files.add(String(path));
    }
  }
  updateToolRun(run);
  // 工具段落始终压在时间线末尾：推理块会不断插到它后面，不搬一次的话，
  // 新步骤会被写进"更早的位置"，读起来像顺序错乱。
  const box = run.el.parentNode;
  if (box && box.lastElementChild !== run.el) box.appendChild(run.el);
  followScroll();
}

function pushToolResult(payload) {
  const run = state.toolRun || ensureToolRun();
  const step = payload.id ? run.steps.get(String(payload.id)) : null;
  let ok;
  if (step) {
    ok = step.settle(payload);
    run.running = Math.max(0, run.running - 1);
    const group = step.group;
    if (group) {
      group.pending = Math.max(0, group.pending - 1);
      if (!ok) group.failed += 1;
      refreshToolGroup(group);
      // 失败必须一眼可见：把所在分组一并展开（否则它会藏在两层折叠里）。
      if (!ok && group.wrap) group.wrap.open = true;
    }
  } else {
    // 没有配对 tool_use 的终态（权限被拒 / 插件热卸载 / 断线重连）：补一枚 chip，
    // 否则这一步在前端会永远停在「进行中」。
    run.total += 1;
    const fallback = createToolStep({ id: payload.id, tool: payload.tool });
    const group = attachToolStep(run, fallback, toolLabel(payload.tool));
    fallback.group = group;
    ok = fallback.settle(payload);
    if (!ok) group.failed += 1;
    refreshToolGroup(group);
  }
  if (!ok) {
    run.failed += 1;
    run.el.open = true;
  }
  updateToolRun(run);
  if (ok) return;
  if (state.turn) state.turn.failed += 1;
}

/// 历史回放：把 role=tool 的结果按 tool_call_id 配回 assistant 的 tool_calls，重建成同样的 chip。
/// `existing` 用于**把连续的工具回合并进同一段折叠**——真实会话里就是
/// `assistant(tool_calls) → tool → assistant(tool_calls) → …` 长链（实测 24 步连成一条），
/// 每步单起一个折叠块会让时间线变成一摞只有一行字的胶囊。
function appendHistoryToolSteps(calls, resultsById, existing = null) {
  const run = existing || { ...createToolRunShell(), total: 0, running: 0, failed: 0, steps: new Map() };
  let failed = 0;
  for (const call of calls) {
    const step = createToolStep({ id: call.id, tool: call.name, args: call.arguments });
    const outcome = resultsById.get(String(call.id || ""));
    let ok = true;
    if (outcome) {
      ok = step.settle(outcome);
      if (!ok) failed += 1;
    } else {
      step.row.classList.remove("is-running");
      step.state.textContent = "无结果";
    }
    const group = attachToolStep(run, step, toolLabel(call.name));
    step.group = group;
    if (!ok) group.failed += 1;
    refreshToolGroup(group);
  }
  run.total += calls.length;
  run.failed += failed;
  updateToolRun(run);
  if (failed > 0) run.el.open = true;
  if (!existing) newMessageBlock(run.el);
  return run;
}

function resetRunBlocks() {
  state.thinking = null;
  state.toolRun = null;
  state.reasoningNoted = false;
}

// ---------- 回合汇报卡：每次调用结束后的工作总结（文件改动 + 耗时 + 消耗） ----------

function diffLineStats(diff) {
  const beforeLines = diff.before != null ? String(diff.before).split("\n") : [];
  const afterLines = diff.after != null ? String(diff.after).split("\n") : [];
  const maxLen = Math.max(beforeLines.length, afterLines.length);
  let added = 0;
  let removed = 0;
  for (let index = 0; index < maxLen; index += 1) {
    const before = index < beforeLines.length ? beforeLines[index] : null;
    const after = index < afterLines.length ? afterLines[index] : null;
    if (before === null) added += 1;
    else if (after === null) removed += 1;
    else if (before !== after) {
      removed += 1;
      added += 1;
    }
  }
  return { added, removed };
}

function fillTurnSummaryMeta(turn) {
  const card = turn && turn.card;
  if (!card) return;
  const server = turn.server || {};
  const wall = server.duration_ms
    ? Math.round(server.duration_ms / 1000)
    : Math.max(1, Math.round((Date.now() - turn.startedAt) / 1000));
  const parts = [`耗时 ${wall}s`, `工具 ${turn.tools} 次`];
  if (turn.stopped) parts.push("未完成");
  const completionStatus = server.completion_status;
  if (completionStatus === "candidate") parts.push("代码变更待验收");
  else if (completionStatus === "unverified") parts.push("结果未验证");
  else if (completionStatus === "blocked") parts.push("存在阻断问题");
  else if (completionStatus === "accepted") parts.push("宿主验收通过");
  const modelCallCount = Array.isArray(server.model_calls)
    ? server.model_calls.length
    : turn.modelCalls;
  if (modelCallCount) parts.push(`模型请求 ${modelCallCount} 次`);
  if (server.steps) parts.push(`共 ${server.steps} 步`);
  const reportedUsage = window.OwoCompletionRecord?.usageFromTurnStats(server);
  if (reportedUsage) parts.push(reportedUsage.text);
  else if (server.total_tokens != null && Number.isFinite(Number(server.total_tokens))) {
    const tokens =
      server.total_tokens >= 1000
        ? `${(server.total_tokens / 1000).toFixed(1)}k`
        : String(server.total_tokens);
    parts.push(`消耗 ${tokens} tokens`);
  }
  if (server.cost_usd > 0) parts.push(`$${Number(server.cost_usd).toFixed(4)}`);
  if (!reportedUsage && server.total_tokens == null) parts.push("用量未知");
  const actualModel = window.OwoCompletionRecord?.fromTurnStats(server).lastExecutionModel;
  if (actualModel) parts.push("实际模型 " + actualModel);
  const completionEvidence = window.OwoCompletionRecord?.summarize(server.completion_record);
  if (completionEvidence) parts.push(completionEvidence.compact);
  const meta = card.querySelector(".turn-summary-meta");
  if (meta) meta.textContent = parts.join(" · ");
}

// 路径归一：比较 diff 相对路径与工具入参路径（容忍 ./ 前缀、绝对/相对、分隔符差异）。
function samePath(left, right) {
  const norm = (value) =>
    String(value || "")
      .replace(/\\/g, "/")
      .replace(/^\.\//, "")
      .replace(/\/+$/, "")
      .toLowerCase();
  const a = norm(left);
  const b = norm(right);
  if (!a || !b) return false;
  return a === b || a.endsWith(`/${b}`) || b.endsWith(`/${a}`);
}

// 汇报卡：文件改动为本回合口径（本回合未记录到文件时退回会话累计并标注）。
async function renderTurnSummary(sessionId, turn) {
  if (!turn) return;
  const card = document.createElement("div");
  card.className = "turn-summary";
  card.innerHTML =
    '<div class="turn-summary-head">' +
    '<span class="turn-summary-icon" aria-hidden="true"></span>' +
    '<strong class="turn-summary-files">读取改动…</strong>' +
    '<span class="turn-summary-chev">⌄</span>' +
    '<span class="turn-summary-deltas"></span>' +
    '<button type="button" class="turn-summary-open" title="在检查器中查看改动">↗</button>' +
    "</div>" +
    '<div class="turn-summary-list hidden"></div>' +
    '<div class="turn-summary-meta"></div>';
  writeBox().appendChild(card);
  followScroll();
  turn.card = card;

  const head = card.querySelector(".turn-summary-head");
  const list = card.querySelector(".turn-summary-list");
  head.addEventListener("click", (event) => {
    if (event.target.closest(".turn-summary-open")) return;
    list.classList.toggle("hidden");
  });
  card.querySelector(".turn-summary-open").addEventListener("click", () => {
    if (!document.body.classList.contains("inspector-open")) $("inspectorToggle").click();
    list.classList.remove("hidden");
  });

  fillTurnSummaryMeta(turn);
  const evidence = window.OwoCompletionRecord?.summarize(turn.server?.completion_record);
  if (evidence) {
    const section = document.createElement("div");
    section.className = "turn-summary-evidence";
    const heading = document.createElement("strong");
    heading.textContent = `宿主完成记录 · ${evidence.status}`;
    section.appendChild(heading);
    for (const [label, value] of evidence.details) {
      const row = document.createElement("div");
      const name = document.createElement("span");
      name.textContent = `${label}：`;
      const detail = document.createElement("code");
      detail.textContent = value;
      row.append(name, detail);
      section.appendChild(row);
    }
    list.appendChild(section);
  }
  try {
    const diffs = await api(`/session/${sessionId}/diff`);
    const all = (diffs || []).map((diff) => ({ path: diff.path, ...diffLineStats(diff) }));
    const touched = turn.files || new Set();
    const scoped = touched.size
      ? all.filter((file) => [...touched].some((path) => samePath(file.path, path)))
      : all;
    const cumulative = touched.size === 0 && all.length > 0;
    const addedTotal = scoped.reduce((sum, file) => sum + file.added, 0);
    const removedTotal = scoped.reduce((sum, file) => sum + file.removed, 0);
    card.querySelector(".turn-summary-files").textContent = scoped.length
      ? `${scoped.length} 个文件已更改${cumulative ? "（会话累计）" : ""}`
      : "本次没有文件改动";
    card.querySelector(".turn-summary-deltas").innerHTML =
      (addedTotal ? `<span class="plus">+${addedTotal}</span>` : "") +
      (removedTotal ? `<span class="minus">-${removedTotal}</span>` : "");
    if (scoped.length) {
      // 用 DOM 构建而非拼 HTML：路径里可能出现引号等字符，逐节点赋值才不会破坏结构；
      // 同时让每行路径可点，直接送编辑器打开。
      for (const file of scoped) {
        const row = document.createElement("div");
        row.className = "turn-summary-row";
        const link = document.createElement("button");
        link.type = "button";
        link.className = "path path-link";
        link.dataset.path = file.path;
        link.title = `在编辑器中打开：${file.path}`;
        link.textContent = file.path;
        link.addEventListener("click", (event) => {
          event.stopPropagation();
          openWorkspacePath(file.path);
        });
        const added = document.createElement("span");
        added.className = "plus";
        added.textContent = `+${file.added}`;
        const removed = document.createElement("span");
        removed.className = "minus";
        removed.textContent = `-${file.removed}`;
        row.append(link, added, removed);
        list.appendChild(row);
      }
    }
    if (!scoped.length && !evidence) {
      card.querySelector(".turn-summary-chev").classList.add("hidden");
    }
  } catch (_) {
    card.querySelector(".turn-summary-files").textContent = "改动读取失败";
  }
}

function addMessage(kind, text, meta = "") {
  // 循环防护等「对模型的提醒」不该以对话气泡展示：转成折叠的时间线 chip。
  // （LOOP_NUDGE / LOOP_WRAP_UP / EMPTY_REPLY_RETRY 都以「（系统提示）」开头，
  // 历史里以 user 角色入库，这里统一折叠，不占对话流。）
  if (typeof text === "string" && text.startsWith("（系统提示）")) {
    return addEventChip(
      "loop",
      "循环防护提醒",
      text.replace(/^（系统提示）/, "")
    );
  }
  const div = document.createElement("div");
  div.className = `msg ${kind}`;
  if (meta) {
    const span = document.createElement("span");
    span.className = "meta";
    span.textContent = meta;
    div.appendChild(span);
  }
  if (kind === "assistant" || kind === "user") {
    const body = document.createElement("div");
    body.className = "message-body";
    body.innerHTML = renderMarkdown(text);
    div.appendChild(body);
  } else {
    div.appendChild(document.createTextNode(text));
  }
  bindCopyButtons(div);
  writeBox().appendChild(div);
  followScroll();
  // Administrative notices do not replace the workspace entrance.
  syncSessionWelcome();
  return div;
}

// ---------- 事件 chip：把「中断 / 审批拒绝 / 上下文压缩」写成时间线里的显式事件 ----------
// 对标 Codex：这些状态变化属于回合的一部分，不能只藏在 toast、title 或一行灰字里。

const EVENT_CHIP_ICONS = {
  stop: "■",
  deny: "⊘",
  compact: "⤓",
  fail: "!",
  info: "•",
};

function addEventChip(kind, text, detail = "") {
  const chip = document.createElement("details");
  chip.className = `event-chip kind-${kind}`;
  const summary = document.createElement("summary");
  summary.innerHTML =
    `<span class="event-chip-icon" aria-hidden="true">${EVENT_CHIP_ICONS[kind] || EVENT_CHIP_ICONS.info}</span>` +
    `<span class="event-chip-text">${esc(text)}</span>` +
    (detail ? '<span class="event-chip-chev" aria-hidden="true">⌄</span>' : "");
  chip.appendChild(summary);
  if (detail) {
    const body = document.createElement("div");
    body.className = "event-chip-body";
    body.textContent = detail;
    chip.appendChild(body);
  }
  writeBox().appendChild(chip);
  followScroll();
  const empty = $("emptyState");
  if (empty) empty.classList.add("hidden");
  return chip;
}

function bindCopyButtons(root) {
  for (const button of root.querySelectorAll(".md-copy")) {
    button.setAttribute("aria-label", "复制代码");
    button.addEventListener("click", async () => {
      button.disabled = true;
      try {
        const code = decodeURIComponent(button.dataset.code || "");
        await copyText(code);
        button.textContent = "已复制";
        button.setAttribute("aria-label", "代码已复制");
        button.title = "代码已复制到剪贴板";
      } catch (_) {
        button.textContent = "复制失败";
        button.setAttribute("aria-label", "复制失败，请重试");
        button.title = "剪贴板不可用，请检查系统权限后重试";
      } finally {
        setTimeout(() => {
          button.disabled = false;
          button.textContent = "复制";
          button.setAttribute("aria-label", "复制代码");
          button.title = "";
        }, 1200);
      }
    });
  }
}

function esc(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}


// ---------- 头部状态 ----------

async function refreshHealth() {
  try {
    const health = await api("/health", { public: true });
    $("health").textContent = `API 就绪 ${health.version}`;
    $("health").style.color = "var(--green)";
    const bar = $("menubarHealth");
    if (bar) {
      bar.textContent = "服务已连接";
      bar.style.color = "var(--green)";
      bar.style.background = "var(--green-soft)";
      bar.style.borderColor = "transparent";
    }
  } catch (error) {
    $("health").textContent = "本地服务未连接";
    $("health").style.color = "var(--yellow)";
    const bar = $("menubarHealth");
    if (bar) {
      bar.textContent = "服务未连接";
      bar.style.color = "var(--yellow)";
      bar.style.background = "var(--yellow-soft)";
      bar.style.borderColor = "transparent";
    }
  }
}

async function refreshPerception() {
  if (!$("taskAdvanced").open) return;
  try {
    const snapshot = await api("/context/snapshot");
    const levelLabels = {
      l0_l1: "基础感知（前台与界面）",
      l2_visual: "视觉感知",
      l3_semantic: "语义理解",
    };
    const level = levelLabels[snapshot.permission_level] || "状态未知";
    $("permission").textContent = `感知：${level}`;
    const actions = Array.isArray(snapshot.recent_actions) && snapshot.recent_actions.length
      ? window.OwoTaskDetails.describeActions(snapshot.recent_actions)
      : "暂无近期操作";
    $("snapshot").textContent = `可读取的信息：${level}\n近期活动：${actions}`;
  } catch (_) {
    $("snapshot").textContent = "（无法获取情景快照）";
  }
}

async function refreshLearn() {
  try {
    const status = await api("/learn/status");
    $("learnState").textContent = `学习：${status.state}（${status.samples}）`;
  } catch (_) {
    $("learnState").textContent = "学习：—";
  }
}

async function learnControl(action) {
  try {
    await api(`/learn/${action}`, { method: "POST" });
    await refreshLearn();
  } catch (error) {
    addMessage("error", `学习控制失败：${error.message}`);
  }
}

async function sinkSkill() {
  const name = $("sinkName").value.trim();
  const apps = $("sinkApps").value.split(",").map((item) => item.trim()).filter(Boolean);
  const sensitivity = $("sinkSensitivity").value;
  const description = $("sinkDesc").value.trim();
  if (!name || !apps.length) return;
  try {
    const result = await api("/learn/sink", {
      method: "POST",
      body: JSON.stringify({ name, target_apps: apps, sensitivity, description }),
    });
    $("sinkName").value = "";
    $("sinkDesc").value = "";
    addMessage("system", `已沉淀技能包 ${result.name}（变量：${result.variables.join(", ") || "无"}）`);
    await refreshPackages();
  } catch (error) {
    addMessage("error", `沉淀失败：${error.message}`);
  }
}

// 扩展面板的读取请求有界等待；超时会取消底层 fetch，避免 UI 永久停在加载态。
async function apiWithTimeout(path, timeoutMs = 15000) {
  const controller = new AbortController();
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      controller.abort();
      reject(new Error(`请求超时（${Math.ceil(timeoutMs / 1000)} 秒），可点击刷新重试`));
    }, timeoutMs);
  });
  try {
    // 同时限制认证引导与 HTTP 请求；仅 abort fetch 不会中断 fetch 前的 token 握手。
    return await Promise.race([api(path, { signal: controller.signal }), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

// 插件页（Codex 风格）：已安装网格 + 热门推荐列表。
// 防御：面板元素不存在（视图切换中）时直接跳过，避免竞态报错。
async function refreshPlugins() {
  const grid = $("pluginGrid");
  if (!grid) return;
  try {
    const data = await apiWithTimeout("/plugins");
    const plugins = data.plugins || [];
    const count = $("pluginCount");
    if (count) count.textContent = plugins.length ? `${plugins.length} 个` : "暂无";
    grid.innerHTML = "";
    for (const plugin of plugins) grid.appendChild(pluginCard(plugin));
    if (!plugins.length) grid.innerHTML = '<div class="sub">尚未安装插件</div>';
  } catch (error) {
    const count = $("pluginCount");
    if (count) count.textContent = "加载失败";
    grid.innerHTML = `<div class="sub">加载插件失败：${esc(friendlyError(error))}<br>请点击上方“刷新”重试。</div>`;
  }
}

function pluginCard(plugin) {
  const enabled = plugin.enabled !== false;
  const card = document.createElement("div");
  card.className = "ps-card";
  const initial = esc((plugin.name || plugin.id || "?").trim().slice(0, 1).toUpperCase());
  const mcp = plugin.mcp
    ? `${esc(plugin.mcp.transport)}｜${esc(plugin.mcp.command)}`
    : "未配置 MCP 服务器";
  card.innerHTML =
    '<div class="ps-card-top">' +
    `<span class="ps-tile" aria-hidden="true">${initial}</span>` +
    '<div class="ps-card-meta">' +
    `<strong>${esc(plugin.name)}</strong>` +
    `<span class="sub">v${esc(plugin.version)} ｜ ${esc(plugin.id)}</span>` +
    "</div>" +
    (enabled ? '<span class="ps-check" title="已启用">✓</span>' : "") +
    "</div>" +
    `<p class="ps-card-desc">${esc(plugin.description || "暂无描述")}</p>` +
    '<div class="ps-chips">' +
    `<span class="ps-chip">${enabled ? "已启用" : "已禁用"}</span>` +
    `<span class="ps-chip">${esc((plugin.permissions || []).join("、") || "无权限声明")}</span>` +
    "</div>" +
    '<div class="ps-card-foot">' +
    `<span class="sub" title="${mcp}">${mcp}</span>` +
    `<button type="button" class="ps-card-btn">${enabled ? "禁用" : "启用"}</button>` +
    "</div>";
  card.querySelector("button").addEventListener("click", async () => {
    try {
      await api(`/plugins/${encodeURIComponent(plugin.id)}/enabled`, {
        method: "POST",
        body: JSON.stringify({ enabled: !enabled }),
      });
      showToast(`插件 ${plugin.name} 已${enabled ? "禁用" : "启用"}`);
      await refreshPlugins();
    } catch (error) {
      showToast(`切换失败：${friendlyError(error)}`);
    }
  });
  return card;
}

// 热门推荐：来自市场目录（source=market，尚未本地安装），支持一键安装。
async function refreshPluginMarket() {
  const box = $("pluginPopular");
  if (!box) return;
  try {
    const data = await apiWithTimeout("/plugins/market", 20000);
    const entries = (data.plugins || []).filter((item) => item.source === "market");
    box.innerHTML = "";
    if (!entries.length) {
      box.innerHTML = '<div class="sub">市场目录为空，可在「扩展面板 → 插件市场」中刷新远端目录</div>';
      return;
    }
    for (const entry of entries) {
      const row = document.createElement("div");
      row.className = "ps-row";
      const initial = esc((entry.name || entry.id || "?").trim().slice(0, 1).toUpperCase());
      row.innerHTML =
        `<span class="ps-tile ps-tile-sm" aria-hidden="true">${initial}</span>` +
        '<div class="ps-row-meta">' +
        `<strong>${esc(entry.name || entry.id)}</strong>` +
        `<span class="sub">v${esc(entry.version || "?")} ｜ 最低支持 App ${esc(entry.min_app_version || "—")}</span>` +
        "</div>";
      const button = document.createElement("button");
      button.type = "button";
      button.className = "ps-install";
      button.textContent = "安装";
      button.addEventListener("click", () => {
        confirmModal({
          title: "安装插件",
          message: `从市场安装 ${entry.name || entry.id}${entry.version ? " v" + entry.version : ""}？安装时会进行签名校验。`,
          confirmText: "安装",
          onConfirm: async () => {
            try {
              await api("/plugins/market/install-remote", {
                method: "POST",
                body: JSON.stringify({ id: entry.id, version: entry.version || undefined }),
              });
              showToast(`已安装 ${entry.name || entry.id}`);
              await refreshPluginMarket();
              await refreshPlugins();
            } catch (error) {
              showToast(`安装失败：${friendlyError(error)}`);
            }
          },
        });
      });
      row.appendChild(button);
      box.appendChild(row);
    }
  } catch (error) {
    box.innerHTML = `<div class="sub">加载插件市场失败：${esc(friendlyError(error))}<br>请点击“刷新目录”重试。</div>`;
  }
}

async function refreshPackages() {
  try {
    const packages = await api("/learn/packages");
    const list = $("packageList");
    list.innerHTML = "";
    for (const pkg of packages) {
      const li = document.createElement("li");
      const health = pkg.health || "active";
      li.innerHTML = `<strong>${esc(pkg.name)}</strong><span class="sub">健康：${esc(health)} ｜ 目标：${esc(pkg.target_apps.join(","))} ｜ 变量：${esc(pkg.variables.join(",")) || "无"}</span>`;
      li.addEventListener("click", () => executePackage(pkg));
      if (health === "degraded") {
        const resetBtn = document.createElement("button");
        resetBtn.textContent = "重置健康";
        resetBtn.addEventListener("click", async (event) => {
          event.stopPropagation();
          try {
            await api(`/skills/health/${encodeURIComponent(pkg.name)}/reset`, {
              method: "POST",
            });
            await refreshPackages();
            addMessage("system", `已重置 ${pkg.name} 健康度`);
          } catch (error) {
            addMessage("system", `重置失败：${error.message || error}`);
          }
        });
        li.appendChild(resetBtn);
      }
      const exportBtn = document.createElement("button");
      exportBtn.textContent = "导出";
      exportBtn.addEventListener("click", async (event) => {
        event.stopPropagation();
        await exportPackage(pkg.name);
      });
      li.appendChild(exportBtn);
      const detailBtn = document.createElement("button");
      detailBtn.textContent = "查看";
      detailBtn.addEventListener("click", async (event) => {
        event.stopPropagation();
        try {
          const detail = await api(`/learn/packages/${encodeURIComponent(pkg.name)}`);
          $("packageDetail").textContent = JSON.stringify(detail, null, 2);
        } catch (error) {
          addMessage("system", `查看失败：${friendlyError(error, { resource: true })}`);
        }
      });
      li.appendChild(detailBtn);
      const deleteBtn = document.createElement("button");
      deleteBtn.textContent = "删除";
      deleteBtn.addEventListener("click", async (event) => {
        event.stopPropagation();
        confirmModal({
          title: "删除流程技能包",
          message: `删除后无法恢复，确认删除「${pkg.name}」？`,
          confirmText: "删除",
          kind: "danger",
          onConfirm: async () => {
            try {
              await api(`/learn/packages/${encodeURIComponent(pkg.name)}`, { method: "DELETE" });
              await refreshPackages();
              showToast(`已删除流程技能包 ${pkg.name}`, "ok");
            } catch (error) {
              showToast(`删除失败：${friendlyError(error, { resource: true })}`, "error");
            }
          },
        });
      });
      li.appendChild(deleteBtn);
      list.appendChild(li);
    }
    if (!packages.length) list.innerHTML = '<li class="sub">暂无流程技能包（先录制再沉淀）</li>';
  } catch (error) {
    $("packageList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

async function exportPackage(name) {
  try {
    const response = await apiRaw(`/learn/export/${encodeURIComponent(name)}`);
    if (!response.ok) throw new Error(await response.text());
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${name}.owskill`;
    link.click();
    URL.revokeObjectURL(url);
  } catch (error) {
    addMessage("error", `导出失败：${friendlyError(error, { resource: true })}`);
  }
}

async function importPackage(file) {
  try {
    const response = await apiRaw("/learn/import", {
      method: "POST",
      headers: { "Content-Type": "application/zip" },
      body: file,
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || response.statusText);
    showToast(`已导入技能包 ${result.name}`, "ok");
    await refreshPackages();
  } catch (error) {
    showToast(`导入失败：${error.message}`, "error");
  }
}

// 自动化 UI（表单/任务列表/提醒）已迁入扩展面板 panels/automations.panel.js，
// 此处的零散函数与全局轮询一并移除（面板自带挂载期轮询）。

async function refreshSettings() {
  try {
    const settings = await api("/settings");
    state.settings = settings;
    if (window.OwoStatusBar) {
      const runtimeModel = settings.runtime || {};
      window.OwoStatusBar.reportModel({
        provider: runtimeModel.provider || "",
        model: runtimeModel.model || settings.model || "",
      });
      window.OwoStatusBar.reportPermission({
        profile: settings.permission_profile || settings.permissionProfile || null,
        pendingApprovals: state.pendingApprovals.size,
      });
    }
    // 自定义模型只存本机，服务端回读不携带该字段
    state.settings.custom_models = loadCustomModels();
    const cloudEnabled = !!(settings.egress && settings.egress.cloud_enabled);
    const toggle = $("egressToggle");
    toggle.classList.toggle("on", cloudEnabled);
    toggle.setAttribute("aria-checked", String(cloudEnabled));
    toggle.dataset.enabled = String(cloudEnabled);
    const model = getDefaultModel();
    const modelSelect = $("settingsModel");
    if (modelSelect.querySelector(`option[value="${CSS.escape(model)}"]`)) {
      modelSelect.value = model;
    } else {
      // 当前模型不在候选里（如端点实际在用的模型名不在预设中）：
      // 补一个选项并选中，否则下拉回落 HTML 默认值，
      // 保存模型接入时会把这个错误值写回 settings.json。
      const option = document.createElement("option");
      option.value = model;
      option.textContent = model;
      modelSelect.appendChild(option);
      modelSelect.value = model;
    }
    // 会话覆盖只从具体会话详情恢复，默认值来自当前运行服务商。
    $("modelChipText").textContent = getComposerModel() || "默认模型";
    renderEffortChip();
    $("connectionSummary").textContent = cloudEnabled ? "云端模型已启用" : "云端模型已关闭";
    // 主动建议（个性化）
    const proactive = !!(settings.proactive && settings.proactive.enabled);
    const proactiveToggle = $("prefProactive");
    if (proactiveToggle) {
      proactiveToggle.classList.toggle("on", proactive);
      proactiveToggle.setAttribute("aria-checked", String(proactive));
    }
    syncLocalPrefs();
    renderCustomModels();
    syncProviderModels();
    updateModelChipMeta();
    renderProviderForm();
    await refreshModelOutputSettings();
    updateModelGate();
  } catch (error) {
    const summary = $("connectionSummary");
    if (summary) summary.textContent = friendlyError(error);
    // 输出预算来自 Electron 本机配置，即使 core API 暂时不可达也可读取/修改。
    void refreshModelOutputSettings();
  }
}

// 本地偏好（仅存 localStorage，不进入服务端设置）
const LOCAL_PREFS = {
  fileOpener: "system",
  compact: false,
  theme: "light",
  zoom: 1,
  speechLang: "zh-CN",
  sound: false,
};
function loadLocalPrefs() {
  try {
    Object.assign(LOCAL_PREFS, JSON.parse(localStorage.getItem("owo.prefs") || "{}"));
  } catch (_) {
    // 本地偏好损坏时回退默认值
  }
  return LOCAL_PREFS;
}
function saveLocalPrefs() {
  localStorage.setItem("owo.prefs", JSON.stringify(LOCAL_PREFS));
}
function applyLocalPrefs() {
  document.body.classList.toggle("compact-mode", LOCAL_PREFS.compact);
  if (window.owo?.setZoom) window.owo.setZoom([1,1.25,1.5].includes(Number(LOCAL_PREFS.zoom)) ? Number(LOCAL_PREFS.zoom) : 1).catch(() => {});
  const recognitionLang = LOCAL_PREFS.speechLang;
  if (recognition) recognition.lang = recognitionLang;
  $("defaultWorkspacePath").textContent =
    localStorage.getItem("owo.workspace") || "未设置";
}
function syncLocalPrefs() {
  const prefs = loadLocalPrefs();
  const setSelect = (id, value) => {
    const el = $(id);
    if (el) el.value = value;
  };
  setSelect("prefFileOpener", prefs.fileOpener);
  setSelect("prefSpeechLang", prefs.speechLang);
  setSelect("prefTheme", prefs.theme === "system" ? "system" : (document.body.classList.contains("dark-theme") ? "dark" : "light"));
  setSelect("prefZoom", String(prefs.zoom || 1));
  setToggle("prefCompact", prefs.compact);
  setToggle("prefSound", prefs.sound);
}

// ---------- 桌面桌宠（A8-3：经引擎通道控制桌面端挂件） ----------
// 工作台不再内嵌自己的桌宠（会与桌面端悬浮桌宠叠加成「两只」）；这里的开关
// 直接控制桌面端（LingXi overlay）的桌宠：写期望值 → 桌面端心跳应用 → 回读实际值。

/// 开关按钮状态（`.on` + aria-checked）；本地偏好与引擎驱动的开关共用。
function setToggle(id, value) {
  const el = $(id);
  if (!el) return;
  el.classList.toggle("on", !!value);
  el.setAttribute("aria-checked", String(!!value));
}

let petLinkState = {
  desired: null,
  actual: null,
  desiredAt: 0,
  actualAt: 0,
  overlayOnline: false,
  effective: null,
};

async function refreshPetState() {
  try {
    const data = await api("/desktop/pet");
    petLinkState = {
      desired: typeof data.desired === "boolean" ? data.desired : null,
      actual: typeof data.actual === "boolean" ? data.actual : null,
      desiredAt: Date.parse(data.desired_at || "") || 0,
      actualAt: Date.parse(data.actual_at || "") || 0,
      overlayOnline: !!data.overlay_online,
      effective: null,
    };
  } catch (_) {
    petLinkState = {
      desired: null,
      actual: null,
      desiredAt: 0,
      actualAt: 0,
      overlayOnline: false,
      effective: null,
    };
  }
  // 桌面端在线时取「较新的一端」为准：刚写入期望值时先显示期望，
  // 等桌面端心跳追上来（实际值时间戳更新）再以实际值为准。
  let effective = petLinkState.desired;
  if (petLinkState.overlayOnline && petLinkState.actual !== null) {
    effective =
      petLinkState.desiredAt > petLinkState.actualAt
        ? petLinkState.desired
        : petLinkState.actual;
  }
  petLinkState.effective = effective;
  setToggle("prefPet", effective === true);
  const note = $("petLinkNote");
  if (note) {
    note.textContent = petLinkState.overlayOnline
      ? effective === false
        ? "桌面端在线 · 桌宠已隐藏"
        : "桌面端在线 · 桌宠显示中"
      : "桌面端未运行：开关会在它启动后生效";
  }
}

/// 切换桌面桌宠显隐（写期望值；桌面端约 2.5 秒内跟进）。
async function toggleDesktopPet() {
  await refreshPetState();
  const next = !(petLinkState.effective === true);
  // 乐观更新：先把开关拨过去，网络失败时由 refreshPetState 纠正。
  setToggle("prefPet", next);
  try {
    await api("/desktop/pet", {
      method: "POST",
      body: JSON.stringify({ visible: next }),
    });
    showToast(next ? "已请求显示桌面桌宠" : "已请求隐藏桌面桌宠", "ok");
  } catch (error) {
    showToast(`桌宠切换失败：${friendlyError(error, { resource: true })}`, "error");
  }
  await refreshPetState();
}

// ---------- 设置页交互：分类导航 / 模态框 / 自定义模型 / 本地偏好 ----------

// 左侧分类 ↔ 右侧表单页
function setSettingsTab(name) {
  for (const item of document.querySelectorAll(".settings-nav-item")) {
    item.classList.toggle("active", item.dataset.settingsTab === name);
  }
  for (const tab of document.querySelectorAll(".settings-tab")) {
    tab.hidden = tab.dataset.settingsTab !== name;
  }
}
for (const item of document.querySelectorAll(".settings-nav-item")) {
  item.addEventListener("click", () => setSettingsTab(item.dataset.settingsTab));
}

function syncTaskDetails(statusFacts) {
  if (!window.OwoTaskDetails) return;
  const active = currentTurn(), session = (state.sessionIndex || []).find(item => item.id === state.sessionId);
  const request = active?.prompt || state.lastSessionPrompts.get(state.sessionId) || "";
  const view = state.sessionId ? $("messages").querySelector(`.session-view[data-sid="${state.sessionId}"]`) : null;
  const steps = document.body.classList.contains("inspector-open") && view
    ? Array.from(view.querySelectorAll(".tool-step")).slice(-5).map(row => ({
        label: [row.querySelector(".tool-step-label")?.textContent, row.querySelector(".tool-step-text")?.textContent].filter(Boolean).join(" · "),
        state: row.querySelector(".tool-step-state")?.textContent || (row.classList.contains("is-ok") ? "已完成" : row.classList.contains("is-fail") ? "执行失败" : row.classList.contains("is-running") ? "进行中" : "状态未知"),
      })) : [];
  if (document.body.classList.contains("inspector-open")) {
    const form = $("chatForm")?.getBoundingClientRect?.();
    const approval = !$("approvalBar")?.classList.contains("hidden") ? $("approvalBar")?.getBoundingClientRect?.() : null;
    if (form) {
      const top = approval?.height ? Math.min(form.top, approval.top) : form.top;
      $("right")?.style.setProperty("--task-detail-bottom", Math.max(16, window.innerHeight - top + 12) + "px");
    }
  }
  window.OwoTaskDetails.render(document, {
    statusFacts: Array.isArray(statusFacts) ? statusFacts : window.OwoStatusBar?.snapshotFacts() || [],
    workspace: $("composerProjectName")?.textContent || "个人工作台",
    title: session?.title || "", request, running: Boolean(active), phase: active?.phase,
    pendingApprovals: state.pendingApprovals.size, pendingQuestion: state.pendingQuestion, authFailed: authorizationUnavailable,
    steps, hasFileChanges: Boolean($("diffList")?.querySelector(".diff-item")),
  });
}
function setTaskDetailsVisible(open) {
  document.body.classList.toggle("inspector-open", open);
  $("inspectorToggle").setAttribute("aria-expanded", String(open));
  $("inspectorToggle").setAttribute("aria-label", open ? "关闭任务详情" : "打开任务详情");
  syncTaskDetails();
  if (open) { $("right").querySelector(".task-details-content").scrollTop = 0; $("taskDetailsClose").focus(); }
}
function initGlobalStatusBar() {
  const host = $("globalStatusBar");
  if (!host || !window.OwoStatusBar) return;
  window.OwoStatusBar.mount(host, {
    onRefresh: syncTaskDetails,
    getFacts: () => ({
      workspaceRoot: $("workspace")?.value || localStorage.getItem("owo.workspace") || "",
      reading: Boolean(currentTurn()),
      stopping: currentTurn()?.stopping === true,
      pendingApproval: Boolean(state.pendingApproval) || state.pendingApprovals.size > 0,
      lastTurnOutcome: currentTurn() ? "" : state.completedTurnFacts.get(state.sessionId)?.outcome || "",
      selectedModel: currentTurn()?.requestedModel || getComposerModel(),
      settings: { model: getDefaultModel(), custom_models: loadCustomModels() },
      ...(state.completedTurnFacts.get(state.sessionId) || {}),
    }),
  });
  window.OwoStatusBar.reportPermission({
    profile: state.settings?.permission_profile || state.settings?.permissionProfile || null,
    pendingApprovals: state.pendingApprovals.size,
  });
}

window.addEventListener("owo:statusbar-navigate", (event) => {
  const key = event.detail?.key;
  $("connectionDetails").open = false;
  if (key === "backend") {
    setSettingsPageVisible(true);
    setSettingsTab("general");
  } else if (key === "workspace") {
    setSettingsPageVisible(false);
    openWorkspaceMenu($("sidebarWorkspaceBtn"));
  } else if (key === "model") {
    setSettingsPageVisible(true);
    setSettingsTab("models");
  } else if (key === "permission") {
    openPanelById("permissions");
  } else if (key === "chat") {
    setSettingsPageVisible(false);
  }
});

// 通用模态框（替代 alert / confirm / prompt）
const modalRoot = $("modalRoot");
let modalOnClose = null;
function openModal({ title, body, actions = [], onClose = null }) {
  $("modalTitle").textContent = title || "";
  const bodyEl = $("modalBody");
  bodyEl.innerHTML = "";
  if (typeof body === "string") bodyEl.innerHTML = body;
  else if (body) bodyEl.appendChild(body);
  const foot = $("modalFoot");
  foot.innerHTML = "";
  for (const action of actions) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = action.label;
    if (action.kind) button.className = action.kind;
    button.addEventListener("click", () =>
      action.onClick && action.onClick({ close: closeModal, body: bodyEl })
    );
    foot.appendChild(button);
  }
  modalOnClose = onClose;
  modalRoot.classList.remove("hidden");
  const focusTarget = bodyEl.querySelector("input,select,textarea");
  if (focusTarget) focusTarget.focus();
}
function closeModal() {
  modalRoot.classList.add("hidden");
  const onClose = modalOnClose;
  modalOnClose = null;
  if (onClose) onClose();
}
function confirmModal({ title, message, confirmText = "确定", cancelText = "取消", kind = "", onConfirm, onCancel = null }) {
  // done 标记确认路径，避免「确认后 close()」又被 onClose 当成取消回调一次。
  let done = false;
  openModal({
    title,
    body: `<p class="modal-message">${esc(message)}</p>`,
    actions: [
      { label: cancelText, kind: "ghost", onClick: ({ close }) => close() },
      {
        label: confirmText,
        kind: kind === "danger" ? "danger" : "primary",
        onClick: async ({ close }) => {
          done = true;
          close();
          if (onConfirm) await onConfirm();
        },
      },
    ],
    onClose: () => {
      if (!done && onCancel) onCancel();
    },
  });
}
/// 单行文本输入弹窗：替代 window.prompt（原生弹窗无法样式化、会阻塞渲染）。
function promptModal({ title, label, value = "", placeholder = "", confirmText = "确定", required = false, onConfirm, onCancel = null }) {
  let done = false;
  openModal({
    title,
    body:
      `<p class="modal-message">${esc(label)}</p>` +
      `<input id="modalPromptInput" type="text" aria-label="${esc(title)}" ${required ? "required" : ""} value="${esc(value)}" placeholder="${esc(placeholder)}" />`,
    actions: [
      { label: "取消", kind: "ghost", onClick: ({ close }) => close() },
      {
        label: confirmText,
        kind: "primary",
        onClick: ({ close, body }) => {
          const input = body.querySelector("#modalPromptInput");
          const text = input ? input.value.trim() : "";
          if (required && !text) {
            if (input) {
              input.setCustomValidity("\u8bf7\u8f93\u5165\u5185\u5bb9\u3002");
              input.reportValidity();
              input.focus();
            }
            return;
          }
          done = true;
          close();
          if (onConfirm) onConfirm(text);
        },
      },
    ],
    onClose: () => {
      if (!done && onCancel) onCancel();
    },
  });
  const input = $("modalPromptInput");
  if (input) {
    input.addEventListener("input", () => input.setCustomValidity(""));
    input.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      const text = input.value.trim();
      if (required && !text) {
        input.setCustomValidity("\u8bf7\u8f93\u5165\u5185\u5bb9\u3002");
        input.reportValidity();
        input.focus();
        return;
      }
      done = true;
      closeModal();
      if (onConfirm) onConfirm(text);
    });
  }
}
// Promise 版封装：给「先弹窗、后继续」的线性流程用（取消返回 null / false）。
function askConfirm(options) {
  return new Promise((resolve) => {
    confirmModal({ ...options, onConfirm: () => resolve(true), onCancel: () => resolve(false) });
  });
}
function askText(options) {
  return new Promise((resolve) => {
    promptModal({ ...options, onConfirm: (text) => resolve(text), onCancel: () => resolve(null) });
  });
}
$("modalMask").addEventListener("click", closeModal);
$("modalClose").addEventListener("click", closeModal);
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  const menuWasOpen = Boolean(composerMenuEl);
  const modalWasOpen = !modalRoot.classList.contains("hidden");
  if (menuWasOpen) closeComposerMenu();
  if (modalWasOpen) closeModal();
  const connectionWasOpen = $("connectionDetails")?.open;
  if (connectionWasOpen) { $("connectionDetails").open = false; $("connectionDetails").querySelector("summary")?.focus(); }
  if (!menuWasOpen && !modalWasOpen && !connectionWasOpen && document.body.classList.contains("inspector-open") && !menuAnchor) {
    setTaskDetailsVisible(false); $("inspectorToggle").focus();
  }
  // Close the mobile drawer only when no overlay or dialog consumed Escape.
  if (!menuWasOpen && !modalWasOpen && document.body.classList.contains("mobile-sidebar-open")) {
    setMobileSidebarOpen(false);
    mobileSidebarToggle.focus();
  }
});

// ---------- Composer 交互：模型选择 / 访问级别下拉、发送-中断双态按钮 ----------

const SEND_SVG =
  '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 12.4V4.2M4.6 7.6 8 4.2l3.4 3.4"/></svg>';
const STOP_SVG =
  '<svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><rect x="4" y="4" width="8" height="8" rx="1.4"/></svg>';
const MIC_SVG =
  '<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="2.2" width="4" height="7.2" rx="2"/><path d="M3.8 7.8a4.2 4.2 0 0 0 8.4 0M8 12v1.8"/></svg>';
const MIC_REC_SVG =
  '<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="5"/><circle cx="8" cy="8" r="1.8" fill="currentColor" stroke="none"/></svg>';

/// 当前视图会话的活跃回合（并行回合按会话隔离；本会话未运行则为 null）。
function currentTurn() {
  return (state.sessionId && state.activeTurns.get(state.sessionId)) || null;
}

// 回合进行中：发送钮变为中断钮（Codex 行为）——按「当前会话是否在跑」判定。
// 此前用全局 `state.reading`：A 会话在跑时切到 B，B 的发送钮也显示成停止，
// 点下去会把 A 的回合掐掉（并行对话被打断的根因）。
function updateComposerRunning() {
  const running = !!currentTurn();
  const button = $("sendBtn");
  if (!button) return;
  button.disabled = currentTurn()?.stopping === true;
  button.classList.toggle("is-stop", running);
  button.innerHTML = running ? STOP_SVG : SEND_SVG;
  button.title = running ? "中断当前回合" : "发送";
}
function setMicRecording(on) {
  const button = $("micBtn");
  if (!button) return;
  button.classList.toggle("recording", on);
  button.innerHTML = on ? MIC_REC_SVG : MIC_SVG;
  button.title = on ? "停止录音" : "语音输入";
}
async function abortTurn() {
  const turn = currentTurn(), sessionId = state.sessionId;
  if (!turn || !sessionId || turn.stopping) return;
  if (!turn.turnId) { showToast("缺少回合身份，无法安全取消；请查看宿主状态"); return; }
  turn.stopping = true; updateComposerRunning(); setRunPhase("waiting", "正在请求停止；已产生的变更会保留");
  try {
    const result = await api("/session/" + encodeURIComponent(sessionId) + "/abort", { method: "POST", body: JSON.stringify({ turn_id: turn.turnId }) });
    if (result.state === "already_finished") {
      const detail = await api("/session/" + encodeURIComponent(sessionId));
      restoreSessionFacts(sessionId, detail);
      if (turn.remote && state.activeTurns.get(sessionId) === turn) {
        turn.observer?.stop(); state.activeTurns.delete(sessionId);
        if (detail.active_turn_id) resumeHostTurn(sessionId, detail);
      }
      turn.stopping = false;
      if (state.sessionId === sessionId) { if (!currentTurn()) stopRunStatus(); updateComposerRunning(); window.OwoStatusBar?.repaint(); }
    }
  } catch (error) { turn.stopping = false; updateComposerRunning(); showToast("停止请求未成功：" + friendlyError(error), "error"); }
}

// ---------- 运行状态条：转圈 + 阶段文案 + 计时（阻塞态一眼可见） ----------

const RUN_PHASE_DEFAULT = {
  thinking: "正在思考…",
  reasoning: "深度思考中…",
  answering: "正在生成回答…",
  waiting: "等待你的审批",
  asking: "等待你的回答",
};

function startRunStatus() {
  const el = $("runStatus");
  if (!el) return;
  state.runStartedAt = currentTurn()?.startedAt || Date.now();
  el.classList.remove("hidden", "is-waiting");
  el.classList.add("is-live");
  el.dataset.phase = currentTurn()?.phase || "thinking";
  document.body.dataset.agent = /^(waiting|asking)$/.test(el.dataset.phase) ? "waiting" : "running";
  $("runStatusText").textContent = currentTurn()?.phaseText || RUN_PHASE_DEFAULT[currentTurn()?.phase] || RUN_PHASE_DEFAULT.thinking;
  $("runStatusTimer").textContent = "0s";
  if (state.runTicker) clearInterval(state.runTicker);
  state.runTicker = setInterval(() => {
    const timer = $("runStatusTimer");
    if (!timer) return;
    const seconds = Math.round((Date.now() - state.runStartedAt) / 1000);
    timer.textContent = `${seconds}s`;
  }, 1000);
}

// 阶段切换：文案可覆盖（如带工具名），等待审批为黄色阻塞态。
function setRunPhase(phase, text) {
  const owner = writeTargetSid || state.sessionId;
  const active = state.activeTurns.get(owner);
  if (active) { active.phase = phase; active.phaseText = text || RUN_PHASE_DEFAULT[phase] || phase; }
  if (owner !== state.sessionId) return;
  const el = $("runStatus");
  if (!el || el.classList.contains("hidden")) return;
  el.classList.toggle("is-waiting", phase === "waiting");
  el.dataset.phase = phase;
  document.body.dataset.agent = /^(waiting|asking)$/.test(phase) ? "waiting" : "running";
  const label = $("runStatusText");
  const next = text || RUN_PHASE_DEFAULT[phase];
  if (label && next) label.textContent = next;
}

function stopRunStatus() {
  if (state.runTicker) {
    clearInterval(state.runTicker);
    state.runTicker = null;
  }
  const el = $("runStatus");
  if (el) {
    el.classList.add("hidden");
    el.classList.remove("is-live", "is-waiting");
    delete el.dataset.phase;
  }
  document.body.dataset.agent = state.pendingApprovals && state.pendingApprovals.size ? "waiting" : "idle";
}

function attachTurnFailureActions(failure, reason, presentation) {
  if (presentation && presentation.detail && presentation.detail !== presentation.message) {
    const details = document.createElement("details");
    details.className = "turn-error-details";
    const summary = document.createElement("summary");
    summary.textContent = "技术详情";
    const raw = document.createElement("code");
    raw.textContent = presentation.detail;
    details.append(summary, raw);
    failure.appendChild(details);
  }
  if (window.OwoModelRecovery && window.OwoModelRecovery.shouldOfferOutputBudget(reason)) {
    const action = document.createElement("button");
    action.type = "button";
    action.className = "turn-model-recover turn-output-recover";
    action.textContent = "调整输出上限…";
    action.setAttribute("aria-label", "调整当前模型的输出 Token 上限");
    action.addEventListener("click", () => {
      setSettingsPageVisible(true);
      setSettingsTab("models");
      const model = getComposerModel() || getDefaultModel();
      const modelInput = $("modelOutputModel");
      if (modelInput && model) modelInput.value = model;
      refreshModelOutputSettings().then(() => $("modelOutputLimit")?.focus());
    });
    failure.appendChild(action);
  }
  if (window.OwoModelRecovery && window.OwoModelRecovery.shouldOfferModelSwitch(reason)) {
    const action = document.createElement("button");
    action.type = "button";
    action.className = "turn-model-recover";
    action.textContent = "切换模型…";
    action.setAttribute("aria-label", "为当前会话切换模型");
    action.addEventListener("click", () => openModelMenu());
    failure.appendChild(action);
  }
}

// 历史回放时复用实时失败卡，保留模型切换/输出上限恢复入口。
function addHistoricalTurnFailure(storedText) {
  const prefix = "回合未完成：";
  const stored = String(storedText || "");
  const reason = stored.startsWith(prefix) ? stored.slice(prefix.length) : stored;
  const presentation = window.OwoModelRecovery && window.OwoModelRecovery.summarizeTurnFailure
    ? window.OwoModelRecovery.summarizeTurnFailure(reason) : null;
  const failure = addMessage("error", prefix + ((presentation && presentation.message) || reason));
  failure.appendChild(turnRecoveryActions(state.sessionId, state.lastSessionPrompts?.get(state.sessionId) || "", reason));
  if (presentation) attachTurnFailureActions(failure, reason, presentation);
  return failure;
}

// 回合失败卡：每回合都必须有明确终态（失败原因 + 汇报卡），不能静默停住。
function showTurnFailure(message) {
  finishThinking();
  settlePendingQuestion("aborted");
  const run = state.toolRun;
  if (run && run.running > 0) {
    run.running = 0;
    updateToolRun(run);
  }
  const reason = String(message || "").replace(/^gateway error:\s*/i, "");
  // 工具预算耗尽不是"报错"，是"任务太大这一轮装不下"：给原因 + 一键继续，
  // 而不是只丢一句「回合未完成」让用户以为整轮白跑。
  if (reason.includes("循环保护") || reason.includes("工具调用达到上限")) {
    showTurnLimitCard(reason);
  } else {
    const presentation = window.OwoModelRecovery && window.OwoModelRecovery.summarizeTurnFailure
      ? window.OwoModelRecovery.summarizeTurnFailure(reason) : null;
    const failure = addMessage("error", "回合未完成：" + ((presentation && presentation.message) || reason));
    if (presentation) attachTurnFailureActions(failure, reason, presentation);
    const sid = writeTargetSid || state.sessionId;
    const input = state.activeTurns.get(sid)?.prompt || state.lastSessionPrompts?.get(sid) || "";
    failure.appendChild(turnRecoveryActions(sid, input, reason));
  }
  const turn = state.turn;
  // 只有真的跑过（有模型调用/工具）才补汇报卡；请求级失败（如鉴权）不打扰。
  if (turn && (turn.tools > 0 || turn.modelCalls > 0)) {
    turn.stopped = true;
    renderTurnSummary(state.sessionId, turn);
  }
}

/// 单回合工具预算耗尽卡：说清原因，并给一个「继续执行」的出口。
function showTurnLimitCard(reason) {
  const card = document.createElement("div");
  card.className = "msg turn-limit";
  const title = document.createElement("strong");
  title.className = "turn-limit-title";
  title.textContent = "本回合的步数用完了";
  const text = document.createElement("span");
  text.className = "turn-limit-text";
  text.textContent = reason;
  const actions = document.createElement("div");
  actions.className = "turn-limit-actions";
  const cont = document.createElement("button");
  cont.type = "button";
  cont.className = "primary";
  cont.textContent = "继续执行";
  cont.addEventListener("click", () => continueTurn());
  const hint = document.createElement("span");
  hint.className = "turn-limit-hint";
  hint.textContent = "已完成的步骤会保留，继续时不会重做";
  actions.append(cont, hint);
  card.append(title, text, actions);
  newMessageBlock(card);
}

/// 继续执行：把「接着做」作为一条普通指令发出去（步数上限是服务端的策略，
/// 前端只负责把用户意图表达清楚并复用同一条发送链路）。
function continueTurn() {
  const box = $("prompt");
  if (!box) return;
  box.value = "继续完成上面未完成的部分，不要重复已经完成的步骤。";
  updateComposerHint();
  sendPrompt();
}

// ---------- composer 通用下拉浮层（模型 / 访问级别共用） ----------

let composerMenuEl = null;
function closeComposerMenu() {
  if (composerMenuEl) {
    composerMenuEl.remove();
    composerMenuEl = null;
  }
  $("sidebarWorkspaceBtn")?.setAttribute("aria-expanded", "false");
  $("composerProjectBtn")?.setAttribute("aria-expanded", "false");
  document.removeEventListener("pointerdown", onComposerMenuOutside, true);
}
function onComposerMenuOutside(event) {
  if (!composerMenuEl) return;
  if (composerMenuEl.contains(event.target)) return;
  const trigger = event.target.closest && event.target.closest("#modelChip,#accessChip,#composerProjectBtn,#sidebarWorkspaceBtn,#emptyStateWorkspaceBtn");
  if (trigger) return;
  // 命令补全依附于输入框：在输入框内点击不应关闭
  if (composerMenuEl.dataset.owner === "slash" && event.target.closest("#prompt")) return;
  closeComposerMenu();
}
function composerMenuPosition(triggerRect, menuRect, viewportWidth) {
  const margin = 12;
  const left = Math.max(margin, Math.min(
    triggerRect.right - menuRect.width,
    viewportWidth - menuRect.width - margin
  ));
  let top = triggerRect.top - menuRect.height - 8;
  if (top < 48) top = triggerRect.bottom + 8;
  return { left, top };
}

function openComposerMenu(trigger, html, bind) {
  closeComposerMenu();
  const menu = document.createElement("div");
  menu.className = "menu-popover composer-menu";
  menu.setAttribute("role", "menu");
  menu.innerHTML = html;
  document.body.appendChild(menu);
  composerMenuEl = menu;
  // Apply size variants before measuring; otherwise the model/project width is
  // expanded after placement and can overflow the viewport on narrow windows.
  if (bind) bind(menu);
  const rect = trigger.getBoundingClientRect();
  const position = composerMenuPosition(rect, menu.getBoundingClientRect(), window.innerWidth);
  menu.style.left = `${position.left}px`;
  menu.style.top = `${position.top}px`;
  document.addEventListener("pointerdown", onComposerMenuOutside, true);
}

// ---------- 模型选择下拉 ----------

function filterComposerModelMenu(menu, query) {
  const needle = String(query || "").trim().toLocaleLowerCase();
  let visibleCount = 0;
  for (const group of menu.querySelectorAll("[data-model-group]")) {
    let groupCount = 0;
    for (const choice of group.querySelectorAll("[data-model-choice]")) {
      const label = String(choice.dataset.modelSearch || choice.textContent || "").toLocaleLowerCase();
      const visible = !needle || label.includes(needle);
      choice.hidden = !visible;
      if (visible) groupCount += 1;
    }
    group.hidden = groupCount === 0;
    visibleCount += groupCount;
  }
  const empty = menu.querySelector("[data-model-empty]");
  if (empty) empty.hidden = visibleCount !== 0;
  return visibleCount;
}

function focusComposerModelChoice(menu, key, currentTarget) {
  const choices = Array.from(menu.querySelectorAll("[data-model-choice]"))
    .filter((choice) => !choice.hidden && !choice.disabled);
  if (!choices.length) return null;
  const current = choices.indexOf(currentTarget);
  let next;
  if (key === "Home") next = 0;
  else if (key === "End") next = choices.length - 1;
  else if (current < 0) next = key === "ArrowUp" ? choices.length - 1 : 0;
  else next = (current + (key === "ArrowDown" ? 1 : -1) + choices.length) % choices.length;
  return choices[next] || null;
}

function openModelMenu() {
  const selected = state.sessionId ? state.selectedModel : state.pendingModelOverride;
  const effectiveModel = getComposerModel();
  const defaultModel = getDefaultModel();
  const preset = [];
  const custom = [];
  const runtimeProvider = state.settings && state.settings.runtime && state.settings.runtime.provider;
  const registry = window.OwoProviderPresets;
  const providerPreset = registry && registry.presets
    ? ModelRouting.findProviderPreset(runtimeProvider, registry.presets(),
        state.settings && state.settings.provider && state.settings.provider.base_url)
    : null;
  const supportedModels = providerPreset && Array.isArray(providerPreset.models)
    ? providerPreset.models.map(String) : [];
  let effectiveItem = null;
  for (const option of $("settingsModel").options) {
    if (!option.value) continue;
    const item = { id: option.value, label: option.textContent };
    if (item.id === effectiveModel) effectiveItem = option;
    if (option.dataset.custom === "1") custom.push(item);
    else if (!supportedModels.length || supportedModels.includes(item.id)) preset.push(item);
  }
  const compatibility = ModelRouting.providerModelCompatibility(effectiveModel, supportedModels,
    !!(effectiveItem && effectiveItem.dataset.custom === "1"));
  const unsupportedCurrent = compatibility.known && !compatibility.compatible;
  const scope = state.sessionId ? "只影响当前会话" : "用于下一条新会话";
  let html = '<div class="composer-menu-title">选择模型</div>' +
    '<div class="composer-model-scope">' + scope + '</div>' +
    (unsupportedCurrent
      ? '<div class="composer-model-warning"><strong>当前模型与服务商不匹配</strong><span>' +
        esc(effectiveModel) + ' 不在 ' + esc(providerPreset.label || providerPreset.id) +
        ' 的可用模型清单中。请选择下方兼容模型，或切换“跟随默认模型”。</span></div>'
      : '') +
    '<label class="composer-model-search-wrap"><span class="sr-only">筛选模型</span>' +
      '<input type="search" class="composer-model-search" data-model-search aria-label="筛选模型" placeholder="筛选模型…" autocomplete="off" spellcheck="false"></label>' +
    '<div class="composer-model-choice-group" data-model-group>' +
      '<div class="composer-menu-group">默认</div>' +
      '<button type="button" class="composer-menu-item has-desc' + (!selected ? " active" : "") +
      '" data-model-default data-model-choice data-model-search="' +
      escapeAttribute("跟随默认模型 " + defaultModel) + '">' +
      '<span class="composer-menu-label">跟随默认模型</span>' +
      '<span class="composer-menu-desc">' + esc(defaultModel || "由当前服务商决定") + '</span>' +
      (!selected ? '<span class="composer-menu-check">✓</span>' : '') + '</button></div>';
  const renderGroup = (label, list, activeModel) => {
    if (!list.length) return "";
    let out = '<div class="composer-model-choice-group" data-model-group><div class="composer-menu-group">' + esc(label) + '</div>';
    for (const item of list) {
      const active = item.id === activeModel ? " active" : "";
      const check = item.id === activeModel ? '<span class="composer-menu-check">✓</span>' : "";
      out += '<button type="button" class="composer-menu-item' + active + '" data-model="' + escapeAttribute(item.id) +
        '" data-model-choice data-model-search="' + escapeAttribute(item.id + " " + item.label) +
        '"><span>' + esc(item.label) + '</span>' + check + '</button>';
    }
    return out + '</div>';
  };
  html += renderGroup(providerPreset ? "当前服务商可用模型" : "可用模型", preset, selected);
  if (unsupportedCurrent) html += '<div class="composer-menu-group">当前会话模型（不兼容）</div>' +
    '<div class="composer-model-incompatible"><span>' + esc(effectiveModel) + '</span><span>需切换</span></div>';
  html += renderGroup("自定义模型", custom, selected);
  html += '<div class="composer-model-empty" data-model-empty hidden>没有找到匹配的模型</div>' +
    '<div class="composer-menu-foot"><button type="button" class="composer-menu-link" data-go-settings>管理模型与预设…</button></div>';
  openComposerMenu($("modelChip"), html, (menu) => {
    menu.classList.add("composer-menu-model");
    const filter = menu.querySelector("[data-model-search]");
    if (filter) {
      filter.addEventListener("input", () => filterComposerModelMenu(menu, filter.value));
      filter.addEventListener("keydown", (event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          closeComposerMenu();
          $("modelChip").focus();
        } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
          const choice = focusComposerModelChoice(menu, event.key, event.target);
          if (choice) {
            event.preventDefault();
            choice.focus();
          }
        }
      });
      filter.focus();
    }
    for (const button of menu.querySelectorAll("[data-model-choice]")) {
      button.addEventListener("keydown", (event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          closeComposerMenu();
          $("modelChip").focus();
          return;
        }
        if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
        const choice = focusComposerModelChoice(menu, event.key, event.target);
        if (choice) {
          event.preventDefault();
          choice.focus();
        }
      });
    }
    for (const button of menu.querySelectorAll("[data-model]")) {
      button.addEventListener("click", async () => { const id = button.dataset.model; closeComposerMenu(); await selectModel(id); });
    }
    const useDefault = menu.querySelector("[data-model-default]");
    if (useDefault) useDefault.addEventListener("click", async () => { closeComposerMenu(); await selectModel(""); });
    const go = menu.querySelector("[data-go-settings]");
    if (go) go.addEventListener("click", () => { closeComposerMenu(); setSettingsPageVisible(true); setSettingsTab("models"); });
  });
  if (composerMenuEl) composerMenuEl.dataset.owner = "model";
}

async function selectModel(id) {
  const model = String(id || "").trim();
  const sessionId = state.sessionId;
  try {
    if (sessionId) {
      const result = await sessionModelUpdateQueue.enqueue(sessionId, model);
      if (!result.latest || state.sessionId !== sessionId) return;
      if (result.error) throw result.error;
      state.selectedModel = model;
    } else {
      state.pendingModelOverride = model || null;
    }
    refreshComposerModelChip();
    showToast(model
      ? "此" + (state.sessionId ? "会话" : "新会话") + "将使用 " + model
      : "已恢复跟随默认模型", "ok");
  } catch (error) {
    showToast("模型切换失败：" + friendlyError(error), "error");
  }
}

// ---------- 推理档位（reasoning_effort）：对标 Codex 输入框里的「模型 + 档位」 ----------
// 默认值不下发该参数：不支持 reasoning_effort 的 OpenAI 兼容端点遇到未知字段可能直接 400，
// 因此只有用户显式选择档位时才写进请求体（后端 settings → OWO_REASONING_EFFORT → 请求体）。

const EFFORT_MODES = [
  { id: "", label: "默认", desc: "跟随模型自身默认，不发送 reasoning_effort" },
  { id: "minimal", label: "极简", desc: "最少思考、最快返回（仅部分模型支持）" },
  { id: "low", label: "低", desc: "轻度思考，偏向速度" },
  { id: "medium", label: "中", desc: "速度与深度平衡" },
  { id: "high", label: "高", desc: "尽量深入思考，更慢也更贵" },
];

function getEffort() {
  const raw = (state.settings && state.settings.reasoning_effort) || "";
  const id = String(raw).trim().toLowerCase();
  return EFFORT_MODES.some((mode) => mode.id === id) ? id : "";
}

function renderEffortChip() {
  const mode = EFFORT_MODES.find((item) => item.id === getEffort()) || EFFORT_MODES[0];
  const text = $("effortChipText");
  const chip = $("effortChip");
  if (text) text.textContent = mode.id ? `推理 ${mode.label}` : "推理默认";
  if (chip) {
    chip.dataset.effort = mode.id;
    chip.title = `推理档位：${mode.label}（${mode.desc}）`;
  }
}

async function selectEffort(id) {
  state.settings = state.settings || {};
  state.settings.reasoning_effort = id || null;
  renderEffortChip();
  const result = await saveSettings({ silent: true });
  const mode = EFFORT_MODES.find((item) => item.id === id) || EFFORT_MODES[0];
  if (result.ok) showToast(`推理档位：${mode.label}`, "ok");
}

function openEffortMenu() {
  const current = getEffort();
  let html = '<div class="composer-menu-title">推理档位（thinking effort）</div>';
  for (const mode of EFFORT_MODES) {
    const active = mode.id === current ? " active" : "";
    const check = mode.id === current ? '<span class="composer-menu-check">✓</span>' : "";
    html +=
      `<button type="button" class="composer-menu-item has-desc${active}" data-effort="${mode.id}">` +
      `<span class="composer-menu-label">${esc(mode.label)}</span>` +
      `<span class="composer-menu-desc">${esc(mode.desc)}</span>` +
      `${check}</button>`;
  }
  openComposerMenu($("effortChip"), html, (menu) => {
    for (const button of menu.querySelectorAll("[data-effort]")) {
      button.addEventListener("click", () => {
        const id = button.dataset.effort;
        closeComposerMenu();
        if (id === getEffort()) return;
        selectEffort(id);
      });
    }
  });
}

// 审批由服务端策略裁决；前端不会替用户自动批准待确认请求。
const ACCESS_MODES = [{ id: "ask", label: "每次询问", desc: "待确认请求由你决定；已授权操作按服务端策略执行" }];
function getAccessMode() { return "ask"; }
function setAccessMode() { localStorage.removeItem("owo.access-mode"); renderAccessChip(); }
function renderAccessChip() {
  const text = $("accessChipText"), dot = $("accessDot"), chip = $("accessChip");
  if (text) text.textContent = "每次询问";
  if (dot) dot.className = "codex-access-dot mode-ask";
  if (chip) { chip.dataset.mode = "ask"; chip.title = "审批方式：每次询问；自动授权请查看服务端权限中心"; }
}
function openAccessMenu() {
  openComposerMenu($("accessChip"), '<div class="composer-menu-title">本机审批方式</div><div class="composer-menu-note">待确认请求不会自动批准。已授权操作由服务端策略执行；范围与有效期请在权限中心查看。</div><button type="button" class="composer-menu-item" data-open-permissions>打开权限中心 →</button>', menu => {
    menu.querySelector("[data-open-permissions]").addEventListener("click", () => { closeComposerMenu(); openPanelById("permissions"); });
  });
}

// ---------- 斜杠命令（composer 内输入 / 唤起，对标 Codex/Claude Code 会话内命令） ----------

const SLASH_COMMANDS = [
  { name: "new", desc: "新建会话", run: () => newSession() },
  { name: "undo", desc: "回退最近一个回合", run: () => sessionUndo() },
  { name: "redo", desc: "重做被回退的回合", run: () => sessionRedo() },
  { name: "export", desc: "导出当前会话为 Markdown", run: () => exportSession("md") },
  { name: "stop", desc: "中断正在执行的回合", run: () => abortTurn() },
  { name: "settings", desc: "打开设置页", run: () => setSettingsPageVisible(true) },
  { name: "help", desc: "查看全部命令", run: () => showSlashHelp() },
];

let slashMatches = [];
let slashActiveIndex = 0;

/// 当前输入是否处于命令补全状态（以 / 开头且尚未输入空格）。
function slashQuery() {
  const promptEl = $("prompt");
  if (!promptEl) return null;
  const value = promptEl.value;
  if (!value.startsWith("/")) return null;
  const rest = value.slice(1);
  if (/\s/.test(rest)) return null;
  return rest.toLowerCase();
}

function matchingSlashCommands(query) {
  return SLASH_COMMANDS.filter((cmd) => cmd.name.startsWith(query));
}

function closeSlashMenu() {
  if (composerMenuEl && composerMenuEl.dataset.owner === "slash") closeComposerMenu();
  slashMatches = [];
  slashActiveIndex = 0;
}

function refreshSlashMenu() {
  const query = slashQuery();
  if (query === null) {
    closeSlashMenu();
    return;
  }
  const matches = matchingSlashCommands(query);
  if (!matches.length) {
    closeSlashMenu();
    return;
  }
  const sameSet =
    composerMenuEl &&
    composerMenuEl.dataset.owner === "slash" &&
    slashMatches.length === matches.length &&
    slashMatches.every((cmd, i) => cmd.name === matches[i].name);
  slashMatches = matches;
  if (slashActiveIndex >= matches.length) slashActiveIndex = 0;
  const html =
    '<div class="composer-menu-title">命令</div>' +
    matches
      .map(
        (cmd, index) =>
          `<button type="button" class="composer-menu-item has-desc${index === slashActiveIndex ? " active" : ""}" data-slash="${index}">` +
          `<span class="composer-menu-label">/${esc(cmd.name)}</span>` +
          `<span class="composer-menu-desc">${esc(cmd.desc)}</span></button>`
      )
      .join("");
  if (sameSet) {
    // 复用已打开的浮层，只刷新高亮，避免每次按键都重建 DOM
    composerMenuEl.querySelectorAll("[data-slash]").forEach((button, index) => {
      button.classList.toggle("active", index === slashActiveIndex);
    });
    return;
  }
  openComposerMenu($("prompt"), html, (menu) => {
    menu.dataset.owner = "slash";
    menu.addEventListener("mousemove", (event) => {
      const button = event.target.closest("[data-slash]");
      if (!button) return;
      slashActiveIndex = Number(button.dataset.slash);
      menu.querySelectorAll("[data-slash]").forEach((item, index) => {
        item.classList.toggle("active", index === slashActiveIndex);
      });
    });
    menu.addEventListener("click", (event) => {
      const button = event.target.closest("[data-slash]");
      if (!button) return;
      runSlashCommand(slashMatches[Number(button.dataset.slash)]);
    });
  });
  if (composerMenuEl) composerMenuEl.dataset.owner = "slash";
}

/// 执行命令并清空输入框；命令返回 Promise 时静默吞掉错误交由各命令自身提示。
function runSlashCommand(command) {
  if (!command) return;
  closeSlashMenu();
  $("prompt").value = "";
  updateComposerHint();
  try {
    const result = command.run();
    if (result && typeof result.catch === "function") {
      result.catch((error) => {
        showToast(friendlyError(error, { resource: true }), "error");
      });
    }
  } catch (error) {
    showToast(friendlyError(error, { resource: true }), "error");
  }
}

function showSlashHelp() {
  const rows = SLASH_COMMANDS.map(
    (cmd) => `<li><strong>/${esc(cmd.name)}</strong><span>${esc(cmd.desc)}</span></li>`
  ).join("");
  openModal({
    title: "可用命令",
    body: `<ul class="slash-help">${rows}</ul>`,
    actions: [{ label: "关闭", kind: "primary", onClick: ({ close }) => close() }],
  });
}

// 输入框为空时给出命令入口提示（不干扰正常输入）
function updateComposerHint() {
  const chip = $("slashHint");
  if (!chip) return;
  const promptEl = $("prompt");
  chip.classList.toggle("hidden", !!promptEl.value);
}

// Credentials are never returned to this renderer.
function customModelKeys() { return {}; }
function loadCustomModels() { return window.OwoCustomModels.models(); }
function saveCustomModels() { throw new Error("请使用桌面安全存储保存模型"); }

const REASONING_MODEL_RE =
  /reason|z1|thinking|o[13](-mini)?$|deepseek-(flash|v\d)/i;

function loadReasoningModels() {
  try {
    return new Set(JSON.parse(localStorage.getItem("owo.reasoning-models") || "[]"));
  } catch {
    return new Set();
  }
}

function rememberReasoningModel(id) {
  const model = String(id || "").trim();
  if (!model || loadReasoningModels().has(model)) return;
  const set = loadReasoningModels();
  set.add(model);
  try {
    localStorage.setItem("owo.reasoning-models", JSON.stringify([...set]));
  } catch {
    /* 隐私模式下不可持久化，不影响本会话 */
  }
  updateModelChipMeta();
}

function isReasoningModel(id) {
  return REASONING_MODEL_RE.test(String(id || "")) || loadReasoningModels().has(String(id || ""));
}

/// 模型 chip 元信息：推理模型给"深度思考"徽标 + 提示，普通模型只显示名字。
function updateModelChipMeta() {
  const chip = $("modelChip");
  if (!chip) return;
  const model = getComposerModel();
  const reasoner = isReasoningModel(model);
  const registry = window.OwoProviderPresets;
  const settings = state.settings || {};
  const preset = registry && registry.presets
    ? ModelRouting.findProviderPreset(settings.runtime && settings.runtime.provider, registry.presets(),
        settings.provider && settings.provider.base_url) : null;
  const supportedModels = preset && Array.isArray(preset.models) ? preset.models : [];
  const select = $("settingsModel");
  const custom = !!(select && Array.from(select.options).some((option) =>
    option.value === model && option.dataset.custom === "1"));
  const compatibility = ModelRouting.providerModelCompatibility(model, supportedModels, custom);
  const incompatible = compatibility.known && !compatibility.compatible;
  chip.classList.toggle("is-reasoner", reasoner && !incompatible);
  chip.classList.toggle("is-model-incompatible", incompatible);
  chip.title = incompatible
    ? model + " 不在当前服务商 " + (preset.label || preset.id) + " 的可用清单中；发送可能失败，点击切换兼容模型"
    : reasoner ? model + " · 深度思考模型：回合会流式展示推理过程（点「思考过程」展开看全文）"
      : "选择模型（当前 " + (model || "默认") + "）";
  let reasonerBadge = chip.querySelector(".reasoner-badge");
  if (reasoner && !reasonerBadge) { reasonerBadge = document.createElement("span"); reasonerBadge.className = "reasoner-badge"; reasonerBadge.textContent = "深度思考"; chip.appendChild(reasonerBadge); }
  else if (!reasoner && reasonerBadge) reasonerBadge.remove();
  let warningBadge = chip.querySelector(".model-warning-badge");
  if (incompatible && !warningBadge) { warningBadge = document.createElement("span"); warningBadge.className = "model-warning-badge"; warningBadge.textContent = "需切换"; chip.appendChild(warningBadge); }
  else if (!incompatible && warningBadge) warningBadge.remove();
}

/// 按当前服务商筛选模型候选：保留兼容模型与自定义模型；当前默认模型若不兼容则禁用并提示。
/// provider-presets.js 的 models 清单驱动兼容性判断；空清单（自定义端点）不做限制。
/// 判定顺序：runtime.provider（核心识别出的服务商 id）→ settings.provider.base_url.
function syncProviderModels(presetOverride) {
  const registry = window.OwoProviderPresets;
  const select = $("settingsModel");
  if (!registry || !select || typeof registry.presets !== "function") return;
  const settings = state.settings || {};
  const runtime = settings.runtime || {};
  const baseUrl = String((settings.provider && settings.provider.base_url) || "").toLowerCase();
  const preset = presetOverride !== undefined
    ? presetOverride
    : ModelRouting.findProviderPreset(runtime.provider, registry.presets(), baseUrl);
  const supportedModels = preset && Array.isArray(preset.models)
    ? preset.models.map(String)
    : [];
  for (const id of supportedModels) {
    if (!id || select.querySelector('option[value="' + CSS.escape(id) + '"]')) continue;
    const option = document.createElement("option");
    option.value = id;
    option.textContent = id + (isReasoningModel(id) ? "（深度思考）" : "");
    option.dataset.provider = "1";
    select.appendChild(option);
  }
  const currentModel = String(select.value || getDefaultModel());
  let currentUnsupported = false;
  for (const option of select.options) {
    if (!option.value) continue;
    const state = ModelRouting.providerModelOptionState(
      option.value, supportedModels, currentModel, option.dataset.custom === "1"
    );
    option.hidden = state.hidden;
    option.disabled = state.disabled;
    if (option.value === currentModel && state.disabled) currentUnsupported = true;
  }
  const hint = $("modelCompatibilityHint");
  if (hint) {
    hint.hidden = !currentUnsupported;
    hint.textContent = currentUnsupported
      ? "\u5f53\u524d\u9ed8\u8ba4\u6a21\u578b\u4e0d\u5728\u8be5\u670d\u52a1\u5546\u7684\u5e38\u7528\u6a21\u578b\u5217\u8868\u4e2d\uff0c\u8bf7\u5207\u6362\u5230\u5f53\u524d\u670d\u52a1\u5546\u652f\u6301\u7684\u6a21\u578b\u3002"
      : "";
  }
}
function renderCustomModels() {
  const list = $("customModelList");
  if (!list) return;
  const models = loadCustomModels().filter(model => !model.disabled);
  const storage = $("customModelStorageState");
  if (storage) storage.textContent = window.OwoCustomModels.status().problem ? "部分旧配置尚未迁移或凭据存储不可用。原数据已保留，请检查桌面核心与加密存储。" : "模型配置跨重启保存；密钥由桌面壳加密托管，不返回给页面。";
  const select = $("settingsModel");
  for (const option of [...select.options]) {
    if (option.dataset.custom === "1") option.remove();
  }
  for (const model of models) {
    const option = document.createElement("option");
    option.value = model.id;
    option.textContent = model.label || model.id;
    option.dataset.custom = "1";
    select.appendChild(option);
  }
  if (!models.length) {
    list.innerHTML = '<div class="sub">暂无自定义模型，点击右上角添加。</div>';
    return;
  }
  list.innerHTML = "";
  for (const model of models) {
    const row = document.createElement("div");
    row.className = "model-row";
    row.innerHTML = `
      <div class="model-row-main">
        <strong>${esc(model.label || model.id)}</strong>
        <span class="sub">${esc(model.id)} ｜ ${esc(model.apiFormat === "anthropic" ? "Anthropic" : "OpenAI 兼容")} ｜ ${esc(model.baseUrl || "默认端点")}${model.hasKey ? " ｜ 已加密存储" : ""}</span>
      </div>
      <div class="model-row-actions">
        <button type="button" class="ghost" data-act="default">设为默认</button>
        <button type="button" class="ghost" data-act="edit">编辑</button>
        <button type="button" class="ghost danger-text" data-act="remove">移除</button>
      </div>`;
    row.querySelector('[data-act="default"]').addEventListener("click", async () => {
      select.value = model.id;
      await saveSettings();
      showToast(`默认模型已切换为 ${model.label || model.id}`, "ok");
    });
    row.querySelector('[data-act="edit"]').addEventListener("click", () => openCustomModelDialog(model));
    row.querySelector('[data-act="remove"]').addEventListener("click", () =>
      confirmModal({
        title: "移除自定义模型",
        message: `确定移除「${model.label || model.id}」？该操作只影响本机设置。`,
        confirmText: "移除",
        kind: "danger",
        onConfirm: async () => {
          await window.OwoCustomModels.remove(model.id);
          state.settings.custom_models = loadCustomModels();
          renderCustomModels();
          showToast("已移除自定义模型", "ok");
        },
      })
    );
    list.appendChild(row);
  }
}

function openCustomModelDialog(existing) {
  const model = existing || { apiFormat: "openai" };
  openModal({
    title: existing ? "编辑自定义模型" : "添加自定义模型",
    body: `
      <div class="form-grid">
        <label class="field-label">API 格式
          <select id="cmApiFormat">
            <option value="openai">OpenAI 兼容（/chat/completions）</option>
            <option value="anthropic">Anthropic（/messages）</option>
          </select>
        </label>
        <label class="field-label">模型 ID
          <input id="cmId" placeholder="例如 my-model-v1" value="${esc(model.id || "")}">
        </label>
        <label class="field-label">展示名称
          <input id="cmLabel" placeholder="例如 我的模型" value="${esc(model.label || "")}">
        </label>
        <label class="field-label">请求地址（Base URL）
          <input id="cmBaseUrl" placeholder="https://api.example.com/v1" value="${esc(model.baseUrl || "")}">
        </label>
        <label class="form-check">
          <input type="checkbox" id="cmFullUrl" ${model.useFullUrl ? "checked" : ""}>
          <span>使用完整 URL（不做路径拼接）</span>
        </label>
        <label class="field-label">API 密钥（桌面壳加密保存，不进入浏览器存储）
          <input type="password" id="cmKey" autocomplete="new-password" placeholder="${model.hasKey ? "已加密保存，留空保留" : "填写此端点的密钥"}">
        </label>
        <details class="form-advanced">
          <summary>高级配置</summary>
          <div class="form-grid">
            <label class="field-label">温度（0–2，留空为默认）
              <input id="cmTemperature" placeholder="例如 0.7" value="${model.temperature ?? ""}">
            </label>
            <label class="field-label">超时（秒，留空为默认 120）
              <input id="cmTimeout" placeholder="120" value="${model.timeoutSecs ?? ""}">
            </label>
          </div>
        </details>
        <div class="form-inline-result" id="cmTestResult" hidden></div>
      </div>`,
    actions: [
      {
        label: "重置",
        kind: "ghost",
        onClick: ({ body }) => {
          body.querySelector("#cmApiFormat").value = "openai";
          for (const input of body.querySelectorAll("input")) {
            if (input.type === "checkbox") input.checked = false;
            else input.value = "";
          }
          const result = body.querySelector("#cmTestResult");
          result.hidden = true;
        },
      },
      {
        label: "连通性测试",
        onClick: async ({ body }) => {
          const result = body.querySelector("#cmTestResult");
          const baseUrl = body.querySelector("#cmBaseUrl").value.trim();
          result.hidden = false;
          if (!baseUrl) {
            result.textContent = "请先填写请求地址";
            return;
          }
          result.textContent = "正在测试连接…";
          try {
            const response = await window.OwoCustomModels.probe({
              id: body.querySelector("#cmId").value.trim(), previousId: model.id, baseUrl,
              apiFormat: body.querySelector("#cmApiFormat").value, useFullUrl: body.querySelector("#cmFullUrl").checked,
              apiKey: body.querySelector("#cmKey").value.trim(),
            });
            result.textContent = response.note || (response.ok ? "连接成功：端点返回 " + (response.models ?? "未知数量") + " 个模型" : "连接未通过，HTTP " + (response.status ?? "未知"));
          } catch (error) { result.textContent = "连接测试失败：" + friendlyError(error); }

        },
      },
      {
        label: existing ? "保存修改" : "添加模型",
        kind: "primary",
        onClick: async ({ close }) => {
          const id = $("cmId").value.trim();
          if (!id) {
            showToast("请填写模型 ID", "error");
            return;
          }
          const temperature = $("cmTemperature").value.trim();
          const timeoutSecs = $("cmTimeout").value.trim();
          const next = {
            id,
            label: $("cmLabel").value.trim() || id,
            apiFormat: $("cmApiFormat").value,
            baseUrl: $("cmBaseUrl").value.trim(),
            useFullUrl: $("cmFullUrl").checked,
            ...(temperature !== "" ? { temperature: Number(temperature) } : {}),
            ...(timeoutSecs !== "" ? { timeoutSecs: Number(timeoutSecs) } : {}),
          };
          try { ModelRouting.buildCustomModelConnection(id, [next], {}); }
          catch (error) { showToast(error.message, "error"); return; }
          try {
            await window.OwoCustomModels.upsert(next, $("cmKey").value.trim(), existing?.id);
            $("cmKey").value = "";
            state.settings = state.settings || {};
            state.settings.custom_models = loadCustomModels();
          } catch (error) { showToast("模型未保存：" + friendlyError(error), "error"); return; }
          renderCustomModels();
          close();
          showToast(`已保存自定义模型 ${next.label}`, "ok");
        },
      },
    ],
  });
  $("cmApiFormat").value = model.apiFormat || "openai";
}
$("customModelAddBtn").addEventListener("click", () => openCustomModelDialog(null));
$("modelOutputModel").addEventListener("change", () => refreshModelOutputSettings());
$("modelOutputSaveBtn").addEventListener("click", () => saveModelOutputSettings());
$("modelOutputResetBtn").addEventListener("click", () => resetModelOutputSettings());
$("settingsModel").addEventListener("change", () => saveSettings());

// 本地偏好：select / toggle → LOCAL_PREFS（仅本机生效）
const PREF_SELECT_MAP = {
  prefFileOpener: "fileOpener",
  prefSpeechLang: "speechLang",
  prefTheme: "theme",
};
for (const [id, key] of Object.entries(PREF_SELECT_MAP)) {
  $(id).addEventListener("change", () => {
    LOCAL_PREFS[key] = $(id).value;
    saveLocalPrefs();
    applyLocalPrefs();
    if (key === "theme") {
      const dark =
        LOCAL_PREFS.theme === "dark" ||
        (LOCAL_PREFS.theme === "system" &&
          window.matchMedia("(prefers-color-scheme: dark)").matches);
      applyTheme(dark ? "dark" : "light");
    }
  });
}
const themeMedia = window.matchMedia("(prefers-color-scheme: dark)");
themeMedia.addEventListener?.("change", event => {
  if (loadLocalPrefs().theme === "system") applyTheme(event.matches ? "dark" : "light");
});
const PREF_TOGGLE_MAP = {
  prefCompact: "compact",
  prefSound: "sound",
};
function unlockNotificationAudioIfEnabled() {
  if (LOCAL_PREFS.sound) window.OwoNotificationSound?.unlock(true);
}
document.addEventListener("pointerdown", unlockNotificationAudioIfEnabled, { capture: true, passive: true });
document.addEventListener("keydown", unlockNotificationAudioIfEnabled, { capture: true });
for (const [id, key] of Object.entries(PREF_TOGGLE_MAP)) {
  $(id).addEventListener("click", () => {
    LOCAL_PREFS[key] = !LOCAL_PREFS[key];
    saveLocalPrefs();
    syncLocalPrefs();
    applyLocalPrefs();
    if (key === "sound" && LOCAL_PREFS.sound) window.OwoNotificationSound?.unlock(true);
  });
}
// 桌面桌宠开关（A8-3）：走引擎通道控制桌面端挂件，不是本地偏好。
$("prefPet").addEventListener("click", () => {
  toggleDesktopPet();
});
// 主动建议是服务端设置（settings.proactive.enabled）
$("prefProactive").addEventListener("click", async () => {
  const next = !(state.settings.proactive && state.settings.proactive.enabled);
  try {
    const settings = { ...(state.settings || {}) };
    delete settings.custom_models;
    settings.proactive = { ...(settings.proactive || {}), enabled: next };
    await api("/settings", { method: "POST", body: JSON.stringify(settings) });
    await refreshSettings();
    showToast(`主动建议已${next ? "开启" : "关闭"}`, "ok");
  } catch (error) {
    showToast(`切换失败：${error.message || error}`, "error");
  }
});

// 常规：更改默认工作区 → 原生文件夹选择器
$("changeWorkspaceBtn").addEventListener("click", () => pickDirectory());
// 外观：恢复默认布局（会话栏 / 检查器宽度）
$("resetLayoutBtn").addEventListener("click", () => {
  document.body.style.removeProperty("--session-width");
  document.body.style.removeProperty("--inspect-width");
  showToast("已恢复默认布局", "ok");
});
// 语音：麦克风权限测试
$("micTestBtn").addEventListener("click", async () => {
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
    showToast("当前浏览器不支持麦克风采集", "error");
    return;
  }
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    stream.getTracks().forEach((track) => track.stop());
    showToast("麦克风可用（权限已授予）", "ok");
  } catch (error) {
    showToast(`麦克风不可用：${error.message || error}`, "error");
  }
});
// 语音：输入模式状态胶囊（本地录音优先，系统识别回退）
(function initSttModePill() {
  const pill = $("sttModePill");
  if (!pill) return;
  const canRecord = !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  const canRecognize = !!(window.SpeechRecognition || window.webkitSpeechRecognition);
  pill.textContent = canRecord
    ? "本地录音优先（16k WAV）"
    : canRecognize
      ? "系统识别回退"
      : "当前环境不可用";
  pill.classList.toggle("offline", !canRecord && !canRecognize);
})();

// 账户：重连 / 退出登录（与左下用户卡菜单同款逻辑）
async function reconnectService() {
  window.OwoApi.resetCoreConnection();
  try {
    await ensureApiToken();
    await refreshHealth();
    showToast("服务已重新连接", "ok");
  } catch (error) {
    showToast(`重连失败：${error.message || error}`, "error");
  }
}
function logoutLocal() {
  window.OwoApi.resetCoreConnection();
  localStorage.removeItem("owo.workspace");
  showToast("已退出登录，下次操作将重新配对");
  serviceWatch.start();
}
$("accountReconnectBtn").addEventListener("click", reconnectService);
$("accountLogoutBtn").addEventListener("click", () =>
  confirmModal({
    title: "退出登录",
    message: "将清除本机配对信息，下次操作时重新连接服务。设置与数据不受影响。",
    confirmText: "退出登录",
    kind: "danger",
    onConfirm: logoutLocal,
  })
);

// 保存模型与服务端设置：基于最近一次 GET /settings 的全量副本改字段，
// 不再暴露原始 JSON 编辑器（表单是唯一入口）。
async function saveSettings(options = {}) {
  try {
    const settings = { ...(state.settings || {}) };
    delete settings.custom_models; // 自定义模型只存本机，不发给服务端
    settings.model = $("settingsModel").value;
    const resp = await api("/settings", {
      method: "POST",
      body: JSON.stringify(settings),
    });
    const note = `设置已保存：${(resp && resp.note) || "ok"}`;
    if (!options.silent) showToast(note, "ok");
    await refreshSettings();
    return { ok: true, note };
  } catch (error) {
    const note = `设置保存失败：${error.message || error}`;
    if (!options.silent) showToast(note, "error");
    return { ok: false, note };
  }
}

// ---------- 模型服务预设（config/provider-presets.js 为唯一事实源） ----------
// 前端只展示"端点 / 默认模型 / 所需环境变量"并生成可复制的设置命令；
// 端点与密钥由启动核心服务的终端环境变量决定，前端不写入、不假装保存。

function findPreset(id) {
  const registry = window.OwoProviderPresets;
  if (!registry || !id) return null;
  return registry.presets().find((preset) => preset.id === id) || null;
}

function updatePresetApplyAvailability(presetOverride) {
  const button = $("presetApplyModelBtn");
  if (!button) return;
  const preset = presetOverride || findPreset($("providerPreset")?.value);
  const gate = window.OwoCoreActionAvailability;
  button.disabled = !preset?.model || !gate || !gate.isAvailable();
}

function renderPresetInfo() {
  const info = $("presetInfo");
  if (!info) return;
  const preset = findPreset($("providerPreset").value);
  if (!preset) {
    info.classList.add("hidden");
    syncProviderModels();
    return;
  }
  const command = window.OwoProviderPresets.envCommand(preset);
  $("presetBaseUrl").textContent = preset.baseUrl || "（自定义，请在终端设置 OPENAI_BASE_URL）";
  $("presetModel").textContent = preset.model || "（不指定，保持当前模型）";
  $("presetKeyEnv").textContent = preset.keyEnv || "（本地端点无需密钥）";
  $("presetCmdPreview").textContent = command || "（自定义端点无预设命令）";
  $("presetHint").textContent = `${preset.note} 选预设会填入上方端点与模型，密钥填在「API 密钥」后点「保存并连接」。`;
  updatePresetApplyAvailability(preset);
  // 预设一键填入表单：端点 + 默认模型，用户只需补密钥。
  if (preset.baseUrl) $("providerBaseUrl").value = preset.baseUrl;
  syncProviderModels(preset);
  if (preset.model && $("settingsModel").querySelector(`option[value="${CSS.escape(preset.model)}"]`)) {
    $("settingsModel").value = preset.model;
  }
  syncProviderModels(preset);
  info.classList.remove("hidden");
}

// 回填模型接入表单：端点与密钥状态来自 GET /settings（密钥永不明文回传）。
function shellCommand(command, args = {}) {
  const owner = window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === "function"
    ? window.__TAURI__.core
    : window.__TAURI_INTERNALS__;
  if (!owner || typeof owner.invoke !== "function") return Promise.reject(new Error("此操作需要 OwO Agent 桌面端"));
  return Promise.resolve(owner.invoke(command, args));
}

async function refreshModelOutputSettings() {
  const modelInput = $("modelOutputModel");
  const limitInput = $("modelOutputLimit");
  if (!modelInput || !limitInput) return;
  const hint = $("modelOutputHint");
  try {
    const status = await shellCommand("get_provider_status");
    state.modelOutputTokens = status.modelOutputTokens && typeof status.modelOutputTokens === "object"
      ? status.modelOutputTokens
      : {};
    state.defaultModelOutputTokens = Number(status.maxOutputTokens) || window.OwoModelOutputBudget.DEFAULT;
    const candidateList = $("modelOutputCandidates");
    if (candidateList) {
      const names = new Set([
        ...Array.from($("settingsModel").options || []).map((option) => option.value),
        ...(Array.isArray(status.models) ? status.models : []),
        ...((state.settings && Array.isArray(state.settings.custom_models))
          ? state.settings.custom_models.map((item) => item && (item.id || item.model)).filter(Boolean)
          : []),
        ...Object.keys(state.modelOutputTokens),
        String(status.model || ""),
      ].map((name) => String(name || "").trim()).filter(Boolean));
      candidateList.replaceChildren(...Array.from(names, (name) => {
        const option = document.createElement("option");
        option.value = name;
        return option;
      }));
    }
    if (!modelInput.value.trim()) modelInput.value = String(status.model || getDefaultModel() || "");
    const selected = modelInput.value.trim();
    const hasOverride = Object.prototype.hasOwnProperty.call(state.modelOutputTokens, selected);
    limitInput.value = String(hasOverride ? state.modelOutputTokens[selected] : state.defaultModelOutputTokens);
    if (hint) hint.textContent = hasOverride
      ? `${selected} 当前单独上限：${state.modelOutputTokens[selected].toLocaleString("zh-CN")} Tokens`
      : `${selected || "当前模型"} 使用默认上限：${state.defaultModelOutputTokens.toLocaleString("zh-CN")} Tokens`;
  } catch (error) {
    if (hint) hint.textContent = `读取模型上限失败：${error.message || error}`;
  }
}

async function resetModelOutputSettings() {
  const model = $("modelOutputModel").value.trim();
  const hint = $("modelOutputHint");
  if (!model) {
    if (hint) hint.textContent = "请先填写要恢复默认值的模型名称。";
    $("modelOutputModel").focus();
    return;
  }
  const button = $("modelOutputResetBtn");
  if (button) button.disabled = true;
  try {
    const outputTokens = { ...state.modelOutputTokens };
    delete outputTokens[model];
    const result = await shellCommand("set_model_config", { model_output_tokens: outputTokens });
    if (!result || !result.ok) throw new Error((result && result.error) || "恢复失败");
    state.modelOutputTokens = result.modelOutputTokens || outputTokens;
    state.defaultModelOutputTokens = Number(result.maxOutputTokens) || window.OwoModelOutputBudget.DEFAULT;
    $("modelOutputLimit").value = String(state.defaultModelOutputTokens);
    if (window.OwoApi && typeof window.OwoApi.resetCoreConnection === "function") window.OwoApi.resetCoreConnection();
    if (typeof window.owoRecoverService === "function") window.owoRecoverService();
    if (hint) hint.textContent = `${model} 已恢复默认输出上限：${state.defaultModelOutputTokens.toLocaleString("zh-CN")} Tokens。`;
    showToast(`${model} 已恢复默认输出上限`, "ok");
  } catch (error) {
    if (hint) hint.textContent = `恢复失败：${error.message || error}`;
    showToast(`恢复默认失败：${error.message || error}`, "error");
  } finally {
    if (button) button.disabled = false;
  }
}

async function saveModelOutputSettings() {
  const model = $("modelOutputModel").value.trim();
  const raw = $("modelOutputLimit").value.trim();
  const hint = $("modelOutputHint");
  if (!model) {
    if (hint) hint.textContent = "请填写要配置的模型名称。";
    $("modelOutputModel").focus();
    return;
  }
  const limit = Number(raw);
  if (!Number.isInteger(limit) || limit < 1 || limit > window.OwoModelOutputBudget.MAX) {
    if (hint) hint.textContent = `输出上限必须是 1–${window.OwoModelOutputBudget.MAX.toLocaleString("zh-CN")} 的整数 Tokens。`;
    $("modelOutputLimit").focus();
    return;
  }
  const button = $("modelOutputSaveBtn");
  if (button) button.disabled = true;
  if (hint) hint.textContent = "正在保存模型上限并重启核心…";
  try {
    const outputTokens = { ...state.modelOutputTokens, [model]: limit };
    const result = await shellCommand("set_model_config", { model_output_tokens: outputTokens });
    if (!result || !result.ok) throw new Error((result && result.error) || "保存失败");
    state.modelOutputTokens = result.modelOutputTokens || outputTokens;
    state.defaultModelOutputTokens = Number(result.maxOutputTokens) || window.OwoModelOutputBudget.DEFAULT;
    if (window.OwoApi && typeof window.OwoApi.resetCoreConnection === "function") window.OwoApi.resetCoreConnection();
    if (typeof window.owoRecoverService === "function") window.owoRecoverService();
    if (hint) hint.textContent = `${model} 的输出上限已设为 ${limit.toLocaleString("zh-CN")} Tokens，核心已重启。`;
    showToast(`已保存 ${model} 的输出上限：${limit.toLocaleString("zh-CN")} Tokens`, "ok");
  } catch (error) {
    if (hint) hint.textContent = `保存失败：${error.message || error}`;
    showToast(`输出上限保存失败：${error.message || error}`, "error");
  } finally {
    if (button) button.disabled = false;
  }
}

function renderProviderForm() {
  const settings = state.settings || {};
  const provider = settings.provider || {};
  $("providerBaseUrl").value = provider.base_url || "";
  // 密钥只显示状态，不回填明文；已保存时留空即代表"不改动"。
  $("providerApiKey").value = "";
  $("providerApiKey").placeholder = provider.api_key_set ? "已保存（加密）· 留空则不修改" : "未保存";
  const state$ = $("providerKeyState");
  if (provider.api_key_set) {
    state$.textContent = "已保存";
    state$.classList.add("ok");
    state$.classList.remove("warn");
  } else {
    state$.textContent = "未保存";
    state$.classList.add("warn");
    state$.classList.remove("ok");
  }
}

// 保存模型接入：端点 + 模型 + 密钥（留空密钥 = 保留已存值），写本机设置并即时生效。
async function saveProvider() {
  const baseUrl = $("providerBaseUrl").value.trim();
  const apiKey = $("providerApiKey").value.trim();
  if (!baseUrl) {
    // 端点缺失时不能只存密钥：服务端会回落到内置默认 https://api.openai.com/v1，
    // 用户的密钥会被静默打到错误端点（曾实测请求发往 OpenAI 而非用户的服务商）。
    showToast("请填写 API 端点（可从上方服务商预设一键填入）", "error");
    return null;
  }
  try {
    const settings = { ...(state.settings || {}) };
    delete settings.custom_models; // 自定义模型只存本机，不发给服务端
    settings.model = $("settingsModel").value;
    settings.provider = {
      base_url: baseUrl,
      // 留空不下发密钥字段，服务端据此保留加密信封里的旧值。
      ...(apiKey ? { api_key: apiKey } : {}),
    };
    const resp = await api("/settings", {
      method: "POST",
      body: JSON.stringify(settings),
    });
    await refreshSettings();
    renderProviderForm();
    updateModelGate();
    showToast(`模型服务已连接：${settings.model}`, "ok");
    return { ok: true, note: (resp && resp.note) || "ok" };
  } catch (error) {
    showToast(`连接失败：${error.message || error}`, "error");
    return { ok: false };
  }
}

function copyText(text) {
  // 非安全上下文、剪贴板权限拒绝或 Electron 壳限制时，回退到临时 textarea。
  const fallback = () => new Promise((resolve, reject) => {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (_) {
      ok = false;
    }
    document.body.removeChild(area);
    if (ok) resolve();
    else reject(new Error("复制失败"));
  });
  if (!navigator.clipboard || !navigator.clipboard.writeText) return fallback();
  try {
    return Promise.resolve(navigator.clipboard.writeText(text)).catch(fallback);
  } catch (_) {
    return fallback();
  }
}

async function copyPresetCommand() {
  const preset = findPreset($("providerPreset").value);
  const button = $("presetCopyCmdBtn");
  if (!preset || !button) return;
  const command = window.OwoProviderPresets.envCommand(preset);
  if (!command) return;
  const original = button.textContent;
  try {
    await copyText(command);
    button.textContent = "已复制";
  } catch (error) {
    button.textContent = "复制失败，请手动选择";
  }
  setTimeout(() => {
    button.textContent = original;
  }, 1600);
}

function initProviderPresets() {
  const select = $("providerPreset");
  const registry = window.OwoProviderPresets;
  if (!select || !registry) return;
  for (const preset of registry.presets()) {
    const option = document.createElement("option");
    option.value = preset.id;
    option.textContent = preset.label;
    select.appendChild(option);
  }
  select.addEventListener("change", renderPresetInfo);
  $("presetCopyCmdBtn").addEventListener("click", copyPresetCommand);
  $("presetApplyModelBtn").addEventListener("click", async () => {
    const preset = findPreset(select.value);
    if (!preset || !preset.model) return;
    const modelSelect = $("settingsModel");
    if (!modelSelect.querySelector(`option[value="${CSS.escape(preset.model)}"]`)) {
      $("presetHint").textContent = `预设默认模型 ${preset.model} 不在可选列表中，请在终端设置 OPENAI_MODEL 后重启服务。`;
      return;
    }
    modelSelect.value = preset.model;
    const result = await saveSettings({ silent: true });
    $("presetHint").textContent = result.ok
      ? `已保存模型 ${preset.model}（下一回合生效，无需重启；若终端里设了 OPENAI_MODEL，此处会覆盖它）。`
      : result.note;
  });
  renderPresetInfo();
  $("providerSaveBtn").addEventListener("click", saveProvider);
  const gateBtn = $("modelGateBtn");
  if (gateBtn) gateBtn.addEventListener("click", openModelGateSettings);
}

// ---------- 首启门：未接入自己的模型服务前，拦住发送 ----------
//
// 判定以服务端 `GET /settings` 的 `runtime` 段为准（settings_api::effective_runtime_config）：
//   * provider           —— 由 base_url 推断（bigmodel/qwen/deepseek/ollama/openai-compatible）
//   * endpoint_kind      —— "local" | "cloud"
//   * credential_source  —— "not_required"（本地）| "environment" | "missing"
//   * cloud_enabled      —— 出网开关
//
// **不要读 `settings.provider_ready` / `settings.provider.base_url`**：服务端从未返回这两个
// 字段（`Settings` 结构里根本没有 provider 项，provider 信息只以 runtime 投影存在）。
// 读了不存在的字段 → 恒为 undefined → modelGateMissing() 恒 true → **无论用户是否已正确
// 配置并保存模型，发送按钮永远禁用、提示条永远挂着**，且"测试连接通过"也不放行
// （连接测试只做 TCP 探测，不落盘配置）。
function modelGateMissing() {
  if (loadCustomModels().some((item) => item.id === getComposerModel())) return false;
  const settings = state.settings || {};
  const runtime = settings.runtime || {};
  // 显式布尔优先（服务端若将来补上该字段）。
  if (typeof runtime.provider_ready === "boolean") return !runtime.provider_ready;
  if (typeof settings.provider_ready === "boolean") return !settings.provider_ready;
  // 本地端点不需要凭据（Ollama 之类），因此 endpoint_kind=local 即视为就绪。
  if (runtime.endpoint_kind === "local") return false;
  // 云端：拿到凭据即视为就绪（来源可能是环境变量，也可能是设置页保存后写入的）。
  const source = String(runtime.credential_source || "").trim();
  if (source === "environment" || source === "not_required") return false;
  // 兜底：兼容旧形状（若未来 provider 段被提升到顶层）。
  const provider = settings.provider || {};
  if (provider.base_url || provider.api_key_set) return false;
  return true;
}

function updateModelGate() {
  const gate = $("modelGate");
  const missing = modelGateMissing();
  if (gate) {
    gate.classList.toggle("hidden", !missing);
    if (missing) {
      const hint = $("modelGateHint");
      if (hint) {
        const runtime = (state.settings && state.settings.runtime) || {};
        const source = String(runtime.credential_source || "").trim();
        hint.textContent =
          source === "missing"
            ? "服务端报告凭据缺失：填入 API 密钥并保存，或设置 OPENAI_API_KEY 环境变量后重启核心。"
            : runtime.endpoint_kind === "local"
              ? "本地端点已就绪。若仍无法发送，请确认该端点已在「模型」页保存。"
              : "填写你自己的 OpenAI 兼容端点与密钥即可开始，密钥只加密保存在本机。";
      }
    }
  }
  const send = $("sendBtn");
  if (send) send.disabled = missing;
  document.body.classList.toggle("model-gate-active", missing);
}

function openModelGateSettings() {
  setSettingsPageVisible(true);
  setSettingsTab("models");
  showToast("请先连接你自己的模型服务", "error");
}

function formatCount(value) {
  if (value == null || value === "" || !Number.isFinite(Number(value))) return "未知";
  return Number(value).toLocaleString("zh-CN");
}

function formatUsageCost(data) {
  const budget = data?.budget || {};
  const hasPrices = Number(budget.input_price_per_mtok) > 0 && Number(budget.output_price_per_mtok) > 0;
  if (!hasPrices) return "未知（单价未配置）";
  return data.cost_usd == null || !Number.isFinite(Number(data.cost_usd)) ? "未知" : "$" + Number(data.cost_usd).toFixed(4);
}

async function refreshUsage() {
  try {
    const data = await api("/usage");
    const usage = data.usage || {};
    const budget = data.budget || {};
    const cards = [
      { label: "累计 Tokens", value: formatCount(usage.total_tokens) },
      { label: "输入 Tokens", value: formatCount(usage.prompt_tokens) },
      { label: "输出 Tokens", value: formatCount(usage.completion_tokens) },
      { label: "累计估算成本", value: formatUsageCost(data) },
      { label: "Token 预算", value: budget.token_cap != null ? formatCount(budget.token_cap) : "未配置" },
      { label: "成本预算", value: budget.cost_cap_usd != null ? `$${budget.cost_cap_usd}` : "未配置" },
    ];
    const violation = budget.violation
      ? '<div class="usage-alert">已超出预算上限，新的云端请求会被拒绝</div>'
      : "";
    $("usagePanel").innerHTML =
      cards
        .map(
          (card) =>
            `<div class="usage-card"><span class="sub">${esc(card.label)}</span><strong>${esc(card.value)}</strong></div>`
        )
        .join("") + violation;
  } catch (error) {
    $("usagePanel").innerHTML = `<div class="sub">${esc(friendlyError(error))}</div>`;
  }
}

const savedWorkspace = localStorage.getItem("owo.workspace");
if (savedWorkspace) $("workspace").value = savedWorkspace;
function applyTheme(theme) {
  const dark = theme === "dark";
  document.body.classList.toggle("dark-theme", dark);
  const appearanceTheme = $("prefTheme");
  if (appearanceTheme && LOCAL_PREFS.theme !== "system") appearanceTheme.value = dark ? "dark" : "light";
  $("themeToggle").textContent = dark ? "☀" : "☾";
  $("themeToggle").title = dark ? "切换白色主题" : "切换深色主题";
  // 主题按钮用线性图标（与其余图标同一笔触），文字符号在不同字体下粗细不一。
  const themeIcon = dark
    ? '<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="2.9"/><path d="M8 1.8v1.4M8 12.8v1.4M1.8 8h1.4M12.8 8h1.4M3.6 3.6l1 1M11.4 11.4l1 1M3.6 12.4l1-1M11.4 4.6l1-1"/></svg>'
    : '<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" aria-hidden="true"><path d="M13.2 9.6A5.6 5.6 0 0 1 6.4 2.8a5.6 5.6 0 1 0 6.8 6.8z"/></svg>';
  const side = $("themeToggleSide");
  if (side) {
    side.innerHTML = themeIcon;
    side.title = $("themeToggle").title;
    side.setAttribute("aria-label", side.title);
  }
  const bar = $("menubarTheme");
  if (bar) {
    bar.innerHTML = themeIcon;
    bar.title = $("themeToggle").title;
    bar.setAttribute("aria-label", bar.title);
  }
  localStorage.setItem("owo.theme", dark ? "dark" : "light");
}
applyTheme(localStorage.getItem("owo.theme") || "light");
$("themeToggle").addEventListener("click", () => {
  applyTheme(document.body.classList.contains("dark-theme") ? "light" : "dark");
});
function enableResize(handleId, variable, min, max, fromRight = false) {
  const handle = $(handleId);
  handle.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    const railWidth = $("sidebar").getBoundingClientRect().left;
    const move = (next) => {
      const raw = fromRight ? window.innerWidth - next.clientX : next.clientX - railWidth;
      document.body.style.setProperty(variable, `${Math.max(min, Math.min(max, raw))}px`);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  });
}
enableResize("sidebarResize", "--session-width", 220, 460);
enableResize("rightResize", "--inspect-width", 260, 520, true);
const mobileSidebarToggle = $("mobileSidebarToggle");
const mobileSidebarBackdrop = $("mobileSidebarBackdrop");
function setMobileSidebarOpen(open) {
  const visible = Boolean(open) && window.matchMedia("(max-width: 860px)").matches;
  document.body.classList.toggle("mobile-sidebar-open", visible);
  mobileSidebarToggle.setAttribute("aria-expanded", String(visible));
  mobileSidebarToggle.setAttribute("aria-label", visible ? "关闭会话侧栏" : "打开会话侧栏");
}
mobileSidebarToggle.addEventListener("click", () => {
  setMobileSidebarOpen(!document.body.classList.contains("mobile-sidebar-open"));
});
mobileSidebarBackdrop.addEventListener("click", () => setMobileSidebarOpen(false));
document.addEventListener("click", (event) => {
  if (!document.body.classList.contains("mobile-sidebar-open")) return;
  if (event.target.closest("#sidebar #sessionList li:not(.codex-session-group), #sidebar .codex-nav button, #sidebar [data-codex-group]")) {
    setMobileSidebarOpen(false);
  }
});
window.addEventListener("resize", () => {
  if (!window.matchMedia("(max-width: 860px)").matches) setMobileSidebarOpen(false);
});
// 会话、工具与设置三态由 core/workbench-view.js 唯一维护；本文件保留兼容调用名。
const workbenchView = window.OwoWorkbenchView.create({
  body: document.body,
  toggleButton: $("toggleTools"),
  clearToolGroups: () => clearCodexGroup(),
  scrollSettings: () => document.querySelector("#sidebar section.settings-section")?.scrollIntoView({ block: "start" }),
  replaceRoute: (route) => window.OwoWorkbenchView.replaceRouteHash(window.location, window.history, route, true),
  leavePanel: () => unmountActivePanel(),
});
function setToolsVisible(visible, preserveRoute) {
  workbenchView.showTools(visible, preserveRoute);
}
function setSettingsPageVisible(visible) {
  workbenchView.showSettings(visible);
}
// toggleTools 仅为状态机保留（视觉入口为 codexToolsEntry / sidebarToolsBtn）
$("toggleTools").addEventListener("click", () => workbenchView.toggleTools());
const CODEX_GROUPS = ["workspace", "intelligence", "automation", "system"];
function clearCodexGroup() {
  window.OwoWorkbenchView.clearGroupFilters(document.body, CODEX_GROUPS);
  syncToolsJump();
}

// Codex 侧栏导航：进入工具视图并聚焦对应分组；再点一次返回会话。

// 工具视图吸顶导航：点分组只看该组卡片（长页面立刻变短），点「全部」恢复所有卡片。
function syncToolsJump() {
  for (const button of document.querySelectorAll("[data-jump]")) {
    const target = button.dataset.jump;
    const active =
      target === "all"
        ? !CODEX_GROUPS.some((group) => document.body.classList.contains(`tools-group-${group}`))
        : document.body.classList.contains(`tools-group-${target}`);
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", String(active));
  }
}
for (const button of document.querySelectorAll("[data-jump]")) {
  button.addEventListener("click", () => {
    const target = button.dataset.jump;
    clearCodexGroup();
    if (target !== "all") document.body.classList.add(`tools-group-${target}`);
    setToolsVisible(true);
    if (target === "system" || target === "all") void refreshPluginSkillOverview();
    syncToolsJump();
    // #sidebar owns the tools-page scroll. Reset its own scroll offset when
    // switching groups so the selected section starts below the sticky jump bar.
    const sidebar = $("sidebar");
    if (sidebar) sidebar.scrollTop = 0;
  });
}
function refreshPluginSkillOverview() {
  // 插件概览不属于启动关键请求：进入工具页时再加载，避免无关启动开销。
  return Promise.allSettled([
    refreshPlugins(),
    refreshPluginMarket(),
    refreshSkills(),
    refreshSkillHealth(),
  ]);
}
function openToolsView() {
  clearCodexGroup();
  setToolsVisible(true);
  void refreshPluginSkillOverview();
}
for (const button of document.querySelectorAll("[data-codex-group]")) {
  button.addEventListener("click", () => {
    const group = button.dataset.codexGroup;
    const focused = document.body.classList.contains(`tools-group-${group}`);
    clearCodexGroup();
    if (focused) {
      // 已聚焦该分组 → 返回会话
      setSettingsPageVisible(false);
      return;
    }
    document.body.classList.add(`tools-group-${group}`);
    button.classList.add("active");
    setToolsVisible(true);
    if (group === "system") void refreshPluginSkillOverview();
    syncToolsJump();
  });
}
for (const button of document.querySelectorAll("[data-workbench-panel]")) {
  button.addEventListener("click", () => openPanelById(button.dataset.workbenchPanel));
}
function bindToolsEntry(id) {
  $(id).addEventListener("click", () => {
    if (id === "codexToolsEntry") { openWorkbenchSearch(); return; }
    if (document.body.classList.contains("tools-open")) setSettingsPageVisible(false);
    else openToolsView();
  });
}
bindToolsEntry("codexToolsEntry");
bindToolsEntry("sidebarToolsBtn");
$("codexSettingsEntry").addEventListener("click", () => setSettingsPageVisible(true));
$("codexViewBack").addEventListener("click", () => setSettingsPageVisible(false));
$("inspectorToggle").addEventListener("click", () => setTaskDetailsVisible(!document.body.classList.contains("inspector-open")));
// 功能页内主侧栏常驻：在功能页点会话或「新对话」，先回到对话视图，再交给原有处理器。
document.addEventListener("click", (event) => {
  if (!document.body.classList.contains("panel-open")) return;
  const target = event.target instanceof Element ? event.target : null;
  if (target && target.closest("#sidebar .codex-sidebar [data-session-select], #sidebar .codex-sidebar #newSession")) {
    setSettingsPageVisible(false);
  }
}, true);
$("taskStatusPill").addEventListener("click", () => {
  if (document.body.classList.contains("tools-open") || document.body.classList.contains("settings-open")) setSettingsPageVisible(false);
  setTaskDetailsVisible(true);
});
$("taskDetailsClose").addEventListener("click", () => { setTaskDetailsVisible(false); $("inspectorToggle").focus(); });
$("taskDetailReview").addEventListener("click", () => {
  setTaskDetailsVisible(false);
  const target = state.pendingQuestion ? document.querySelector(".msg.question") : $("approvalBar");
  target?.scrollIntoView({ block: "nearest" }); target?.querySelector("button,input,textarea")?.focus();
});
$("taskAdvanced").addEventListener("toggle", () => {
  if ($("taskAdvanced").open) void Promise.allSettled([refreshPerception(), refreshSuggestions(), refreshObservations(), refreshAudit()]);
});
document.addEventListener("click", event => {
  const connection = $("connectionDetails");
  if (connection.open && !connection.contains(event.target)) connection.open = false;
});

$("themeToggleSide").addEventListener("click", () => {
  applyTheme(document.body.classList.contains("dark-theme") ? "light" : "dark");
});

// ---------- 全局菜单栏（文件/编辑/视图/帮助 + 左下用户卡菜单） ----------

function showToast(text, kind = "") {
  let root = $("toastRoot");
  if (!root) {
    root = document.createElement("div");
    root.id = "toastRoot";
    document.body.appendChild(root);
  }
  const toast = document.createElement("div");
  toast.className = `toast${kind ? ` toast-${kind}` : ""}`;
  toast.textContent = text;
  root.appendChild(toast);
  setTimeout(() => {
    toast.style.opacity = "0";
    toast.style.transition = "opacity .3s";
    setTimeout(() => toast.remove(), 320);
  }, 2600);
}

// 菜单项图标（与侧栏同一套 16px 线性图标）
const MENU_ICONS = {
  newChat:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M8 3.2v9.6M3.2 8h9.6"/></svg>',
  folder:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M2.2 4.4c0-.66.54-1.2 1.2-1.2h2.3l1.4 1.6h4.5c.66 0 1.2.54 1.2 1.2v5.6c0 .66-.54 1.2-1.2 1.2H3.4c-.66 0-1.2-.54-1.2-1.2V4.4z"/></svg>',
  download:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2.8v8M4.8 7.4 8 10.6l3.2-3.2M2.8 13.2h10.4"/></svg>',
  focus:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M2.4 8h11.2M8 2.4v11.2"/><circle cx="8" cy="8" r="5.6"/></svg>',
  copy:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><rect x="5.4" y="5.4" width="8.2" height="8.2" rx="1.4"/><path d="M10.6 5.4V3.8c0-.66-.54-1.2-1.2-1.2H3.8c-.66 0-1.2.54-1.2 1.2v5.6c0 .66.54 1.2 1.2 1.2h1.6"/></svg>',
  eraser:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M6.4 13.2h6.4M2.8 10.4l5.2-5.2 3.6 3.6-5.2 5.2H4.4l-1.6-1.6z"/></svg>',
  sidebar:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2.2" y="2.8" width="11.6" height="10.4" rx="1.6"/><path d="M6.4 2.8v10.4"/></svg>',
  inspector:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2.2" y="2.8" width="11.6" height="10.4" rx="1.6"/><path d="M9.6 2.8v10.4"/></svg>',
  moon:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M13.4 9.6A5.8 5.8 0 0 1 6.4 2.6a5.8 5.8 0 1 0 7 7z"/></svg>',
  expand:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6V3h3M13 10v3h-3M3 10v3h3M13 6V3h-3"/></svg>',
  tools:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6"><rect x="2.2" y="2.2" width="5" height="5" rx="1.2"/><rect x="8.8" y="2.2" width="5" height="5" rx="1.2"/><rect x="2.2" y="8.8" width="5" height="5" rx="1.2"/><rect x="8.8" y="8.8" width="5" height="5" rx="1.2"/></svg>',
  keyboard:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4"><rect x="1.8" y="4.2" width="12.4" height="7.6" rx="1.4"/><path d="M4.4 6.4h.01M6.6 6.4h.01M8.8 6.4h.01M11 6.4h.01M4.4 9h7.2"/></svg>',
  privacy:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M8 2.2 13 4v4c0 3-2.2 4.6-5 5.6-2.8-1-5-2.6-5-5.6V4z"/><path d="M5.8 8l1.6 1.6 3-3.2"/></svg>',
  book:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M2.6 3.4c1.8-.8 3.6-.8 5.4 0v9.4c-1.8-.8-3.6-.8-5.4 0zM13.4 3.4c-1.8-.8-3.6-.8-5.4 0v9.4c1.8-.8 3.6-.8 5.4 0z"/></svg>',
  info:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="5.8"/><path d="M8 7.2v4M8 4.9h.01"/></svg>',
  gear:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="2.1"/><path d="M8 2.6v1.6M8 11.8v1.6M3.5 3.5l1.1 1.1M11.4 11.4l1.1 1.1M2.6 8h1.6M11.8 8h1.6M3.5 12.5l1.1-1.1M11.4 4.6l1.1-1.1"/></svg>',
  chart:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M2.8 13.2V6.4M7.2 13.2V2.8M11.6 13.2V9"/></svg>',
  refresh:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M13.2 8a5.2 5.2 0 1 1-1.6-3.7M13.4 2.6v3h-3"/></svg>',
  logout:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M6.4 13.2H3.6c-.66 0-1.2-.54-1.2-1.2V4c0-.66.54-1.2 1.2-1.2h2.8M10.4 11 13.2 8l-2.8-3M13.2 8H6"/></svg>',
};

const menuPopover = $("menuPopover");
let menuAnchor = null;

function buildMenuItems(items) {
  const frag = document.createDocumentFragment();
  for (const item of items) {
    if (item.sep) {
      const sep = document.createElement("div");
      sep.className = "menu-sep";
      frag.appendChild(sep);
      continue;
    }
    if (item.title) {
      const title = document.createElement("div");
      title.className = "menu-group-title";
      title.textContent = item.title;
      frag.appendChild(title);
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "menu-item";
    button.setAttribute("role", "menuitem");
    if (item.disabled) button.disabled = true;
    button.innerHTML =
      `${item.icon || ""}<span class="menu-label">${esc(item.label)}</span>` +
      `${item.shortcut ? `<span class="menu-shortcut">${esc(item.shortcut)}</span>` : ""}` +
      `${item.hint ? `<span class="menu-hint">${esc(item.hint)}</span>` : ""}`;
    button.addEventListener("click", () => {
      closeMenu();
      if (item.action) item.action();
    });
    frag.appendChild(button);
  }
  return frag;
}

function openMenuAt(anchor, items) {
  menuPopover.innerHTML = "";
  menuPopover.appendChild(buildMenuItems(items));
  menuPopover.classList.remove("hidden");
  menuPopover.setAttribute("aria-hidden", "false");
  const rect = anchor.getBoundingClientRect();
  const menuRect = menuPopover.getBoundingClientRect();
  let left = rect.left;
  let top = rect.bottom + 4;
  if (left + menuRect.width > window.innerWidth - 8) {
    left = Math.max(8, window.innerWidth - menuRect.width - 8);
  }
  if (top + menuRect.height > window.innerHeight - 8) {
    top = Math.max(8, rect.top - menuRect.height - 4);
  }
  menuPopover.style.left = `${left}px`;
  menuPopover.style.top = `${top}px`;
  menuAnchor = anchor;
  for (const button of document.querySelectorAll(".menubar-menu-btn")) {
    button.classList.toggle("open", button === anchor);
  }
}

function closeMenu() {
  menuPopover.classList.add("hidden");
  menuPopover.setAttribute("aria-hidden", "true");
  menuPopover.innerHTML = "";
  menuAnchor = null;
  for (const button of document.querySelectorAll(".menubar-menu-btn")) {
    button.classList.remove("open");
  }
}

// 打开系统原生文件夹选择器（由同机核心服务代劳；取消返回空路径）。
// 服务暂不支持该端点时退回手动打开工作区分组，保证旧版本可用。
function shellInvoke(name, args) {
  const bridge = window.OwoApiClient;
  const owner = bridge && typeof bridge.tauriInvokeOwner === "function"
    ? bridge.tauriInvokeOwner(window)
    : null;
  if (!owner || typeof owner.invoke !== "function") return Promise.resolve(null);
  return Promise.resolve(owner.invoke.call(owner, name, args || {}));
}

const workspaceSelection = window.OwoWorkspaceRouting.createWorkspaceSelectionController(
  async (path) => {
    return window.OwoWorkspaceRouting.requireWorkspacePersistence(
      await shellInvoke("set_workspace", { path }),
      "当前页面没有连接桌面工作区服务，未切换本地目录。请在 Electron 工作台中重试。",
    );
  },
  (path) => {
    const input = $("workspace");
    input.value = path;
    input.dispatchEvent(new Event("change"));
    rememberWorkspace(path);
    syncProjectChip();
  }
);

function recentWorkspaces() {
  try {
    const stored = JSON.parse(localStorage.getItem("owo.recentWorkspaces") || "[]");
    return Array.isArray(stored)
      ? stored.filter((item) => typeof item === "string" && item.trim()).slice(0, 6)
      : [];
  } catch (_) {
    return [];
  }
}

function rememberWorkspace(target) {
  const value = String(target || "").trim();
  if (!value) return;
  const items = [value].concat(recentWorkspaces().filter(
    (item) => item.toLowerCase() !== value.toLowerCase()
  )).slice(0, 6);
  try {
    localStorage.setItem("owo.recentWorkspaces", JSON.stringify(items));
  } catch (_) {}
}

async function persistWorkspace(target, revision) {
  const result = await workspaceSelection.select(target, revision);
  return result.latest;
}

async function pickDirectory() {
  const result = await window.OwoFolderPicker.pick(window);
  if (result.canceled) return null;
  if (!result.ok) {
    const message = result.unavailable
      ? "浏览器预览无法访问本机目录，请在 OwO Agent Electron 工作台中选择。"
      : (result.error || "选择器无响应");
    showToast("打开文件夹选择器失败：" + message, "error");
    return null;
  }
  const workspace = result.workspace;
  const input = $("workspace");
  input.value = workspace;
  input.dataset.path = workspace;
  input.title = "当前项目：" + workspace;
  localStorage.setItem("owo.workspace", workspace);
  rememberWorkspace(workspace);
  syncProjectChip();
  showToast("工作区已切换；当前会话仍保留原工作区", "ok");
  return workspace;
}
function openWorkspaceMenu(trigger) {
  const anchor = trigger || $("sidebarWorkspaceBtn") || $("composerProjectBtn");
  const current = $("workspace").value.trim();
  const nativeWorkspace = Boolean(window.OwoFolderPicker?.isNativeAvailable(window));
  const actionDisabled = nativeWorkspace ? "" : ' disabled aria-disabled="true"';
  const recent = recentWorkspaces().filter((item) => item.toLowerCase() !== current.toLowerCase());
  const currentName = current ? current.split(/[\\/]/).filter(Boolean).pop() : "未选择项目";
  let html = '<div class="composer-project-current"><strong>' + esc(currentName) +
    '</strong><small>' + esc(current || "尚未选择工作区") + '</small></div>' +
    '<button type="button" class="composer-menu-item project-action" data-project-action="create"' + actionDisabled + '>＋ 新建项目文件夹…</button>' +
    '<button type="button" class="composer-menu-item project-action" data-project-action="switch"' + actionDisabled + '>▱ 切换到已有文件夹…</button>' +
    '<div class="composer-menu-group">最近工作区</div>';
  if (!nativeWorkspace) {
    html += '<div class="composer-project-note" role="status">浏览器预览不能访问本地目录；请在 OwO Agent Electron 工作台中选择或切换工作区。</div>';
  } else if (recent.length) {
    html += recent.map((item) =>
      '<button type="button" class="composer-menu-item project-recent" data-project-path="' + esc(item) + '"' + actionDisabled +
      ' title="' + esc(item) + '"><span>' + esc(item.split(/[\\/]/).filter(Boolean).pop() || item) + '</span></button>'
    ).join("");
  } else {
    html += '<div class="composer-project-note">新会话会在所选目录中运行；当前会话的工作区不会变更。</div>';
  }
  openComposerMenu(anchor, html, (menu) => {
    if (anchor) anchor.setAttribute("aria-expanded", "true");
    menu.classList.add("composer-menu-project");
    menu.dataset.owner = "project";
    menu.querySelector('[data-project-action="switch"]').addEventListener("click", () => {
      closeComposerMenu();
      pickDirectory();
    });
    menu.querySelector('[data-project-action="create"]').addEventListener("click", () => {
      closeComposerMenu();
      createProjectWorkspace();
    });
    for (const button of menu.querySelectorAll("[data-project-path]")) {
      button.addEventListener("click", async () => {
        closeComposerMenu();
        try {
          await persistWorkspace(button.dataset.projectPath);
          showToast("已切换工作区；新会话会使用该目录", "ok");
        } catch (error) {
          showToast("工作区切换失败：" + friendlyError(error), "error");
        }
      });
    }
  });
  if (composerMenuEl) composerMenuEl.dataset.owner = "project";
}

async function createProjectWorkspace() {
  const name = await askText({
    title: "新建项目工作区",
    label: "输入新项目文件夹名称，然后选择保存位置。",
    placeholder: "例如 my-app",
    confirmText: "选择保存位置",
    required: true,
  });
  if (!name) return;
  const revision = workspaceSelection.begin();
  let phase = "create";
  try {
    // 先只创建目录；激活工作区由有序选择队列完成，避免 IPC 与用户切换竞争。
    const result = await shellInvoke("create_project_workspace", { name, activate: false });
    if (!result) throw new Error("新建项目需要在 Electron 桌面版中使用");
    if (result.canceled) return;
    if (!result.ok) throw new Error(result.error || "创建项目目录失败");
    phase = "activate";
    rememberWorkspace(result.path);
    if (!workspaceSelection.isCurrent(revision)) {
      showToast("项目目录已创建；工作区已另行切换，新项目未自动打开。", "ok");
      return;
    }
    const applied = await persistWorkspace(result.path, revision);
    if (!applied) {
      showToast("项目目录已创建；工作区已另行切换，新会话未创建。", "ok");
      return;
    }
    // Keep a model explicitly selected for the next session; newSession consumes and clears it.
    phase = "session";
    await newSession();
    showToast("项目工作区已创建并打开：" + result.path, "ok");
  } catch (error) {
    if (workspaceSelection.isCurrent(revision)) {
      const message = window.OwoWorkspaceRouting.projectCreationFailureMessage(phase, error);
      showToast(message, "error");
    }
  }
}

function copyLastReply() {
  const replies = document.querySelectorAll("#messages .msg.assistant");
  const last = replies[replies.length - 1];
  if (!last) {
    showToast("当前会话还没有助手回复");
    return;
  }
  copyText(last.innerText)
    .then(() => showToast("已复制最后一条回复", "ok"))
    .catch(() => showToast("复制失败，请手动选择", "error"));
}

const MENU_DEFS = {
  file: () => [
    { icon: MENU_ICONS.newChat, label: "新对话", shortcut: "Ctrl+N", action: () => newSession() },
    { icon: MENU_ICONS.folder, label: "打开文件夹…", shortcut: "Ctrl+O", action: () => pickDirectory() },
    { sep: true },
    {
      icon: MENU_ICONS.download,
      label: "导出会话 Markdown",
      disabled: !state.sessionId,
      action: () => exportSession("md"),
    },
    {
      icon: MENU_ICONS.download,
      label: "导出会话 HTML",
      disabled: !state.sessionId,
      action: () => exportSession("html"),
    },
  ],
  edit: () => [
    { icon: MENU_ICONS.focus, label: "聚焦输入框", shortcut: "Ctrl+L", action: () => $("prompt").focus() },
    { icon: MENU_ICONS.copy, label: "复制最后一条回复", action: copyLastReply },
    { icon: MENU_ICONS.eraser, label: "清空输入框", action: () => { $("prompt").value = ""; $("prompt").focus(); } },
  ],
  view: () => [
    {
      icon: MENU_ICONS.sidebar,
      label: document.body.classList.contains("sidebar-collapsed") ? "显示会话栏" : "隐藏会话栏",
      shortcut: "Ctrl+B",
      action: () => document.body.classList.toggle("sidebar-collapsed"),
    },
    {
      icon: MENU_ICONS.inspector,
      label: "检查器（情景/审计/diff）",
      action: () => $("inspectorToggle").click(),
    },
    {
      icon: MENU_ICONS.moon,
      label: document.body.classList.contains("dark-theme") ? "切换到浅色主题" : "切换到深色主题",
      action: () => applyTheme(document.body.classList.contains("dark-theme") ? "light" : "dark"),
    },
    { sep: true },
    { icon: MENU_ICONS.expand, label: "全屏 / 窗口", shortcut: "F11", action: () => toggleFullscreen() },
    { icon: MENU_ICONS.tools, label: "工具与设置", action: () => openToolsView() },
  ],
  help: () => [
    {
      icon: MENU_ICONS.keyboard,
      label: "快捷键与帮助",
      action: () => openAboutPanel(),
    },
    { icon: MENU_ICONS.book, label: "OpenAPI 文档", action: () => window.open("openapi.json", "_blank", "noopener") },
    { icon: MENU_ICONS.privacy, label: "隐私声明", action: () => window.open("privacy.md", "_blank", "noopener") },
    { sep: true },
    { icon: MENU_ICONS.info, label: "关于 OwO Agent", action: () => showAboutVersion() },
  ],
};

// 打开「帮助与关于」面板（快捷键表 / 能力面速览 / 文档 / 许可 / 诊断）。
function openAboutPanel() {
  clearCodexGroup();
  setSettingsPageVisible(false);
  setToolsVisible(true);
  mountPanel("about");
}

// 关于：版本号取自 /health，避免前端硬编码与后端漂移。
function showAboutVersion() {
  api("/health", { public: true })
    .then((health) => showToast(`OwO Agent 工作台 · 本地优先 · v${(health && health.version) || "未知"}`, ""))
    .catch(() => showToast("OwO Agent 工作台 · 本地优先", ""));
}

function toggleFullscreen() {
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => {});
  } else {
    document.documentElement.requestFullscreen().catch(() => showToast("当前环境不允许全屏", "error"));
  }
}

for (const button of document.querySelectorAll(".menubar-menu-btn")) {
  const name = button.dataset.menu;
  const open = () => {
    openMenuAt(button, MENU_DEFS[name]());
    for (const other of document.querySelectorAll(".menubar-menu-btn")) {
      other.classList.toggle("open", other === button);
    }
  };
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    if (menuAnchor === button) closeMenu();
    else open();
  });
  // 桌面端惯例：菜单已打开时，悬停其他菜单直接切换
  button.addEventListener("mouseenter", () => {
    if (menuAnchor && menuAnchor !== button) open();
  });
}

// 左下用户卡菜单（设置 / 使用情况和计费 / 重连 / 退出登录）
const userCard = $("codexUserCard");
function openUserMenu() {
  openMenuAt(userCard, [
    { title: "用户1 · 本地账户" },
    {
      icon: MENU_ICONS.gear,
      label: "设置",
      shortcut: "Ctrl+,",
      action: () => setSettingsPageVisible(true),
    },
    {
      icon: MENU_ICONS.chart,
      label: "使用情况和计费",
      hint: "设置页",
      action: () => {
        setSettingsPageVisible(true);
        setTimeout(() => $("usagePanel")?.scrollIntoView({ block: "center", behavior: "smooth" }), 80);
      },
    },
    { sep: true },
    {
      icon: MENU_ICONS.refresh,
      label: "重新连接服务",
      action: reconnectService,
    },
    {
      icon: MENU_ICONS.logout,
      label: "退出登录（清除本地配对）",
      action: () =>
        confirmModal({
          title: "退出登录",
          message: "将清除本机配对信息，下次操作时重新连接服务。设置与数据不受影响。",
          confirmText: "退出登录",
          kind: "danger",
          onConfirm: logoutLocal,
        }),
    },
  ]);
}
userCard.addEventListener("click", (event) => {
  if (event.target.closest("#themeToggleSide")) return; // 主题按钮独立处理
  event.stopPropagation();
  if (menuAnchor === userCard) closeMenu();
  else openUserMenu();
});
userCard.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    userCard.click();
  }
});

// 点击空白处 / Esc 关闭菜单
document.addEventListener("click", (event) => {
  if (!menuAnchor) return;
  if (menuPopover.contains(event.target)) return;
  if (event.target.closest(".menubar-menu-btn") || event.target.closest("#codexUserCard")) return;
  closeMenu();
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && menuAnchor) closeMenu();
});

// 全局快捷键：Ctrl+N 新对话 / Ctrl+O 打开文件夹 / Ctrl+B 会话栏 / Ctrl+, 设置 / F11 全屏
document.addEventListener("keydown", (event) => {
  if (!(event.ctrlKey || event.metaKey)) {
    if (event.key === "F11") {
      event.preventDefault();
      toggleFullscreen();
    }
    return;
  }
  const key = event.key.toLowerCase();
  if (key === "k") {
    event.preventDefault();
    openWorkbenchSearch();
  } else if (key === "r") {
    event.preventDefault();
    if (state.activeTurns.size) showToast("任务运行中，请结束任务后刷新界面");
    else window.location.reload();
  } else if (key === "n") {
    event.preventDefault();
    newSession().catch((error) => addMessage("error", `创建会话失败：${error.message || error}`));
  } else if (key === "o") {
    event.preventDefault();
    pickDirectory();
  } else if (key === "b") {
    event.preventDefault();
    document.body.classList.toggle("sidebar-collapsed");
  } else if (key === ",") {
    event.preventDefault();
    setSettingsPageVisible(true);
  }
});

// 菜单栏主题按钮与连接状态
$("menubarTheme").addEventListener("click", () => {
  applyTheme(document.body.classList.contains("dark-theme") ? "light" : "dark");
});

// 工作区设置使用与侧栏/首启引导相同的原生选择器和 Electron IPC。
const workspaceFolderPicker = window.OwoFolderPicker.attach(
  $("workspace"),
  $("workspaceBrowseBtn"),
  {
    displayAlias: false,
    title: "选择项目工作区（Electron 原生目录选择器）",
    onPicked(workspace) {
      localStorage.setItem("owo.workspace", workspace);
      rememberWorkspace(workspace);
      syncProjectChip();
      refreshWorkspaceTrust();
      showToast("工作区已切换；当前会话仍保留原工作区", "ok");
    },
    onError(message) {
      showToast("工作区选择失败：" + message, "error");
    },
  },
);
const workspacePickerHint = $("workspacePickerHint");
if (workspacePickerHint) {
  workspacePickerHint.textContent = workspaceFolderPicker.native
    ? "切换项目目录后，新会话使用新目录；已有会话保留原项目。"
    : "浏览器预览无法访问本机目录。请在 OwO Agent Electron 工作台中选择或切换工作区。";
}
// composer 项目 chip 与工作区菜单均复用同一个原生目录选择器。

$("composerProjectBtn").addEventListener("click", () => {
  if (composerMenuEl && composerMenuEl.dataset.owner === "project") closeComposerMenu();
  else openWorkspaceMenu($("composerProjectBtn"));
});
$("emptyStateWorkspaceBtn").addEventListener("click", () => openWorkspaceMenu($("composerProjectBtn")));
$("sidebarWorkspaceBtn").addEventListener("click", () => {
  if (composerMenuEl && composerMenuEl.dataset.owner === "project") closeComposerMenu();
  else openWorkspaceMenu($("sidebarWorkspaceBtn"));
});
// 会话搜索：图标按钮切换输入框，输入实时过滤
$("sidebarSearchBtn").addEventListener("click", () => {
  const input = $("sessionSearch");
  input.hidden = !input.hidden;
  if (input.hidden) {
    input.value = "";
    filterSessionList();
  } else {
    input.focus();
  }
});
$("sessionSearch").addEventListener("input", filterSessionList);
function syncProjectChip() {
  const value = $("workspace").value.trim();
  const name = value ? value.split(/[\\/]/).filter(Boolean).pop() : "";
  const displayName = name || "未选择项目";
  if ($("composerProjectName")) $("composerProjectName").textContent = displayName;
  if ($("sidebarWorkspaceName")) $("sidebarWorkspaceName").textContent = displayName;
  const title = value ? "当前工作区：" + value : "未设置工作区（点击选择）";
  if ($("composerProjectBtn")) $("composerProjectBtn").title = title;
  if ($("sidebarWorkspaceBtn")) $("sidebarWorkspaceBtn").title = title;
  const headerProject = document.querySelector(".project-label strong");
  if (headerProject) headerProject.textContent = displayName;
  const needsWorkspace = !value && !state.sessionId;
  const emptyState = $("emptyState");
  if (emptyState) emptyState.classList.toggle("needs-workspace", needsWorkspace);
  const emptyTitle = $("emptyStateTitle");
  if (emptyTitle) emptyTitle.textContent = needsWorkspace ? "先选择一个项目工作区" : "你想完成什么？";
  const emptyDescription = $("emptyStateDescription");
  if (emptyDescription) emptyDescription.textContent = needsWorkspace
    ? "新任务需要一个工作目录。选择已有文件夹，或创建一个新项目文件夹。"
    : "直接描述目标，或先定义范围与执行方式。已有结果可在产物与证据中查看。";
  const emptyWorkspaceButton = $("emptyStateWorkspaceBtn");
  if (emptyWorkspaceButton) emptyWorkspaceButton.hidden = !needsWorkspace;
}

// 审查 N1：工作区信任（GET/POST /workspace/trust）。未信任时后端会跳过
// hooks / 插件 MCP / 权限放宽 / 白名单；这里把状态与"将加载什么"讲清楚，
// 避免用户把"配置没生效"误判成功能坏了。
async function refreshWorkspaceTrust() {
  const hint = $("workspaceTrustHint");
  const button = $("workspaceTrustBtn");
  if (!hint || !button) return;
  try {
    const status = await api("/workspace/trust");
    const reasons = Array.isArray(status.reasons) ? status.reasons : [];
    if (status.trusted) {
      hint.textContent = status.gated_at_start
        ? "已信任。hooks/权限已即时生效；插件 MCP 子进程需重启核心后加载。"
        : "已信任这份配置。";
      button.textContent = "撤销信任";
      button.classList.remove("primary");
    } else if (status.gated) {
      hint.textContent =
        (status.changed ? "配置内容已变化，需要重新确认。" : "未信任：已跳过工作区 hooks / 插件 MCP / 权限放宽。") +
        (reasons.length ? " 待加载：" + reasons.join("；") : "");
      button.textContent = "信任此工作区";
      button.classList.add("primary");
    } else {
      hint.textContent = "该工作区没有会自动执行的配置（无需确认）。";
      button.textContent = "信任此工作区";
      button.classList.add("primary");
    }
    button.dataset.trusted = status.trusted ? "1" : "";
    button.disabled = false;
  } catch (error) {
    hint.textContent = "无法读取信任状态：" + friendlyError(error);
    button.disabled = true;
  }
}

async function toggleWorkspaceTrust() {
  const button = $("workspaceTrustBtn");
  if (!button) return;
  const trusted = button.dataset.trusted !== "1";
  button.disabled = true;
  try {
    await api("/workspace/trust", {
      method: "POST",
      body: JSON.stringify({ trusted }),
    });
    showToast(
      trusted ? "已信任此工作区（hooks/权限即时生效，插件需重启核心）" : "已撤销信任（hooks 已停用）",
      trusted ? "ok" : "warn",
    );
    await refreshWorkspaceTrust();
  } catch (error) {
    showToast("信任操作失败：" + friendlyError(error), "error");
  } finally {
    button.disabled = false;
  }
}
$("workspace").addEventListener("change", () => {
  const workspace = $("workspace").value.trim();
  if (workspace) {
    localStorage.setItem("owo.workspace", workspace);
    rememberWorkspace(workspace);
  }
  syncProjectChip();
  refreshWorkspaceTrust();
});
$("workspaceTrustRefreshBtn")?.addEventListener("click", () => refreshWorkspaceTrust());
$("workspaceTrustBtn")?.addEventListener("click", () => toggleWorkspaceTrust());
$("newSession").addEventListener("click", () => {
  newSession().catch((error) =>
    error && error.userFacing
      ? showToast(error.message, "error")
      : addMessage("error", `创建会话失败：${friendlyError(error)}`)
  );
});
$("showArchived").addEventListener("change", () => refreshSessions(state.sessionId));
$("attachmentBtn").addEventListener("click", () => $("attachmentInput").click());
$("attachmentInput").addEventListener("change", () => uploadAttachments($("attachmentInput").files));
$("chatForm").addEventListener("submit", (event) => {
  event.preventDefault();
  sendPrompt();
});
// 审批队列：事件委托（多张卡各自带 request_id，允许/拒绝按类名区分）。
$("approvalList").addEventListener("click", (event) => {
  const button = event.target.closest("button[data-rid]");
  if (!button) return;
  const allow = button.classList.contains("allow");
  const row = button.closest(".approval-item");
  const scope = row && row.querySelector(".approval-scope");
  respondApproval(button.dataset.rid, allow, allow && scope ? scope.value : "once");
});
// 发送钮双态：空闲=发送，回合进行中=中断（Codex 行为）
$("sendBtn").addEventListener("click", () => {
  // 双态按钮按「当前会话」判定：本会话在跑 = 停止，否则 = 发送。
  if (currentTurn()) {
    abortTurn();
    return;
  }
  sendPrompt();
});
$("modelChip").addEventListener("click", () => {
  if (composerMenuEl && composerMenuEl.dataset.owner === "model") {
    closeComposerMenu();
    return;
  }
  openModelMenu();
  if (composerMenuEl) composerMenuEl.dataset.owner = "model";
});
$("effortChip").addEventListener("click", () => {
  if (composerMenuEl && composerMenuEl.dataset.owner === "effort") {
    closeComposerMenu();
    return;
  }
  openEffortMenu();
  if (composerMenuEl) composerMenuEl.dataset.owner = "effort";
});
$("accessChip").addEventListener("click", () => {
  if (composerMenuEl && composerMenuEl.dataset.owner === "access") {
    closeComposerMenu();
    return;
  }
  openAccessMenu();
  if (composerMenuEl) composerMenuEl.dataset.owner = "access";
});
$("revertBtn").addEventListener("click", revertAll);
$("auditFilterBtn").addEventListener("click", () => refreshAudit());
$("learnStart").addEventListener("click", () => learnControl("start"));
$("learnPause").addEventListener("click", () => learnControl("pause"));
$("learnResume").addEventListener("click", () => learnControl("resume"));
$("learnStop").addEventListener("click", () => learnControl("stop"));
$("learnClear").addEventListener("click", () => learnControl("clear"));
$("sinkForm").addEventListener("submit", (event) => {
  event.preventDefault();
  sinkSkill();
});
$("skillImportBtn").addEventListener("click", () => {
  const file = $("skillImport").files[0];
  if (!file) {
    showToast("请先选择 .owskill 文件", "error");
    return;
  }
  importPackage(file);
  $("skillImport").value = "";
});
// 自动化已迁入扩展面板：分组页卡片只留入口按钮。
$("openAutomationsPanel").addEventListener("click", () => {
  openPanelById("automations");
});
$("egressToggle").addEventListener("click", async () => {
  const enabled = $("egressToggle").dataset.enabled !== "true";
  try {
    await api("/settings/egress", {
      method: "POST",
      body: JSON.stringify({ cloud_enabled: enabled }),
    });
    await refreshSettings();
    showToast(`云端模型已${enabled ? "开启" : "关闭"}（已即时生效）`, "ok");
  } catch (error) {
    showToast(`切换云端模型失败：${error.message || error}`, "error");
  }
});
$("storageBackupBtn").addEventListener("click", () => storageBackup());
$("storageExportBtn").addEventListener("click", () => storageExport());
const settingsHelpBtn = $("settingsHelpBtn");
if (settingsHelpBtn) settingsHelpBtn.addEventListener("click", () => openAboutPanel());
$("storageRestoreBtn").addEventListener("click", () => $("storageRestoreFile").click());
$("storageRestoreFile").addEventListener("change", (event) => {
  const file = event.target.files && event.target.files[0];
  if (file) storageRestore(file);
  event.target.value = "";
});
$("storageClearBtn").addEventListener("click", () => storageClear());
$("recallBtn").addEventListener("click", () => recallMemory());
$("recallQ").addEventListener("keydown", (event) => {
  if (event.key === "Enter") recallMemory();
});
$("evalRunBtn").addEventListener("click", () => runEval());
$("tracesRefreshBtn").addEventListener("click", () => refreshTraces());
$("exportMdBtn").addEventListener("click", () => exportSession("md"));
$("exportHtmlBtn").addEventListener("click", () => exportSession("html"));
$("subagentRunBtn").addEventListener("click", () => runSubagent());
$("agentsSaveBtn").addEventListener("click", () => saveAgentsRules());
$("agentsTemplateBtn").addEventListener("click", () => generateAgentsTemplate());
$("mcpForm").addEventListener("submit", (event) => {
  event.preventDefault();
  addMcpServer();
});
$("mcpTransport").addEventListener("change", syncMcpFields);
syncMcpFields();
$("computerTaskForm").addEventListener("submit", (event) => {
  event.preventDefault();
  createComputerTask();
});
$("whitelistForm").addEventListener("submit", async (event) => {
  event.preventDefault();
  const appId = $("wlAppId").value.trim();
  if (!appId) return;
  try {
    await api("/whitelist/manage", {
      method: "POST",
      body: JSON.stringify({
        action: "upsert",
        entry: {
          app_id: appId,
          name: appId,
          tier: "productivity",
          learn_allowed: true,
          auto_ops_allowed: true,
          chat_authorized: false,
          sensitive: false,
        },
      }),
    });
    $("wlAppId").value = "";
    await refreshWhitelist();
  } catch (error) {
    addMessage("error", `白名单更新失败：${error.message || error}`);
  }
});
// 斜杠命令：输入 / 唤起补全，↑↓ 选择，Enter 执行，Esc 关闭
$("prompt").addEventListener("input", () => {
  updateComposerHint();
  refreshSlashMenu();
});
$("prompt").addEventListener("keydown", (event) => {
  const slashOpen = composerMenuEl && composerMenuEl.dataset.owner === "slash";
  if (slashOpen) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      slashActiveIndex = (slashActiveIndex + step + slashMatches.length) % slashMatches.length;
      refreshSlashMenu();
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      const cmd = slashMatches[slashActiveIndex];
      if (cmd) {
        $("prompt").value = `/${cmd.name} `;
        closeSlashMenu();
        updateComposerHint();
      }
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      runSlashCommand(slashMatches[slashActiveIndex]);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      closeSlashMenu();
      return;
    }
  }
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    sendPrompt();
  }
});
updateComposerHint();
$("slashHint").addEventListener("click", () => {
  const promptEl = $("prompt");
  promptEl.value = "/";
  promptEl.focus();
  updateComposerHint();
  refreshSlashMenu();
});
$("micBtn").addEventListener("click", async () => {
  if (listening) {
    if (localRecorder) {
      const { blob, sampleCount } = await localRecorder.stop();
      localRecorder = null;
      listening = false;
      setMicRecording(false);
      if (sampleCount < 1600) {
        addMessage("system", "录音太短，未识别");
        return;
      }
      try {
        const response = await apiRaw("/stt/transcribe", {
          method: "POST",
          headers: { "Content-Type": "audio/wav" },
          body: blob,
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || response.statusText);
        const prompt = $("prompt");
        prompt.value = (prompt.value ? prompt.value + " " : "") + result.text;
      } catch (error) {
        addMessage("error", `本地语音识别失败（${error.message}）`);
      }
      return;
    }
    if (recognition) recognition.stop();
    return;
  }
  listening = true;
  setMicRecording(true);
  const started = await startLocalRecording();
  if (!started) {
    listening = false;
    setMicRecording(false);
    if (!recognition) {
      initSpeech();
    }
    if (recognition) {
      try {
        recognition.start();
      } catch (_) {
        listening = false;
        setMicRecording(false);
        showToast("无法访问麦克风或系统语音识别", "error");
      }
    } else {
      showToast("无法访问麦克风（请允许麦克风权限）", "error");
    }
  } else {
    setTimeout(async () => {
      if (listening && localRecorder) {
        $("micBtn").click();
      }
    }, 10000);
  }
});

// ---------- 启动 ----------

const INVALIDATE_HANDLERS = Object.freeze({
  automations: () => refreshMountedPanel("automations"),
  whitelist: refreshWhitelist,
  mcp: refreshMcp,
  settings: refreshSettings,
  packages: refreshPackages,
  learn: refreshLearn,
  plugins: () => Promise.all([refreshPlugins(), refreshPluginMarket()]),
  computer: refreshComputerTasks,
  traces: refreshTraces,
  projects: refreshProjectRules,
  memory: () => refreshMountedPanel("memory"),
  skills: refreshSkills,
  sessions: refreshSessions,
  usage: refreshUsage,
});

function refreshMountedPanel(id) {
  const root = document.getElementById("panelRoot");
  const mounted = root?.querySelector("section[data-panel]")?.dataset.panel;
  if (mounted !== id) return;
  return window.OwoPanels?.[id]?.refresh?.();
}

function refreshAllInvalidatedDomains() {
  if (uiHidden()) return Promise.resolve();
  return window.OwoRecovery.runWithConcurrency(Object.values(INVALIDATE_HANDLERS), 3);
}

function stopInvalidation() {
  if (invalidator) invalidator.stop();
  invalidator = null;
  invalidationKey = "";
}

function startInvalidation() {
  const apiClient = window.OwoApi;
  if (!shellHydrated || !window.OwoInvalidation || !apiClient?.openEventStream) return;
  const base = String(apiClient.baseUrl || API_BASE).replace(/\/+$/, "");
  const key = base + "#" + (apiClient.coreInstanceId || "");
  if (invalidator && invalidationKey === key) return;
  stopInvalidation();
  const next = window.OwoInvalidation.createDomainInvalidator({
    baseUrl: base,
    openStream: (path, options) => apiClient.openEventStream(path, options),
    pollIntervalMs: 600000,
  });
  for (const [domain, refresh] of Object.entries(INVALIDATE_HANDLERS)) {
    next.on(domain, () => {
      if (!uiHidden()) return refresh();
    });
  }
  next.setPollFallback(refreshAllInvalidatedDomains);
  next.start();
  invalidator = next;
  invalidationKey = key;
}

window.addEventListener("owo:connection", (event) => {
  const detail = event.detail || {};
  if (detail.ready) {
    // A public health response confirms reachability only. Token issuance is the
    // point at which desktop authorization has actually succeeded.
    if (detail.reason === "token") markConnectionReady();
    if (shellHydrated) startInvalidation();
  } else {
    markConnectionUnavailable(detail.error);
    stopInvalidation();
  }
});

const WORKBENCH_REFRESH_PLANS = [
  { refresh: refreshHealth, intervalMs: 30000 },
  { refresh: refreshPerception, intervalMs: 30000 },
  { refresh: refreshPetState, intervalMs: 60000 },
];
workbenchRefresh = window.OwoRefresh.createScheduler(WORKBENCH_REFRESH_PLANS, {
  concurrency: 2,
  isHidden: uiHidden,
});
window.addEventListener("beforeunload", () => {
  workbenchRefresh.stop();
  stopInvalidation();
  for (const entry of state.activeTurns.values()) entry.observer?.stop();
});

const BOOT_HYDRATE_TASKS = [refreshSessions, refreshSettings, refreshSkills, refreshWhitelist];

async function hydrateShell() {
  await serviceWatch.start(); // 健康检查必须先于业务水合，失败时由就绪探针继续退避重试
  return window.OwoRecovery.runWithConcurrency(BOOT_HYDRATE_TASKS, 4);
}

async function boot() {
  await window.OwoCustomModels.init();
  initSpeech();
  window.OwoAccessibility?.annotate($("settingsContent"));
  initPanels();
  initProviderPresets();
  initPluginSkillTabs();
  initChatScroll();
  renderAccessChip();
  renderEffortChip();
  setMicRecording(false);
  updateComposerRunning();
  // 先读回本机偏好再应用：否则刷新后已保存的紧凑模式/桌宠等开关会被内存默认值覆盖。
  loadLocalPrefs();
  applyLocalPrefs();
  syncProjectChip();
  initGlobalStatusBar();
  if (await needsSetup()) {
    renderSetupGuide();
    return;
  }
  // A8-3：桌面桌宠开关状态来自引擎（每 15 秒刷新，跟随桌面端侧的改动）。
  await hydrateShell();
  await restoreLastSession();
  refreshWorkspaceTrust();
  shellHydrated = true;
  // 初始深链要等服务水合/恢复完成再打开，避免面板请求早于授权与 ready。
  applyDeepLink();
  window.owo?.pendingFeed?.().then(openPetFeedRun).catch(()=>{});
  startInvalidation();
  workbenchRefresh.start();
}

boot();

$("prefZoom")?.addEventListener("change", async event => {
 const factor = Number(event.target.value);
 if (!window.owo?.setZoom) { showToast("浏览器请使用浏览器缩放"); return; }
 try { await window.owo.setZoom(factor); LOCAL_PREFS.zoom = factor; saveLocalPrefs(); }
 catch { showToast("缩放未应用，请重试", "error"); }
});

function openPetFeedRun(ref) {
  if (!shellHydrated) return;
  if(ref?.panel==='permissions'){openPanelById('permissions');return;}
  openPanelById('workswarm');
  if(ref?.id) window.OwoPanels.workswarm.open(ref.id);
}
window.owo?.onOpenFeed?.(openPetFeedRun);
