// OwOBot 整窗模式契约：入口把当前窗口整体切换为 OwOBot（不嵌在工作台工作区，
// 也不新开系统窗口）。进入时 panelRoot 移到 body 直下并铺满菜单栏以下区域，
// 退出时还原父节点/内联 pin/标题并回到会话视图；#owobot 深链刷新后仍留在整窗。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const domain = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../owobot.css", import.meta.url), "utf8");

function element(tag) {
  return {
    tag,
    id: "",
    type: "",
    textContent: "",
    className: "",
    children: [],
    listeners: {},
    parentNode: null,
    nextSibling: null,
    _html: "",
    style: {
      props: {},
      setProperty(key, value, priority) { this.props[key] = [value, priority]; },
      removeProperty(key) { delete this.props[key]; },
    },
    get innerHTML() { return this._html; },
    set innerHTML(value) { this._html = value; },
    addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); },
    setAttribute(name, value) { this[name] = value; },
    append(...nodes) { this.children.push(...nodes); },
  };
}

function modeWith() {
  const calls = { mount: [], open: [], tools: [], approvals: 0, restored: [] };
  const classes = new Set();
  const body = element("body");
  body.classList = {
    add: name => classes.add(name),
    remove: name => classes.delete(name),
    contains: name => classes.has(name),
  };
  body.appendChild = node => { node.parentNode = body; body.children.push(node); };
  const layout = element("div");
  layout.appendChild = node => { calls.restored.push(node); node.parentNode = layout; };
  layout.insertBefore = (node, ref) => { calls.restored.push(node); node.parentNode = layout; assert.equal(ref, null); };
  const root = element("div");
  root.id = "panelRoot";
  root.parentNode = layout;
  const document = {
    body,
    title: "OwO Agent 工作台",
    createElement: element,
    querySelectorAll: () => [],
  };
  const context = {
    BASE_DOC_TITLE: "OwO Agent 工作台",
    $: id => (id === "panelRoot" ? root : body.children.find(child => child.id === id) || null),
    document,
    window: { OwoPanels: { owobot: { open: task => calls.open.push(task) } } },
    mountPanel: id => calls.mount.push(id),
    setToolsVisible: (visible, preserveRoute) => calls.tools.push([visible, preserveRoute]),
    renderApprovals: () => { calls.approvals += 1; },
  };
  const match = /let owobotModeActive = false;[\s\S]*?function exitOwobotMode\(\) \{[\s\S]*?\n\}/.exec(domain);
  assert.ok(match, "app-domain.js 必须定义整窗模式 enterOwobotMode/exitOwobotMode");
  const api = runInNewContext(match[0] + "\n({ enterOwobotMode, exitOwobotMode, get active() { return owobotModeActive; } });", context);
  return { api, calls, classes, body, root, layout, document };
}

test("进入整窗：剥外壳、全屏 pin 与 OwOBot 挂载一次到位", () => {
  const { api, calls, classes, body, root, document } = modeWith();
  assert.equal(api.enterOwobotMode(null), true);
  assert.equal(api.active, true);
  assert.ok(classes.has("owobot-mode"));
  assert.equal(document.title, "OwOBot");
  assert.equal(root.parentNode, body, "panelRoot 必须移出工作台布局成为 body 直下容器");
  assert.deepEqual(root.style.props["max-width"], ["none", "important"]);
  assert.deepEqual(root.style.props.margin, ["0", "important"]);
  assert.deepEqual(root.style.props.padding, ["0", "important"]);
  assert.deepEqual(calls.mount, ["owobot"]);
  assert.equal(body.children.length, 1, "进入协作只挂载工作区，不再插入独立标题栏");
});

test("任务深链进入整窗：任务先交给面板再挂载", () => {
  const { api, calls } = modeWith();
  api.enterOwobotMode({ thread: "t1", workspace: "w1" });
  assert.deepEqual(calls.open, [{ thread: "t1", workspace: "w1" }]);
  assert.deepEqual(calls.mount, ["owobot"]);
});

test("已在整窗时无任务不重载 iframe，带任务才重挂载消费", () => {
  const { api, calls } = modeWith();
  api.enterOwobotMode(null);
  api.enterOwobotMode(null);
  assert.deepEqual(calls.mount, ["owobot"], "重复点入口不得重建画面");
  api.enterOwobotMode({ thread: "t2" });
  assert.deepEqual(calls.open, [{ thread: "t2" }]);
  assert.deepEqual(calls.mount, ["owobot", "owobot"]);
});

test("退出整窗：还原父节点/pin/标题，回会话视图且工作区不残留 OwOBot", () => {
  const { api, calls, classes, root, layout, document } = modeWith();
  api.enterOwobotMode(null);
  assert.equal(api.exitOwobotMode(), true);
  assert.equal(api.active, false);
  assert.ok(!classes.has("owobot-mode"));
  assert.deepEqual(calls.tools, [[false, undefined]], "退出必须经视图状态机收起工具视图并清路由");
  assert.equal(root.parentNode, layout, "panelRoot 回到原来的工作台布局");
  assert.deepEqual(calls.restored, [root]);
  assert.deepEqual(root.style.props, {}, "内联 important pin 必须撤掉，工作台盒模型恢复");
  assert.match(root.innerHTML, /选择工具面板/, "iframe 随占位丢弃，再开工具视图看不到残留 OwOBot");
  assert.equal(document.title, "OwO Agent 工作台");
  assert.equal(calls.approvals, 1, "标题恢复后审批前缀要立即重算");
});

test("系统菜单栏有显式返回入口", () => {
  const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
  assert.match(html, /id="owobotReturn"[^>]*aria-label="返回工作台"/);
  assert.match(app, /\$\("owobotReturn"\)\?\.addEventListener\("click", \(\) => exitOwobotMode\(\)\)/);
});

test("所有入口统一走整窗：导航/快捷搜索/深链/任务深链不再内嵌或转壳", () => {
  assert.match(domain, /if \(id === "owobot"\) return enterOwobotMode\(null\);/);
  assert.match(domain, /openOwobot: task => \{ enterOwobotMode\(task \|\| null\); \},/);
  assert.doesNotMatch(domain, /owobotExternal|openOwobotExternal|OWOBOT_SURFACE|owobot-external/);
  assert.doesNotMatch(app, /OWOBOT_SURFACE|prepareOwobotSurface|mountOwobotSurface|onOwobotTask|surfacePendingTask/);
});

test("#owobot 深链：hashchange 进出整窗，刷新后仍留在 OwOBot", () => {
  assert.match(domain, /window\.addEventListener\("hashchange", \(\) => \{[\s\S]*?if \(owobotModeActive && raw !== "owobot"\) exitOwobotMode\(\);[\s\S]*?applyDeepLink\(\);/);
  assert.match(domain, /if \(location\.hash && initialRoute\.kind !== "chat" && rawHash !== "owobot"\)/);
  assert.match(domain, /if \(root && rawHash !== "owobot"\) root\.innerHTML = PANEL_EMPTY_STATE_HTML;/);
});

test("审批标题跟随整窗模式，不回落成工作台标题", () => {
  assert.match(domain, /const baseTitle = owobotModeActive \? "OwOBot" : BASE_DOC_TITLE;/);
  assert.match(domain, /const BASE_DOC_TITLE = typeof document !== "undefined" \? \(document\.title \|\| ""\) : "";/);
});

test("整窗保留菜单栏和弹层，侧轨与聊天共用同一行", () => {
  assert.match(css, /body\.owobot-mode > \*:not\(#menubar\):not\(#panelRoot\):not\(#setupRoot\):not\(#modalRoot\):not\(#toastRoot\)/);
  assert.doesNotMatch(css + domain, /owobotModeBar|owobot-mode-title/);
});
