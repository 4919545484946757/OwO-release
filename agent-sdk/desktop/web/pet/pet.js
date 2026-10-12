/* OwO 桌宠 · 已并入 Agent 框架（不再作为独立桌面端存在）
 *
 * 定位：**给用户省事的前台**——不是会动的装饰。它替用户盯住内核正在跑的回合，
 * 在需要人做决定（审批 300s 超时默认拒绝）时用最不容易被忽略的方式提醒，并让
 * 允许/拒绝/停止三个动作一步可达。
 *
 * 三条边界（改这个文件前请先看）：
 *  1. 状态唯一来源是内核 `GET /activity` + `GET /approvals/pending`，桌宠不做本地
 *     臆测。这正是服务端 `activity_api.rs` 里写明的用途（"桌宠等外部进度面板的
 *     只读数据源"），phase 由内核从 TurnEvent 统一推导，桌宠与工作台不会各说各话。
 *  2. 鉴权一律走 `window.OwoApi`（全站唯一的 fetch 出口）。它自行补 bearer /
 *     pairing / instance 三件套；桌宠若自己 fetch，会因缺两道 HTTP 头而恒 403，
 *     表现为"桌宠永远是 idle"。
 *  3. 窗口能力（移动/隐藏/拉起工作台）走壳的 IPC（window.petShell）。桌宠页面无
 *     Node 权限，也不许自己开窗。
 *
 * 与旧 LingXi 桌宠的差异：删掉 QQ 轮询（与框架无关且构成第四个状态源）、删掉
 * 双击彩蛋与 ✦ 粒子、把 Tauri invoke 换成 OwoApi，并补上审批倒计时（超时＝拒绝
 * 是最大的隐性坑）。
 *
 * 按键口径（2026-10-01 定）：**左键点击只互动**——摸摸头（happy 摇动 + ❤×4 + 皮肤
 * 台词），**右键才是功能面**——允许 / 拒绝审批 · 停止回合 · 打开工作台 · 换形象 ·
 * 隐藏。左键不再弹菜单："点着玩"是手最顺的动作，不该在按下去之前先担心会弹出一屏
 * 操作项；要办事的意图由右键明确表达。拖动仍是纯移动（不弹菜单）。
 */
"use strict";

const $ = (id) => document.getElementById(id);
const pet = $("pet");
const avatar = $("pet-avatar");
const avatarImg = $("pet-avatar-img");
const bubble = $("bubble");
const fxLayer = $("fx");
const menu = $("skin-menu");

// 桌宠与 Electron 壳之间的桥（preload 注入）。浏览器直接打开时为 null，
// 此时桌宠仍可显示状态，只是不能移动/拉起工作台。
const shellBridge = globalThis.petShell || null;

// ---- 帧动画驱动（petdex spritesheet）----
// 一个状态用 sheet 的某一行循环播放；rAF 按帧时长推进 background-position。
let animRun = null;
let animSheetLoaded = "";
let animRafId = 0;
let animEpoch = 0;

/// 帧动画慢放系数（1 = 按皮肤声明的原始节奏；2 = 整体慢一倍）。
/// 素材原始节奏偏快（眨眼过密），统一在这里调节。
const ANIM_SLOWDOWN = 2;

const sheetInfoCache = {};
const sheetInfoPending = {};

function sheetInfo(url, frameW, frameH) {
  const hit = sheetInfoCache[url];
  if (hit) return hit;
  if (!sheetInfoPending[url]) {
    sheetInfoPending[url] = true;
    analyzeSheet(url, frameW, frameH)
      .then((info) => {
        if (!info) return;
        sheetInfoCache[url] = info;
        if (animRun && animRun.sheet === url) applySheetInfo(animRun, info);
      })
      .catch(() => {
        /* 分析失败：保持声明帧数的原行为 */
      });
  }
  return null;
}

/// petdex 素材每行末尾常有空白帧（画师按 8 列导出但只画了 5~7 帧），按声明帧数
/// 播放就会周期性地"人物闪一下"。这里采样 alpha 探测每行的**有效帧列索引**。
async function analyzeSheet(url, frameW, frameH) {
  const img = new Image();
  img.src = url;
  await img.decode();
  const cols = Math.max(1, Math.round(img.naturalWidth / frameW));
  const rows = Math.max(1, Math.round(img.naturalHeight / frameH));
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(img, 0, 0);
  let pixels;
  try {
    pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  } catch {
    return null; // 跨源污染：退回声明值
  }
  const width = canvas.width;
  const stepX = Math.max(1, Math.floor(frameW / 24));
  const stepY = Math.max(1, Math.floor(frameH / 24));
  const minHits = 12;
  const validFrames = [];
  for (let row = 0; row < rows; row++) {
    const valid = [];
    for (let col = 0; col < cols; col++) {
      const x0 = col * frameW;
      const y0 = row * frameH;
      let hits = 0;
      for (let y = 0; y < frameH && hits < minHits * 4; y += stepY) {
        for (let x = 0; x < frameW; x += stepX) {
          const alpha = pixels[((y0 + y) * width + (x0 + x)) * 4 + 3];
          if (alpha > 8) hits++;
        }
      }
      if (hits >= minHits) valid.push(col);
    }
    validFrames.push(valid);
  }
  return { rows, validFrames };
}

function applySheetInfo(run, info) {
  run.rows = info.rows;
  const valid = info.validFrames[run.row];
  if (valid && valid.length) run.frameCols = valid;
}

function stopAnim() {
  animRun = null;
  cancelAnimationFrame(animRafId);
  animRafId = 0;
}

function startAnim(a) {
  const frame = config.frame;
  if (!frame || !frame.width || !frame.height) return false;
  const dispW = 200;
  const dispH = Math.round((dispW * frame.height) / frame.width);
  avatar.style.height = dispH + "px";
  if (animSheetLoaded !== a.sheet) {
    avatar.style.backgroundImage = `url("${a.sheet}")`;
    animSheetLoaded = a.sheet;
  }
  const row = a.row || 0;
  const info = sheetInfo(a.sheet, frame.width, frame.height);
  if (animRun && animRun.sheet === a.sheet && animRun.row === row) return true;
  animRun = {
    sheet: a.sheet,
    row,
    frames: Math.max(1, a.frames || 1),
    cols: Math.max(1, a.cols || 8),
    durationMs: Math.max(80, a.durationMs || 900),
    dispW,
    dispH,
    rows: info && info.rows ? info.rows : Math.max(1, a.rows || 9),
    frameCols:
      info && info.validFrames[row] && info.validFrames[row].length
        ? info.validFrames[row]
        : null,
  };
  animEpoch = performance.now();
  if (!animRafId) animRafId = requestAnimationFrame(animTick);
  return true;
}

function animTick(now) {
  animRafId = 0;
  if (!animRun) return;
  const frameCols =
    animRun.frameCols && animRun.frameCols.length ? animRun.frameCols : null;
  const count = frameCols ? frameCols.length : animRun.frames;
  const per = (animRun.durationMs * ANIM_SLOWDOWN) / count;
  const step = Math.floor((now - animEpoch) / per) % count;
  const col = frameCols ? frameCols[step] : step;
  avatar.style.backgroundSize = `${animRun.dispW * animRun.cols}px ${animRun.dispH * animRun.rows}px`;
  avatar.style.backgroundPosition = `${-col * animRun.dispW}px ${-animRun.row * animRun.dispH}px`;
  animRafId = requestAnimationFrame(animTick);
}

// ---- 皮肤装配 ----

const STATES = ["idle", "thinking", "speaking", "alert"];
const DEFAULT_BUBBLES = {
  idle: "OwO",
  thinking: "思考中…",
  speaking: "说完了",
  alert: "需要你",
};

let config = { images: {}, anims: null, frame: null, bubbles: DEFAULT_BUBBLES };
let currentSkinId = "";
let skinIndex = { default: "", skins: [] };
let status = "idle";

const ASSETS = "/pet-assets/skins";

function skinEntry(id) {
  return skinIndex.skins.find((item) => item.id === id) || null;
}

/// 素材预加载探测：在就返回 true。
function probeAsset(url) {
  return new Promise((resolve) => {
    const probe = new Image();
    probe.onload = () => resolve(true);
    probe.onerror = () => resolve(false);
    probe.src = url;
  });
}

/// 切换形象：**先确认素材可达再装配**。
///
/// 为什么不直接 applySkin：素材缺失时页面只剩一个气泡、形象凭空消失，用户完全
/// 无法判断发生了什么（上游 petdex-mika 的 skin.json 就引用了不存在的
/// spritesheet.webp，切过去桌宠看着就像"自己没了"）。宁可不切，也不能切完变空。
async function switchSkin(entry, silent) {
  const idle = entry.states.idle || {};
  const file = idle.sheet || idle.image;
  if (file) {
    const ok = await probeAsset(`${ASSETS}/${entry.id}/${file}`);
    if (!ok) {
      if (!silent) sayTemp("这套形象素材缺失");
      return false;
    }
  }
  applySkin(entry);
  return true;
}

/// 静态图加载失败（文件损坏/被删）的兜底：换回默认形象，绝不留空白。
avatarImg.addEventListener("error", () => {
  const fallback = skinEntry(skinIndex.default) || skinIndex.skins[0];
  if (fallback && fallback.id !== currentSkinId) applySkin(fallback);
});

/// 皮肤偏好存壳侧文件（`<data_root>/pet.json`），不用 localStorage：
/// 桌宠页来自 `http://127.0.0.1:<端口>`，核心每次都 `--port 0` 重分配端口，
/// origin 一变 localStorage 就是空的 —— 用户换的皮肤重启即丢。
async function saveSkinPref(id) {
  if (shellBridge && typeof shellBridge.setPref === "function") {
    try {
      await shellBridge.setPref({ skin: id });
      return;
    } catch {
      /* 壳侧不可写：回落 localStorage */
    }
  }
  try {
    localStorage.setItem("owo.pet.skin", id);
  } catch {
    /* 浏览器预览模式下的兜底 */
  }
}

async function loadSkinPref() {
  if (shellBridge && typeof shellBridge.getPref === "function") {
    try {
      const pref = await shellBridge.getPref();
      if (pref && pref.skin) return pref.skin;
    } catch {
      /* IPC 不可用（旧壳/预览）：回落 localStorage */
    }
  }
  try {
    return localStorage.getItem("owo.pet.skin") || "";
  } catch {
    return "";
  }
}

/// 静态图皮肤（states.<s>.image）与帧动画皮肤（states.<s>.sheet）共用一套装配。
///
/// **注意 sheet 必须拼成完整 URL**：skin.json 里写的是裸文件名（`spritesheet.webp`），
/// 直接丢给 `background-image: url(...)` 会相对当前页 `/pet/` 解析成
/// `/pet/spritesheet.webp` → 404 → 形象整只消失，只剩气泡（踩过，且因为静态皮肤
/// 走的是另一条分支（img.src 由本函数拼好）所以只有帧动画皮肤会中招）。
function applySkin(entry) {
  const images = {};
  const anims = {};
  for (const state of STATES) {
    const holder = entry.states[state];
    if (!holder) continue;
    if (holder.sheet) {
      anims[state] = { ...holder, sheet: `${ASSETS}/${entry.id}/${holder.sheet}` };
    } else if (holder.image) {
      images[state] = holder.image;
    }
  }
  config = {
    images,
    anims: Object.keys(anims).length ? anims : null,
    frame: entry.frame || null,
    bubbles: entry.bubbles || DEFAULT_BUBBLES,
  };
  currentSkinId = entry.id;
  if (globalThis.OwoPetActor?.role === "planner") saveSkinPref(entry.id);
  render(status);
}

async function loadSkinIndex() {
  const response = await window.OwoApi.resource(`${ASSETS}/index.json`, { cache: "no-store" });
  if (!response.ok) throw new Error(`皮肤清单 HTTP ${response.status}`);
  skinIndex = await response.json();
}

/// 恢复上次皮肤；偏好里已没有（旧皮肤下架）时回落默认。
async function restoreSkin() {
  const role = globalThis.OwoPetActor?.role;
  const bound = new URLSearchParams(globalThis.location?.search || "").get("skin") || { implementer: "petdex-coco", w1: "petdex-coco", reviewer: "petdex-boba" }[role];
  if (bound && skinEntry(bound)) return bound;
  const saved = await loadSkinPref();
  if (saved && skinEntry(saved)) return saved;
  if (skinIndex.default && skinEntry(skinIndex.default)) return skinIndex.default;
  return skinIndex.skins.length ? skinIndex.skins[0].id : "";
}

globalThis.OwoPetActor = { render, role: new URLSearchParams(globalThis.location?.search || "").get("role") || "planner" };
// The commander reads the brief in place; sprite motion carries its state.
pet.classList.toggle("commander", ["planner", "lead"].includes(globalThis.OwoPetActor.role));

function render(next) {
  if ((globalThis.OwoFeedUI?.controller.ref || !activity.pending_approvals) && globalThis.OwoFeedUI?.state?.()) next = globalThis.OwoFeedUI.state();
  status = next || "idle";
  // 只切状态类，保留 dragging/dropped/antic/happy 等临时动画类。
  for (const s of STATES) pet.classList.toggle(s, s === status);
  const anim = (config.anims && config.anims[status]) || null;
  if (anim && startAnim(anim)) {
    avatarImg.style.display = "none";
    avatar.classList.add("is-anim");
  } else {
    const file = config.images[status];
    // 既没有帧动画也没有静态图（皮肤数据异常）时**保持当前画面**，不清成空白：
    // 「形象旧一帧」远好过「形象凭空消失」——后者用户只会看到气泡，无从判断。
    if (file) {
      stopAnim();
      avatar.classList.remove("is-anim");
      avatar.style.backgroundImage = "";
      avatar.style.width = "";
      avatar.style.height = "";
      animSheetLoaded = "";
      avatarImg.style.display = "";
      const nextAvatar = `${ASSETS}/${currentSkinId}/${file}`;
      if (!avatarImg.src.endsWith(nextAvatar)) avatarImg.src = nextAvatar;
    }
  }
  // 临时台词没说完前不被状态轮询覆盖。
  if (!sayActive && globalThis.OwoFeedUI && (globalThis.OwoFeedUI.controller.ref || !activity.pending_approvals) && globalThis.OwoFeedUI.text()) bubble.textContent = globalThis.OwoFeedUI.text();
  else if (!sayActive) bubble.textContent = describeHead() || config.bubbles[status] || DEFAULT_BUBBLES.idle;
}

// ---- 数据通道：全部经 window.OwoApi ----
//
// 内核用 `--port 0` 随机端口，`OwoApi.ensureCoreConnection()` 负责把真实端口同步到
// 单例上。桌宠窗口可能比工作台先起来，所以每次请求前都问一次（内部有冷却，不会
// 造成请求风暴）。

async function syncBase() {
  const client = globalThis.OwoApi;
  if (!client) return null;
  if (typeof client.ensureCoreConnection === "function") {
    try {
      await client.ensureCoreConnection();
    } catch {
      /* 核心还没起来：沿用旧 baseUrl，本次请求失败由调用方吞掉 */
    }
  }
  return client;
}

/// 统一出口：认证 / 401 重查连接 / 重试都由 ApiClient 负责，这里只做静默降级。
async function api(path, options) {
  const client = await syncBase();
  if (!client) return null;
  try {
    return await client.request(path, options);
  } catch {
    return null;
  }
}

// ---- 活跃回合 → 桌宠状态 ----
//
// phase 由内核 `update_activity` 从 TurnEvent 推导：starting / thinking /
// speaking / tool / waiting_approval。映射保持 alert 优先，避免审批提示被后续
// 的 token_delta 冲掉。

const PHASE_TO_STATE = {
  waiting_approval: "alert",
  starting: "thinking",
  thinking: "thinking",
  speaking: "speaking",
  tool: "speaking",
};
const PHASE_LABEL = {
  starting: "准备中",
  thinking: "思考中",
  speaking: "回复中",
  tool: "执行工具",
  waiting_approval: "等你审批",
};

let activity = { active: [], pending: [], pending_approvals: 0 };
const approvalSeenAt = new Map();

/// 审批倒计时：服务端 300s 超时按拒绝处理（audit 里 `permission denied` 的真凶）。
/// pending 项本身不带发起时间，这里以"桌宠首次看到它"起算。
const APPROVAL_TIMEOUT_MS = 300000;

function approvalRemaining(requestId) {
  const seen = approvalSeenAt.get(requestId);
  if (!seen) return null;
  return Math.max(0, APPROVAL_TIMEOUT_MS - (Date.now() - seen));
}

function formatRemaining(ms) {
  const total = Math.ceil(ms / 1000);
  const mm = String(Math.floor(total / 60)).padStart(2, "0");
  const ss = String(total % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}

function noteApprovals(items) {
  const alive = new Set();
  for (const item of items) {
    alive.add(item.request_id);
    if (!approvalSeenAt.has(item.request_id)) {
      approvalSeenAt.set(item.request_id, Date.now());
    }
  }
  for (const id of [...approvalSeenAt.keys()]) {
    if (!alive.has(id)) approvalSeenAt.delete(id);
  }
}

function deriveStateFromActivity() {
  if ((activity.active || []).length) {
    const head = activity.active[0];
    return PHASE_TO_STATE[head.phase] || "thinking";
  }
  return activity.pending_approvals > 0 ? "alert" : "idle";
}

function describeHead() {
  const head = (activity.active || [])[0];
  if (!head) {
    return activity.pending_approvals ? `待审批 ${activity.pending_approvals} 项` : "";
  }
  const label = PHASE_LABEL[head.phase] || head.phase || "运行中";
  const title = head.title || "会话";
  return head.tool ? `${title} · ${label}：${head.tool}` : `${title} · ${label}`;
}

async function refreshActivity() {
  const snapshot = await api("/activity", { method: "GET" });
  if (snapshot) {
    activity.active = Array.isArray(snapshot.active) ? snapshot.active : [];
    activity.pending_approvals = Number(snapshot.pending_approvals || 0);
  }
  const pending = await api("/approvals/pending", { method: "GET" });
  activity.pending = Array.isArray(pending && pending.pending) ? pending.pending : [];
  noteApprovals(activity.pending);
  render(deriveStateFromActivity());
}

// ---- 右键菜单：把当前能做的事一次性列出来 ----
// 唯一入口是**右键**（左键留给逗它玩，见文件头「按键口径」）：能做什么全写在菜单
// 里、按状态出现，不需要记"单击做什么、双击做什么"。

function closeMenu() {
  menu.hidden = true;
  menu.replaceChildren();
}

function menuItem(label, onClick, extraClass) {
  const item = document.createElement("button");
  item.type = "button";
  item.className = "skin-item" + (extraClass ? " " + extraClass : "");
  const text = document.createElement("span");
  text.textContent = label;
  item.append(text);
  item.addEventListener("click", () => {
    closeMenu();
    Promise.resolve()
      .then(onClick)
      .catch(() => {
        /* 单项失败不打断菜单 */
      });
  });
  return item;
}

function menuHead(text, extraClass) {
  const head = document.createElement("div");
  head.className = "menu-head" + (extraClass ? " " + extraClass : "");
  head.textContent = text;
  return head;
}

function placeMenu(x, y) {
  const rect = menu.getBoundingClientRect();
  menu.style.left = Math.max(6, Math.min(x - rect.width / 2, 214 - rect.width)) + "px";
  menu.style.top = Math.max(6, Math.min(y - 10, 254 - rect.height)) + "px";
}

async function decide(requestId, sessionId, allow) {
  const result = await api(`/session/${sessionId}/permission/${requestId}`, {
    method: "POST",
    json: { allow },
  });
  sayTemp(result ? (allow ? "已允许" : "已拒绝") : "没成功，去工作台看看");
  if (result) setTimeout(refreshActivity, 300);
}

async function abortCurrent(sessionId) {
  const result = await api(`/session/${sessionId}/abort`, { method: "POST", json: {} });
  sayTemp(result ? "已请求停止" : "停止没成功");
  if (result) setTimeout(refreshActivity, 500);
}

function openMenu(x, y) {
  menu.replaceChildren();
  menu.appendChild(menuHead(globalThis.OwoFeedUI?.text() || describeHead() || "空闲"));
  if (globalThis.OwoFeedUI) {
    menu.appendChild(menuItem("喂入剪贴板需求", () => globalThis.OwoFeedUI.paste()));
    menu.appendChild(menuItem("查看任务与成果", () => globalThis.OwoFeedUI.controller.open()));
    if ((globalThis.OwoFeedUI.controller.busy || globalThis.OwoFeedUI.controller.interrupted) && globalThis.OwoFeedUI.controller.ref?.id) menu.appendChild(menuItem("停止这次任务", () => globalThis.OwoFeedUI.controller.cancel(), "is-deny"));
  }

  const boundTeam = !!globalThis.OwoFeedUI?.controller.ref;
  if(boundTeam && activity.pending.length && shellBridge?.openApprovals)menu.appendChild(menuItem("打开审批中心（核对任务后处理）",()=>shellBridge.openApprovals()));
  if(globalThis.OwoFeedUI?.controller.ref?.pending){
    menu.appendChild(menuItem("查找已启动的任务",()=>globalThis.OwoFeedUI.controller.reconcile()));
    menu.appendChild(menuItem("解除不确定绑定",()=>{if(globalThis.confirm("任务可能仍在运行。已查看团队记录并确认可以解除观察绑定？"))return globalThis.OwoFeedUI.controller.forget();}));
  }
  const waiting = boundTeam ? null : (activity.pending || [])[0];
  if (waiting) {
    const remaining = approvalRemaining(waiting.request_id);
    menu.appendChild(
      menuItem(
        `允许：${waiting.tool || "操作"}${
          remaining !== null ? ` · 剩 ${formatRemaining(remaining)}` : ""
        }`,
        () => decide(waiting.request_id, waiting.session_id, true),
        "is-allow"
      )
    );
    menu.appendChild(
      menuItem(`拒绝：${waiting.tool || "操作"}`, () =>
        decide(waiting.request_id, waiting.session_id, false)
      , "is-deny")
    );
  }
  const head = boundTeam ? null : (activity.active || [])[0];
  if (head && head.session_id) {
    menu.appendChild(menuItem("停止当前回合", () => abortCurrent(head.session_id), "is-deny"));
  }
  if (shellBridge && typeof shellBridge.showWorkbench === "function") {
    menu.appendChild(menuItem("打开工作台", () => shellBridge.showWorkbench()));
  }
  // 皮肤区：只列清单里真实存在的（缺 idle 的在生成 index 时已剔除）。
  if (skinIndex.skins.length && !globalThis.OwoFeedUI) {
    menu.appendChild(menuHead("换形象", "menu-sub"));
    for (const entry of skinIndex.skins) {
      menu.appendChild(
        menuItem(entry.name, () => switchSkin(entry), entry.id === currentSkinId ? "is-active" : "")
      );
    }
  }
  if (shellBridge && typeof shellBridge.setVisible === "function") {
    menu.appendChild(menuItem("隐藏桌宠", async () => {
      // 写期望值而不是直接隐藏：所有显隐入口都只写这一个真相源（见 reportVisibility 说明）。
      visibleRegistry = false;
      await api("/desktop/pet", { method: "POST", json: { visible: false } });
      await shellBridge.setVisible(false);
    }));
    if (typeof shellBridge.resetPosition === "function") {
      menu.appendChild(menuItem("回到右下角", () => shellBridge.resetPosition()));
    }
  }
  menu.hidden = false;
  placeMenu(x, y);
}

// 右键＝功能面：开着就关（toggle），关着就按当前状态重列一遍。
pet.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  if (!menu.hidden) {
    closeMenu();
    return;
  }
  openMenu(event.clientX, event.clientY);
});

document.addEventListener("mousedown", (event) => {
  if (!menu.hidden && !menu.contains(event.target)) closeMenu();
});
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !menu.hidden) closeMenu();
});

// ---- 拖动：交给壳移动真实窗口 ----
//
// 两个老坑必须守住：① 用 PointerCapture，指针移出元素后 move/up 仍送达；
// ② screenX/Y 是 CSS 像素，moveBy 按物理像素走，Windows 缩放 125%/150% 时不乘
// devicePixelRatio 会出现"桌宠追不上鼠标、像被粘住"。

// 拖动＝纯移动：这里不再记账"划动距离"。旧实现累计 >420px 就顺手摸摸头
// （`dragDistance` / `pettedThisDrag` / `petCooldown`），后果是把桌宠从屏幕这头
// 挪到那头会一路飘 ❤，拖动途中还夹着 happy 摇动——移窗这种纯搬迁动作被塞了
// 互动语义。按 2026-10-01 口径删除：要互动就点一下（见 endDrag 的左键分支）。
//
// 5px 死区必须**闩住**（`down.moved`）而不是每帧与按下点比距离：手指拖出去再
// 拖回来时，"离按下点"的距离会重新变 0，窗口就被判定为不需要跟随（桌宠被甩在
// 鼠标后面，下次移动又突然追回去／累积偏移），同时松手时还会被误判成"单击"
// 顺手摸摸头。闩住之后：死区只影响开头那 5px，之后每一段位移都照实交给壳，
// 而"是点击还是拖动"也改用同一把尺子（动过就是拖动）。
let down = null;
let lastPointer = null;

pet.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  try {
    pet.setPointerCapture(event.pointerId);
  } catch {
    /* capture 失败不影响拖动本身 */
  }
  down = { x: event.screenX, y: event.screenY, moved: false };
  lastPointer = { x: event.screenX, y: event.screenY };
  pet.classList.add("dragging");
  pet.classList.remove("dropped");
});

pet.addEventListener("pointermove", (event) => {
  if (!down || event.buttons !== 1) return;
  const dx = event.screenX - lastPointer.x;
  const dy = event.screenY - lastPointer.y;
  lastPointer = { x: event.screenX, y: event.screenY };
  const total = Math.hypot(event.screenX - down.x, event.screenY - down.y);
  // 5px 死区：单击的微小抖动不应推动窗口；越过一次就永久生效（回拖也不失效）。
  if (total > 5) down.moved = true;
  if (down.moved && (dx || dy) && shellBridge && typeof shellBridge.moveBy === "function") {
    const dpr = globalThis.devicePixelRatio || 1;
    shellBridge.moveBy(Math.round(dx * dpr), Math.round(dy * dpr));
  }
});

function endDrag(event, movedOverride) {
  if (!down) return;
  // 判"点击 vs 拖动"与指针移动用的是同一个闩：只要拖动过程中越过过死区就算拖动。
  const moved =
    movedOverride === true ||
    down.moved === true ||
    Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5;
  try {
    pet.releasePointerCapture(event.pointerId);
  } catch {
    /* 已释放 */
  }
  down = null;
  lastPointer = null;
  pet.classList.remove("dragging");
  if (moved) {
    pet.classList.add("dropped");
    setTimeout(() => pet.classList.remove("dropped"), 620);
    return;
  }
  // 左键点击＝只互动：收起右键菜单（若还开着）＋摸摸头（happy 摇动 + ❤×4 + 皮肤台词）。
  if (!menu.hidden) closeMenu();
  petted();
}

pet.addEventListener("pointerup", (event) => endDrag(event));
pet.addEventListener("pointercancel", (event) => endDrag(event, true));
window.addEventListener("pointerup", () => {
  if (down) {
    down = null;
    lastPointer = null;
    pet.classList.remove("dragging");
  }
});

// ---- idle 彩蛋与摸摸头 ----

let anticTimer = 0;
function scheduleAntic() {
  clearTimeout(anticTimer);
  anticTimer = setTimeout(() => {
    if (status === "idle" && !down && !pet.classList.contains("commander")) {
      pet.classList.add("antic");
      setTimeout(() => pet.classList.remove("antic"), 1700);
    }
    scheduleAntic();
  }, 9000 + Math.random() * 15000);
}

const PET_LINES = {
  "petdex-nailong": ["duang～再摸摸", "肚肚不许戳！", "龙龙很满意", "嘿嘿嘿"],
  "petdex-coco": ["呼噜呼噜…", "下巴这边再挠挠", "尾巴不许拽！", "喵呜～好舒服"],
};
const PET_FALLBACK_LINES = ["嘿嘿，好痒～", "再摸摸我嘛", "(*´▽`*)"];

let sayTimer = 0;
let sayActive = false;

function pick(list) {
  return list[Math.floor(Math.random() * list.length)];
}

function sayTemp(text, ms) {
  clearTimeout(sayTimer);
  sayActive = true;
  bubble.textContent = text;
  sayTimer = setTimeout(() => {
    sayActive = false;
    bubble.textContent = describeHead() || config.bubbles[status] || DEFAULT_BUBBLES.idle;
  }, ms || 2200);
}

function spawnFx(glyph, count) {
  for (let i = 0; i < count; i++) {
    const s = document.createElement("span");
    s.className = "fx";
    s.textContent = glyph;
    s.style.left = 40 + Math.floor(Math.random() * 120) + "px";
    s.style.top = 55 + Math.floor(Math.random() * 60) + "px";
    s.style.animationDelay = (Math.random() * 0.3).toFixed(2) + "s";
    s.style.setProperty("--fx-rot", Math.floor(Math.random() * 50 - 25) + "deg");
    fxLayer.appendChild(s);
    setTimeout(() => s.remove(), 1700);
  }
}

function petted() {
  pet.classList.add("happy");
  setTimeout(() => pet.classList.remove("happy"), 1100);
  spawnFx("❤", 4);
  sayTemp(pick(PET_LINES[currentSkinId] || PET_FALLBACK_LINES));
}

// ---- 显隐（内核期望值是唯一真相源）----
//
// 规则：**可见性只由内核的 `desired` 决定**，本页不自己发明状态。
// 心跳（10s）做三件事：
//   1. 上报自己当前的显隐，让服务端算出 overlay_online（判据：15 秒内有心跳）；
//   2. 取回 desired（工作台开关 / 托盘菜单 / 桌宠菜单三处写入的都是它）；
//   3. 与窗口**真实**可见性对账——不一致就纠正。
//
// 第 3 步是必需的：只要有一次隐藏不是本页发起的（窗口被外部 hide、被拖出可视区），
// 页面还傻傻以为"我是可见的"，就会永远不再把它弄回来，表现为"桌宠凭空消失"。
//
// 心跳间隔必须 < 15s，否则工作台的桌宠开关会一直显示"桌面端离线"。

let visibleRegistry = true;

async function reportVisibility() {
  const result = await api("/desktop/pet/report", {
    method: "POST",
    json: { visible: visibleRegistry },
  });
  const bridge = shellBridge;
  if (!bridge || typeof bridge.query !== "function") return;
  const truth = await bridge.query();
  if (!truth || truth.alive === false) return;
  // desired 为 null = 用户从未动过开关 → 保持可见（桌宠的价值就在于被看到）。
  const want = result && typeof result.desired === "boolean" ? result.desired : true;
  if (truth.visible !== want) {
    visibleRegistry = want;
    await bridge.setVisible(want);
  } else {
    visibleRegistry = truth.visible;
  }
}

// ---- 启动 ----

async function boot() {
  render("idle");
  scheduleAntic();
  try {
    await loadSkinIndex();
  } catch {
    return; // 清单都拿不到：静态资源没挂上，保持默认外观即可（浏览器预览）
  }
  // 皮肤恢复单独兜底：它失败**绝不能**连带把状态/审批能力一起废掉
  // （桌宠的主要价值是盯回合与放行审批，好看是次要的）。
  try {
    const wanted = await restoreSkin();
    const entry = skinEntry(wanted);
    if (entry && !(await switchSkin(entry, true))) {
      // 记在案的皮肤素材已损坏（文件被删/上游就缺图）：换回默认并把偏好改掉，
      // 否则每次启动都要再失败一次。
      const fallback = skinEntry(skinIndex.default) || skinIndex.skins[0];
      if (fallback) applySkin(fallback);
      sayTemp("上次的形象素材缺失，已换回默认");
    }
  } catch {
    /* 保持默认外观，继续往下走 */
  }
  // 心跳与状态轮询都是低频轻请求，失败了自然重来，不必打断桌宠显示。
  if (globalThis.OwoPetActor?.role === "planner") {
    reportVisibility();
    setInterval(reportVisibility, 10000);
  }
  schedulePoll(0);
}

/// 自适应节奏：有活跃回合时紧跟（1.5s），空闲时放松（6s）。
function schedulePoll(delayMs) {
  setTimeout(async () => {
    await refreshActivity();
    const busy = (activity.active || []).length > 0 || activity.pending_approvals > 0;
    schedulePoll(busy ? 1500 : 6000);
  }, delayMs);
}

boot();
