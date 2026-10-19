// Electron 主进程：OwO Agent 桌面壳（替代 Tauri 壳，见 docs/adr/ADR-003）。
//
// 为什么换：用户明确要求"别再用 Rust 壳、前端改成组件化动态渲染"。关键好处是
// **渲染层从磁盘加载（核心服务托管 desktop/web）**——改 UI 只需刷新，不必重新编译，
// 这正好治住"改一次编译半天"这个痛点。
//
// 职责（与旧 Tauri 壳对齐，语义不变）：
//   1. 拉起核心服务 `owo-agent.exe serve --port 0`，从 stdout 的 core_ready 行取端口；
//   2. 读取 `%LOCALAPPDATA%\OwO\Agent\config.json`，把模型配置注入核心环境变量
//      （地址/模型名/上下文窗口/温度/超时…全部文件驱动，见 docs/model-config.md）；
//   3. 取一次 bearer token 交给渲染层（渲染层不再自己引导凭据）；
//   4. 提供 IPC：读/写配置、重载配置、重启核心；
//   5. 退出时优雅关闭核心（POST /server/shutdown，兜底 taskkill）。
//
// ADR-003 增补（自旧 Tauri 监管栈移植，语义与 core_supervisor.rs / core_runtime.rs
// / provider.rs / single_instance.rs 对齐）：
//   M2 ledger：壳侧 loopback 请求统一 `x-owo-client: shell`
//   M3 身份：注入 OWO_DESKTOP_INSTANCE_ID，/health 双重校验实例身份 + API 版本
//   M4 监管：generation 代际守卫 + 崩溃指数退避自动重启（上限 3 次）
//   M5 复用：启动时若发现已存活的兼容核心则直接接管，不重复拉起
//   M6 校验：保存配置前做结构性校验（凭据缺失只告警）+ 旧 provider.json 迁移
//   M7 壳：单实例锁、托盘（显示/隐藏/重启核心/开机自启）、外链走系统浏览器
// 纯逻辑在 `core-supervision.js`（带 node --test 单测），本文件只做编排。
//
// 不弹控制台：Windows 上以 `windowsHide: true` + `CREATE_NO_WINDOW` 语义启动核心。
//
// ELECTRON_RUN_AS_NODE=1 会让 electron.exe 退化成纯 Node 解释器：此时
// require("electron") 只返回一段路径字符串（内容是 electron.exe 的可执行路径），
// 下面的解构会得到一堆 undefined，随后第一处 ipcMain.handle(...) 抛
// "TypeError: Cannot read properties of undefined (reading 'handle')"，壳起不来。
//
// **注意：主进程内无法自愈。** 该变量由 electron.exe 在**进程启动时**读取并决定运行模式，
// 等本文件执行到时模式已定——清掉它只影响子进程继承（例如 spawn 出的核心），
// 对当前进程无效。因此正确做法是**在启动前**清（start.ps1 已做；`npm start` 等
// 其它入口需自行 `unset ELECTRON_RUN_AS_NODE`）。
// 这里的检查只是把原本晦涩的 undefined TypeError 换成一句能照着做的诊断。
const electronApi = require("electron");
if (!electronApi || typeof electronApi !== "object" || !electronApi.app) {
  console.error(
    '[fatal] require("electron") 未返回 Electron 模块对象 —— electron.exe 退化成了普通 Node。\n' +
      "        最可能的原因：环境变量 ELECTRON_RUN_AS_NODE 被设成了非空值。\n" +
      "        它在进程启动时生效，主进程内无法清除，请在启动 shell 里先清掉：\n" +
      "          PowerShell:  Remove-Item Env:\\ELECTRON_RUN_AS_NODE\n" +
      "          bash:        unset ELECTRON_RUN_AS_NODE\n" +
      "        或直接用包装脚本： pwsh -File desktop\\electron\\start.ps1（已内置清理）"
  );
  process.exit(78); // EX_CONFIG
}
const {
  app,
  BrowserWindow,
  ipcMain,
  shell,
  dialog,
  Menu,
  Tray,
  nativeImage,
  globalShortcut,
  screen,
  safeStorage, session, net,
} = electronApi;
const { spawn, execFile } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const http = require("node:http");
const crypto = require("node:crypto");

// ---------- IPC 信任门与导航锁（审查 N16：Electron 纵深防御） ----------
//
// 威胁：主窗口一旦被导航到外部页面（链接、window.location、被注入的脚本），
// 那个页面就继承了 preload 暴露的特权桥（config:write、shell:invoke、
// artifact 下载、配对密钥…）。contextIsolation/nodeIntegration 只是"别让页面拿到
// Node"，不解决"页面是谁加载的"。这里在注册任何 handler 之前统一包裹
// `ipcMain.handle`：只接受来自本应用窗口主 frame、且来源为本地页面（壳启动页
// data: 或核心回环 http://127.0.0.1）的请求；其余一律拒绝。
//
// 用一个 WeakSet 记录"本进程创建过的窗口 webContents"：外部页面即使被某种方式
// 放进窗口，也拿不到其它窗口的 sender 身份，且来源 origin 校验会再拦一层。

const trustedWindows = new WeakSet();

function isLoopbackOrBootUrl(raw) {
  let url = null;
  try {
    url = new URL(String(raw || ""));
  } catch {
    return false;
  }
  // 壳自生成的启动页（data:）只在主窗口主 frame 使用；页面自身发起的 data: 导航
  // 已被下面的导航锁拦住。
  if (url.protocol === "data:") return true;
  if (url.protocol !== "http:" && url.protocol !== "https:") return false;
  return url.hostname === "127.0.0.1" || url.hostname === "localhost" || url.hostname === "::1";
}

function isTrustedIpcSender(event) {
  const contents = event && event.sender;
  if (!contents || !trustedWindows.has(contents)) return false;
  const frame = event.senderFrame;
  if (!frame || frame !== contents.mainFrame) return false;
  return isLoopbackOrBootUrl(frame.url);
}

let ipcGuardInstalled = false;
function installIpcSenderGuard() {
  if (ipcGuardInstalled) return;
  ipcGuardInstalled = true;
  const original = ipcMain.handle.bind(ipcMain);
  ipcMain.handle = (channel, listener) =>
    original(channel, (event, ...args) => {
      if (!isTrustedIpcSender(event)) {
        console.warn(`[ipc] 拒绝非受信来源：${channel}`);
        throw new Error(`ipc/untrusted_sender: ${channel}`);
      }
      return listener(event, ...args);
    });
}

/** 锁定窗口导航：只允许本地页面；外链交给系统浏览器。 */
function lockNavigation(win) {
  if (!win || win.isDestroyed()) return;
  const guard = (event, targetUrl) => {
    if (isLoopbackOrBootUrl(targetUrl)) return;
    event.preventDefault();
    let target = null;
    try {
      target = new URL(String(targetUrl || ""));
    } catch {
      target = null;
    }
    if (target && (target.protocol === "http:" || target.protocol === "https:")) {
      shell.openExternal(target.href).catch(() => {});
    }
  };
  win.webContents.on("will-navigate", guard);
  win.webContents.on("will-redirect", guard);
}

// 模块加载即装上（幂等）：必须早于 pet-task-confirmation / pet-feed 等模块的
// `ipcMain.handle` 注册，否则那些 handler 不在守卫内。
installIpcSenderGuard();

const supervision = require("./core-supervision.js");
const shellCommands = require("./shell-commands.js");
const { configuredWorkspacePath } = require("./workspace-state.js");
const { CustomModelVault } = require("./custom-model-vault.js");
const modelBroker = require("./custom-model-broker.js");
const { migrateOrigins } = require("./legacy-model-migration.js");
let vaultInstance = null, vaultStartupError = "";
function modelVault() {
  if (!vaultInstance) vaultInstance = new CustomModelVault(path.join(dataRoot(), "custom-models.secure.json"), safeStorage);
  return vaultInstance;
}
async function syncModelCredentials(records, before) {
  if (!core?.token) throw Error("credentials/core_not_ready");
  const headers = { Authorization: "Bearer " + core.token };
  const caps = await httpGet(core.port, "/capabilities", headers);
  if (caps.status !== 200 || JSON.parse(caps.body).constraints?.custom_model_env_credentials !== true) throw Error("credentials/core_upgrade_required");
  const previousRefs = new Set(before.map(record => record.ref));
  for (const record of records) {
    if (record.disabled || !record.sealed || previousRefs.has(record.ref)) continue;
    const result = await httpPost(core.port, "/desktop/model-credentials", headers, {
      credential_ref: record.ref, model: record.id, base_url: record.baseUrl, api_format: record.apiFormat, api_key: modelVault().secret(record),
    });
    if (result.status !== 200) throw Error("credentials/core_registration_failed");
  }
  const retained = new Set(records.filter(record => !record.disabled).map(record => record.ref));
  for (const record of before) {
    if (!record.sealed || retained.has(record.ref)) continue;
    const result = await httpPost(core.port, "/desktop/model-credentials", headers, {
      credential_ref: record.ref, model: record.id, base_url: record.baseUrl, api_format: record.apiFormat,
    });
    if (result.status !== 200) throw Error("credentials/core_revocation_failed");
  }
}


const {
  CORE_API_VERSION,
  parseReadyLine,
  parseFatalLine,
  evaluateHealth,
  healthReasonText,
  pollDelayMs,
  nextBackoffMs,
  shouldAutoRestart,
  discoveryPort,
  validateConfig,
} = supervision;

// 自动重启上限：连续崩溃超过这个次数就停在 failed，交给用户决定（避免崩溃—重启风暴）。
const MAX_AUTO_RESTART = 3;
const HEALTH_TIMEOUT_MS = 8000;

// 进程配对证明（对齐旧 Tauri 壳的 CoreRuntime::pairing）：两个 UUID 去掉连字符，
// 共 64 位十六进制。经 OWO_DESKTOP_PAIRING_SECRET 注入核心，壳的 HTTP 请求再带
// x-owo-desktop-pairing 头——服务端据此把「领 token」限定给本壳拉起的那个核心
// （auth_token.rs::pairing_gate_allows）。不注入也能跑（服务端放行），但等于把
// 本地 token 引导开放给任意本地浏览器，因此这里保持与旧壳同强度。
const PAIRING_SECRET = `${crypto.randomUUID().replace(/-/g, "")}${crypto.randomUUID().replace(/-/g, "")}`;
const PAIRING_HEADER = "x-owo-desktop-pairing";
// 实例身份头（auth_token.rs::DESKTOP_INSTANCE_HEADER）：服务端在注入了实例身份时，
// 只允许**同一桌面实例**的引导请求取 token（instance_gate_allows），防"旧核心仍在 +
// 新壳新密钥"组合下静默 403 空壳。壳拉起核心时把 instanceId 记在这里，HTTP 请求带上。
//
// 这是 ADR-003 S6/M13 移植时的遗漏：配对头搬过来了，实例头没搬 →
// 主进程与渲染层取 token 全部 403 → 全线 401 → 界面显示"服务未连接"+"Failed to fetch"。
let currentInstanceId = "";
const INSTANCE_HEADER = supervision.DESKTOP_INSTANCE_HEADER;

let mainWindow = null;
let tray = null;
let core = null; // { proc, pid, port, token }
let coreState = { state: "starting" };

// 桌宠窗口：不再是独立桌面端，而是壳的第二个窗口 + 核心托管的一份静态页
// （`GET /pet` → desktop/web/pet）。它的数据全部来自核心 HTTP  API，只有"移动/
// 隐藏/拉起工作台"三件事需要主进程代劳——因为桌宠页面没有 Node 权限，自己开不
// 了窗也移动不了 Window。
let petWindow = null;
let petUiPort = null; // 与 uiPort 同理：核心重启换端口后必须重新导航
let petWatchdog = null; // 显隐看门狗（见 startPetWatchdog）
const PET_SIZE = { width: 270, height: 290 };
const agentPetWindows = new Map();

// ---------- 监管状态 ----------
let generation = 0; // 每次拉起核心 +1：过期那一轮的异步回调一律失效
let restartAttempts = 0; // 连续失败计数，ready 成功后归零
let shuttingDown = false; // 壳退出中：不再自动重启

// ---------- 渲染层：核心服务托管的工作台（ADR-003） ----------
//
// 核心服务在 `/` 静态托管 web 工作台（server lib.rs：`fallback_service(ServeDir::new(desktop_web_dir()))`），
// 因此壳不再自带任何前端代码——core ready 后直接导航到核心根路径即可，加载的是与
// 浏览器/旧 Tauri 壳**完全相同的同一份 desktop/web 文件**（服务端全局 no-store，
// 改 UI 后 Ctrl+R 立刻生效，"改一次编译半天"依然治得住）。
let uiPort = null; // 已导航过的端口：核心每次重启都是 --port 0 重分配，端口变了必须重新导航
let bootShown = false;

function webUiUrl(port) {
  return `http://127.0.0.1:${port}/`;
}

function petUrl(port) {
  return `http://127.0.0.1:${port}/pet/`;
}

// 核心起来之前的占位页：只有"正在拉起/失败原因"两态，不承担任何业务 UI。
function bootPage(state) {
  let detail = "正在拉起核心服务…";
  if (state.state === "failed") {
    detail = `核心启动失败<br/>错误码：${state.errorCode || "unknown"}<br/>${state.message || ""}`;
  } else if (state.state === "exited") {
    detail = `核心已退出：${state.message || ""}`;
  } else if (state.state === "restarting") {
    detail = `${state.message || "正在重启核心…"}`;
  }
  return `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><body style="margin:0;height:100vh;display:flex;align-items:center;justify-content:center;background:#f6f7f9;font:14px/1.7 system-ui,-apple-system,Segoe UI,sans-serif">
    <div style="text-align:center;max-width:560px;padding:24px">
      <div style="font-size:16px;font-weight:600;margin-bottom:10px">OwO Agent 工作台</div>
      <div style="color:#5b6570">${detail}</div>
    </div></body></html>`;
}

function showBootPage() {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  bootShown = true;
  mainWindow.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(bootPage(coreState))}`);
}

// ---------- 路径解析 ----------

function localAppData() {
  return process.env.LOCALAPPDATA || process.env.TEMP || os.tmpdir();
}

// 缺省数据根（%LOCALAPPDATA%\OwO\Agent）。
function defaultDataRoot() {
  return path.join(localAppData(), "OwO", "Agent");
}

// 数据目录可被用户改指（旧 Tauri 的 choose_data_directory / load_data_root_override）：
// 指针文件固定放在**缺省**数据根下，因此读它不能用 dataRoot()（否则自指）。
function dataRootPointerPath() {
  return path.join(defaultDataRoot(), "data_root.json");
}

function loadDataRootOverride() {
  try {
    const parsed = JSON.parse(fs.readFileSync(dataRootPointerPath(), "utf8"));
    const target = parsed && typeof parsed.path === "string" ? parsed.path : "";
    if (target && fs.statSync(target).isDirectory()) return target;
  } catch (_) {
    /* 没改过就用缺省 */
  }
  return null;
}

function dataRoot() {
  return loadDataRootOverride() || defaultDataRoot();
}

function saveDataRootOverride(target) {
  const canonical = fs.realpathSync(path.resolve(target));
  if (!fs.statSync(canonical).isDirectory()) throw new Error("所选路径不是目录");
  const pointer = dataRootPointerPath();
  fs.mkdirSync(path.dirname(pointer), { recursive: true });
  fs.writeFileSync(pointer, JSON.stringify({ path: canonical }, null, 2));
  return canonical;
}

function configPath() {
  return process.env.OWO_CONFIG_FILE || path.join(dataRoot(), "config.json");
}

// 旧壳遗留：`<data_root>/provider.json`（ADR-003 M6 的一次性迁移来源）。
function legacyProviderPath() {
  return path.join(dataRoot(), "provider.json");
}

function agentDataDir() {
  return process.env.OWO_AGENT_DATA || path.join(dataRoot(), "data");
}

// 核心日志落盘（对齐旧 Tauri 壳的 runtime.log_path()）。
// 打包形态没有终端，stdout 不落盘就等于日志全丢；web 的诊断页/错误卡会把这个
// 路径展示给用户，托盘排障与 `open_core_logs` 也以它为唯一来源。
function logDir() {
  return path.join(dataRoot(), "logs");
}

function coreLogPath() {
  return path.join(logDir(), "core.log");
}

// 配对证明绝不能落进日志（旧壳 core_runtime.rs::redact 同一职责）：
// 核心若把环境或请求回显出来，日志就成了凭据泄漏面。
function redactSecrets(text) {
  return String(text).split(PAIRING_SECRET).join("***pairing***");
}

function workspacePath() {
  // 与旧壳同源：数据目录下 workspace.json 的 path 字段。
  const configured = configuredWorkspacePath(dataRoot());
  if (configured) return configured;
  // 默认**不能**用 `process.cwd()`：那个值是 electron 的启动目录（
  // `desktop/electron` 甚至 electron 安装目录），作为"用户工作区"毫无意义——
  // 实测它会让核心以程序目录为工作区启动，而用户在会话里选自己的项目目录后，
  // 两者不一致会连累一批文件类工具（详见 core `resolve_session_path` 注释）。
  // 用户主目录是唯一无歧义、有意义且一定存在的默认值。
  try {
    return app.getPath("home");
  } catch (_) {
    return process.cwd();
  }
}

function saveWorkspace(target) {
  const file = path.join(dataRoot(), "workspace.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify({ path: target }));
  return target;
}

// 核心可执行文件：优先用同级目录（打包后），否则用仓库产物（开发时）。
function coreCandidates() {
  const here = path.dirname(app.getAppPath());
  const list = [
    // 显式指定（start.ps1 注入 / 部署脚本注入）优先。
    ...(process.env.OWO_CORE_EXE ? [process.env.OWO_CORE_EXE] : []),
    path.join(here, "owo-agent.exe"),
    path.join(process.cwd(), "owo-agent.exe"),
    path.join(__dirname, "..", "..", "..", "..", "target", "release", "owo-agent.exe"),
    path.join(__dirname, "..", "..", "..", "..", "dist", "OwO-Agent", "owo-agent.exe"),
  ];
  return list.filter((candidate) => fs.existsSync(candidate));
}

// ---------- 配置读写（唯一事实源：config.json） ----------

const DEFAULT_CONFIG = {
  version: 1,
  model: {
    provider: "unset",
    base_url: "",
    name: "",
    api_key: "",
    api_key_env: "OPENAI_API_KEY",
    context_window: null,
    max_output_tokens: supervision.DEFAULT_MODEL_OUTPUT_TOKENS,
    model_output_tokens: {},
    temperature: null,
    timeout_secs: null,
    keep_recent: null,
    compaction: null,
    models: [],
  },
};

// M6：首次启动（config.json 尚不存在）时把旧壳的 provider.json 迁进来，只做一次。
function migrateLegacyProvider() {
  if (fs.existsSync(configPath())) return;
  try {
    const text = fs.readFileSync(legacyProviderPath(), "utf8");
    const legacy = JSON.parse(text);
    const model = legacy && legacy.model ? legacy.model : legacy;
    if (!model || typeof model !== "object") return;
    const merged = {
      version: 1,
      model: {
        ...DEFAULT_CONFIG.model,
        provider: model.provider || model.mode || "unset",
        base_url: model.base_url || "",
        name: model.name || model.model || "",
        api_key: model.api_key || "",
        api_key_env: model.api_key_env || "OPENAI_API_KEY",
      },
    };
    writeConfig(merged);
  } catch (_) {
    /* 没有旧配置或格式不符：按默认走 */
  }
}

function readConfig() {
  try {
    const text = fs.readFileSync(configPath(), "utf8");
    const parsed = JSON.parse(text);
    return { ...DEFAULT_CONFIG, ...parsed, model: { ...DEFAULT_CONFIG.model, ...(parsed.model || {}) } };
  } catch (_) {
    return JSON.parse(JSON.stringify(DEFAULT_CONFIG));
  }
}

function writeConfig(config) {
  const file = configPath();
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const text = JSON.stringify({ version: 1, ...config }, null, 2) + "\n";
  const temp = `${file}.tmp`;
  fs.writeFileSync(temp, text);
  fs.renameSync(temp, file);
  // 可能含明文密钥：收紧为仅当前用户可读写（icacls，失败不影响功能）。
  try {
    execFile("icacls", [file, "/inheritance:r", "/grant:r", `${process.env.USERNAME}:F`], {
      windowsHide: true,
    }, () => {});
  } catch (_) {
    /* 忽略：权限收紧是加分项，不是前置条件 */
  }
}

// ---------- 核心：环境变量注入 ----------

function coreEnv(config, extra = {}) {
  let bindings = {};
  try { bindings = modelVault().environment(); } catch { vaultStartupError = "credentials/unlock_failed"; }
  return shellCommands.applyModelEnvironment({
    ...process.env,
    ...bindings,
    OWO_AGENT_DATA: agentDataDir(),
    OWO_DESKTOP_RELEASE: "1",
    OWO_DESKTOP_PAIRING_SECRET: PAIRING_SECRET,
    ...extra,
  }, config.model || {});
}

// ---------- HTTP（M2：壳侧请求统一带 ledger 来源标签） ----------

// M2：壳侧请求统一带 ledger 来源标签 + 配对证明（配对见 PAIRING_SECRET 说明）。
function shellHeaders(headers) {
  const out = { ...headers, "x-owo-client": "shell", [PAIRING_HEADER]: PAIRING_SECRET };
  // 实例头只在身份已知时带：接管已存活核心（M5）时那个核心属于别的实例，
  // 带了反而会被 instance_gate_allows 拒掉。
  if (currentInstanceId) out[INSTANCE_HEADER] = currentInstanceId;
  return out;
}

function httpGet(port, urlPath, headers = {}) {
  return new Promise((resolve, reject) => {
    const request = http.request(
      { host: "127.0.0.1", port, path: urlPath, method: "GET", headers: shellHeaders(headers), timeout: 5000 },
      (response) => {
        let body = "";
        response.on("data", (chunk) => (body += chunk));
        response.on("end", () => resolve({ status: response.statusCode, body }));
      }
    );
    request.on("timeout", () => request.destroy(new Error("timeout")));
    request.on("error", reject);
    request.end();
  });
}

function httpPost(port, urlPath, headers = {}, payload = null) {
  return new Promise((resolve, reject) => {
    const data = payload ? JSON.stringify(payload) : "";
    const request = http.request(
      {
        host: "127.0.0.1",
        port,
        path: urlPath,
        method: "POST",
        timeout: 8000,
        headers: {
          ...shellHeaders(headers),
          ...(data ? { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(data) } : {}),
        },
      },
      (response) => {
        let body = "";
        response.on("data", (chunk) => (body += chunk));
        response.on("end", () => resolve({ status: response.statusCode, body }));
      }
    );
    request.on("timeout", () => request.destroy(new Error("timeout")));
    request.on("error", reject);
    request.end(data);
  });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// M3：只接受 /health 返回 200 且能解析成 JSON 的响应，其余按"还没起来"处理。
async function probeHealth(port) {
  try {
    const result = await httpGet(port, "/health");
    if (result.status !== 200) return null;
    const parsed = JSON.parse(result.body);
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch (_) {
    return null;
  }
}

// M3：core_ready 之后仍要握手——端口可能属于上一轮没退干净（或别的）核心。
async function waitForInstance(port, instanceId, timeoutMs = HEALTH_TIMEOUT_MS) {
  const started = Date.now();
  let attempt = 0;
  let lastReason = "no_response";
  while (Date.now() - started < timeoutMs) {
    const health = await probeHealth(port);
    const verdict = evaluateHealth(health, { instanceId, apiVersion: CORE_API_VERSION });
    if (verdict.ok) return { ok: true, health };
    lastReason = verdict.reason;
    await sleep(pollDelayMs(attempt));
    attempt += 1;
  }
  return { ok: false, reason: lastReason };
}

// M5：壳崩溃后重启时，别把还活着的核心杀掉再拉一个——先尝试接管。
async function adoptExistingCore(myGeneration, apiVersion = CORE_API_VERSION) {
  let descriptor = null;
  try {
    descriptor = JSON.parse(fs.readFileSync(path.join(agentDataDir(), "runtime", "daemon.json"), "utf8"));
  } catch (_) {
    return false;
  }
  const port = discoveryPort(descriptor);
  if (!port) return false;
  const health = await probeHealth(port);
  // 接管的是"别人拉起的核心"，因此不能用实例身份校验，只校验健康与 API 版本。
  const verdict = evaluateHealth(health, { apiVersion });
  if (!verdict.ok) return false;
  let token = "";
  try {
    const auth = await httpGet(port, "/auth/token");
    token = JSON.parse(auth.body).token || "";
  } catch (_) {
    /* 见下方：拿不到 token 的核心不能接管 */
  }
  if (myGeneration !== generation) return false; // 代际已被新一轮取代

  // 配对证明校验（M13 连带）：/auth/token 会校验壳的 pairing。能拿到 token，
  // 说明这个核心"我们认证得了"（手动 owo-agent serve 起的核心没有 pairing
  // 约束，同样拿得到）。拿不到 token 却仍然接管是错的：web 的每条认证请求
  // 都要带本壳的 pairing，对不上就是永久 401，且重启壳也无法自愈（新随机值
  // 仍然对不上）。此时这个核心只可能属于一个已死掉的壳 → 杀掉重拉。
  if (!token) {
    const stalePid = Number(health && health.pid) || 0;
    if (stalePid > 0) {
      try {
        execFile("taskkill", ["/PID", String(stalePid), "/T", "/F"], { windowsHide: true }, () => {});
      } catch (_) {
        /* 杀不掉也继续：新核心 --port 0 换端口，不冲突 */
      }
    }
    return false;
  }
  // 接管的核心属于**别的**桌面实例（本壳没注入过它的 OWO_DESKTOP_INSTANCE_ID），
  // 必须清掉实例头，否则 shellHeaders 带上一个对不上的 id → instance_mismatch 403。
  currentInstanceId = "";
  core = { proc: null, pid: health.pid || 0, port, token, adopted: true };
  restartAttempts = 0;
  coreState = {
    state: "ready",
    port,
    token,
    pid: health.pid || 0,
    apiVersion: health.api_version,
    buildId: "",
    adopted: true,
    executable: "",
    configPath: configPath(),
    workspace: workspacePath(),
  };
  notifyState();
  return true;
}

// ---------- 核心：启动 / 就绪 / 关闭 ----------

function killCore() {
  if (!core || !core.proc) {
    core = null;
    return;
  }
  const { proc, port, token } = core;
  core = null;
  // 先请它优雅退出；给 1.5s，再强杀兜底（与旧壳同语义）。
  const finish = () => {
    try {
      proc.kill();
    } catch (_) {
      /* 已退出 */
    }
    try {
      execFile("taskkill", ["/PID", String(proc.pid), "/T", "/F"], { windowsHide: true }, () => {});
    } catch (_) {
      /* 忽略 */
    }
  };
  if (port && token) {
    httpPost(port, "/server/shutdown", { Authorization: `Bearer ${token}` }, { confirm: true })
      .then(() => setTimeout(finish, 1500))
      .catch(() => finish());
  } else {
    finish();
  }
}

// M4：核心意外退出后的自动重启——代际守卫 + 指数退避 + 上限。
function scheduleAutoRestart(myGeneration) {
  const decision = shouldAutoRestart({
    generation: myGeneration,
    currentGeneration: generation,
    attempts: restartAttempts,
    maxAttempts: MAX_AUTO_RESTART,
    shuttingDown,
    userInitiated: false,
  });
  if (!decision) {
    coreState = {
      state: "failed",
      errorCode: "core/restart_exhausted",
      message: `核心连续退出 ${restartAttempts} 次，已停止自动重启（可在托盘菜单手动重试）`,
    };
    notifyState();
    return;
  }
  restartAttempts += 1;
  const delay = nextBackoffMs(restartAttempts - 1);
  coreState = { state: "restarting", message: `${delay}ms 后自动重启（第 ${restartAttempts}/${MAX_AUTO_RESTART} 次）` };
  notifyState();
  setTimeout(() => {
    if (myGeneration === generation && !shuttingDown) startCore({ reason: "auto" });
  }, delay);
}

async function startCore({ reason = "auto", userInitiated = false } = {}) {
  const myGeneration = ++generation;
  killCore();

  // M5：仅在启动/自动重启路径尝试接管已存活核心；用户显式要求重启则必须重开。
  if (!userInitiated) {
    try {
      const adopted = await adoptExistingCore(myGeneration);
      if (adopted) return coreState;
    } catch (_) {
      /* 接管失败按正常拉起处理 */
    }
  }

  const candidates = coreCandidates();
  if (!candidates.length) {
    coreState = { state: "failed", errorCode: "core/binary_missing", message: "找不到核心服务 owo-agent.exe" };
    notifyState();
    return coreState;
  }
  const exe = candidates[0];
  const config = readConfig();
  const workspace = workspacePath();
  // M3：本轮身份。核心把它从 /health 公开，壳据此确认"这个端口确实是我拉起的那个核心"。
  const instanceId = `shell-${crypto.randomUUID()}`;
  // 记到模块级：shellHeaders 拿不到局部变量，但取 token 必须带实例头（见 INSTANCE_HEADER 注释）。
  currentInstanceId = instanceId;
  coreState = { state: "starting", executable: exe, configPath: configPath(), workspace };
  notifyState();

  // 打包形态：web 工作台随包携带在 resources/web，指给核心，避免回落到编译期源码路径。
  const extraEnv = { OWO_DESKTOP_INSTANCE_ID: instanceId };
  if (app.isPackaged) {
    extraEnv.OWO_WEB_UI_DIR = path.join(process.resourcesPath, "web");
  }

  const proc = spawn(exe, ["serve", "--port", "0", "--workspace", workspace], {
    cwd: path.dirname(exe),
    env: coreEnv(config, extraEnv),
    windowsHide: true, // 不弹控制台窗口
    stdio: ["ignore", "pipe", "pipe"],
  });

  // 核心 stdout/stderr 追加写入日志文件（见 coreLogPath 的说明）。
  // 写入统一走 safeLogWrite（见 core-supervision.js）：磁盘满、句柄被外部关闭、
  // 只读文件系统等情况下 write 可能同步抛错，而调用点在核心 stdout 的 'data'
  // 回调里，抛出去就是主进程崩。
  let logStream = null;
  const writeLogFile = supervision.safeLogWrite((text) => {
    if (logStream) logStream.write(text);
  });
  try {
    fs.mkdirSync(logDir(), { recursive: true });
    logStream = fs.createWriteStream(coreLogPath(), { flags: "a" });
    writeLogFile(
      `\n===== [${new Date().toISOString()}] 拉起核心 generation=${myGeneration} instance=${instanceId} exe=${exe} =====\n`,
    );
  } catch (_) {
    /* 日志落盘失败不阻断拉起：控制台输出仍在 */
  }

  const handleLine = async (line) => {
    const text = String(line || "").trim();
    if (!text) return;
    if (core && core.log) core.log(text);
    writeLogFile(`${redactSecrets(text)}\n`);

    const fatal = parseFatalLine(text);
    if (fatal) {
      if (myGeneration !== generation) return;
      coreState = { state: "failed", errorCode: fatal.code, message: fatal.message };
      notifyState();
      return;
    }

    const ready = parseReadyLine(text);
    if (!ready) return; // 非握手行：仅进日志
    if (myGeneration !== generation) return; // 代际守卫

    // M3：core_ready 只是"端口出来了"，还要用实例身份 + API 版本确认是不是自己那一个。
    const handshake = await waitForInstance(ready.port, instanceId);
    if (myGeneration !== generation) return;
    if (!handshake.ok) {
      coreState = {
        state: "failed",
        errorCode: `core/${handshake.reason}`,
        message: healthReasonText(handshake.reason, { apiVersion: CORE_API_VERSION }),
      };
      notifyState();
      return;
    }

    // 主进程自己先取一次 token，成功则注入渲染层（省掉渲染层那次 /auth/token 引导）。
    // 注意 httpGet 只 resolve 不 reject：非 2xx 时 body 是错误 JSON，`.token` 为 undefined，
    // 旧写法 `JSON.parse(auth.body).token || ""` 会**静默**退化成空串，
    // 渲染层拿不到注入 token 就退回自己引导 → 配对门 403 → 全线 401 → 界面"服务未连接"。
    // 这里显式判状态码并记日志，否则这类失败没有任何痕迹。
    let token = "";
    let authStatus = 0;
    try {
      const auth = await httpGet(ready.port, "/auth/token");
      authStatus = auth.status;
      if (auth.status === 200 && auth.body) {
        token = JSON.parse(auth.body).token || "";
      } else {
        writeLogFile(
          `[shell] /auth/token 返回 ${auth.status}：${String(auth.body || "").slice(0, 200)}\n`
        );
      }
    } catch (error) {
      writeLogFile(`[shell] /auth/token 请求异常：${error && error.message}\n`);
    }
    if (myGeneration !== generation) return;

    const authentication = supervision.authenticationState(authStatus, token);
    core = { proc, port: ready.port, token, pid: proc.pid, version: ready.api_version, buildId: ready.build_id, log: core && core.log };
    if (authentication.state === "ready") restartAttempts = 0; // 认证成功后清空失败预算
    coreState = {
      ...authentication,
      port: ready.port,
      token,
      pid: proc.pid,
      apiVersion: ready.api_version,
      buildId: ready.build_id,
      executable: exe,
      configPath: configPath(),
      workspace,
      instanceId,
    };
    notifyState();
  };

  let buffer = "";
  proc.stdout.on("data", (chunk) => {
    buffer += chunk.toString("utf8");
    let index;
    while ((index = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, index);
      buffer = buffer.slice(index + 1);
      handleLine(line);
    }
  });
  proc.stderr.on("data", (chunk) => handleLine(chunk.toString("utf8")));
  proc.on("exit", (code) => {
    writeLogFile(`===== [${new Date().toISOString()}] 核心退出 code=${code} =====\n`);
    // 关流防句柄泄漏：自动重启会再开一个新流（flags "a"），不关的话每次重启泄漏一个 fd。
    // end() 本身也可能抛（句柄已失效），同样不能让它掀翻主进程。
    try {
      if (logStream) logStream.end();
    } catch (_) {
      /* 已关闭或失效：无所谓 */
    }
    if (myGeneration !== generation) return; // 已被新一轮（重启/接管）取代
    if (coreState.state === "ready" || coreState.state === "restarting") {
      coreState = { state: "exited", errorCode: "core/exited", message: `核心已退出（code=${code}）` };
    } else if (coreState.state === "starting") {
      coreState = { state: "failed", errorCode: "core/exited", message: `核心启动失败（code=${code}）` };
    }
    notifyState();
    scheduleAutoRestart(myGeneration);
  });
  // stdout 日志必须使用带异步 error 监听的 safeStreamLogWrite：对端可能随时消失
  // （管道被上游截断、重定向到已退处的程序、CI 里跑），此时 write 抛 EPIPE，
  // 而这里处在核心 stdout 的 'data' 回调里 —— 一次未捕获异常就会崩掉整个主进程
  //（表现为「A JavaScript error occurred in the main process」，窗口直接消失）。
  // 日志只是诊断手段，写不进去绝不能影响壳的可用性。
  const writeStdout = supervision.safeStreamLogWrite(process.stdout);
  core = { proc, log: (line) => writeStdout(`[core] ${line}\n`) };
  return coreState;
}

function notifyState() {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  mainWindow.webContents.send("core:state", coreState);
  if (tray) refreshTrayMenu();

  // ready → 导航到核心托管的工作台；端口变了（核心重启后重新分配）必须重新导航。
  if (coreState.state === "ready" && coreState.port && uiPort !== coreState.port) {
    uiPort = coreState.port;
    mainWindow.loadURL(webUiUrl(coreState.port));
    syncPetWindow(coreState.port);
    return;
  }
  // 还没进过工作台时，由壳页负责把启动失败说清楚；
  // 已经进过工作台之后核心崩溃，交给工作台自带的 recovery / service-error 处理，不抢它的画面。
  if (uiPort === null && (!bootShown || coreState.state === "failed" || coreState.state === "exited")) {
    showBootPage();
  }
}

// ---------- 窗口 ----------

function createWindow() {
  // IPC 信任门必须在任何窗口创建/任何 handler 注册之前装好（幂等）。
  installIpcSenderGuard();
  mainWindow = new BrowserWindow({
    width: 1360,
    height: 880,
    minWidth: 900,
    minHeight: 600,
    backgroundColor: "#f6f7f9",
    title: "OwO Agent 工作台",
    // 去掉 Electron 默认菜单栏（File/Edit/View/Window/Help 那几项英文）。
    // 工作台页面自带中文导航（OwO logo + 文件/编辑/视图/帮助），两层菜单并存
    // 既重复又是英文，与"壳与工作台界面一致"的目标不符。旧 Tauri 壳也没有菜单栏。
    // 说明：这不是"隐藏"而是置空——autoHideMenuBar 仍会在 Alt 键按下时把默认菜单唤出来。
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  // 彻底移除默认菜单（含 Alt 唤出）。必须在 createWindow 早期调用。
  Menu.setApplicationMenu(null);
  trustedWindows.add(mainWindow.webContents);
  lockNavigation(mainWindow);
  // ADR-003：先给占位页，core ready 后由 notifyState 导航到核心托管的工作台。
  mainWindow.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(bootPage(coreState))}`);
  bootShown = true;
  // 工作台里的外链交给系统浏览器，壳内不新开窗口。
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//.test(String(url))) shell.openExternal(url);
    return { action: "deny" };
  });
  // 关闭窗口 = 收进托盘（有托盘时），退出走托盘菜单。
  mainWindow.on("close", (event) => {
    if (tray && !app.isQuitting) {
      event.preventDefault();
      mainWindow.hide();
    }
  });
  if (process.argv.includes("--dev")) {
    mainWindow.webContents.openDevTools({ mode: "detach" });
  }
}

// ---------- 桌宠窗口（第二个窗口，不是独立进程） ----------
//
// 三个不能省的 Windows/Electron 细节：
//  1. `backgroundThrottling: false` —— 窗口被遮挡/隐藏时 Chromium 会把定时器降频到
//     每分钟一次。桌宠靠 10s 心跳让服务端认为 `overlay_online`（判据是"最近 15 秒
//     收到过心跳"），一旦被节流，工作台的桌宠开关就会一直显示"桌面端离线"。
//  2. `transparent: true` + `#00000000` —— 桌宠是不规则形状，背景必须真透明；只写
//     transparent 不写背景色会出现黑块。
//  3. `movable: false` —— 由 PointerEvents 算出位移后走 IPC `pet:move` 调
//     setPosition，避免 Electron 自己的拖拽与自定义手势打架。

// 桌宠偏好（当前皮肤、窗口位置）落在壳侧文件，**不能放 localStorage**：
// 桌宠页来自 `http://127.0.0.1:<端口>`，而核心每次启动都用 `--port 0` 重新分配端口
// —— origin 一变，localStorage 就是另一个存储区，用户换的皮肤每次重启都会被忘掉。
function petPrefPath() {
  return path.join(dataRoot(), "pet.json");
}

function readPetPref() {
  try {
    const parsed = JSON.parse(fs.readFileSync(petPrefPath(), "utf8"));
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch (_) {
    return {}; // 首次运行或文件损坏：从空偏好开始
  }
}

function writePetPref(patch) {
  const next = { ...readPetPref(), ...(patch && typeof patch === "object" ? patch : {}) };
  try {
    const file = petPrefPath();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(next, null, 2), "utf8");
    return { ok: true, pref: next };
  } catch (error) {
    return { ok: false, error: String(error && error.message ? error.message : error) };
  }
}

// 拖动期间 `pet:move` 每秒会来几十次，每次都写盘既无必要也伤磁盘——防抖到停手后落一次。
let petBoundsTimer = null;
function schedulePetBoundsSave() {
  if (petBoundsTimer) clearTimeout(petBoundsTimer);
  petBoundsTimer = setTimeout(() => {
    petBoundsTimer = null;
    if (!petWindow || petWindow.isDestroyed()) return;
    const [x, y] = petWindow.getPosition();
    writePetPref({ x, y });
  }, 600);
}

/// 把一组坐标收进主显示器工作区（换分辨率/拔外接屏后旧坐标可能落在屏幕外）。
function clampToWorkArea(x, y) {
  const area = screen.getPrimaryDisplay().workArea;
  return {
    x: Math.min(Math.max(x, area.x), area.x + area.width - PET_SIZE.width),
    y: Math.min(Math.max(y, area.y), area.y + area.height - PET_SIZE.height),
  };
}

function petDefaultBounds() {
  const area = screen.getPrimaryDisplay().workArea;
  const saved = readPetPref();
  // 记住用户拖到的位置：桌宠的位置是个人偏好，重启弹回右下角等于每次都要重摆。
  if (Number.isFinite(saved.x) && Number.isFinite(saved.y)) {
    return clampToWorkArea(saved.x, saved.y);
  }
  return {
    x: Math.max(area.x, area.x + area.width - PET_SIZE.width - 24),
    y: Math.max(area.y, area.y + area.height - PET_SIZE.height - 24),
  };
}

function createPetWindow(port) {
  if (petWindow && !petWindow.isDestroyed()) return petWindow;
  petWindow = new BrowserWindow({
    ...PET_SIZE,
    ...petDefaultBounds(),
    frame: false,
    transparent: true,
    backgroundColor: "#00000000",
    resizable: false,
    movable: false,
    minimizable: false,
    maximizable: false,
    hasShadow: false,
    skipTaskbar: true, // 桌宠不进任务栏，也不该出现在 Alt+Tab 里干扰工作
    alwaysOnTop: true,
    // Windows 上 alwaysOnTop 会盖住开始菜单/通知，用 'screen-saver' 之上的层级
    // 保证可见，同时残留在普通应用之上；这里取 Electron 的最高档。
    title: "OwO 桌宠",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      backgroundThrottling: false,
    },
  });
  // 页面内的链接/新窗口一律不外开：桌宠不是浏览器。
  petWindow.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  trustedWindows.add(petWindow.webContents);
  lockNavigation(petWindow);
  // 关闭桌宠 = 隐藏（与工作台同语义），真正的退出走托盘。
  petWindow.on("close", (event) => {
    if (!app.isQuitting) {
      event.preventDefault();
      void setPetDesired(false);
    }
  });
  petWindow.loadURL(petUrl(port));
  return petWindow;
}

const petPresence=require('./pet-presence.js').createPetPresence({primary:()=>petWindow,peers:()=>agentPetWindows});
// Each secondary window runs the existing complete animated pet renderer.
function actorPreferences() {
  try{return JSON.parse(fs.readFileSync(path.join(dataRoot(),'pet-actors.json'),'utf8'));}catch{return {};}
}
let actorSaveTimer=null;
function saveActorPosition(w) {
  clearTimeout(actorSaveTimer);
  actorSaveTimer=setTimeout(()=>{
    const prefs=actorPreferences();
    for(const [role,window] of agentPetWindows)if(!window.isDestroyed()){const [x,y]=window.getPosition();prefs[role]={...prefs[role],x,y};}
    fs.mkdirSync(dataRoot(),{recursive:true});fs.writeFileSync(path.join(dataRoot(),'pet-actors.json'),JSON.stringify(prefs),'utf8');
    if(w===petWindow)schedulePetBoundsSave();
  },600);
}
function movePetActor(w,dx,dy) {
  const [x,y]=w.getPosition();const next=clampToWorkArea(x+Math.max(-500,Math.min(500,dx)),y+Math.max(-500,Math.min(500,dy)));
  w.setPosition(Math.round(next.x),Math.round(next.y));if(w!==petWindow&&petWindow&&!petWindow.isDestroyed())petWindow.setPosition(Math.round(next.x),Math.round(next.y));schedulePetBoundsSave();saveActorPosition(w);return {ok:true};
}
function resetPetActor(w){const area=screen.getPrimaryDisplay().workArea;const bounds=clampToWorkArea(area.x+area.width-PET_SIZE.width-24,area.y+area.height-PET_SIZE.height-24);for(const actor of [petWindow,...agentPetWindows.values()])if(actor&&!actor.isDestroyed())actor.setPosition(bounds.x,bounds.y);petPresence.apply();schedulePetBoundsSave();saveActorPosition(w);return {ok:true};}
function ensureAgentPets(ref) {
  if(!coreState.port)return;
  if(!ref?.id){for(const w of agentPetWindows.values())if(!w.isDestroyed())w.destroy();agentPetWindows.clear();petPresence.bind(ref);return;}
  const members=(ref?.members || []).filter(m=>m.runtime_binding?.kind==='agent');
  const primary=members.find(m=>['lead','planner'].includes(m.role)) || members[0];
  const roles=Array.isArray(ref?.members)?members.filter(m=>m!==primary).map(m=>m.role):['implementer','reviewer'];
  for(const [role,w] of agentPetWindows)if(!roles.includes(role)){w.destroy();agentPetWindows.delete(role);}
  const prefs=actorPreferences(),used=new Set([readPetPref().skin || 'petdex-haibara-ai']);
  for(const role of roles){const w=agentPetWindows.get(role);if(w&&!w.isDestroyed()&&prefs[role]?.skin)used.add(prefs[role].skin);}
  const available=['petdex-coco','petdex-boba','petdex-lulu-capybara','petdex-usagi','petdex-wangcai','petdex-doraemon','petdex-zero-two','petdex-nier-2b'];
  let index=0;
  for(const role of roles){
    index++;
    const existing=agentPetWindows.get(role);if(existing&&!existing.isDestroyed()){if(prefs[role]?.skin)used.add(prefs[role].skin);continue;}
    const requested=prefs[role]?.skin;const skin=requested&&!used.has(requested)?requested:available.find(x=>!used.has(x));
    if(!skin)throw Error('No distinct skin available for configured agent');used.add(skin);
    const origin=petWindow&&!petWindow.isDestroyed()?petWindow.getBounds():petDefaultBounds();
    const saved=prefs[role],position=saved&&Number.isFinite(saved.x)&&Number.isFinite(saved.y)?clampToWorkArea(saved.x,saved.y):clampToWorkArea(origin.x-index*PET_SIZE.width,origin.y);
    const w=new BrowserWindow({...PET_SIZE,...position,show:false,frame:false,transparent:true,backgroundColor:'#00000000',resizable:false,movable:false,skipTaskbar:true,alwaysOnTop:true,hasShadow:false,webPreferences:{preload:path.join(__dirname,'preload.js'),contextIsolation:true,nodeIntegration:false,backgroundThrottling:false}});
    agentPetWindows.set(role,w);prefs[role]={...prefs[role],skin};
    trustedWindows.add(w.webContents);
    lockNavigation(w);
    w.webContents.setWindowOpenHandler(()=>({action:'deny'}));
    w.on('close',event=>{if(!app.isQuitting){event.preventDefault();void setPetDesired(false);}});
    w.on('closed',()=>agentPetWindows.delete(role));
    w.loadURL(petUrl(coreState.port)+'?member=1&role='+encodeURIComponent(role)+'&skin='+encodeURIComponent(skin));
  }
  fs.writeFileSync(path.join(dataRoot(),'pet-actors.json'),JSON.stringify(prefs),'utf8');
  petPresence.bind(ref);
}
function applyPetVisibility(visible){petPresence.setVisible(visible);}

/// 核心 ready 时挂上桌宠；端口变（核心重启）则重新导航。
function syncPetWindow(port) {
  if (!petWindow || petWindow.isDestroyed()) {
    createPetWindow(port);
    petUiPort = port;
    return;
  }
  if (petUiPort !== port) {
    petUiPort = port;
    petWindow.loadURL(petUrl(port));
    for(const w of agentPetWindows.values())if(!w.isDestroyed()){const old=new URL(w.webContents.getURL());w.loadURL(petUrl(port)+old.search);}
  }
}

/// 唯一的显隐开关：**只写内核的期望值，不自己决定窗口可见性**。
///
/// 为什么绕一趟内核：显隐有三个来源（工作台设置开关、托盘菜单、桌宠自己的菜单），
/// 如果谁都直接 hide()/show()，就会出现"页面以为可见、窗口其实被藏了"的错位——
/// 页面继续报 visible:true，而屏幕上一个像素都没有，两个真相源互相骗。
/// 统一写 `POST /desktop/pet` 之后，桌宠页心跳读到 desired 再执行，全链路只有一个真相。
function coreAuthorizationHeaders() {
  return coreState && coreState.token
    ? { Authorization: "Bearer " + coreState.token }
    : {};
}

async function setPetDesired(visible) {
  if (!coreState.port) return { ok: false, reason: "core_not_ready" };
  try {
    await httpPost(coreState.port, "/desktop/pet", coreAuthorizationHeaders(), { visible });
  } catch (error) {
    return { ok: false, reason: String(error && error.message ? error.message : error) };
  }
  // 顺手立刻应用到窗口，不必等桌宠页下一次心跳（最多 10s，手感太差）。
  applyPetVisibility(visible);
  return { ok: true };
}

/// 显隐看门狗：**不依赖桌宠页**，由壳自己把窗口可见性对齐到内核的 desired。
///
/// 为什么需要它：桌宠页在窗口隐藏后会继续跑心跳（实测 10s 一次没停），但它
/// `setVisible(true)` 那一步不可靠——只要这一环出问题，桌宠就会永远回不来，
/// 表现为"点了显示但什么都不出现"。显隐是用户能直接感知的功能，绝不能建立在
/// "页面一定会照做"的假设上：壳负责执行，页面只负责上报实际值。
function startPetWatchdog() {
  clearInterval(petWatchdog);
  petWatchdog = setInterval(async () => {
    if (!coreState.port || !petWindow || petWindow.isDestroyed()) return;
    try {
      const response = await httpGet(coreState.port, "/desktop/pet", coreAuthorizationHeaders());
      if (response.status !== 200) return;
      const data = JSON.parse(response.body);
      if (typeof data.desired !== "boolean") return;
      applyPetVisibility(data.desired);
    } catch (_) {
      /* 核心不可达：下一轮再试 */
    }
  }, 8000);
}

// ---------- 托盘（M7） ----------

function trayIconPath() {
  return path.join(__dirname, "..", "..", "assets", "icon.png");
}

function refreshTrayMenu() {
  if (!tray) return;
  const stateText =
    coreState.state === "ready"
      ? `核心就绪（端口 ${coreState.port}）`
      : coreState.state === "starting"
        ? "核心启动中"
        : coreState.state === "restarting"
          ? "核心重启中"
          : `核心异常：${coreState.errorCode || coreState.state}`;
  const menu = Menu.buildFromTemplate([
    { label: "显示 / 隐藏工作台", click: () => toggleWindow() },
    {
      label: "显示 / 隐藏桌宠",
      click: () => {
        if (!petWindow || petWindow.isDestroyed()) return;
        // 当前可见性取自窗口本身，写回的却是内核期望值（见 setPetDesired 说明）。
        void setPetDesired(!petPresence.isVisible());
      },
    },
    { label: stateText, enabled: false },
    { type: "separator" },
    { label: "打开配置目录", click: () => shell.openPath(path.dirname(configPath())) },
    {
      label: "开机自启",
      type: "checkbox",
      checked: app.isPackaged ? app.getLoginItemSettings().openAtLogin === true : false,
      enabled: app.isPackaged, // 开发态不往注册表里塞开发版路径
      click: (item) => app.setLoginItemSettings({ openAtLogin: item.checked }),
    },
    { type: "separator" },
    { label: "退出", click: () => quitApp() },
  ]);
  tray.setContextMenu(menu);
  tray.setToolTip(`OwO Agent · ${stateText}`);
}

function createTray() {
  const iconFile = trayIconPath();
  if (!fs.existsSync(iconFile)) {
    // 图标缺失不该让壳起不来：降级为无托盘，行为与旧版一致（关窗即退出）。
    console.warn(`[tray] 缺少图标 ${iconFile}，跳过托盘`);
    return;
  }
  tray = new Tray(nativeImage.createFromPath(iconFile));
  refreshTrayMenu();
  tray.on("click", () => toggleWindow());
}

function toggleWindow() {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  if (mainWindow.isVisible()) {
    mainWindow.hide();
  } else {
    mainWindow.show();
    mainWindow.focus();
  }
}

function quitApp() {
  app.isQuitting = true;
  shuttingDown = true;
  killCore();
  app.quit();
}

// ---------- 全局快捷键（对齐旧 Tauri 壳：Ctrl+Alt+Shift+O 唤起工作台） ----------

const WORKSPACE_ACCELERATOR = "Ctrl+Alt+Shift+O";

function showWorkspace() {
  // 窗口已被销毁（例如无托盘模式关过窗）时重建，而不是静默失效。
  if (!mainWindow || mainWindow.isDestroyed()) {
    createWindow();
    return;
  }
  if (mainWindow.isMinimized()) mainWindow.restore();
  if (!mainWindow.isVisible()) mainWindow.show();
  mainWindow.focus();
}

function registerGlobalShortcut() {
  // 与旧壳同一策略：注册失败（被其它程序占用）只降级并记日志，不阻断启动。
  const registered = globalShortcut.register(WORKSPACE_ACCELERATOR, () => showWorkspace());
  if (registered) {
    console.log(`[shortcut] 已注册 ${WORKSPACE_ACCELERATOR}`);
  } else {
    console.warn(`[shortcut] ${WORKSPACE_ACCELERATOR} 注册失败（可能被占用），继续启动`);
  }
  return registered;
}

require('./pet-task-confirmation.js').installTaskConfirmation({pet:()=>petWindow,pets:()=>[petWindow,...agentPetWindows.values()],workspace:()=>workspacePath(),port:()=>coreState.port});
require('./pet-feed.js').installPetFeed({pet:()=>petWindow,main:()=>mainWindow,workspace:()=>showWorkspace(),dataRoot,port:()=>coreState.port,pets:()=>[petWindow,...agentPetWindows.values()],ensureAgents:ensureAgentPets,moveActor:movePetActor,resetActor:resetPetActor,setVisible:setPetDesired,present:state=>petPresence.update(state),queryVisible:()=>petPresence.isVisible()});

// ---------- IPC ----------

ipcMain.handle("core:get", () => coreState);
ipcMain.handle("core:restart", async () => {
  restartAttempts = 0; // 用户主动重启：清空失败预算
  await startCore({ reason: "manual", userInitiated: true });
  return coreState;
});
ipcMain.handle("config:read", () => ({ path: configPath(), config: readConfig() }));
ipcMain.handle("config:write", (_event, config) => {
  // M6：结构性错误直接拒绝写入（凭据缺失只返回 warnings，不阻断）。
  const verdict = validateConfig(config, { envHas: (name) => Boolean(process.env[name]) });
  if (!verdict.ok) return { ok: false, errors: verdict.errors, warnings: verdict.warnings };
  writeConfig(config);
  return { ok: true, path: configPath(), warnings: verdict.warnings };
});
// 保存配置并重启核心：改文件/界面保存走同一条路，避免两份真相。
ipcMain.handle("config:apply", async (_event, config) => {
  const verdict = validateConfig(config, { envHas: (name) => Boolean(process.env[name]) });
  if (!verdict.ok) return { ok: false, errors: verdict.errors, warnings: verdict.warnings };
  writeConfig(config);
  restartAttempts = 0;
  await startCore({ reason: "manual", userInitiated: true });
  return { ok: true, path: configPath(), state: coreState, warnings: verdict.warnings };
});
ipcMain.handle("config:reveal", () => {
  const file = configPath();
  if (!fs.existsSync(file)) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    writeConfig(readConfig());
  }
  shell.showItemInFolder(file);
  return { ok: true, path: file };
});
ipcMain.handle("workspace:get", () => workspacePath());
ipcMain.handle("workspace:choose", async () => {
  const result = await dialog.showOpenDialog(mainWindow, {
    title: "选择项目工作区",
    properties: ["openDirectory"],
  });
  if (result.canceled || !result.filePaths.length) return { ok: false, canceled: true };
  return setWorkspaceTarget(result.filePaths[0]);
});
ipcMain.handle("app:openExternal", (_event, url) => {
  if (/^https?:\/\//.test(String(url))) shell.openExternal(url);
  return { ok: true };
});

for (const operation of ["list", "upsert", "remove", "migrate", "probe"]) {
  ipcMain.handle("models:" + operation, async (event, input = {}) => {
    modelBroker.assertFrame(event, mainWindow, core);
    if (operation === "list") return { ...modelVault().snapshot(), error: vaultStartupError || null };
    if (operation === "probe") return modelBroker.probe(modelVault(), input);
    const result = operation === "upsert" ? await modelVault().upsert(input, syncModelCredentials)
      : operation === "remove" ? await modelVault().remove(String(input.id || ""), syncModelCredentials)
      : await modelVault().migrate(input.models, input.keys, syncModelCredentials);
    if (operation === "migrate" && !result.conflictedIds?.length) vaultStartupError = "";
    else if (!vaultStartupError.includes("migration")) vaultStartupError = "";
    return result;
  });
}
const teamFiles=require('./team-files.js').createTeamFiles({
  get:async route=>{
    if(!core?.port||!core?.token)throw Error('核心服务未连接');
    const response=await httpGet(core.port,route,{Authorization:'Bearer '+core.token});
    if(response.status!==200)throw Error('文件信息暂时不可用（'+response.status+'）');
    return JSON.parse(response.body);
  },
  open:file=>shell.openPath(file),reveal:file=>shell.showItemInFolder(file),
});
for(const operation of ['list','act'])ipcMain.handle('team:files:'+operation,async(event,input)=>{
  modelBroker.assertFrame(event,mainWindow,core);
  return operation==='list'?teamFiles.describe(input?.teamId):teamFiles.act(input||{});
});
ipcMain.handle("artifact:download", (event, id) => {
  modelBroker.assertFrame(event, mainWindow, core);
  if (typeof id !== "string" || !id || id.length > 400) throw Error("artifact/invalid_id");
  mainWindow.webContents.downloadURL(webUiUrl(core.port) + "artifacts/" + encodeURIComponent(id) + "/content?raw=true", { headers: shellHeaders({ Authorization: "Bearer " + core.token }) });
  return { ok: true };
});
ipcMain.handle("view:zoom", (event, factor) => {
  modelBroker.assertFrame(event, mainWindow, core);
  if (![1, 1.25, 1.5].includes(Number(factor))) throw Error("view/invalid_zoom");
  mainWindow.webContents.setZoomFactor(Number(factor)); return { ok: true, factor: Number(factor) };
});
// ---------- 桌宠 IPC ----------
// 桌宠页面无 Node 权限，这三件事只能主进程代劳。与旧 Tauri 壳的
// `move_pet_by` / `set_pet_visible` / `open_workbench` 语义一致。

ipcMain.handle("pet:move", (_event, dx, dy) => {
  if (!petWindow || petWindow.isDestroyed()) return { ok: false };
  const [x, y] = petWindow.getPosition();
  // Clamp 在显示器工作区内：无限拖动会把桌宠拖到屏幕外找不回来。
  const next = clampToWorkArea(x + Number(dx || 0), y + Number(dy || 0));
  petWindow.setPosition(next.x, next.y);
  // 记住新位置（防抖），这样重启后用户不用再把桌宠摆一遍。
  schedulePetBoundsSave();
  return { ok: true };
});

ipcMain.handle("pet:visible", (_event, visible) => {
  if (!petWindow || petWindow.isDestroyed()) return { ok: false };
  console.log(`[pet] setVisible(${visible})`);
  applyPetVisibility(visible);
  return { ok: true };
});

ipcMain.handle("pet:workbench", () => {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.show();
    mainWindow.focus();
    return { ok: true };
  }
  createWindow();
  return { ok: true };
});

// 桌宠页每次心跳都会问一次真实可见性并与期望值对账。没有这个对账，任何一次
// 未经页面的隐藏（外部 hide、窗口被移出可视区）都会让桌宠静默消失且永不回来。
ipcMain.handle("pet:query", () => {
  if (!petWindow || petWindow.isDestroyed()) return { visible: false, alive: false };
  return { visible: petPresence.isVisible(), alive: true };
});

ipcMain.handle("pet:reset", () => {
  if (!petWindow || petWindow.isDestroyed()) return { ok: false };
  const area = screen.getPrimaryDisplay().workArea;
  const bounds = {
    x: Math.max(area.x, area.x + area.width - PET_SIZE.width - 24),
    y: Math.max(area.y, area.y + area.height - PET_SIZE.height - 24),
  };
  petWindow.setBounds({ ...PET_SIZE, ...bounds });
  for(const w of agentPetWindows.values())if(!w.isDestroyed())w.setPosition(bounds.x,bounds.y);
  petPresence.apply();
  // "回到右下角"同时要清掉记住的位置，否则下次启动又回到旧坐标。
  writePetPref({ x: bounds.x, y: bounds.y });
  return { ok: true, bounds };
});

// 偏好读写统一走 readPetPref / writePetPref（定义见 petDefaultBounds 上方）。
ipcMain.handle("pet:pref:get", () => readPetPref());

ipcMain.handle("pet:pref:set", (_event, patch) => writePetPref(patch));

// ---------- Tauri 兼容命令桥（ADR-003） ----------
//
// desktop/web 通过 `window.__TAURI_INTERNALS__.invoke(name, args)` 调壳：14 个命令
// 散落在 api-client.js / folder-picker.js / settings-panel.view.js /
// setup-guide.view.js / service-error.view.js / status-bar.view.js。Tauri 壳移除后
// 这个全局对象消失，会让「模型配置保存」「原生目录选择器」「诊断页重启/开日志」
// 全部静默降级为"非桌面环境"——UI 一模一样，功能却少一半。
//
// 对策：主进程按**同名同形状**实现这 14 个命令（形状与旧 commands.rs 逐字段对齐），
// preload 原样暴露该全局对象。desktop/web 一行不改，这是"UI 与功能一模一样"的
// 实现前提。纯语义（默认值/掩码/读改写）在 shell-commands.js，可 node --test。

// shell.openPath 返回空串表示成功，非空串是错误信息。
async function openPathInExplorer(target, select) {
  if (!target) return false;
  if (select) {
    shell.showItemInFolder(target);
    return true;
  }
  const error = await shell.openPath(target);
  return !error;
}

// get_core_state 的形状：ready 带 port，非 ready 一律 port=0（web 据此判断能否连）。
function coreStateValue() {
  const base = { ...coreState, logPath: coreLogPath() };
  if (base.state === "ready") return base;
  return { ...base, port: 0 };
}

function providerStatus() {
  return shellCommands.providerStatusValue(readConfig().model || {}, {
    envGet: (name) => process.env[name],
    configPath: configPath(),
  });
}

async function pickDirectory(title) {
  const result = await dialog.showOpenDialog(mainWindow, { title, properties: ["openDirectory"] });
  if (result.canceled || !result.filePaths.length) return null;
  return result.filePaths[0];
}

// 写配置 → 受控重启核心（新值经环境变量注入生效）。设置页与引导页共用这一条。
async function applyModelConfig(args) {
  const patched = shellCommands.applyModelConfigPatch(readConfig(), args);
  if (!patched.ok) return { ok: false, error: patched.error };
  writeConfig(patched.config);
  restartAttempts = 0;
  await startCore({ reason: "manual", userInitiated: true });
  return { ok: true, ...providerStatus() };
}

async function setWorkspaceTarget(target) {
  let canonical;
  try {
    canonical = fs.realpathSync(path.resolve(target));
  } catch (_) {
    return { ok: false, error: `路径不可用：${target}` };
  }
  if (!fs.statSync(canonical).isDirectory()) return { ok: false, error: "所选路径不是目录" };
  saveWorkspace(canonical);
  // Session workspace is selected when creating a session; the shared daemon keeps existing turns.

  return { ok: true, workspace: canonical, state: coreState.state, generation };
}

const SHELL_COMMAND_HANDLERS = {
  get_core_state: () => coreStateValue(),

  get_core_connection: () => {
    if (coreState.state === "ready" && coreState.port) {
      return {
        port: coreState.port,
        instanceId: coreState.instanceId || "",
        // 配对证明：api-client 拿到后随请求带 x-owo-desktop-pairing（长度 ≥32 才采纳）。
        pairing: PAIRING_SECRET,
        apiVersion: coreState.apiVersion || CORE_API_VERSION,
        pid: coreState.pid || 0,
        buildId: coreState.buildId || "",
        // 诊断页据此比对 buildId != expectedBuildId 提示"安装包与核心版本错配"
        // （旧壳用编译期 commit，Electron 侧以包版本号承担同一职责）。
        expectedBuildId: app.getVersion(),
        state: "ready",
        generation,
        token: coreState.token || "",
        logPath: coreLogPath(),
      };
    }
    return coreStateValue();
  },

  retry_core_start: async () => {
    restartAttempts = 0;
    await startCore({ reason: "manual", userInitiated: true });
    return coreStateValue();
  },

  open_core_logs: async () => {
    const opened = await openPathInExplorer(logDir(), false);
    return { opened: coreLogPath(), ok: opened };
  },

  get_workspace: () => ({ workspace: workspacePath(), configured: Boolean(configuredWorkspacePath(dataRoot())), state: coreState.state }),

  set_workspace: async (args) => {
    const target = String((args && args.path) || "").trim();
    if (!target) return { ok: false, error: "路径为空" };
    return setWorkspaceTarget(target);
  },

  choose_project_directory: async () => {
    const picked = await pickDirectory("选择项目工作区");
    if (!picked) return { ok: false, canceled: true };
    return setWorkspaceTarget(picked);
  },

  create_project_workspace: async (args) => {
    const validation = shellCommands.validateProjectFolderName(args && args.name);
    if (!validation.ok) return validation;
    const parent = await pickDirectory("选择新项目的上级目录");
    if (!parent) return { ok: false, canceled: true };
    const target = path.join(parent, validation.name);
    try {
      fs.mkdirSync(target);
    } catch (error) {
      if (error && error.code === "EEXIST") {
        return { ok: false, error: "该目录已存在，请使用其他项目名称" };
      }
      return { ok: false, error: String((error && error.message) || error) };
    }
    if (args && args.activate === false) {
      return { ok: true, path: target, activated: false };
    }
    const result = await setWorkspaceTarget(target);
    return result.ok ? { ...result, path: target, activated: true } : result;
  },

  choose_data_directory: async () => {
    const picked = await pickDirectory("选择新的数据目录");
    if (!picked) return { ok: false, canceled: true };
    let canonical;
    try {
      canonical = saveDataRootOverride(picked);
    } catch (error) {
      return { ok: false, error: String((error && error.message) || error) };
    }
    restartAttempts = 0;
    await startCore({ reason: "manual", userInitiated: true });
    return { ok: true, data_root: canonical, state: coreState.state };
  },

  get_provider_status: () => providerStatus(),

  // api-client 的 desktopPairingProof 走这条（旧壳在 main.rs::desktop_pairing）。
  desktop_pairing: () => PAIRING_SECRET,

  set_model_config: (args) => applyModelConfig(args),

  // 兼容旧调用点（引导页）：等价于 set_model_config，但只传 mode/base_url/model，
  // 密钥字段一律不动（旧前端没有密钥输入框）。
  set_provider: (args) => applyModelConfig(args),

  reveal_model_config: async () => {
    const file = configPath();
    if (!fs.existsSync(file)) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      writeConfig(readConfig());
      await openPathInExplorer(path.dirname(file), false);
      return {
        ok: true,
        created: false,
        path: file,
        detail: "配置文件尚未生成：在设置页点一次「保存并重启核心」即可创建",
      };
    }
    const opened = await openPathInExplorer(file, true);
    return { ok: opened, created: true, path: file };
  },

  reload_model_config: async () => {
    const file = configPath();
    if (!fs.existsSync(file)) {
      return {
        ok: false,
        error: `配置文件不存在：${file}（先在设置页保存一次即可创建）`,
        configPath: file,
      };
    }
    // 从磁盘重读（用户可能手改过 config.json），再受控重启让新值经环境变量生效。
    // startCore 内部会自己 readConfig()，这里无需传递。
    restartAttempts = 0;
    await startCore({ reason: "manual", userInitiated: true });
    return { ok: true, reloaded: true, configPath: file, ...providerStatus() };
  },
};

ipcMain.handle("shell:invoke", async (_event, name, args) => {
  const handler = SHELL_COMMAND_HANDLERS[name];
  // 未知命令返回错误对象而不是抛异常：web 侧多处对 invoke 只做 .catch，
  // 抛出去会把整条链路打成 unhandled rejection。
  if (!handler) return { ok: false, error: `未知壳命令：${name}` };
  try {
    return await handler(args || {});
  } catch (error) {
    return { ok: false, error: String((error && error.message) || error) };
  }
});

// ---------- 生命周期 ----------

// M7：单实例——第二次启动唤醒既有的那个窗口，而不是再拉一个核心。
if (process.env.OWO_DESKTOP_PROFILE_DIR) {
  const profile = path.resolve(process.env.OWO_DESKTOP_PROFILE_DIR);
  fs.mkdirSync(profile, { recursive: true });
  app.setPath('userData', profile);
}
const gotSingleInstanceLock = app.requestSingleInstanceLock();
if (!gotSingleInstanceLock) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.show();
      mainWindow.focus();
    }
  });
}

app.whenReady().then(async () => {
  migrateLegacyProvider(); // M6：一次性迁移旧壳配置
  createTray();
  registerGlobalShortcut();
  createWindow();
  try {
    const result = await migrateOrigins({ BrowserWindow, session, net, logFile: coreLogPath(), userData: app.getPath("userData"), vault: modelVault() });
    if (result.conflicts) vaultStartupError = "credentials/migration_conflict";
  }
  catch { vaultStartupError = "credentials/legacy_migration_pending"; }
  await startCore({ reason: "launch" });
  startPetWatchdog();
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  // 有托盘时窗口关闭只是隐藏；真正退出走托盘菜单（quitApp）。
  if (!tray) {
    shuttingDown = true;
    killCore();
    app.quit();
  }
});

app.on("before-quit", () => {
  shuttingDown = true;
  clearInterval(petWatchdog);
  globalShortcut.unregisterAll();
  killCore();
});
process.on("exit", () => killCore());
