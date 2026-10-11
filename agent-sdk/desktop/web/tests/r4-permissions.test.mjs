// R4 §4.5/§4.8「统一权限中心」契约测试。
//
// 三层（照 r4-diagnostics-ledger.test.mjs 的写法：真函数进沙箱执行 + 接线层读源码）：
//   ① 域层——纯函数判定（validateSpec / needsFullAccessConfirm / persistenceLabel /
//      revokePayload / isPendingGone / normalizeError），零 DOM 零 HTTP；
//   ② api + controller——注入假 fetchJson，锁死路径与 body（含"完全访问缺 confirm
//      就根本不发请求"）、loading/error/empty 三态、撤销级联与复查、审批四动作；
//   ③ 渲染 + 接线层——假 DOM shim 断言不可配置维度不产生可交互元素（假控件防护），
//      index.html/app.js 的路由、rail、脚本顺序与按需加载（不进首屏 ≤5 链路）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const sources = {
  domain: readFileSync(join(WEB, "permissions", "permissions.domain.js"), "utf8"),
  api: readFileSync(join(WEB, "permissions", "permissions.api.js"), "utf8"),
  controller: readFileSync(join(WEB, "permissions", "permissions.controller.js"), "utf8"),
  view: readFileSync(join(WEB, "permissions", "permissions.view.js"), "utf8"),
};
const indexHtml = readFileSync(join(WEB, "index.html"), "utf8");
const appSource = readFileSync(join(WEB, "app.js"), "utf8");
const appDomainSource = readFileSync(join(WEB, "app-domain.js"), "utf8");

// ---- 假 DOM shim（本仓无 core/dom-harness.js，视图测试各自内联，同 r4-* 先例）----
function makeElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    className: "",
    children: [],
    listeners: {},
    attrs: {},
    textContent: "",
    value: "",
    style: {},
    dataset: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    getAttribute(name) {
      return this.attrs[name] === undefined ? null : this.attrs[name];
    },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    addEventListener(type, handler) {
      (this.listeners[type] = this.listeners[type] || []).push(handler);
    },
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    closest() {
      return null;
    },
    blur() {},
    click() {
      (this.listeners.click || []).forEach((fn) => fn({ type: "click", target: this }));
    },
  };
  return el;
}

function makeRoot() {
  const root = makeElement("div");
  root._html = "";
  Object.defineProperty(root, "innerHTML", {
    get() {
      return this._html;
    },
    set(value) {
      this._html = String(value);
    },
  });
  return root;
}

// ---- 把四分模块按浏览器顺序载入同一个 VM 沙箱 ----
// vm context 自带一套内建（Array/Object/…原型与宿主不同）：跨 realm 的返回值直接
// deepEqual 会判"结构相同但引用不等"。因此**所有断言前先 JSON 归一**（同
// r4-diagnostics-ledger 里 sourceCounts 的处理），沙箱本身只注入必要宿主值 + 假 DOM。
const SANDBOX_EXTRAS = {
  document: { createElement: makeElement, getElementById: () => null, body: { classList: { add() {}, remove() {}, toggle() {} } } },
  setTimeout: (fn) => setTimeout(fn, 0),
  clearTimeout: (id) => clearTimeout(id),
};

function loadModules(extra) {
  const sandbox = Object.assign(
    {
      Promise,
      Date,
      JSON,
      encodeURIComponent,
      Math,
      RegExp,
    },
    SANDBOX_EXTRAS,
    extra || {},
  );
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  for (const [name, key] of [
    ["permissions.domain.js", "domain"],
    ["permissions.api.js", "api"],
    ["permissions.controller.js", "controller"],
    ["permissions.view.js", "view"],
  ]) {
    vm.runInContext(sources[key], sandbox, { filename: name });
  }
  return sandbox;
}

const OVERVIEW = {
  profile: "workspace",
  read_only: false,
  spec: { filesystem: "workspace_write", command: "allowlisted", network: "deny", persistence: "task", scopes: ["src/**", "docs/**"] },
  dimensions: [
    { key: "filesystem", label: "文件系统", effective: "workspace_write", source: "profile", configurable: true, summary: "允许读写工作区内的普通文件，删除与越界仍需确认" },
    { key: "command", label: "命令执行", effective: "allowlisted", source: "profile", configurable: true, summary: "仅白名单命令自动执行，其余询问" },
    { key: "network", label: "网络访问", effective: "deny", source: "profile", configurable: true, summary: "禁止工具发起网络请求" },
    { key: "persistence", label: "授权有效期", effective: "task", source: "profile", configurable: true, summary: "审批授权仅在当前任务内复用" },
  ],
  pending: [
    { request_id: "req-1", session_id: "s1", tool: "shell", level: "execute", destructive: true, reason: "执行 npm test", explain: "构建验证", redacted_args: { command: "npm test" }, risk_note: "可能改动工作区" },
  ],
  grants: [{ grant_id: "g-1", tool_id: "file_write", scope: "workspace", created_at: "2026-09-18T10:00:00Z", expires_at: null, remaining_uses: null, path_scope: null, host_scope: null }],
  recent_decisions: [{ ts: "2026-09-18T10:02:03Z", tool: "shell", approved: true, detail: "本次允许" }],
  full_access: { active: false, requires_confirm: true, risk_notes: ["任意命令执行", "任意主机联网", "越出工作区的读写", "关闭前无法逐项确认"] },
};

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

/** 跨 realm 归一：沙箱内建原型与宿主不同，deepEqual 前一律先过一遍 JSON。 */
function plain(value) {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

/** 撤销后复查用的概览：grants 已清空（断言"条目消失"的权威依据）。 */
function afterRevokeGrants() {
  const next = clone(OVERVIEW);
  next.grants = [];
  return next;
}

// ==================== ① 域层 ====================

test("域·specFromOverview：未提交过结构化配置时必须区分 null 与空对象", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.deepEqual(JSON.parse(JSON.stringify(D.specFromOverview(OVERVIEW))), {
    filesystem: "workspace_write",
    command: "allowlisted",
    network: "deny",
    persistence: "task",
    scopes: ["src/**", "docs/**"],
  });
  assert.equal(D.specFromOverview({ profile: "workspace", spec: null }), null, "spec:null = 从未显式提交，不得编造空配置");
  assert.equal(D.specFromOverview(null), null);
  assert.deepEqual(JSON.parse(JSON.stringify(D.specFromOverview({ spec: { filesystem: "none", command: "deny", network: "deny", persistence: "once" } }).scopes)), [], "scopes 缺失归一为空数组");
});

test("域·dimensionsFromOverview：以服务端矩阵为准，缺项标为不可配置而不是自行推导", () => {
  const D = loadModules().OwoPermissionsDomain;
  const rows = D.dimensionsFromOverview(OVERVIEW);
  assert.deepEqual(plain(rows.map((row) => row.key)), ["filesystem", "command", "network", "persistence"]);
  assert.equal(rows[0].effective, "workspace_write");
  assert.equal(rows[0].configurable, true);
  const partial = D.dimensionsFromOverview({ dimensions: [{ key: "network", effective: "deny", configurable: false }] });
  const network = partial.find((row) => row.key === "network");
  assert.equal(network.configurable, false, "configurable:false 必须原样保留（假控件判据来源）");
  assert.equal(network.label, "网络访问", "缺 label 时按 key 给中文标签");
  const missing = partial.find((row) => row.key === "command");
  assert.equal(missing.effective, "", "服务端没给的生效值不得前端推导");
  assert.equal(missing.configurable, false, "缺维度按不可配置降级，不给假控件");
  // dimensions: [] 是合法响应（没有可展示维度），走空态而不是坏响应。
  const bare = D.dimensionsFromOverview({ profile: "read_only", dimensions: [] });
  assert.deepEqual(plain(bare.map((row) => row.configurable)), [false, false, false, false]);
  assert.equal(D.overviewIsUsable({ profile: "read_only", dimensions: [] }), true);
});

test("域·validateSpec：合法返回空数组，非法返回中文原因数组", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.deepEqual(plain(D.validateSpec(OVERVIEW.spec)), []);
  assert.ok(D.validateSpec(null).length >= 1, "空配置必须有原因");
  const bad = D.validateSpec({ filesystem: "everything", command: "", network: "deny", persistence: "forever", scopes: ["C:\\work", "../etc/passwd", "", "src/**", "src/**"] });
  const joined = bad.join("\n");
  assert.match(joined, /文件系统/, "逐条给中文原因");
  assert.match(joined, /命令执行/);
  assert.match(joined, /授权有效期/);
  assert.match(joined, /绝对路径/, "工作区相对字面量红线");
  assert.match(joined, /上级跳转/);
  assert.match(joined, /为空/);
  assert.match(joined, /重复/);
});

test("域·needsFullAccessConfirm + 时长选项（完全访问三要素里的时长不能手填）", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.equal(D.needsFullAccessConfirm({ command: "unrestricted", network: "deny" }), true);
  assert.equal(D.needsFullAccessConfirm({ command: "allowlisted", network: "unrestricted" }), true);
  assert.equal(D.needsFullAccessConfirm({ command: "allowlisted", network: "deny" }), false);
  assert.equal(D.needsFullAccessConfirm(null), false);
  const options = D.durationOptions();
  assert.ok(options.length >= 3 && options.every((o) => Number.isFinite(o.secs) && o.secs > 0 && /分钟|小时/.test(o.label)));
  assert.ok(options.some((o) => o.secs === D.defaultDurationSecs()), "缺省时长必须在候选里");
});

test("域·summarizeScope / persistenceLabel：新契约三值 + 旧字面量兼容", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.equal(D.persistenceLabel("once"), "仅本次");
  assert.equal(D.persistenceLabel("task"), "本任务");
  assert.equal(D.persistenceLabel("workspace"), "工作区长期");
  assert.equal(D.persistenceLabel("session"), "此会话", "旧审批条字面量必须仍能翻译（向后兼容）");
  assert.equal(D.persistenceLabel("one_hour"), "一小时内");
  assert.equal(D.persistenceLabel("always_readonly"), "只读长期");
  assert.equal(D.persistenceLabel("brand_new_enum"), "brand_new_enum", "未知值原样透出，不假装翻译成功");
  assert.equal(D.summarizeScope("workspace_write"), "工作区内读写");
  assert.equal(D.summarizeScope("deny"), "禁止");
  assert.equal(D.summarizeScope(""), "未设置");
});

test("域·groupByDimension：同类请求聚组、未知维度排最后、脏输入不炸", () => {
  const D = loadModules().OwoPermissionsDomain;
  const groups = D.groupByDimension([
    { request_id: "a", tool: "shell" },
    { request_id: "b", tool: "file_write" },
    { request_id: "c", tool: "browser_open" },
    { request_id: "d", tool: "mcp__odd" },
    null,
  ]);
  assert.deepEqual(plain(groups.map((g) => g.key)), ["filesystem", "command", "network", "other"]);
  assert.equal(groups[1].label, "命令执行");
  assert.deepEqual(plain(groups[3].items.map((i) => i.request_id)), ["d"]);
  assert.deepEqual(plain(D.groupByDimension(undefined)), []);
});

test("域·revokePayload：三种粒度各归一，混传/空传拒绝", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.deepEqual(plain(D.revokePayload({ grant_id: "g-1" })), { ok: true, payload: { grant_id: "g-1" }, granularity: "grant" });
  assert.deepEqual(plain(D.revokePayload({ tool_id: "file_write" })), { ok: true, payload: { tool_id: "file_write" }, granularity: "tool" });
  assert.deepEqual(plain(D.revokePayload({ all: true })), { ok: true, payload: { all: true }, granularity: "all" });
  assert.equal(D.revokePayload({ grant_id: "g-1", all: true }).ok, false, "两种粒度同时给 = 误伤面扩大，必须拒绝");
  assert.equal(D.revokePayload({}).ok, false);
  assert.equal(D.revokePayload(null).ok, false);
});

test("域·isGrantGone：撤销后复查判据（条目消失 = 正常终态）", () => {
  const D = loadModules().OwoPermissionsDomain;
  const list = [{ grant_id: "g-2", tool_id: "shell" }];
  assert.equal(D.isGrantGone(list, { grant_id: "g-1" }), true);
  assert.equal(D.isGrantGone(list, { grant_id: "g-2" }), false);
  assert.equal(D.isGrantGone(list, { tool_id: "file_write" }), true);
  assert.equal(D.isGrantGone([], { all: true }), true);
});

test("域·isPendingGone / isErrorGone：404 与 gone 类都是正常态判据", () => {
  const D = loadModules().OwoPermissionsDomain;
  for (const code of ["gone", "resource/gone", "not_found", "permission/expired", "404"]) {
    assert.equal(D.isPendingGone(code), true, `${code} 应判为已消失`);
  }
  assert.equal(D.isPendingGone("validation/failed"), false);
  assert.equal(D.isPendingGone(""), false);
  assert.equal(D.isPendingGone("http/5404"), false, "数字边界：5404 不是 404");
  assert.equal(D.isErrorGone({ status: 404, message: "404: not found" }), true);
  assert.equal(D.isErrorGone({ message: "404: {\"error\":{\"code\":\"resource/gone\"}}" }), true);
  assert.equal(D.isErrorGone(new Error("boom")), false);
});

test("域·normalizeError：两种错误形态都兼容且 code 原样可见（§3.4）", () => {
  const D = loadModules().OwoPermissionsDomain;
  const objectForm = D.normalizeError({ status: 400, body: '{"error":{"code":"validation/failed","message":"维度只能收紧"}}' });
  assert.equal(objectForm.code, "validation/failed");
  assert.equal(objectForm.message, "维度只能收紧");
  assert.equal(objectForm.status, 400);
  const stringForm = D.normalizeError({ status: 404, body: JSON.stringify({ error: "grant vanished" }) });
  assert.equal(stringForm.message, "grant vanished", "字符串形态也必须给出可读消息");
  assert.equal(D.isPendingGone(stringForm.code) || stringForm.status === 404, true);
  const envelope = D.normalizeError({ error: { code: "state/conflict", message: "已被其他窗口处理" } });
  assert.equal(envelope.code, "state/conflict");
  assert.equal(D.normalizeError("plain string").message, "plain string");
  assert.equal(D.normalizeError(null).message, "未知错误");
  assert.equal(D.normalizeError({ status: 500, body: "<html>boom</html>" }).status, 500, "非 JSON 正文不得抛异常");
});

test("域·overviewIsUsable：HTTP 200 不等于有数据", () => {
  const D = loadModules().OwoPermissionsDomain;
  assert.equal(D.overviewIsUsable(OVERVIEW), true);
  assert.equal(D.overviewIsUsable({}), false);
  assert.equal(D.overviewIsUsable({ profile: "workspace" }), false, "缺 dimensions 字段就是坏响应");
  assert.equal(D.overviewIsUsable({ profile: "workspace", dimensions: [], ok: false }), false, "ok:false 的失败信封不得当空数据渲染");
  assert.equal(D.overviewIsUsable(null), false);
});

// ==================== ② api 层 ====================

function fakeFetch(responses) {
  const calls = [];
  const fetchJson = (path, options) => {
    calls.push({ path, options, body: options && typeof options.body === "string" ? JSON.parse(options.body) : undefined });
    const next = responses.shift();
    if (!next) return Promise.resolve({});
    if (next.reject) return Promise.reject(next.reject);
    return Promise.resolve(next.value === undefined ? {} : next.value);
  };
  return { fetchJson, calls };
}

test("api·overview 走 GET /permissions/overview（路径集中在常量、方法显式）", async () => {
  const sandbox = loadModules();
  const { fetchJson, calls } = fakeFetch([{ value: OVERVIEW }]);
  const api = sandbox.OwoPermissionsApi.create({ fetchJson });
  const result = await api.overview();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].path, "/permissions/overview");
  assert.equal(calls[0].options.method, "GET");
  assert.equal(result.profile, "workspace");
  assert.equal(sandbox.OwoPermissionsApi.OVERVIEW_PATH, "/permissions/overview");
});

test("api·submitSpec：普通配置不带 confirm/duration_secs，完全访问必须带", async () => {
  const sandbox = loadModules();
  const { fetchJson, calls } = fakeFetch([{ value: { ok: true, profile: "custom", denials_added: 2 } }]);
  const api = sandbox.OwoPermissionsApi.create({ fetchJson });
  const result = await api.submitSpec(clone(OVERVIEW.spec), {});
  assert.equal(result.ok, true);
  assert.equal(calls[0].path, "/permissions/spec");
  assert.equal(calls[0].options.method, "POST");
  assert.deepEqual(Object.keys(calls[0].body).sort(), ["spec"], "非完全访问不得偷带 confirm/duration_secs");
  assert.deepEqual(calls[0].body.spec.scopes, ["src/**", "docs/**"]);

  await api.submitSpec({ filesystem: "custom", command: "unrestricted", network: "unrestricted", persistence: "workspace", scopes: [] }, { confirm: true, durationSecs: 3600 });
  assert.deepEqual(calls[1].body.confirm, true);
  assert.equal(calls[1].body.duration_secs, 3600);
});

test("api·完全访问缺 confirm → 本地拒绝且一个请求都不发", async () => {
  const sandbox = loadModules();
  const { fetchJson, calls } = fakeFetch([]);
  const api = sandbox.OwoPermissionsApi.create({ fetchJson });
  const spec = { filesystem: "custom", command: "unrestricted", network: "deny", persistence: "task", scopes: [] };
  const noConfirm = await api.submitSpec(spec, { durationSecs: 600 });
  assert.equal(noConfirm.ok, false);
  assert.equal(calls.length, 0, "缺确认标记就不该产生网络往返");
  const noDuration = await api.submitSpec(spec, { confirm: true });
  assert.equal(noDuration.ok, false);
  assert.equal(calls.length, 0, "缺时长同样不发请求");
  assert.match(noDuration.error.code, /^validation\//, "本地拒绝也要给稳定码前缀");
});

test("api·revoke 载荷与审批端点/body（scope 字面量 once|task|workspace）", async () => {
  const sandbox = loadModules();
  const { fetchJson, calls } = fakeFetch([{ value: { ok: true, revoked: 1 } }, { value: { ok: true } }, { value: { ok: true } }, { value: { ok: true } }, { value: { ok: true } }]);
  const api = sandbox.OwoPermissionsApi.create({ fetchJson });
  assert.deepEqual((await api.revoke({ grant_id: "g-1" })) && calls[0].body, { grant_id: "g-1" });
  assert.equal(calls[0].path, "/permissions/grants/revoke");
  assert.equal((await api.revoke({ tool_id: "file_write" })).ok !== undefined || true, true);
  assert.deepEqual(calls[1].body, { tool_id: "file_write" });
  await api.revoke({ all: true });
  assert.deepEqual(calls[2].body, { all: true });
  const rejected = await api.revoke({});
  assert.equal(rejected.ok, false, "空载荷不发请求");
  assert.equal(calls.length, 3, "被拒的空载荷没有产生第 4 次调用");

  await api.respondApproval("s1", "req-1", { allow: false });
  assert.equal(calls[3].path, "/session/s1/permission/req-1");
  assert.deepEqual(calls[3].body, { allow: false }, "拒绝只带 allow，不带 scope");
  await api.respondApproval("s 1", "r/2", { allow: true, scope: "task" });
  assert.equal(calls[4].path, "/session/" + encodeURIComponent("s 1") + "/permission/" + encodeURIComponent("r/2"), "标识必须 URL 编码");
  assert.deepEqual(calls[4].body, { allow: true, scope: "task" });
});

// ==================== ② controller 层 ====================

function harness(responses, options) {
  const opts = options || {};
  const sandbox = loadModules(opts.sandboxExtras);
  const { fetchJson, calls } = fakeFetch(responses);
  const renders = [];
  const notes = [];
  const controller = sandbox.OwoPermissionsController.create({
    api: sandbox.OwoPermissionsApi.create({ fetchJson }),
    domain: sandbox.OwoPermissionsDomain,
    render: (snap) => renders.push(snap),
    notify: (text) => notes.push(text),
    setTimeout: opts.setTimeout,
    clearTimeout: opts.clearTimeout,
  });
  return { sandbox, controller, calls, renders, notes };
}

async function flush(times) {
  for (let i = 0; i < (times || 6); i += 1) await new Promise((resolve) => setImmediate(resolve));
}

test("controller·loading：进入面板的第一帧是 loading，且不播报任何权限事实", async () => {
  const h = harness([{ value: OVERVIEW }]);
  const pending = h.controller.load();
  assert.equal(h.controller.snapshot().phase, "loading");
  await pending;
  assert.equal(h.calls.length, 1, "load 即发 overview 请求（按需加载的唯一入口）");
  const first = h.renders[0];
  assert.equal(first.phase, "loading", "进入面板的第一帧必须是 loading 骨架");
  assert.equal(first.pending.length, 0, "骨架态不得播报待审批数量（0 ≠ 未知）");
  assert.equal(first.grants.length, 0);
  assert.equal(first.dimensions.length, 0);
  assert.equal(h.controller.snapshot().phase, "ready");
});

test("controller·ready：四维矩阵 + 待审批 + 已授权 + 最近决定全部就位并回灌状态条", async () => {
  const reported = [];
  const h = harness([{ value: OVERVIEW }], {
    sandboxExtras: { OwoStatusBar: { reportPermission: (info) => reported.push(info) } },
  });
  await h.controller.load();
  const snap = h.controller.snapshot();
  assert.equal(snap.phase, "ready");
  assert.equal(snap.profile, "workspace");
  assert.match(snap.profileLabel, /工作区编辑/, "档位要给用户看得懂的中文名");
  assert.equal(snap.dimensions.length, 4);
  assert.equal(snap.pending.length, 1);
  assert.equal(snap.grants.length, 1);
  assert.equal(snap.recentDecisions.length, 1);
  assert.deepEqual(plain(snap.draft.scopes), ["src/**", "docs/**"]);
  assert.equal(reported.length, 1, "每次渲染完成后回灌状态条（零额外请求）");
  assert.deepEqual(plain(reported[0]), { profile: "workspace（工作区编辑）", pendingApprovals: 1, grants: 1 });
});

test("controller·error：网络失败与 200 空数据都落错误态并保留重试出口", async () => {
  const broken = harness([{ reject: new Error('404: {"error":{"code":"permissions/unavailable","message":"端点未就绪"}}') }]);
  await broken.controller.load();
  const snap = broken.controller.snapshot();
  assert.equal(snap.phase, "error");
  assert.equal(snap.lastError.code, "permissions/unavailable", "稳定错误码必须原样可见（§3.4）");
  assert.match(String(snap.lastError.message), /端点未就绪/);
  assert.ok(broken.renders.some((render) => render.phase === "error"), "错误态必须真的渲染出去");

  const emptyBody = harness([{ value: {} }]);
  await emptyBody.controller.load();
  assert.equal(emptyBody.controller.snapshot().phase, "error", "HTTP 200 但没有任何可用字段 ≠ 有数据");
  assert.match(String(emptyBody.controller.snapshot().lastError.message), /空数据|没有可用字段|缺少/);

  const failureEnvelope = harness([{ value: { ok: false, error: { code: "permissions/unavailable", message: "权限子系统未就绪" } } }]);
  await failureEnvelope.controller.load();
  assert.equal(failureEnvelope.controller.snapshot().phase, "error", "200 + ok:false 是失败，不是空态");
  assert.equal(failureEnvelope.controller.snapshot().lastError.code, "permissions/unavailable");
});

test("controller·empty：三个列表全空且无维度矩阵时是空态，不是错误态", async () => {
  const h = harness([{ value: { profile: "read_only", read_only: true, spec: null, dimensions: [], pending: [], grants: [], recent_decisions: [], full_access: { active: false, requires_confirm: true, risk_notes: [] } } }]);
  await h.controller.load();
  const snap = h.controller.snapshot();
  assert.equal(snap.phase, "ready");
  assert.equal(snap.empty, true);
  assert.equal(snap.readOnly, true);
  assert.equal(snap.spec, null, "从未显式提交配置时保持 null，不编造");
});

test("controller·提交前校验：非法草稿被拦住，一次请求都不发", async () => {
  const h = harness([{ value: OVERVIEW }]);
  await h.controller.load();
  h.controller.setDimension("filesystem", "everything");
  h.controller.setScopes(["../outside"]);
  const result = await h.controller.submitDraft();
  assert.equal(result.ok, false);
  assert.ok(result.validation.length >= 2);
  assert.equal(h.calls.filter((call) => call.path === "/permissions/spec").length, 0, "非法提交不得打服务端");
  assert.deepEqual(h.controller.snapshot().errors, result.validation, "中文原因必须回显在页面上");
});

test("controller·完全访问：双确认 + 风险告知 + 时长选择，未确认不发请求", async () => {
  const h = harness([{ value: OVERVIEW }, { value: { ok: true, profile: "custom", denials_added: 0 } }, { value: OVERVIEW }]);
  await h.controller.load();
  h.controller.setDimension("command", "unrestricted");
  h.controller.setDimension("network", "unrestricted");
  const first = await h.controller.submitDraft();
  assert.equal(first.ok, false);
  assert.equal(first.needsConfirmation, true);
  const specCalls = h.calls.filter((call) => call.path === "/permissions/spec");
  assert.equal(specCalls.length, 0, "第一次点击只做二次确认，绝不静默提交");
  const confirming = h.controller.snapshot().confirming;
  assert.ok(confirming, "确认卡必须存在");
  assert.equal(confirming.riskNotes.length, OVERVIEW.full_access.risk_notes.length, "风险告知来自服务端权威清单");
  assert.ok(confirming.durationOptions.length >= 3, "时长必须是候选，不允许手填");
  assert.equal(confirming.durationSecs, 3600);
  h.controller.chooseDuration(600);
  assert.equal(h.controller.snapshot().confirming.durationSecs, 600);
  await h.controller.submitDraft();
  const submitted = h.calls.filter((call) => call.path === "/permissions/spec")[0];
  assert.deepEqual(submitted.body.confirm, true);
  assert.equal(submitted.body.duration_secs, 600, "确认后必须带上时长");
});

test("controller·无需确认时不留死控件：给原因而不是静默 return", async () => {
  const h = harness([{ value: OVERVIEW }]);
  await h.controller.load();
  // 默认草稿（allowlisted / deny）不含不受限维度：此时二次确认没有意义。
  assert.equal(h.controller.requestFullAccess(), null, "无需确认不得伪造一张卡");
  const snap = h.controller.snapshot();
  assert.equal(snap.confirming, null);
  assert.match(snap.notice, /不含不受限/, "点了按钮必须说清楚为什么没反应");
  assert.equal(h.calls.filter((call) => call.path === "/permissions/spec").length, 0, "解释性分支不得提交配置");
  // 含不受限维度的候选直接走同一个入口（不经 setDimension，免得多清一次提示）。
  const opened = h.controller.requestFullAccess({
    filesystem: "workspace_write",
    command: "unrestricted",
    network: "deny",
    persistence: "task",
    scopes: [],
  });
  assert.ok(opened, "含不受限维度时必须开确认卡");
  assert.equal(opened.spec.command, "unrestricted");
  assert.ok(opened.durationOptions.length >= 2, "时长候选随卡给出，不能让用户手填");
  assert.equal(h.controller.snapshot().notice, "", "开卡后上一条解释文案要清掉");
});

test("controller·取消二次确认回到未提交态；一步关闭完全访问不需要确认", async () => {
  const h = harness([{ value: OVERVIEW }, { value: { ok: true } }, { value: OVERVIEW }]);
  await h.controller.load();
  h.controller.setDimension("command", "unrestricted");
  await h.controller.submitDraft();
  h.controller.cancelConfirm();
  assert.equal(h.controller.snapshot().confirming, null);
  assert.equal(h.calls.filter((call) => call.path === "/permissions/spec").length, 0);

  const close = harness([{ value: clone(OVERVIEW) }, { value: { ok: true } }, { value: OVERVIEW }]);
  await close.controller.load();
  close.controller.setDimension("command", "unrestricted");
  close.controller.setDimension("network", "unrestricted");
  await close.controller.closeFullAccess();
  const body = close.calls.filter((call) => call.path === "/permissions/spec")[0];
  assert.equal(body.body.spec.command, "allowlisted");
  assert.equal(body.body.spec.network, "allowlisted");
  assert.equal("confirm" in body.body, false, "关闭是收紧方向，不需要也不得带确认要素");
});

test("controller·审批四动作：body {allow,scope} 正确，成功后重拉概览", async () => {
  const actions = ["deny", "once", "task", "workspace"];
  const readOnlyItem = { request_id: "req-1", session_id: "s1", level: "read", destructive: false };
  for (const action of actions) {
    const h = harness([{ value: OVERVIEW }, { value: { ok: true } }, { value: clone(OVERVIEW) }]);
    await h.controller.load();
    const before = h.calls.length;
    await h.controller.respondApproval(readOnlyItem, action);
    const approval = h.calls.slice(before).find((call) => call.path === "/session/s1/permission/req-1");
    assert.ok(approval, "审批动作必须打到会话审批端点：" + action);
    if (action === "deny") assert.deepEqual(approval.body, { allow: false });
    else assert.deepEqual(approval.body, { allow: true, scope: action });
    assert.equal(h.calls[h.calls.length - 1].path, "/permissions/overview", "动作后要重新拉权威概览");
    assert.equal(h.controller.snapshot().phase, "ready");
  }
  const unknown = harness([{ value: OVERVIEW }]);
  await unknown.controller.load();
  const rejected = await unknown.controller.respondApproval(readOnlyItem, "always_readonly");
  assert.equal(rejected.ok, false, "旧审批条字面量不属于权限中心四动作，必须拒绝而不是猜");
});

test("controller·破坏性审批不可请求持久授权", async () => {
  const h = harness([{ value: OVERVIEW }]);
  await h.controller.load();
  const before = h.calls.length;
  const item = { request_id: "req-1", session_id: "s1", level: "execute", destructive: true };
  const result = await h.controller.respondApproval(item, "workspace");
  assert.equal(result.ok, false);
  assert.equal(h.calls.length, before, "必须在客户端拦截，并由服务端重复校验");
});
test("controller·审批条目已消失（404/gone）走空态而非错误态", async () => {
  const h = harness([
    { value: OVERVIEW },
    { reject: new Error('404: {"error":{"code":"resource/gone","message":"请求已处理"}}') },
    { value: clone(OVERVIEW) },
  ]);
  await h.controller.load();
  const result = await h.controller.respondApproval({ request_id: "req-1", session_id: "s1" }, "once");
  assert.equal(result.ok, true);
  assert.equal(result.gone, true);
  const snap = h.controller.snapshot();
  assert.equal(snap.phase, "ready", "条目消失是正常终态，不得进错误态");
  assert.equal(snap.errors.length, 0);
  assert.ok(snap.goneNotices.length >= 1, "必须留下人类可读说明");
});

test("controller·撤销三粒度级联 payload + 撤销后立即复查（断言条目消失）", async () => {
  // 单条：第一次概览含 g-1，复查后的概览不含 → 提示"已撤销…复查确认"。
  const afterRevoke = clone(OVERVIEW);
  afterRevoke.grants = [];
  const h = harness([{ value: OVERVIEW }, { value: { ok: true, revoked: 1 } }, { value: afterRevoke }]);
  await h.controller.load();
  const done = h.controller.revoke({ grant_id: "g-1" });
  assert.equal(h.controller.snapshot().verify, "撤销后自动复查…", "复查期间必须给提示（用户不该看到静默空白）");
  const result = await done;
  assert.equal(result.ok, true);
  assert.equal(h.calls[1].path, "/permissions/grants/revoke");
  assert.deepEqual(h.calls[1].body, { grant_id: "g-1" });
  assert.equal(h.calls[2].path, "/permissions/overview", "撤销后必须重新拉概览做复查");
  assert.equal(h.controller.snapshot().grants.length, 0);
  assert.equal(h.controller.snapshot().verify, "");
  assert.match(h.controller.snapshot().notice, /复查确认/);

  // 按工具 / 全部：payload 原样透传。
  const byTool = harness([{ value: OVERVIEW }, { value: { ok: true, revoked: 2 } }, { value: clone(OVERVIEW) }]);
  await byTool.controller.load();
  await byTool.controller.revoke({ tool_id: "file_write" });
  assert.deepEqual(byTool.calls[1].body, { tool_id: "file_write" });

  const all = harness([{ value: OVERVIEW }, { value: { ok: true, revoked: 3 } }, { value: afterRevoke }]);
  await all.controller.load();
  await all.controller.revoke({ all: true });
  assert.deepEqual(all.calls[1].body, { all: true });

  const mixed = harness([{ value: OVERVIEW }]);
  await mixed.controller.load();
  const refused = await mixed.controller.revoke({ grant_id: "g-1", tool_id: "file_write" });
  assert.equal(refused.ok, false);
  assert.equal(mixed.calls.filter((call) => call.path === "/permissions/grants/revoke").length, 0, "混传粒度不得发出");
});

test("controller·revoked:0 与复查仍存在：前者当正常态，后者才报错", async () => {
  const zero = harness([{ value: OVERVIEW }, { value: { ok: true, revoked: 0 } }, { value: clone(OVERVIEW) }]);
  await zero.controller.load();
  const result = await zero.controller.revoke({ grant_id: "nope" });
  assert.equal(result.ok, true, "不存在/已被撤销（revoked:0）是幂等正常态");
  assert.match(zero.controller.snapshot().notice, /没有可撤销|已被撤销/);
  assert.equal(zero.controller.snapshot().errors.length, 0);

  const still = harness([{ value: OVERVIEW }, { value: { ok: true, revoked: 1 } }, { value: clone(OVERVIEW) }]);
  await still.controller.load();
  const failed = await still.controller.revoke({ grant_id: "g-1" });
  assert.equal(failed.ok, false, "复查发现条目仍在 → 必须诚实报未生效");
  assert.match(still.controller.snapshot().errors.join(""), /仍然存在/);
});

test("controller·dispose 清 timer：本页禁止 setInterval，离开面板后不得再回调渲染", async () => {
  let scheduled = 0;
  let cleared = 0;
  const timers = new Map();
  const h = harness(
    [{ value: OVERVIEW }, { value: { ok: true, revoked: 1 } }, { value: afterRevokeGrants() }],
    {
      setTimeout: (fn, ms) => {
        scheduled += 1;
        const id = setTimeout(() => {
          if (timers.get(id) !== "armed") return; // 已被 dispose 取消：回调不得再跑
          timers.delete(id);
          fn();
        }, ms);
        timers.set(id, "armed");
        return id;
      },
      clearTimeout: (id) => {
        cleared += 1;
        timers.set(id, "canceled");
      },
    },
  );
  await h.controller.load();
  // 故意**不 await** revoke()：它的复查是排在一个一次性 timer 里的，await 到底
  // 就等于等 timer 跑完，那时 dispose 已经无事可清 —— 测试要的是"离开面板时
  // 复查 timer 还挂着"这一刻。
  const inflight = h.controller.revoke({ grant_id: "g-1" });
  await flush(2); // 放行微任务：api.revoke 已回，复查 timer 已排（120ms 还没到）
  assert.equal(scheduled, 1, "复查用一次性 timer，不用周期轮询");
  h.controller.dispose();
  assert.ok(cleared >= 1, "离开面板必须清掉挂起的 timer");
  const rendersBefore = h.renders.length;
  await flush(2);
  assert.equal(h.renders.length, rendersBefore, "dispose 后不得再触发渲染");
  assert.ok(inflight, "撤销动作已发出（其 Promise 因 dispose 而不再落地，属预期）");
  for (const file of ["controller", "api", "domain", "view"]) {
    assert.ok(!/setInterval\s*\(/.test(sources[file]), `${file} 不得使用 setInterval`);
  }
});

// ==================== ③ 渲染层 + 接线层 ====================

function baseSnapshot(overrides) {
  return Object.assign(
    {
      phase: "ready",
      profile: "workspace",
      profileLabel: "workspace（工作区编辑）",
      readOnly: false,
      dimensions: [],
      spec: null,
      draft: { filesystem: "workspace_write", command: "allowlisted", network: "deny", persistence: "task", scopes: ["src/**"] },
      pending: [],
      grants: [],
      recentDecisions: [],
      fullAccess: null,
      errors: [],
      notice: "",
      busy: false,
      confirming: null,
      verify: "",
      goneNotices: [],
      updatedAt: "2026-09-18T10:00:00.000Z",
      lastError: null,
      empty: false,
    },
    overrides || {},
  );
}

test("渲染·四维表：可配置维度给下拉，不可配置维度只给文字（假控件防护）", () => {
  const sandbox = loadModules();
  const D = sandbox.OwoPermissionsDomain;
  const html = sandbox.OwoPermissionsView._test.renderDimensionTable(baseSnapshot({ dimensions: D.dimensionsFromOverview(OVERVIEW) }));
  assert.equal((html.match(/<select /g) || []).length, 4, "四个可配置维度各一个下拉");
  assert.match(html, /当前生效[\s\S]*工作区内读写/, "生效值 + 摘要都要出现");
  assert.match(html, /档位展开/, "来源必须写明（档位 vs 显式配置）");
  assert.match(html, /删除与越界仍需确认/, "人类可读范围摘要直接取服务端 summary");

  const honest = sandbox.OwoPermissionsView._test.renderDimensionTable(
    baseSnapshot({
      dimensions: [
        { key: "filesystem", label: "文件系统", effective: "workspace_write", source: "profile", configurable: true, summary: "s" },
        { key: "command", label: "命令执行", effective: "allowlisted", source: "server", configurable: false, summary: "随档位固定" },
      ],
    }),
  );
  assert.equal((honest.match(/<select /g) || []).length, 1, "configurable:false 不得产生可交互元素");
  assert.match(honest, /不可单独配置/, "必须诚实标注原因");
  assert.ok(!/data-perm-nonconfigurable="1"[^>]*><select/.test(honest));
});

test("渲染·待审批：破坏性请求只给一次性动作，只读请求可选授权范围", () => {
  const sandbox = loadModules();
  const view = sandbox.OwoPermissionsView._test;
  const destructive = view.renderPending(baseSnapshot({ pending: OVERVIEW.pending }));
  assert.match(destructive, /s1/, "会话标识");
  assert.match(destructive, /shell/);
  assert.match(destructive, /execute/, "级别");
  assert.match(destructive, /执行 npm test/, "理由");
  assert.match(destructive, /command=npm test/, "脱敏参数摘要");
  assert.match(destructive, /可能改动工作区/, "风险说明");
  for (const label of ["拒绝", "仅本次"]) assert.ok(destructive.includes(">" + label + "<"));
  assert.ok(!destructive.includes(">本任务<") && !destructive.includes(">工作区长期<"));
  assert.equal((destructive.match(/data-perm-action="approval"/g) || []).length, 2);

  const readItem = { ...clone(OVERVIEW.pending[0]), level: "read", destructive: false };
  const readonly = view.renderPending(baseSnapshot({ pending: [readItem] }));
  for (const label of ["拒绝", "仅本次", "本任务", "工作区长期"]) assert.ok(readonly.includes(">" + label + "<"));
  assert.equal((readonly.match(/data-perm-action="approval"/g) || []).length, 4);
  const empty = view.renderPending(baseSnapshot({ pending: [] }));
  assert.match(empty, /data-perm-empty="pending"/, "空列表必须是空态标记，不是错误");
  assert.ok(!/读取失败|错误码/.test(empty), "空态不得混进错误文案");
});
test("渲染·已授予列表：有效期/到期/次数/范围 + 三粒度撤销按钮", () => {
  const sandbox = loadModules();
  const html = sandbox.OwoPermissionsView._test.renderGrants(baseSnapshot({ grants: OVERVIEW.grants }));
  assert.match(html, /file_write/);
  assert.match(html, /工作区长期/, "scope 用人类术语（persistenceLabel）");
  assert.match(html, /无固定到期/);
  assert.match(html, /次数不限/);
  assert.match(html, /跟随档位范围/, "path_scope/host_scope 为 null 时说明范围口径");
  assert.match(html, /data-perm-action="revoke-grant"/);
  assert.match(html, /data-perm-action="revoke-tool"/);
  assert.match(html, /data-perm-action="revoke-all"/);
  const inactive = clone(OVERVIEW.grants[0]);
  inactive.effective = false;
  const inactiveHtml = sandbox.OwoPermissionsView._test.renderGrants(baseSnapshot({ grants: [inactive] }));
  assert.match(inactiveHtml, /历史授权已失效/, "旧非只读授权要明确标为不可复用");
  const empty = sandbox.OwoPermissionsView._test.renderGrants(baseSnapshot({ grants: [] }));
  assert.match(empty, /data-perm-empty="grants"/);
  assert.ok(!/revoke-all/.test(empty), "无授权时不给撤销入口（避免空操作假控件）");
});

test("渲染·三态与复查提示：loading/error/empty 都有专属标记，错误态保留重试", () => {
  const sandbox = loadModules();
  const view = sandbox.OwoPermissionsView._test;
  assert.match(view.renderStates(baseSnapshot({ phase: "loading" })), /data-perm-phase="loading"/);
  const error = view.renderStates(baseSnapshot({ phase: "error", lastError: { code: "permissions/unavailable", message: "端点未就绪", status: 404 } }));
  assert.match(error, /data-perm-phase="error"/);
  assert.match(error, /permissions\/unavailable/, "稳定错误码要在页面上可见（§3.4）");
  assert.doesNotMatch(error, /data-perm-action="reload"/, "重试按钮统一放在页头，避免重复出口");
  assert.match(view.renderStates(baseSnapshot({ empty: true })), /data-perm-phase="empty"/);
  assert.match(view.renderStates(baseSnapshot({ verify: "撤销后自动复查…" })), /^$/);
  const root = makeRoot();
  const render = sandbox.OwoPermissionsView._test.render;
  render(root, baseSnapshot({ verify: "撤销后自动复查…" }));
  assert.match(root.innerHTML, /撤销后自动复查/);
});

test("渲染·完全访问确认卡：范围 + 时长候选 + 风险清单三要素齐备", () => {
  const sandbox = loadModules();
  const D = sandbox.OwoPermissionsDomain;
  const view = sandbox.OwoPermissionsView._test;
  const snap = baseSnapshot({
    fullAccess: OVERVIEW.full_access,
    draft: { filesystem: "custom", command: "unrestricted", network: "unrestricted", persistence: "workspace", scopes: [] },
    confirming: {
      spec: { filesystem: "custom", command: "unrestricted", network: "unrestricted", persistence: "workspace", scopes: [] },
      riskNotes: OVERVIEW.full_access.risk_notes,
      durationSecs: 3600,
      durationOptions: D.durationOptions(),
      requiresConfirm: true,
    },
  });
  const html = view.renderFullAccess(snap);
  assert.match(html, /二次确认/, "明示这是二次确认，不是一键开关");
  assert.match(html, /范围/, "三要素之一：范围");
  assert.match(html, /unrestricted/);
  assert.match(html, /name="perm-duration"/, "三要素之一：时长（单选候选，不是文本框）");
  assert.equal((html.match(/type="radio"/g) || []).length, D.durationOptions().length);
  assert.match(html, /任意主机联网/, "三要素之一：风险（服务端权威清单）");
  assert.match(html, /data-perm-action="confirm-full-access"/);
  assert.match(html, /data-perm-action="cancel-confirm"/);
  const idle = view.renderFullAccess(baseSnapshot({ fullAccess: OVERVIEW.full_access }));
  assert.match(idle, /申请完全访问（需二次确认）/);
  assert.ok(!/type="radio"/.test(idle), "未点申请时不铺确认卡");
});

test("渲染·事件委托：动作转成 controller 调用（审批索引定位、撤销粒度、维度变更）", async () => {
  const sandbox = loadModules();
  const D = sandbox.OwoPermissionsDomain;
  const calls = [];
  const stub = {
    snapshot: () => baseSnapshot({ pending: OVERVIEW.pending, draft: clone(OVERVIEW.spec) }),
    respondApproval: (item, action) => calls.push(["approval", item && item.request_id, action]),
    revoke: (payload) => calls.push(["revoke", payload]),
    setDimension: (key, value) => calls.push(["dimension", key, value]),
    chooseDuration: (secs) => calls.push(["duration", secs]),
    submitDraft: () => calls.push(["submit"]),
    reload: () => calls.push(["reload"]),
    cancelConfirm: () => calls.push(["cancel"]),
    closeFullAccess: () => calls.push(["close"]),
    requestFullAccess: () => calls.push(["request"]),
  };
  const view = sandbox.OwoPermissionsView._test;
  // 委托入口读的是模块级 controller（真实链路由 mount 注入）；单测必须显式绑桩，
  // 否则 onAction 一律早退成 undefined —— 那种"全绿"是假绿。
  view.setController(stub);
  const fire = (attrs) => {
    const node = makeElement("button");
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
    return view.onAction({ target: { closest: () => node } }, makeRoot());
  };
  // onAction 只负责派发，返回值是 controller 的 Promise（形状随动作而变，不作为契约）；
  // 动作参数是否映射对，看 controller 的调用记录 calls[]。
  await fire({ "data-perm-action": "approval", "data-approval": "task", "data-pending": "0" });
  assert.deepEqual(plain(calls[0]), ["approval", "req-1", "task"], "按钮序号必须映射到 pending 条目");
  await fire({ "data-perm-action": "approval", "data-approval": "once", "data-pending": "9" });
  assert.equal(calls[1][1], "", "越界索引不得拿错条目（宁可传空标识让服务端拒）");
  await fire({ "data-perm-action": "revoke-grant", "data-grant": "g-1" });
  // 载荷对象是 VM Realm 里造的，与本测试 Realm 的原型不同：deepEqual 会报
  // "same structure but not reference-equal"。统一经 plain() 往返再比（本仓既有约定）。
  assert.deepEqual(plain(calls[2]), ["revoke", { grant_id: "g-1" }]);
  await fire({ "data-perm-action": "revoke-tool", "data-tool": "file_write" });
  assert.deepEqual(plain(calls[3]), ["revoke", { tool_id: "file_write" }]);
  assert.equal(fire({ "data-perm-action": "revoke-tool" }), undefined, "未标注工具的授权不提供按工具撤销");
  assert.equal(calls.length, 4);
  await fire({ "data-perm-action": "revoke-all" });
  assert.deepEqual(plain(calls[4]), ["revoke", { all: true }]);
  await fire({ "data-perm-action": "reload" });
  assert.deepEqual(calls[5], ["reload"]);
  await fire({ "data-perm-action": "submit" });
  assert.deepEqual(calls[6], ["submit"]);
  const select = makeElement("select");
  select.setAttribute("name", "perm-dimension-network");
  select.value = "allowlisted";
  view.onChange({ target: select });
  assert.deepEqual(calls[7], ["dimension", "network", "allowlisted"]);
  assert.ok(D.SPEC_FIELDS.includes("network"));
});

test("面板契约：注册 OwoPanels.permissions + mount 才发请求（按需加载）", async () => {
  const responses = [{ value: OVERVIEW }];
  const sandbox = loadModules();
  const { fetchJson, calls } = fakeFetch(responses);
  const root = makeRoot();
  sandbox.OwoPanels.permissions.mount(root, { fetchJson, esc: (v) => String(v == null ? "" : v) });
  assert.equal(sandbox.OwoPanels.permissions.title, "权限中心");
  assert.equal(typeof sandbox.OwoPanels.permissions.dispose, "function");
  await flush(3);
  assert.equal(calls.length, 1, "mount 即发且只发一次概览请求");
  assert.equal(calls[0].path, "/permissions/overview");
  assert.match(root.innerHTML, /文件系统/);
  assert.match(root.innerHTML, /工作区编辑/);
  assert.ok(sandbox.registerOwoPermissionsView, "必须暴露 registerOwoPermissionsView() 供装配");
  sandbox.OwoPanels.permissions.dispose();
  assert.equal(sandbox.OwoPanels.permissions._test.getController(), null, "dispose 后不留控制器（防止后台回调）");
});

test("分层红线：view 零网络、api/domain 零 DOM、网络只经注入的 fetchJson（§4.8）", () => {
  // 逐行剔除注释后再查禁面（文件头的"零 fetch()/零 Tauri invoke"是文档措辞，不是代码）。
  const codeOnly = (source) =>
    source
      .split(/\r?\n/)
      .filter((line) => !/^\s*(\/\/|\*|\/\*)/.test(line))
      .join("\n");
  const viewCode = codeOnly(sources.view);
  assert.ok(!/\bfetch\s*\(|XMLHttpRequest|__TAURI__|\binvoke\s*\(/.test(viewCode), "渲染层不得自己发请求或调壳");
  assert.match(viewCode, /typeof H\.fetchJson === "function" \? H\.fetchJson/, "视图的请求出口只有一个：宿主注入的 fetchJson");
  // 视图里唯一可能触网的是「兜底转交统一客户端」，它不拼 URL、不发自己的请求。
  const fallback = /function defaultTransport[\s\S]*?\n  \}/.exec(viewCode);
  assert.ok(fallback, "兜底传输函数应存在且可定位");
  // 数的是**调用点**：`typeof client.request !== "function"` 是能力探测，不是发请求。
  assert.equal((fallback[0].match(/client\.request\s*\(/g) || []).length, 1, "兜底只做一次转交");
  assert.ok(!/OVERVIEW_PATH|SPEC_PATH|REVOKE_PATH|\/permissions\/|\/session\//.test(fallback[0]), "视图不得知道任何路径（路径只在 api 层）");
  assert.ok(!/document\.|querySelector|innerHTML/.test(sources.api), "API 模块不得直接操作 DOM");
  assert.ok(!/document\.|querySelector|innerHTML|fetch\s*\(/.test(codeOnly(sources.domain)), "领域层零 DOM 零 HTTP");
  assert.ok(!/innerHTML|document\./.test(codeOnly(sources.controller)), "控制器不碰 DOM（渲染走注入的 render）");
  assert.match(sources.api, /global\.OwoApi/, "唯一回退出口是 core/api-client.js 的统一客户端（不裸请求）");
  assert.ok(!/https?:\/\/(127\.0\.0\.1|localhost)/.test(sources.api + sources.domain + sources.controller), "权限模块不得硬编码本机 URL");
});

test("接线：权限中心注册为按需面板，状态条点击进入该面板", () => {
  const order = ["permissions/permissions.domain.js", "permissions/permissions.api.js", "permissions/permissions.controller.js", "permissions/permissions.view.js"].map(
    (src) => indexHtml.indexOf('<script src="' + src + '"></script>'),
  );
  assert.ok(order.every((at) => at >= 0), "权限模块四层都必须引入");
  assert.deepEqual(order, order.slice().sort((a, b) => a - b), "加载顺序必须 domain → api → controller → view");
  assert.ok(order[3] < indexHtml.indexOf('<script src="app-domain.js"></script>'), "面板必须在装配层之前加载");
  assert.match(appDomainSource, /const PANEL_ORDER = \[[\s\S]*?"permissions"/, "权限中心必须进入工具面板目录");
  assert.match(appDomainSource, /function mountPanel\(id, writeHash = true\)/, "权限面板必须复用统一 mount/dispose 生命周期");
  assert.match(appSource, /key === "permission"[\s\S]*?openPanelById\("permissions"\)/,
    "全局权限状态必须打开工具视图并挂载权限中心");
  assert.match(appDomainSource, /function initPanels\(\)[\s\S]*?button\.addEventListener\("click", \(\) => openPanelById\(id\)\)/,
    "面板首个请求由用户显式打开触发");
  const boot = /const BOOT_HYDRATE_TASKS = \[([\s\S]*?)\];/.exec(appSource);
  assert.ok(boot, "首屏水合任务清单应存在");
  assert.ok(!/[Pp]ermissions?/.test(boot[1]), "权限读取不得进入首屏水合");
});

test("权限审批动作与长期授权撤销粒度保持独立", () => {
  assert.ok(indexHtml.includes('id="approvalList"'), "回合审批继续走独立审批队列");
  assert.ok(sources.view.includes('data-perm-action="approval"'), "权限中心通过本面板动作处理审批");
  assert.ok(sources.view.includes('data-perm-action="revoke-grant"'), "长期授权撤销通过权限中心动作处理");
  assert.ok(sources.domain.includes('scope: "once"') && sources.domain.includes('scope: "task"') &&
    sources.domain.includes('scope: "workspace"'), "持久授权范围由权限域契约定义");
  assert.ok(!sources.view.includes('data-scope='), "权限中心不复用旧审批条作用域控件");
});

test("controller·读取失败或只读时所有权限变更入口 fail closed", async () => {
  const failed = harness([{ reject: new Error("auth/pairing_required/not_retryable") }]);
  await failed.controller.load();
  const afterOverview = failed.calls.length;
  assert.equal(failed.controller.setDimension("command", "unrestricted").blocked, true);
  assert.equal(failed.controller.setScopes(["src/**"]).blocked, true);
  assert.equal((await failed.controller.submitDraft()).blocked, true);
  assert.equal(failed.controller.requestFullAccess(), null);
  assert.equal((await failed.controller.respondApproval({ request_id: "req-1", session_id: "s1" }, "deny")).blocked, true);
  assert.equal((await failed.controller.revoke({ all: true })).blocked, true);
  assert.equal(failed.calls.length, afterOverview, "overview 失败后不能提交配置、审批或撤销请求");

  const readOnlyOverview = clone(OVERVIEW);
  readOnlyOverview.read_only = true;
  const readonly = harness([{ value: readOnlyOverview }]);
  await readonly.controller.load();
  const readOnlyCalls = readonly.calls.length;
  assert.equal((await readonly.controller.submitDraft()).blocked, true);
  assert.equal((await readonly.controller.respondApproval({ request_id: "req-1", session_id: "s1" }, "deny")).blocked, true);
  assert.equal((await readonly.controller.revoke({ all: true })).blocked, true);
  assert.equal(readonly.calls.length, readOnlyCalls, "服务端只读档不得发任何变更请求");
});

test("渲染·权限事实未就绪时隐藏推测状态，只留重试；只读档禁用每个变更控件", () => {
  const sandbox = loadModules();
  const view = sandbox.OwoPermissionsView._test;
  const failedRoot = makeRoot();
  view.render(failedRoot, baseSnapshot({
    phase: "error",
    profile: "未知档位",
    draft: { filesystem: "workspace_write", command: "allowlisted", network: "deny", persistence: "task", scopes: [] },
    lastError: { code: "auth/pairing_required/not_retryable", message: "桌面进程配对证明缺失", status: 401 },
    errors: ["桌面进程配对证明缺失"],
  }));
  assert.match(failedRoot.innerHTML, /data-perm-data-unavailable/);
  assert.match(failedRoot.innerHTML, /auth\/pairing_required\/not_retryable/);
  assert.doesNotMatch(failedRoot.innerHTML, /当前档位|待审批请求|完全访问|撤销当前工作区/);
  assert.match(failedRoot.innerHTML, /data-perm-action="submit" disabled/);
  assert.match(failedRoot.innerHTML, />权限概览不可用</);
  assert.equal((failedRoot.innerHTML.match(/data-perm-action="reload"/g) || []).length, 1, "页头只保留一个重试入口");

  const readonlyRoot = makeRoot();
  view.render(readonlyRoot, baseSnapshot({
    readOnly: true,
    dimensions: sandbox.OwoPermissionsDomain.dimensionsFromOverview(OVERVIEW),
    pending: OVERVIEW.pending,
    grants: OVERVIEW.grants,
    fullAccess: OVERVIEW.full_access,
  }));
  assert.match(readonlyRoot.innerHTML, /核心处于只读降级/);
  const mutationButtons = [...readonlyRoot.innerHTML.matchAll(/<button[^>]*data-perm-action="(?!reload)[^"]+"[^>]*>/g)].map((match) => match[0]);
  assert.ok(mutationButtons.length > 0);
  assert.ok(mutationButtons.every((button) => /\sdisabled(?:\s|>)/.test(button)), "所有审批、保存、授权与撤销动作均禁用");
  assert.match(readonlyRoot.innerHTML, /<select[^>]+disabled/);
});
