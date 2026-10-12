// 桌宠按键口径契约测试：**左键点击只互动，右键才是功能面（菜单）**。
//
// 为什么值得单独立测：桌宠是"凭手感用"的界面，按键口径漂移不会有编译错误、不会有
// 类型错误，只有用户点下去时才发现"怎么又弹菜单了 / 怎么不弹菜单了"。而左键弹菜单
// 这件事曾经真实存在（两个键都弹同一个菜单），属于只有行为断言才能拦住的回归。
//
// 断言分两层：
//   ① 行为层：VM 沙箱 + 假 DOM / 假壳 IPC，走真实指针事件序列（pointerdown →
//      pointermove → pointerup、contextmenu），断言
//      · 左键点击 = 摸摸头（happy 摇动 + ❤×4 + 互动台词），且**不弹菜单、不移动窗口**；
//      · 拖动 = 纯移动（调壳 moveBy），不互动、不弹菜单；长距/回拖同样是纯移动；
//      · 右键 = 菜单（再按一次关掉），审批动作仍在菜单里一步可达。
//   ② 接线层：源码里"打开菜单"的调用点只能落在右键处理器内、拖动路径不得再夹互动，
//      index.html 的提示文案必须与实际按键一致——防止有人把左键分支改回去而行为
//      测试之外无人发现。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const petSource = readFileSync(join(WEB, "pet", "pet.js"), "utf8");
const petHtml = readFileSync(join(WEB, "pet", "index.html"), "utf8");

const PET_IDS = ["pet", "pet-avatar", "pet-avatar-img", "bubble", "fx", "skin-menu"];

function makeClassList() {
  const names = new Set();
  return {
    add: (...list) => list.forEach((name) => names.add(name)),
    remove: (...list) => list.forEach((name) => names.delete(name)),
    toggle(name, force) {
      const on = force === undefined ? !names.has(name) : Boolean(force);
      if (on) names.add(name);
      else names.delete(name);
      return on;
    },
    contains: (name) => names.has(name),
  };
}

function makeElement(tag) {
  const listeners = {};
  const element = {
    tagName: String(tag).toUpperCase(),
    children: [],
    listeners,
    hidden: false,
    textContent: "",
    className: "",
    src: "",
    type: "",
    display: "",
    dataset: {},
    attrs: {},
    style: { setProperty(name, value) { this[name] = value; } },
    classList: makeClassList(),
    setAttribute(name, value) { this.attrs[name] = String(value); },
    getAttribute(name) { return this.attrs[name] === undefined ? null : this.attrs[name]; },
    addEventListener(type, handler) {
      (listeners[type] = listeners[type] || []).push(handler);
    },
    dispatch(type, event) {
      (listeners[type] || []).forEach((handler) => handler(event));
    },
    appendChild(child) { this.children.push(child); return child; },
    append(...nodes) { nodes.forEach((node) => this.appendChild(node)); },
    replaceChildren(...nodes) { this.children = []; this.append(...nodes); },
    contains(node) {
      return node === this || this.children.some((child) => child === node || child.contains(node));
    },
    getBoundingClientRect() {
      return { width: 120, height: 80, left: 0, top: 0, right: 120, bottom: 80 };
    },
    setPointerCapture() {},
    releasePointerCapture() {},
  };
  return element;
}

/** 把整棵子树的文案拼起来（菜单项文案都在里层 span 上）。 */
function textOf(element) {
  return [element.textContent || ""]
    .concat(element.children.map((child) => textOf(child)))
    .filter(Boolean)
    .join(" ");
}

function loadPet(options) {
  const opts = options || {};
  const ids = {};
  for (const id of PET_IDS) ids[id] = makeElement(id === "pet-avatar-img" ? "img" : "div");
  // 对齐 index.html 的 `<div class="skin-menu" id="skin-menu" hidden>`：菜单初始是收起的，
  // 否则"右键 toggle"会被测成"右键关菜单"，看着绿实则假。
  ids["skin-menu"].hidden = true;

  const documentListeners = {};
  const windowListeners = {};
  const calls = { moveBy: [], setVisible: [], showWorkbench: 0, resetPosition: 0, requests: [] };

  const documentStub = {
    getElementById: (id) => ids[id] || null,
    createElement: (tag) => makeElement(tag),
    addEventListener(type, handler) {
      (documentListeners[type] = documentListeners[type] || []).push(handler);
    },
    dispatch(type, event) {
      (documentListeners[type] || []).forEach((handler) => handler(event));
    },
  };

  const sandbox = {
    document: documentStub,
    URLSearchParams, location: {search: ""},
    console,
    // 计时器全部假掉：boot 的状态轮询/彩蛋定时器不会真跑，测试只驱动被测的那条路径。
    setTimeout: () => 1,
    clearTimeout: () => {},
    setInterval: () => 1,
    clearInterval: () => {},
    requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => {},
    performance: { now: () => 0 },
    devicePixelRatio: 1,
    localStorage: { getItem: () => null, setItem: () => {} },
    // 皮肤清单拿不到（离线/静态资源没挂）→ boot 走"保持默认外观"分支，把被测面
    // 收在按键行为上，不掺皮肤装配。
    fetch: () => Promise.reject(new Error("offline")),
    Image: class { set src(value) { this._src = value; } get src() { return this._src; } },
    addEventListener(type, handler) {
      (windowListeners[type] = windowListeners[type] || []).push(handler);
    },
  };
  sandbox.window = sandbox;

  if (opts.api) {
    sandbox.OwoApi = {
      ensureCoreConnection: async () => {},
      request: async (path) => {
        calls.requests.push(path);
        return opts.api[path] === undefined ? null : opts.api[path];
      },
    };
  }
  if (opts.shell) {
    sandbox.petShell = {
      query: async () => ({ alive: true, visible: true }),
      setVisible: async (visible) => { calls.setVisible.push(visible); },
      moveBy: (dx, dy) => { calls.moveBy.push([dx, dy]); },
      showWorkbench: () => { calls.showWorkbench += 1; },
      resetPosition: () => { calls.resetPosition += 1; },
      getPref: async () => ({}),
      setPref: async () => {},
    };
  }

  vm.createContext(sandbox);
  vm.runInContext(petSource, sandbox, { filename: "pet.js" });
  return {
    sandbox,
    ids,
    calls,
    document: documentStub,
    URLSearchParams, location: {search: ""},
    run: (code) => vm.runInContext(code, sandbox),
  };
}

function pointerEvent(type, x, y) {
  return {
    type,
    button: 0,
    buttons: type === "pointerup" ? 0 : 1,
    pointerId: 1,
    screenX: x,
    screenY: y,
    clientX: x,
    clientY: y,
    preventDefault() {},
  };
}

function leftClick(pet, x, y) {
  pet.dispatch("pointerdown", pointerEvent("pointerdown", x, y));
  pet.dispatch("pointerup", pointerEvent("pointerup", x, y));
}

function rightClick(pet, x, y) {
  pet.dispatch("contextmenu", {
    type: "contextmenu",
    clientX: x,
    clientY: y,
    preventDefault() {},
  });
}

async function flush(times) {
  for (let i = 0; i < (times || 6); i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

/**
 * 去掉注释后的源码：接线守卫要断言"代码里没有这东西"，而文件头和解释里恰恰要提到
 * 被删掉的旧变量名（"为什么删"必须留在注释里），拿原文断言会把好文档罚成红。
 */
function stripComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

const petCode = stripComments(petSource);

test("左键点击＝只互动：happy 摇动 + ❤×4 + 互动台词，且不弹菜单、不移动窗口", async () => {
  const pet = loadPet({ shell: true });
  await flush();
  leftClick(pet.ids.pet, 120, 120);

  assert.equal(pet.ids.pet.classList.contains("happy"), true, "左键点击必须给出摸摸头反应");
  assert.equal(pet.ids["skin-menu"].hidden, true, "左键点击不得弹菜单（旧行为就是两个键都弹）");
  assert.equal(pet.ids.fx.children.length, 4, "摸摸头必须飘 ❤×4");
  assert.deepEqual(pet.ids.fx.children.map((node) => node.textContent), ["❤", "❤", "❤", "❤"]);
  const line = pet.ids.bubble.textContent;
  assert.ok(line && line !== "OwO", `左键点击要换成互动台词，实际=${line}`);
  assert.deepEqual(pet.calls.moveBy, [], "5px 死区内的单击不得推动窗口");
});

test("菜单开着时的左键点击：先收起菜单，不再顺手多弹一次", async () => {
  const pet = loadPet({ shell: true });
  await flush();
  rightClick(pet.ids.pet, 100, 100);
  assert.equal(pet.ids["skin-menu"].hidden, false, "前置：右键菜单已打开");

  const menu = pet.ids["skin-menu"];
  // 点在桌宠身上（不在菜单内）：document mousedown 会先收菜单。
  pet.document.dispatch("mousedown", { type: "mousedown", target: pet.ids.pet });
  leftClick(pet.ids.pet, 120, 120);
  assert.equal(menu.hidden, true, "点击后菜单必须是收起的");
  assert.equal(menu.children.length, 0, "收起的菜单必须清空，不能留下上一次的动作项");
});

test("拖动＝纯移动：交给壳 moveBy，不互动、不弹菜单", async () => {
  const pet = loadPet({ shell: true });
  await flush();
  const petElement = pet.ids.pet;
  petElement.dispatch("pointerdown", pointerEvent("pointerdown", 100, 100));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 160, 140));
  petElement.dispatch("pointerup", pointerEvent("pointerup", 160, 140));

  assert.deepEqual(pet.calls.moveBy, [[60, 40]], "拖动必须按 dpr 折算后交给壳移动真实窗口");
  assert.equal(petElement.classList.contains("dropped"), true, "松手要有掉落反馈");
  assert.equal(petElement.classList.contains("happy"), false, "拖动不是互动，不得触发摸摸头");
  assert.equal(pet.ids.fx.children.length, 0, "拖动不得飘 ❤");
  assert.equal(pet.ids["skin-menu"].hidden, true, "拖动不得弹菜单");
});

test("右键＝功能面：菜单按当前状态列动作（审批一步可达），再按一次关掉", async () => {
  const pet = loadPet({
    shell: true,
    api: {
      "/activity": {
        active: [{ session_id: "s1", phase: "thinking", title: "会话" }],
        pending_approvals: 1,
      },
      "/approvals/pending": {
        pending: [{ request_id: "r1", session_id: "s1", tool: "write_file" }],
      },
    },
  });
  await flush();
  await pet.run("refreshActivity()");

  const menu = pet.ids["skin-menu"];
  rightClick(pet.ids.pet, 100, 100);
  assert.equal(menu.hidden, false, "右键必须打开菜单");
  const text = textOf(menu);
  assert.match(text, /允许：write_file/, `菜单必须给出允许动作，实际=${text}`);
  assert.match(text, /拒绝：write_file/, `菜单必须给出拒绝动作，实际=${text}`);
  assert.match(text, /停止当前回合/);
  assert.match(text, /打开工作台/);
  assert.match(text, /隐藏桌宠/);
  assert.match(text, /回到右下角/);
  assert.equal(pet.ids.pet.classList.contains("happy"), false, "右键不得触发摸摸头");
  assert.deepEqual(pet.ids.fx.children, [], "右键不得飘 ❤");

  rightClick(pet.ids.pet, 100, 100);
  assert.equal(menu.hidden, true, "菜单开着再右键必须关掉（toggle）");
  assert.equal(menu.children.length, 0, "关掉的菜单必须清空");
});

test("长距拖动（累计 >420px）＝仍然是纯移动，不得顺手摸摸头", async () => {
  // 旧实现按"累计划动距离"记账：走得够远就当作用户在摸头（飘 ❤ + happy + 台词）。
  // 这里故意用**来回折线**而不是直线：三段各 200px，累计 600px（旧逻辑必触发），
  // 但净位移只有 200px。折线还顺带钉住另一件事——回拖时窗口必须继续跟随鼠标
  // （5px 死区闩住），而不是"离按下点变近了就不跟随"（窗宠被甩在鼠标后面 + 松手
  // 被误判成单击摸头）。
  const pet = loadPet({ shell: true });
  await flush();
  const petElement = pet.ids.pet;
  const lineBefore = pet.ids.bubble.textContent;
  petElement.dispatch("pointerdown", pointerEvent("pointerdown", 300, 200));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 500, 200));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 300, 200));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 500, 200));
  petElement.dispatch("pointerup", pointerEvent("pointerup", 500, 200));

  assert.deepEqual(
    pet.calls.moveBy,
    [[200, 0], [-200, 0], [200, 0]],
    "每一段位移都要交给壳：回拖也得跟随（死区只在开头 5px 生效）",
  );
  assert.equal(petElement.classList.contains("happy"), false, "长距拖动不得触发摸摸头");
  assert.equal(pet.ids.fx.children.length, 0, "长距拖动不得飘 ❤");
  assert.equal(pet.ids.bubble.textContent, lineBefore, "长距拖动不得换互动台词");
  assert.equal(pet.ids["skin-menu"].hidden, true, "长距拖动不得弹菜单");
  assert.equal(petElement.classList.contains("dropped"), true, "松手仍要有掉落反馈");
});

// 反向：拉出去再**拖回按下点后松手**，仍是拖动——不得因"净位移回到 0"被误判成单击
// 而掉进摸摸头分支。与上一条互补：上一条查窗口跟随，这条查点击/拖动分类。
test("拖回起点再松手：仍算拖动，不摸摸头", async () => {
  const pet = loadPet({ shell: true });
  await flush();
  const petElement = pet.ids.pet;
  const lineBefore = pet.ids.bubble.textContent;
  petElement.dispatch("pointerdown", pointerEvent("pointerdown", 100, 300));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 400, 300));
  petElement.dispatch("pointermove", pointerEvent("pointermove", 100, 300));
  petElement.dispatch("pointerup", pointerEvent("pointerup", 100, 300));

  assert.equal(petElement.classList.contains("happy"), false, "拖回起点不算点击");
  assert.equal(pet.ids.fx.children.length, 0, "拖回起点不得飘 ❤");
  assert.equal(pet.ids.bubble.textContent, lineBefore, "拖回起点不得换互动台词");
  assert.equal(petElement.classList.contains("dropped"), true, "拖回起点松手仍是拖动（要有掉落反馈）");
  assert.deepEqual(pet.calls.moveBy, [[300, 0], [-300, 0]], "两段位移都要跟随");
});

test("接线层：划动摸头残留必须清干净（不许留死变量绕过行为断言）", () => {
  // 行为测试只能覆盖"走的这条路径"，所以再钉一层源码守卫：拖动路径里不许再出现
  // 累计距离记账 / 互动冷却这些旧机制，否则很容易被"顺手加回来"。
  // 断言对象是去注释后的 petCode——"为什么删"这类说明本来就该留在注释里。
  assert.doesNotMatch(petCode, /dragDistance/, "dragDistance（累计划动距离）必须删除");
  assert.doesNotMatch(petCode, /pettedThisDrag/, "pettedThisDrag 必须删除");
  assert.doesNotMatch(petCode, /petCooldown/, "petCooldown（划动互动冷却）必须删除");
  assert.equal(
    [...petCode.matchAll(/petted\(\)/g)].length,
    2,
    "petted() 只应有「1 处定义 + 1 处调用（左键分支）」",
  );
  // 拖动路径（pointermove）里不得出现任何互动调用，职责只有移动窗口。
  const moveStart = petCode.indexOf('pet.addEventListener("pointermove"');
  const moveEnd = petCode.indexOf("function endDrag");
  assert.ok(moveStart > 0 && moveEnd > moveStart, "找不到 pointermove 处理器");
  const moveBody = petCode.slice(moveStart, moveEnd);
  assert.doesNotMatch(moveBody, /petted\(\)/, "pointermove 里不得再夹互动");
  assert.match(moveBody, /moveBy/, "pointermove 的职责只有移动窗口");
});

test("接线层：打开菜单的调用点只能在右键处理器内，文案与按键口径一致", () => {
  const start = petSource.indexOf("function endDrag");
  const end = petSource.indexOf('pet.addEventListener("pointerup"');
  assert.ok(start > 0 && end > start, "找不到 endDrag（交互分支定位失败）");
  const endDragBody = petSource.slice(start, end);
  assert.match(endDragBody, /petted\(\)/, "左键点击分支必须触发互动");
  assert.doesNotMatch(endDragBody, /openMenu\(/, "左键点击分支不得再弹菜单");
  assert.match(petSource, /按键口径/, "文件头必须写明按键口径（免得下一个人又改回去）");

  // 索引必须与 openMenu 计数同一份文本（petCode 去过注释，偏移与源码不同）。
  const contextmenuAt = petCode.indexOf('pet.addEventListener("contextmenu"');
  assert.ok(contextmenuAt > 0, "找不到右键处理器（contextmenu 接线）");
  const callsites = [...petCode.matchAll(/openMenu\(/g)].map((match) => match.index);
  assert.equal(callsites.length, 2, "openMenu 只应有「1 处定义 + 1 处调用」");
  assert.ok(
    callsites[1] > contextmenuAt,
    "唯一一次 openMenu 调用必须落在右键处理器里",
  );

  assert.match(petHtml, /title="[^"]*左键[^"]*摸[^"]*"/, "提示必须写明左键是互动");
  assert.match(petHtml, /title="[^"]*右键[^"]*菜单/, "提示必须写明右键才是菜单");
  assert.doesNotMatch(petHtml, /点我打开操作菜单/, "旧提示（点我=菜单）必须已被替换");
});
