/**
 * FocusFlow 专注计时器
 *
 * 这是一个 **经典脚本（classic script）**，不是 ES Module：整个文件包在一个 IIFE 里，
 * 通过 `root.FocusFlow` 暴露 API。因此 index.html 用
 * `<script src="script.js" defer></script>` 引入即可——双击打开 index.html（file:// 协议）
 * 也能正常执行；而 `type="module"` 在 file:// 下会被浏览器以 CORS（origin 'null'）拒绝加载。
 *
 * 这个文件承担三件事：
 * 1. 纯逻辑：时长校验、时间格式化、倒计时引擎（不依赖 DOM，可在 Node 中加载测试）；
 * 2. 把 API 挂到 window.FocusFlow（同一份代码也兼容 CommonJS）；
 * 3. 在浏览器中自动初始化页面交互（开始 / 暂停 / 重置 / 时长调节 / 完成提示）。
 */

(function (root, factory) {
  'use strict';

  var api = factory();

  // 浏览器 -> window.FocusFlow；Node 测试 -> 从 vm 上下文里取同一个对象。
  if (root) root.FocusFlow = api;

  if (typeof module === 'object' && module && module.exports) module.exports = api;

  if (typeof document !== 'undefined' && document && typeof document.addEventListener === 'function') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', function () {
        api.initFocusApp(document);
      }, { once: true });
    } else {
      api.initFocusApp(document);
    }
  }
})(typeof window !== 'undefined' ? window : typeof globalThis !== 'undefined' ? globalThis : null, function focusFlow() {
// 工厂函数体沿用文件原有缩进（不做整体重排），便于审阅 module -> 经典脚本的迁移 diff。
const DEFAULT_DURATION_MINUTES = 25;
const MIN_DURATION_MINUTES = 1;
const MAX_DURATION_MINUTES = 90;

/** 把任意输入收敛到 1–90 之间的整数分钟，非法输入回落到默认 25 分钟。 */
function clampDurationMinutes(value) {
  if (value === undefined || value === null || value === '') return DEFAULT_DURATION_MINUTES;
  const minutes = Number(value);
  if (!Number.isFinite(minutes)) return DEFAULT_DURATION_MINUTES;
  return Math.min(MAX_DURATION_MINUTES, Math.max(MIN_DURATION_MINUTES, Math.round(minutes)));
}

/** 秒数转 mm:ss，负数与非法值都按 00:00 处理。 */
function formatTime(totalSeconds) {
  const raw = Number(totalSeconds);
  const seconds = Number.isFinite(raw) ? Math.max(0, Math.round(raw)) : 0;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}`;
}

/** 分钟数转成中文标签，例如 25 -> "25 分钟"。 */
function formatDurationLabel(minutes) {
  return `${clampDurationMinutes(minutes)} 分钟`;
}

/**
 * 倒计时引擎：不依赖 DOM，时间来源可注入，便于用假时钟测试。
 *
 * @param {object} [options]
 * @param {number} [options.durationMinutes] 初始时长（分钟）
 * @param {() => number} [options.now] 时间来源（毫秒），默认 Date.now
 * @param {(state: object) => void} [options.onTick] 每次状态变化时回调
 * @param {(state: object) => void} [options.onComplete] 倒计时归零时回调
 */
function createFocusTimer(options = {}) {
  const onTick = typeof options.onTick === 'function' ? options.onTick : () => {};
  const onComplete = typeof options.onComplete === 'function' ? options.onComplete : () => {};
  const now = typeof options.now === 'function' ? options.now : () => Date.now();

  let durationSeconds = clampDurationMinutes(options.durationMinutes) * 60;
  let remainingSeconds = durationSeconds;
  let running = false;
  let completed = false;
  let segmentStartedAt = 0;

  function snapshot() {
    return {
      durationSeconds,
      remainingSeconds,
      running,
      completed,
      progress: durationSeconds > 0 ? 1 - remainingSeconds / durationSeconds : 0,
    };
  }

  function emit() {
    onTick(snapshot());
  }

  /** 根据真实流逝时间推进倒计时，返回最新状态。 */
  function tick() {
    if (!running) return snapshot();
    const elapsed = Math.floor((now() - segmentStartedAt) / 1000);
    if (elapsed <= 0) return snapshot();

    segmentStartedAt += elapsed * 1000;
    remainingSeconds = Math.max(0, remainingSeconds - elapsed);

    if (remainingSeconds === 0) {
      running = false;
      completed = true;
      const finished = snapshot();
      emit();
      onComplete(finished);
      return finished;
    }

    emit();
    return snapshot();
  }

  function start() {
    if (running) return snapshot();
    if (remainingSeconds <= 0) {
      remainingSeconds = durationSeconds;
    }
    completed = false;
    running = true;
    segmentStartedAt = now();
    const state = snapshot();
    emit();
    return state;
  }

  function pause() {
    if (!running) return snapshot();
    tick();
    if (running) {
      running = false;
      const state = snapshot();
      emit();
      return state;
    }
    // tick() 内部已因归零而停止，并触发过 onComplete。
    return snapshot();
  }

  function reset() {
    running = false;
    completed = false;
    segmentStartedAt = 0;
    remainingSeconds = durationSeconds;
    const state = snapshot();
    emit();
    return state;
  }

  function toggle() {
    return running ? pause() : start();
  }

  /** 调整时长会重置这一轮计时。 */
  function setDurationMinutes(minutes) {
    durationSeconds = clampDurationMinutes(minutes) * 60;
    return reset();
  }

  return {
    start,
    pause,
    reset,
    toggle,
    tick,
    setDurationMinutes,
    getState: snapshot,
  };
}

/* ------------------------------------------------------------------ */
/* 浏览器侧：DOM 绑定                                                  */
/* ------------------------------------------------------------------ */

/**
 * 把计时器挂载到页面元素上。元素缺失时安全降级，不会抛错。
 * @param {Document} doc
 * @param {{ now?: () => number }} [options] 可选注入时间来源（测试用假时钟）
 */
function initFocusApp(doc, options) {
  if (!doc || typeof doc.querySelector !== 'function') return null;
  const overrides = options || {};

  const timeDisplay = doc.querySelector('#time-display');
  if (!timeDisplay) return null;

  const card = doc.querySelector('[data-timer]');
  const statusEl = doc.querySelector('#status-text');
  const progressRing = doc.querySelector('#progress-ring');
  const toggleBtn = doc.querySelector('#toggle-btn');
  const resetBtn = doc.querySelector('#reset-btn');
  const durationInput = doc.querySelector('#duration-input');
  const durationValue = doc.querySelector('#duration-value');
  const banner = doc.querySelector('#complete-banner');
  const toast = doc.querySelector('#toast');
  const presetButtons = Array.from(doc.querySelectorAll('[data-preset]'));

  const initialMinutes = clampDurationMinutes(durationInput ? durationInput.value : DEFAULT_DURATION_MINUTES);
  const radius = progressRing ? Number(progressRing.getAttribute('r')) || 88 : 88;
  const circumference = 2 * Math.PI * radius;

  if (progressRing) {
    progressRing.style.strokeDasharray = String(circumference);
    progressRing.style.strokeDashoffset = '0';
  }

  let toastTimer = null;
  let intervalId = null;
  let audioContext = null;

  function showToast(message) {
    if (!toast) return;
    toast.textContent = message;
    toast.hidden = false;
    // 强制一次样式读取，保证过渡动画在 hidden 切换后仍然生效。
    void toast.offsetWidth;
    toast.classList.add('is-visible');
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toast.classList.remove('is-visible');
      toast.hidden = true;
    }, 4200);
  }

  /**
   * 创建并「解锁」AudioContext。
   *
   * 必须在用户手势（点击 #toggle-btn）的回调里调用：否则自动播放策略会让 context 停在
   * suspended 状态，倒计时结束时再 new 出来的 AudioContext 不会发声。
   * 这里创建的实例会被 beep() 复用，完成时才真正播放提示音。
   */
  function primeAudio() {
    try {
      const AudioCtor = (typeof window !== 'undefined' && (window.AudioContext || window.webkitAudioContext)) || null;
      if (!AudioCtor) return null;
      if (!audioContext) audioContext = new AudioCtor();
      if (audioContext.state === 'suspended' && typeof audioContext.resume === 'function') {
        const resumed = audioContext.resume();
        if (resumed && typeof resumed.catch === 'function') resumed.catch(() => {});
      }
      return audioContext;
    } catch {
      return null;
    }
  }

  /** 复用已解锁的 AudioContext 播一声短提示音；不支持时静默降级。 */
  function beep() {
    try {
      const ctx = primeAudio();
      if (!ctx || typeof ctx.createOscillator !== 'function' || typeof ctx.createGain !== 'function') return;
      const oscillator = ctx.createOscillator();
      const gain = ctx.createGain();
      oscillator.type = 'sine';
      oscillator.frequency.value = 880;
      gain.gain.setValueAtTime(0.0001, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.22, ctx.currentTime + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.6);
      oscillator.connect(gain);
      gain.connect(ctx.destination);
      oscillator.start();
      oscillator.stop(ctx.currentTime + 0.65);
    } catch {
      /* 浏览器不允许自动播放时静默降级，横幅 / toast / 标题提示照常出现。 */
    }
  }

  function syncPresets(minutes) {
    presetButtons.forEach((button) => {
      const preset = clampDurationMinutes(button.dataset ? button.dataset.preset : button.getAttribute('data-preset'));
      button.setAttribute('aria-pressed', String(preset === minutes));
    });
  }

  function stopInterval() {
    if (intervalId !== null) {
      clearInterval(intervalId);
      intervalId = null;
    }
  }

  function ensureInterval() {
    if (intervalId !== null || typeof setInterval !== 'function') return;
    intervalId = setInterval(() => {
      timer.tick();
    }, 250);
  }

  function render(state) {
    timeDisplay.textContent = formatTime(state.remainingSeconds);

    if (statusEl) {
      if (state.completed) statusEl.textContent = '专注完成';
      else if (state.running) statusEl.textContent = '专注中…';
      else if (state.remainingSeconds < state.durationSeconds) statusEl.textContent = '已暂停';
      else statusEl.textContent = '准备开始';
    }

    if (toggleBtn) {
      if (state.completed) toggleBtn.textContent = '再来一轮';
      else if (state.running) toggleBtn.textContent = '暂停';
      else if (state.remainingSeconds < state.durationSeconds) toggleBtn.textContent = '继续';
      else toggleBtn.textContent = '开始专注';
      toggleBtn.setAttribute('aria-pressed', String(state.running));
      if (toggleBtn.classList) toggleBtn.classList.toggle('is-running', state.running);
    }

    if (progressRing) {
      progressRing.style.strokeDashoffset = String(circumference * (1 - state.progress));
    }

    if (banner) banner.hidden = !state.completed;
    if (card && card.classList) card.classList.toggle('is-completed', state.completed);

    if (durationValue) durationValue.textContent = formatDurationLabel(state.durationSeconds / 60);
    if (durationInput && Number(durationInput.value) !== state.durationSeconds / 60) {
      durationInput.value = String(state.durationSeconds / 60);
    }
    syncPresets(state.durationSeconds / 60);

    if (state.running) {
      doc.title = `${formatTime(state.remainingSeconds)} · FocusFlow 专注计时器`;
    } else if (state.completed) {
      doc.title = '✅ 专注完成 · FocusFlow 专注计时器';
    } else {
      doc.title = 'FocusFlow 专注计时器 · 把注意力收回这一件事';
    }
  }

  function handleComplete() {
    stopInterval();
    beep();
    showToast('🎉 专注完成！这一轮结束了，起身活动一下。');
  }

  const timer = createFocusTimer({
    durationMinutes: initialMinutes,
    now: overrides.now,
    onTick: render,
    onComplete: handleComplete,
  });

  if (toggleBtn) {
    toggleBtn.addEventListener('click', () => {
      // 用户手势内解锁音频，保证完成提示音真的会响。
      primeAudio();
      const state = timer.toggle();
      if (state.running) ensureInterval();
      else stopInterval();
      render(state);
    });
  }

  if (resetBtn) {
    resetBtn.addEventListener('click', () => {
      stopInterval();
      render(timer.reset());
    });
  }

  if (durationInput) {
    durationInput.value = String(initialMinutes);
    durationInput.addEventListener('input', () => {
      stopInterval();
      render(timer.setDurationMinutes(durationInput.value));
    });
    durationInput.addEventListener('change', () => {
      stopInterval();
      render(timer.setDurationMinutes(durationInput.value));
    });
  }

  presetButtons.forEach((button) => {
    button.addEventListener('click', () => {
      const minutes = clampDurationMinutes(button.dataset ? button.dataset.preset : button.getAttribute('data-preset'));
      stopInterval();
      render(timer.setDurationMinutes(minutes));
    });
  });

  render(timer.getState());

  return {
    timer,
    stop: () => {
      stopInterval();
      if (toastTimer) clearTimeout(toastTimer);
    },
    render: () => render(timer.getState()),
  };
}

return {
  DEFAULT_DURATION_MINUTES,
  MIN_DURATION_MINUTES,
  MAX_DURATION_MINUTES,
  clampDurationMinutes,
  formatTime,
  formatDurationLabel,
  createFocusTimer,
  initFocusApp,
};
});
