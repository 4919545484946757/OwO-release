/**
 * 测试用的最小浏览器环境。
 *
 * `script.js` 是经典脚本（IIFE + window.FocusFlow），不能 `import`，也没有导出语句，
 * 所以这里用 node:vm 在隔离上下文里执行它，取出 `window.FocusFlow`，
 * 并配合下面的极简 fake DOM / fake AudioContext 覆盖 DOM 绑定层。
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const root = dirname(dirname(dirname(fileURLToPath(import.meta.url))));

/** 可控假时钟，用来验证倒计时推进逻辑。 */
export function createClock(startMs = 0) {
  let current = startMs;
  return {
    now: () => current,
    advance(seconds) {
      current += seconds * 1000;
    },
  };
}

function createClassList() {
  const names = new Set();
  return {
    add(...items) {
      items.forEach((item) => names.add(item));
    },
    remove(...items) {
      items.forEach((item) => names.delete(item));
    },
    contains(name) {
      return names.has(name);
    },
    toggle(name, force) {
      const shouldAdd = force === undefined ? !names.has(name) : Boolean(force);
      if (shouldAdd) names.add(name);
      else names.delete(name);
      return shouldAdd;
    },
  };
}

/** 极简元素：支持 querySelector 用到的属性、classList、dataset、事件派发。 */
export function createElement(tagName, attributes = {}) {
  const listeners = new Map();
  const element = {
    tagName: String(tagName).toUpperCase(),
    attributes: { ...attributes },
    dataset: {},
    style: {},
    classList: createClassList(),
    textContent: attributes.textContent === undefined ? '' : String(attributes.textContent),
    hidden: Boolean(attributes.hidden),
    value: attributes.value === undefined ? '' : String(attributes.value),
    offsetWidth: 0,
    getAttribute(name) {
      return Object.prototype.hasOwnProperty.call(element.attributes, name) ? element.attributes[name] : null;
    },
    setAttribute(name, value) {
      element.attributes[name] = String(value);
      if (name.startsWith('data-')) element.dataset[name.slice(5)] = String(value);
    },
    addEventListener(type, handler) {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(handler);
    },
    dispatch(type) {
      const bucket = listeners.get(type) || [];
      bucket.forEach((handler) => handler({ type, target: element }));
    },
    click() {
      element.dispatch('click');
    },
  };

  for (const [key, value] of Object.entries(attributes)) {
    if (key.startsWith('data-')) element.dataset[key.slice(5)] = String(value);
  }

  return element;
}

/** 极简 document：按选择器返回预先注册的元素。 */
export function createFakeDocument({ title = 'FocusFlow 专注计时器', readyState = 'loading' } = {}) {
  const single = new Map();
  const many = new Map();
  const listeners = new Map();

  const doc = {
    title,
    readyState,
    querySelector(selector) {
      return single.get(selector) || null;
    },
    querySelectorAll(selector) {
      return many.get(selector) || [];
    },
    addEventListener(type, handler) {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(handler);
    },
    register(selector, element) {
      single.set(selector, element);
      return doc;
    },
    registerAll(selector, elements) {
      many.set(selector, elements);
      return doc;
    },
  };

  return doc;
}

/**
 * 按 index.html 的挂载点搭一个假页面，字段名与 DOM 里的一一对应。
 * `readyState: 'loading'` 让 script.js 的自动初始化只注册 DOMContentLoaded，
 * 测试里再显式调用 FocusFlow.initFocusApp(doc)，避免重复初始化。
 */
export function createTimerPage() {
  const els = {
    card: createElement('div', { id: 'timer', 'data-timer': '' }),
    timeDisplay: createElement('span', { id: 'time-display', textContent: '25:00' }),
    status: createElement('span', { id: 'status-text', textContent: '准备开始' }),
    ring: createElement('circle', { id: 'progress-ring', r: '88' }),
    toggle: createElement('button', { id: 'toggle-btn', textContent: '开始专注' }),
    reset: createElement('button', { id: 'reset-btn', textContent: '重置' }),
    input: createElement('input', { id: 'duration-input', value: '25' }),
    durationValue: createElement('output', { id: 'duration-value', textContent: '25 分钟' }),
    banner: createElement('p', { id: 'complete-banner', hidden: true }),
    toast: createElement('div', { id: 'toast', hidden: true }),
    presets: ['15', '25', '45', '60'].map((minutes) =>
      createElement('button', { 'data-preset': minutes, textContent: `${minutes} 分钟` }),
    ),
  };

  const doc = createFakeDocument()
    .register('#time-display', els.timeDisplay)
    .register('[data-timer]', els.card)
    .register('#status-text', els.status)
    .register('#progress-ring', els.ring)
    .register('#toggle-btn', els.toggle)
    .register('#reset-btn', els.reset)
    .register('#duration-input', els.input)
    .register('#duration-value', els.durationValue)
    .register('#complete-banner', els.banner)
    .register('#toast', els.toast)
    .registerAll('[data-preset]', els.presets);

  return { doc, els };
}

/**
 * 假 AudioContext：记录创建次数与是否被 resume，用来验证「在用户手势里解锁音频」。
 */
export function createFakeAudioContextClass() {
  const created = [];
  const oscillators = [];

  class FakeAudioContext {
    constructor() {
      this.state = 'suspended';
      this.currentTime = 0;
      this.destination = { name: 'destination' };
      this.resumeCalls = 0;
      created.push(this);
    }

    resume() {
      this.resumeCalls += 1;
      this.state = 'running';
      return Promise.resolve();
    }

    createOscillator() {
      const oscillator = {
        type: 'sine',
        frequency: { value: 0 },
        started: 0,
        stopped: 0,
        connectedTo: [],
        connect(node) {
          this.connectedTo.push(node);
        },
        start() {
          this.started += 1;
        },
        stop() {
          this.stopped += 1;
        },
      };
      oscillators.push(oscillator);
      return oscillator;
    }

    createGain() {
      return {
        gain: {
          setValueAtTime() {},
          exponentialRampToValueAtTime() {},
        },
        connectedTo: [],
        connect(node) {
          this.connectedTo.push(node);
        },
      };
    }

    close() {
      this.state = 'closed';
    }
  }

  return { FakeAudioContext, created, oscillators };
}

/** 在隔离上下文里执行经典脚本 script.js，返回挂到 window 上的 FocusFlow API。 */
export function loadFocusFlow({ document: fakeDocument, window: windowOverrides = {} } = {}) {
  const source = readFileSync(join(root, 'script.js'), 'utf8');
  const windowObject = { ...windowOverrides };
  const sandbox = {
    window: windowObject,
    document: fakeDocument,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    console,
  };
  sandbox.globalThis = sandbox;

  vm.runInContext(source, vm.createContext(sandbox), { filename: 'script.js' });

  const api = windowObject.FocusFlow;
  if (!api) throw new Error('script.js 没有把 API 暴露到 window.FocusFlow 上');
  return { api, window: windowObject, sandbox };
}
