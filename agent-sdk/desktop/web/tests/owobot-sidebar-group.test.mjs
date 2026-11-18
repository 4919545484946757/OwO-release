import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const domain = readFileSync(new URL("../app-domain.js", import.meta.url), "utf8");
const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const idsBody = /function owoWorkerSessionIds\(links\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
const groupBody = /function renderOwobotGroup\(list, links, options\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
assert.ok(idsBody && groupBody, "协作分组渲染与归属索引 helper 必须存在");

function el(tag) {
  const classes = new Set();
  return {
    tag, children: [], attributes: {}, listeners: {}, dataset: {},
    hidden: false, title: "", textContent: "", innerHTML: "", className: "",
    classList: {
      add: (...names) => names.forEach((name) => classes.add(name)),
      remove: (...names) => names.forEach((name) => classes.delete(name)),
      toggle(name, on) { const want = on === undefined ? !classes.has(name) : Boolean(on); if (want) classes.add(name); else classes.delete(name); return want; },
      contains: (name) => classes.has(name),
    },
    setAttribute(name, value) { this.attributes[name] = String(value); },
    getAttribute(name) { return name in this.attributes ? this.attributes[name] : null; },
    append(...nodes) { this.children.push(...nodes); },
    appendChild(node) { this.children.push(node); return node; },
    addEventListener(name, callback) { (this.listeners[name] ||= []).push(callback); },
    dispatch(name, event = {}) {
      for (const callback of this.listeners[name] || []) {
        callback({ stopPropagation() {}, preventDefault() {}, ...event });
      }
    },
    querySelector() { return null; },
    querySelectorAll() { return []; },
  };
}

function harness(storage = {}) {
  const store = new Map(Object.entries(storage));
  const sandbox = {
    document: { createElement: (tag) => el(tag) },
    localStorage: {
      getItem: (key) => (store.has(key) ? store.get(key) : null),
      setItem: (key, value) => store.set(key, String(value)),
    },
  };
  vm.createContext(sandbox);
  const ids = vm.runInContext(idsBody + "\nowoWorkerSessionIds", sandbox);
  const render = vm.runInContext(groupBody + "\nrenderOwobotGroup", sandbox);
  return { ids, render, store };
}

const links = {
  sessions: { "s-lead": { role: "组长" }, "s-worker": { role: "执行员" } },
  tasks: [{
    threadId: "t1", groupId: "g1", workspace: "D:/proj",
    title: "整理本周交付清单", updatedAt: "2026-11-17T10:01:00Z",
    sessionIds: ["s-lead", "s-worker"],
  }],
};
const sessions = [
  { id: "s-lead", mode: "agent" },
  { id: "s-worker", mode: "agent" },
  { id: "s-user", mode: "agent" },
];

function renderGroup(h, overrides = {}) {
  const list = el("ul");
  const rendered = [];
  const opened = [];
  h.render(list, overrides.links === undefined ? links : overrides.links, {
    sessions,
    activeTurns: overrides.activeTurns || new Set(),
    renderSession: (session, depth, workspace, container, pinned, titleOverride) => {
      rendered.push({ session, depth, workspace, container, pinned, titleOverride });
      container.appendChild(el("li"));
    },
    openTask: (task) => opened.push(task),
  });
  return { list, rendered, opened };
}

test("归属索引 helper 容忍旧核心空响应，只收服务端标记的会话", () => {
  const h = harness();
  assert.equal(h.ids(null).size, 0);
  assert.equal(h.ids({}).size, 0);
  assert.deepEqual([...h.ids({ sessions: { a: {}, b: {} } })], ["a", "b"]);
});

test("无协作任务时不渲染分区，旧核心降级不留空块", () => {
  const h = harness();
  for (const empty of [null, {}, { tasks: [] }]) {
    const { list } = renderGroup(h, { links: empty });
    assert.equal(list.children.length, 0);
  }
});

test("任务行=目标标题+运行状态+时间，角色会话收进审计层且不进主列表", () => {
  const h = harness();
  const { list, rendered } = renderGroup(h, { activeTurns: new Set(["s-lead"]) });
  assert.equal(list.children.length, 2);
  const [label, area] = list.children;
  assert.equal(label.className, "codex-section-label");
  assert.equal(label.textContent, "协作");
  assert.equal(area.className, "owobot-area");
  const [header, taskList] = area.children;
  assert.equal(header.attributes.role, "button");
  assert.equal(header.attributes.tabindex, "0");
  assert.equal(header.attributes["aria-expanded"], "true");
  assert.equal(header.children[2].textContent, "1");
  assert.equal(taskList.hidden, false);
  const task = taskList.children[0];
  assert.equal(task.dataset.owobotThread, "t1");
  const [row, memberList] = task.children;
  const [open, disclosure] = row.children;
  assert.equal(open.children[0].textContent, "整理本周交付清单");
  assert.equal(open.children[1].textContent, "运行中");
  assert.equal(open.children[2].textContent, "11-17 10:01");
  assert.match(open.title, /打开 OwOBot 任务：整理本周交付清单/);
  assert.equal(disclosure.attributes["aria-expanded"], "false");
  assert.equal(memberList.hidden, true);
  assert.equal(rendered.length, 2);
  assert.deepEqual(rendered.map((r) => r.titleOverride), ["角色：组长", "角色：执行员"]);
  assert.ok(rendered.every((r) => r.container === memberList && r.workspace === "D:/proj" && r.pinned === false));
  assert.ok(!rendered.some((r) => r.session.id === "s-user"), "用户会话不得混入审计层");
});

test("无运行中回合时不显示运行徽标，缺失角色时回退会话自身标题", () => {
  const h = harness();
  const { list, rendered } = renderGroup(h, {
    links: { sessions: { "s-lead": {} }, tasks: [{ ...links.tasks[0], sessionIds: ["s-lead"] }] },
  });
  const open = list.children[1].children[1].children[0].children[0].children[0];
  assert.equal(open.children.length, 2, "只有标题与时间，没有徽标");
  assert.equal(rendered[0].titleOverride, "");
});

test("点任务深链 OwOBot 整窗 thread，展开符切换审计层并记忆", () => {
  const h = harness();
  const { list, opened } = renderGroup(h);
  const task = list.children[1].children[1].children[0];
  const [row, memberList] = task.children;
  const [open, disclosure] = row.children;
  open.dispatch("click");
  assert.equal(JSON.stringify(opened), JSON.stringify([{ thread: "t1", workspace: "D:/proj" }]));
  disclosure.dispatch("click");
  assert.equal(memberList.hidden, false);
  assert.equal(disclosure.attributes["aria-expanded"], "true");
  assert.deepEqual(JSON.parse(h.store.get("owo.owobotTasksOpen")), ["t1"]);
  disclosure.dispatch("click");
  assert.equal(memberList.hidden, true);
  assert.deepEqual(JSON.parse(h.store.get("owo.owobotTasksOpen")), []);
});

test("分组标题点击与键盘均可折叠，状态落盘并在下次渲染恢复", () => {
  const h = harness();
  const { list } = renderGroup(h);
  const [header, taskList] = list.children[1].children;
  header.dispatch("click");
  assert.equal(taskList.hidden, true);
  assert.equal(header.attributes["aria-expanded"], "false");
  assert.ok(header.children[0].classList.contains("collapsed"));
  assert.equal(h.store.get("owo.owobotGroupCollapsed"), "1");
  header.dispatch("keydown", { key: "Enter" });
  assert.equal(taskList.hidden, false);
  assert.equal(h.store.get("owo.owobotGroupCollapsed"), "0");

  const restored = renderGroup(harness({
    "owo.owobotGroupCollapsed": "1",
    "owo.owobotTasksOpen": JSON.stringify(["t1"]),
  }));
  const [header2, taskList2] = restored.list.children[1].children;
  assert.equal(taskList2.hidden, true);
  assert.equal(header2.attributes["aria-expanded"], "false");
  const memberList2 = taskList2.children[0].children[1];
  assert.equal(memberList2.hidden, false, "已展开的审计层跨刷新保留");
});

test("主列表与最近任务按服务端归属排除角色会话，搜索过滤覆盖协作区", () => {
  const refresh = /async function refreshSessionsImpl\(selectId, generation, selectionVersion\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(refresh, "refreshSessionsImpl 必须存在");
  assert.match(refresh, /api\("\/owobot\/worker-sessions"\)\.catch\(\(\) => null\)/, "旧核心无端点时静默降级");
  assert.match(refresh, /state\.owobotWorkerIds = owoWorkerSessionIds\(owobotLinks\)/);
  assert.match(refresh, /!state\.owobotWorkerIds\.has\(session\.id\)/);
  assert.match(refresh, /renderOwobotGroup\(list, owobotLinks, \{/);
  assert.match(refresh, /openTask: \(task\) => enterOwobotMode\(task\)/);
  const recent = /function renderHomeRecent\(emptyOverride = false\) \{[\s\S]*?\n\}/.exec(app)?.[0];
  assert.ok(recent && recent.includes("state.owobotWorkerIds"), "首页最近任务同样排除角色会话");
  const filter = /function filterSessionList\(\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(filter && filter.includes('querySelectorAll(".owobot-area")'), "搜索/折叠恢复必须处理协作区");
  assert.match(filter, /li\.closest\("\.owobot-area"\)/, "协作区行不受项目折叠影响");
});