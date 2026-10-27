/**
 * src/ui.js — 纯逻辑 HUD 模块（不导入 THREE，不直接依赖 window）
 *
 * 设计约定：
 *  - 所有 DOM 访问都通过注入的根元素 / 文档对象（rootElement / ownerDocument）完成，
 *    因此本模块可在 Node 中用最小桩对象测试，无需 jsdom 或真实浏览器。
 *  - 导出：formatTime / computeScore / createHud / bindRestart / getHudElementIds
 *          （兼容别名：formatHud / updateHud）
 */

/** 与 index.html 中元素 id 的单一事实来源。 */
export const HUD_IDS = Object.freeze({
  canvas: 'game',
  hud: 'hud',
  score: 'hud-score',
  time: 'hud-time',
  status: 'hud-status',
  hint: 'hint',
  overlay: 'overlay',
  overlayTitle: 'overlay-title',
  overlayText: 'overlay-text',
  restart: 'restart',
});

/** 返回 index.html / styles.css 中使用的 id 映射（每次返回新对象，避免外部改写常量）。 */
export function getHudElementIds() {
  return { ...HUD_IDS };
}

/** 计分规则（确定、可测试，且不依赖任何 DOM）。 */
export const SCORE_RULES = Object.freeze({
  base: 1000,
  perCollectible: 100,
  perMeter: 2,
  victoryBonus: 500,
  timePenaltyPerSecond: 1,
  maxTimePenalty: 600,
});

/** 胜利条件的默认文案。 */
export const HUD_MESSAGES = Object.freeze({
  ready: '按 Enter / 空格 开始',
  running: '跑到终点平台即可通关',
  fallen: '掉落了，回到最近的检查点',
  victoryTitle: '胜利！',
});

const DEFAULT_STATUS_LABELS = Object.freeze({
  ready: HUD_MESSAGES.ready,
  running: HUD_MESSAGES.running,
  fallen: HUD_MESSAGES.fallen,
  finished: HUD_MESSAGES.victoryTitle,
  victory: HUD_MESSAGES.victoryTitle,
});

function pad(value, length) {
  return String(value).padStart(length, '0');
}

function toFiniteNumber(value, fallback = 0) {
  const num = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(num) ? num : fallback;
}

/**
 * 把毫秒格式化成 mm:ss.mmm。
 * 边界：0 -> "00:00.000"，999 -> "00:00.999"，1000 -> "00:01.000"，60000 -> "01:00.000"。
 * 负值 / NaN / undefined 一律按 0 处理，绝不抛错。
 */
export function formatTime(ms) {
  const value = toFiniteNumber(ms, 0);
  const safe = value > 0 ? Math.floor(value) : 0;
  const minutes = Math.floor(safe / 60_000);
  const seconds = Math.floor((safe % 60_000) / 1000);
  const millis = safe % 1000;
  return `${pad(minutes, 2)}:${pad(seconds, 2)}.${pad(millis, 3)}`;
}

/**
 * 计分：基础分 + 收集物分 + 距离分 + 胜利奖励 - 时间惩罚。
 * 收集物 / 距离 / 胜利状态增加 -> 得分增加；时间增加 -> 得分减少；
 * 结果始终为非负整数（未提供字段按 0 处理）。
 */
export function computeScore(state = {}) {
  const input = state && typeof state === 'object' ? state : {};
  const collectibles = Math.max(0, toFiniteNumber(input.collectibles, 0));
  const distance = Math.max(0, toFiniteNumber(input.distance, 0));
  const timeMs = Math.max(0, toFiniteNumber(input.timeMs, 0));
  const finished = Boolean(input.finished);

  const collectiblePoints = Math.round(collectibles) * SCORE_RULES.perCollectible;
  const distancePoints = Math.floor(distance * SCORE_RULES.perMeter);
  const victoryBonus = finished ? SCORE_RULES.victoryBonus : 0;
  const seconds = Math.floor(timeMs / 1000);
  const timePenalty = Math.min(seconds * SCORE_RULES.timePenaltyPerSecond, SCORE_RULES.maxTimePenalty);

  const total = SCORE_RULES.base + collectiblePoints + distancePoints + victoryBonus - timePenalty;
  return Math.max(0, Math.round(total));
}

/** 兼容别名：一次性格式化 HUD 文本值。 */
export function formatHud(state = {}) {
  const input = state && typeof state === 'object' ? state : {};
  const timeMs = input.timeMs ?? input.time ?? 0;
  const score = typeof input.score === 'number' ? input.score : computeScore(input);
  return {
    time: formatTime(timeMs),
    score: String(score),
    scoreLabel: `得分 ${score}`,
    timeLabel: `时间 ${formatTime(timeMs)}`,
    status: resolveStatusLabel(input.status),
  };
}

function resolveStatusLabel(status) {
  if (typeof status !== 'string' || status.length === 0) return DEFAULT_STATUS_LABELS.ready;
  return DEFAULT_STATUS_LABELS[status] || status;
}

/** 从注入的根元素 / 文档对象里取出真正的“文档样”对象。 */
function resolveDocument(rootElement) {
  if (!rootElement) return null;
  if (typeof rootElement.getElementById === 'function' && typeof rootElement.createElement === 'function') {
    return rootElement;
  }
  if (rootElement.ownerDocument && typeof rootElement.ownerDocument.getElementById === 'function') {
    return rootElement.ownerDocument;
  }
  if (rootElement.document && typeof rootElement.document.getElementById === 'function') {
    return rootElement.document;
  }
  if (typeof rootElement.getElementById === 'function') return rootElement;
  return null;
}

function lookupElement(rootElement, doc, id) {
  if (!id) return null;
  const scopes = [];
  if (doc && typeof doc.getElementById === 'function') scopes.push(doc);
  if (rootElement && typeof rootElement.getElementById === 'function') scopes.push(rootElement);
  if (rootElement && typeof rootElement.querySelector === 'function') {
    try {
      const found = rootElement.querySelector(`#${id}`);
      if (found) return found;
    } catch {
      /* 桩对象不支持选择器时忽略 */
    }
  }
  for (const scope of scopes) {
    const found = scope.getElementById(id);
    if (found) return found;
  }
  return null;
}

function collectElements(rootElement) {
  const doc = resolveDocument(rootElement);
  const elements = {};
  for (const [key, id] of Object.entries(HUD_IDS)) {
    if (key === 'canvas') continue;
    elements[key] = lookupElement(rootElement, doc, id);
  }
  return { doc, elements };
}

function setText(element, text) {
  if (!element) return;
  element.textContent = text;
}

function setHidden(element, hidden) {
  if (!element) return;
  element.hidden = hidden;
  if (element.classList && typeof element.classList.toggle === 'function') {
    element.classList.toggle('visible', !hidden);
  } else if (element.classList && typeof element.classList.add === 'function') {
    if (hidden) element.classList.remove('visible');
    else element.classList.add('visible');
  }
}

function readState(state) {
  const input = state && typeof state === 'object' ? state : {};
  const timeMs = toFiniteNumber(input.timeMs ?? input.time ?? 0, 0);
  const score = typeof input.score === 'number' && Number.isFinite(input.score)
    ? Math.round(input.score)
    : computeScore(input);
  const collectibles = Math.max(0, Math.round(toFiniteNumber(input.collectibles, 0)));
  return {
    input,
    timeMs,
    score,
    collectibles,
    finished: Boolean(input.finished),
    status: input.status,
  };
}

/**
 * 在给定根元素（或文档对象）上创建 HUD 控制器。
 * 返回 { update(state), showVictory(state), hide(), dispose(), elements }。
 */
export function createHud(rootElement) {
  const { doc, elements } = collectElements(rootElement);
  let disposed = false;

  function apply(state) {
    const snapshot = readState(state);
    const formatted = formatHud({
      score: snapshot.score,
      timeMs: snapshot.timeMs,
      status: snapshot.status,
    });
    setText(elements.time, formatted.timeLabel);
    setText(elements.score, formatted.scoreLabel);
    setText(elements.status, formatted.status);
    if (elements.hud && elements.hud.classList && typeof elements.hud.classList.remove === 'function') {
      /* 保持 HUD 可见 */
      elements.hud.hidden = false;
    }
    return snapshot;
  }

  const hud = {
    root: rootElement || null,
    document: doc,
    elements,

    /** 用最新游戏状态刷新计时与得分文本。 */
    update(state) {
      if (disposed) return null;
      const snapshot = apply(state);
      return { time: formatTime(snapshot.timeMs), score: snapshot.score };
    },

    /** 显示胜利面板（含胜利文案与最终得分）。 */
    showVictory(state) {
      if (disposed) return null;
      const snapshot = apply({ ...(state || {}), finished: true });
      setText(elements.overlayTitle, HUD_MESSAGES.victoryTitle);
      setText(
        elements.overlayText,
        `得分 ${snapshot.score} · 用时 ${formatTime(snapshot.timeMs)} · 收集 ${snapshot.collectibles}`
      );
      setHidden(elements.overlay, false);
      return { score: snapshot.score, time: formatTime(snapshot.timeMs) };
    },

    /** 隐藏浮层（用于 restart 或展示提示）。 */
    hide() {
      if (disposed) return;
      setHidden(elements.overlay, true);
    },

    /** 释放引用，之后调用任何方法都安全地返回 null。 */
    dispose() {
      if (disposed) return;
      disposed = true;
      hud.elements = {};
    },
  };

  return hud;
}

/**
 * 把 restart 按钮的点击（以及可选的键盘 R 键）绑定到 onRestart。
 * 返回解绑函数；对缺失元素安全降级，绝不抛错。
 */
export function bindRestart(buttonElement, onRestart, options = {}) {
  const callback = typeof onRestart === 'function' ? onRestart : () => {};
  const opts = options && typeof options === 'object' ? options : {};
  const keyTarget = opts.keyTarget || null;

  const handleClick = (event) => {
    if (event && typeof event.preventDefault === 'function') event.preventDefault();
    callback(event);
  };
  const handleKeyDown = (event) => {
    if (!event) return;
    const key = event.key ?? event.code;
    if (key !== 'r' && key !== 'R' && key !== 'KeyR') return;
    if (typeof event.preventDefault === 'function') event.preventDefault();
    callback(event);
  };

  const canClick = buttonElement && typeof buttonElement.addEventListener === 'function';
  if (canClick) buttonElement.addEventListener('click', handleClick);

  const canKey = keyTarget && typeof keyTarget.addEventListener === 'function';
  if (canKey) keyTarget.addEventListener('keydown', handleKeyDown);

  let unbound = false;
  return function unbind() {
    if (unbound) return;
    unbound = true;
    if (canClick && typeof buttonElement.removeEventListener === 'function') {
      buttonElement.removeEventListener('click', handleClick);
    }
    if (canKey && typeof keyTarget.removeEventListener === 'function') {
      keyTarget.removeEventListener('keydown', handleKeyDown);
    }
  };
}

/**
 * 兼容入口：直接更新 HUD。可传 createHud() 的结果，也可传 { document } / 根元素。
 * 优先使用 state.score（若已提供），否则用 computeScore(state) 计算。
 */
export function updateHud(hudOrRoot, state = {}) {
  if (!hudOrRoot) return null;
  if (typeof hudOrRoot.update === 'function') return hudOrRoot.update(state);

  const snapshot = readState(state);
  const { elements } = collectElements(hudOrRoot);
  setText(elements.time, `时间 ${formatTime(snapshot.timeMs)}`);
  setText(elements.score, `得分 ${snapshot.score}`);
  setText(elements.status, resolveStatusLabel(snapshot.status));
  return { time: formatTime(snapshot.timeMs), score: snapshot.score };
}
