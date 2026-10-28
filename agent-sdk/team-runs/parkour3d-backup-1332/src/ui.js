/**
 * src/ui.js — HUD 纯逻辑层（不依赖真实 DOM，不导入 three）
 *
 * 设计要点：
 * - 所有 DOM 访问都经过 createHud(injectDocument) 注入的元素桩，模块加载时无任何副作用。
 * - 计时（tick）、得分（进度 + 检查点 + 终点奖励）与 restart 重置都是纯数据运算。
 * - formatHud / formatTime / formatScore 为可复用的稳定格式化输出。
 */

export const PROGRESS_POINTS = 1000;
export const CHECKPOINT_BONUS = 100;
export const VICTORY_BONUS = 500;

export const STATUS_LABELS = Object.freeze({
  ready: '准备中',
  running: '进行中',
  paused: '暂停',
  fallen: '重生中',
  victory: '胜利'
});

export const ELEMENT_IDS = Object.freeze({
  root: 'hud',
  score: 'score',
  time: 'time',
  status: 'status',
  overlay: 'overlay',
  overlayTitle: 'overlay-title',
  overlayText: 'overlay-text',
  restart: 'restart'
});

function toNumber(value, fallback = 0) {
  const number = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function clamp01(value) {
  if (!(value > 0)) return 0;
  if (value > 1) return 1;
  return value;
}

function pad2(value) {
  return String(value).padStart(2, '0');
}

/** 状态文案：已知状态用中文标签，未知状态原样透出。 */
export function statusLabel(status) {
  const key = typeof status === 'string' && status.length > 0 ? status : 'ready';
  const label = STATUS_LABELS[key];
  return label === undefined ? key : label;
}

/** 秒 -> "MM:SS.s"（稳定、非负，超过 59 分钟时分钟位继续增长）。 */
export function formatTime(time) {
  const total = Math.max(0, toNumber(time, 0));
  const totalTenths = Math.floor(total * 10 + 1e-6);
  const tenths = totalTenths % 10;
  const totalSeconds = Math.floor(totalTenths / 10);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60);
  return `${pad2(minutes)}:${pad2(seconds)}.${tenths}`;
}

/** 分数格式化：四舍五入取整且永不为负。 */
export function formatScore(score) {
  return String(Math.round(Math.max(0, toNumber(score, 0))));
}

/** 得分规则：进度分 + 检查点奖励 + 终点奖励，恒为非负整数。 */
export function computeScore(state) {
  const source = state && typeof state === 'object' ? state : {};
  const progress = clamp01(toNumber(source.progress, 0));
  const checkpoints = Math.max(0, Math.floor(toNumber(source.checkpoints, 0)));
  const victory = source.finished === true || source.status === 'victory';
  const raw = Math.round(progress * PROGRESS_POINTS) + checkpoints * CHECKPOINT_BONUS +
    (victory ? VICTORY_BONUS : 0);
  return Math.max(0, raw);
}

/** 单行 HUD 文本，供无 DOM 环境（测试、日志、overlay）复用。 */
export function formatHud(state) {
  const source = state && typeof state === 'object' ? state : {};
  const score = formatScore(source.score);
  const time = formatTime(source.time);
  const status = statusLabel(source.status);
  return `得分 ${score} | 时间 ${time} | 状态 ${status}`;
}

/** 生成一份归一化且合法的 HUD 状态（得分单调：不低于由进度/检查点/终点推导出的分数）。 */
export function createHudState(overrides) {
  const source = overrides && typeof overrides === 'object' ? overrides : {};
  const status = typeof source.status === 'string' && source.status.length > 0 ? source.status : 'ready';
  const state = {
    score: Math.max(0, toNumber(source.score, 0)),
    time: Math.max(0, toNumber(source.time, 0)),
    progress: clamp01(toNumber(source.progress, 0)),
    checkpoints: Math.max(0, Math.floor(toNumber(source.checkpoints, 0))),
    status,
    finished: source.finished === true || status === 'victory'
  };
  state.score = Math.max(state.score, computeScore(state));
  return state;
}

export const INITIAL_HUD_STATE = Object.freeze(createHudState());

function setText(element, text) {
  if (!element) return;
  const value = String(text);
  try {
    element.textContent = value;
  } catch {
    /* 只读元素桩：忽略写入失败 */
  }
}

function setHidden(element, hidden) {
  if (!element) return;
  const flag = hidden === true;
  try {
    element.hidden = flag;
  } catch {
    /* 忽略 */
  }
  try {
    if (element.classList && typeof element.classList.toggle === 'function') {
      element.classList.toggle('hidden', flag);
    }
  } catch {
    /* 忽略 */
  }
  try {
    if (element.dataset && typeof element.dataset === 'object') {
      element.dataset.state = flag ? 'hidden' : 'visible';
    }
    if (element.style && typeof element.style === 'object') {
      element.style.display = flag ? 'none' : 'flex';
    }
  } catch {
    /* 忽略 */
  }
}

/**
 * 把状态写入注入的元素桩。
 * @returns {object|null} 归一化后的状态副本；hud 非法时返回 null。
 */
export function updateHud(hud, state) {
  if (!hud || typeof hud !== 'object') return null;
  const current = hud.state && typeof hud.state === 'object' ? hud.state : {};
  const patch = state && typeof state === 'object' ? state : current;
  const next = createHudState({ ...current, ...patch });
  hud.state = next;

  const elements = hud.elements && typeof hud.elements === 'object' ? hud.elements : {};
  setText(elements.score, formatScore(next.score));
  setText(elements.time, formatTime(next.time));
  setText(elements.status, statusLabel(next.status));

  const showOverlay = next.finished === true;
  setHidden(elements.overlay, !showOverlay);
  if (elements.overlayTitle) {
    setText(elements.overlayTitle, showOverlay ? '到达终点！' : '准备开始');
  }
  if (elements.overlayText) {
    setText(
      elements.overlayText,
      showOverlay ? `本局结束 · ${formatHud(next)}` : '按 Enter 或点击下方按钮开始 / 重新开始'
    );
  }
  return { ...next };
}

/**
 * 创建 HUD 控制器：所有 DOM 都来自注入的 documentRef（可为 null），并自带 restart 重置。
 */
export function createHud(documentRef, options) {
  const documentLike =
    documentRef && typeof documentRef.getElementById === 'function' ? documentRef : null;
  const ids = { ...ELEMENT_IDS, ...((options && options.ids) || {}) };
  const lookup = (id) => {
    if (!documentLike || !id) return null;
    try {
      const element = documentLike.getElementById(id);
      return element === undefined ? null : element;
    } catch {
      return null;
    }
  };

  const elements = {
    root: lookup(ids.root),
    score: lookup(ids.score),
    time: lookup(ids.time),
    status: lookup(ids.status),
    overlay: lookup(ids.overlay),
    overlayTitle: lookup(ids.overlayTitle),
    overlayText: lookup(ids.overlayText),
    restart: lookup(ids.restart)
  };

  const hud = { elements, state: createHudState() };

  hud.render = () => updateHud(hud, hud.state);

  /** 累加运行时间（秒）。非法/非正增量、结束后的 tick 均被忽略。 */
  hud.tick = (deltaSeconds) => {
    const delta = toNumber(deltaSeconds, 0);
    if (delta > 0 && hud.state.finished !== true) {
      updateHud(hud, {
        ...hud.state,
        time: hud.state.time + delta,
        status: hud.state.status === 'ready' ? 'running' : hud.state.status
      });
    }
    return { ...hud.state };
  };

  /** 设置进度（0..1），得分随进度提升且不回退。 */
  hud.setProgress = (progress) => {
    updateHud(hud, {
      ...hud.state,
      progress,
      status: hud.state.status === 'ready' ? 'running' : hud.state.status
    });
    return { ...hud.state };
  };

  /** 通过一个检查点，追加检查点奖励。 */
  hud.addCheckpoint = () => {
    updateHud(hud, {
      ...hud.state,
      checkpoints: hud.state.checkpoints + 1,
      status: hud.state.status === 'ready' ? 'running' : hud.state.status
    });
    return { ...hud.state };
  };

  /** 到达终点：状态切到胜利并追加终点奖励，显示 overlay。 */
  hud.finish = () => {
    updateHud(hud, { ...hud.state, status: 'victory', finished: true });
    return { ...hud.state };
  };

  /** 重置：分数、计时、进度、检查点与玩家状态归零，返回新一局初始状态。 */
  hud.restart = () => {
    hud.state = createHudState();
    updateHud(hud, hud.state);
    return { ...hud.state };
  };

  hud.reset = hud.restart;

  if (elements.restart && typeof elements.restart.addEventListener === 'function') {
    elements.restart.addEventListener('click', (event) => {
      if (event && typeof event.preventDefault === 'function') event.preventDefault();
      hud.restart();
    });
  }

  hud.render();
  return hud;
}

export default { formatHud, formatTime, formatScore, computeScore, createHud, updateHud, createHudState };
