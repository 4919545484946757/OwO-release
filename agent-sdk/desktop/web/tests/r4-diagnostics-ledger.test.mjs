// R4-3 §4.6「诊断请求台账」契约测试。
//
// 两层：
//   ① 行为层——把 views/diagnostics-ledger.view.js 载入 VM 沙箱（假 DOM），
//      喂一份合成 ledger/overview/壳快照，断言四类计数、慢请求 Top N、
//      按 route_template 的 P50/P95、来源分布、最近引导、SSE 计数都真的渲染出来；
//      并断言 §4.6 禁止回显清单（本地绝对路径 / Bearer / query）在任何渲染与导出
//      字符串里都不出现。这条最容易「页面做了、脱敏漏了」，所以两个出口都测。
//   ② 接线层——index.html 的脚本引用与容器、app.js 的按需加载（不得进首屏清单）。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const viewSource = readFileSync(join(WEB, "views", "diagnostics-ledger.view.js"), "utf8");
const indexHtml = readFileSync(join(WEB, "index.html"), "utf8");
const appSource = readFileSync(join(WEB, "app.js"), "utf8");
const appDomainSource = readFileSync(join(WEB, "app-domain.js"), "utf8");

// ---- 假 DOM：只实现本视图用到的面（innerHTML / querySelector / addEventListener）----
function makeRoot() {
  const nodes = new Map();
  const root = {
    _html: "",
    isConnected: true,
    get innerHTML() {
      return this._html;
    },
    set innerHTML(value) {
      this._html = String(value);
      nodes.clear();
    },
    querySelector(selector) {
      const key = String(selector);
      if (!nodes.has(key)) {
        nodes.set(key, {
          id: key,
          hidden: false,
          textContent: "",
          value: "",
          style: {},
          listeners: {},
          addEventListener(type, handler) {
            (this.listeners[type] = this.listeners[type] || []).push(handler);
          },
          setAttribute() {},
          appendChild() {},
          remove() {},
          select() {},
          click() {
            (this.listeners.click || []).forEach((fn) => fn());
          },
        });
      }
      return nodes.get(key);
    },
  };
  return root;
}

function makeSandbox(extra) {
  const sandbox = Object.assign(
    {
      document: {
        createElement: () => ({ style: {}, setAttribute() {}, appendChild() {}, remove() {}, click() {}, select() {}, value: "" }),
        body: { appendChild() {}, removeChild() {} },
        execCommand: () => true,
      },
      navigator: {},
      URL: { createObjectURL: () => "blob:stub", revokeObjectURL() {} },
      Blob: function Blob() {},
      setTimeout: () => 0,
      clearTimeout: () => {},
    },
    extra || {},
  );
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(viewSource, sandbox, { filename: "diagnostics-ledger.view.js" });
  return sandbox;
}

const SEED_RECORDS = [
  { method: "GET", route_template: "/health", started_at: "2026-09-18T10:00:00.100Z", duration_ms: 3, status: 200, source: "shell" },
  { method: "GET", route_template: "/health", started_at: "2026-09-18T10:00:01.100Z", duration_ms: 4, status: 200, source: "shell" },
  { method: "POST", route_template: "/auth/token", started_at: "2026-09-18T10:00:01.200Z", duration_ms: 12, status: 200, source: "web" },
  { method: "GET", route_template: "/events", started_at: "2026-09-18T10:00:02.000Z", duration_ms: 900, status: 200, source: "web" },
  { method: "GET", route_template: "/sessions", started_at: "2026-09-18T10:00:03.000Z", duration_ms: 40, status: 200, source: "web" },
  { method: "GET", route_template: "/sessions/{id}", started_at: "2026-09-18T10:00:04.000Z", duration_ms: 80, status: 200, source: "web" },
  { method: "GET", route_template: "/sessions/{id}", started_at: "2026-09-18T10:00:05.000Z", duration_ms: 800, status: 200, source: "web" },
  { method: "POST", route_template: "/turns", started_at: "2026-09-18T10:00:06.000Z", duration_ms: 1500, status: 500, source: "shell" },
];

function seedLedger() {
  const health = SEED_RECORDS.filter((r) => r.route_template === "/health").length;
  const auth = SEED_RECORDS.filter((r) => r.route_template === "/auth/token").length;
  return {
    total: 900,
    returned: SEED_RECORDS.length,
    cap: 512,
    aggregates: { health, auth_token: auth, business: SEED_RECORDS.length - health - auth },
    records: SEED_RECORDS.slice(),
  };
}

const OVERVIEW = { sse: { active_connections: 2, total_connections: 7, lagged_total: 1 } };

test("四类计数：health/auth 取服务端口径，events 单列且 business 去掉 SSE", () => {
  const sandbox = makeSandbox();
  const ledger = sandbox.OwoDiagnosticsLedger;
  const buckets = ledger.bucketCounts(seedLedger());
  assert.equal(buckets.health, 2);
  assert.equal(buckets.auth, 1);
  assert.equal(buckets.events, 1, "/events 必须被认成 SSE 一类");
  assert.equal(buckets.business_server, 5, "服务端 business 含 SSE");
  assert.equal(buckets.business, 4, "页面 business 口径剔除 SSE，四类可互相核对");
});

test("P50/P95 按 route_template 聚合：最近秩法 + 错误计数", () => {
  const sandbox = makeSandbox();
  const ledger = sandbox.OwoDiagnosticsLedger;
  const rows = ledger.routeAggregate(SEED_RECORDS);
  const detail = rows.find((r) => r.route === "/sessions/{id}");
  assert.ok(detail, "/sessions/{id} 应聚合成一行");
  assert.equal(detail.count, 2);
  assert.equal(detail.p50_ms, 80, "最近秩：2 条时 P50 取第 1 小");
  assert.equal(detail.p95_ms, 800, "P95 取最大");
  assert.equal(detail.errors, 0);
  const turns = rows.find((r) => r.route === "/turns");
  assert.equal(turns.errors, 1, "status≥400 计入错误");
  assert.equal(rows[0].route, "/turns", "默认按 P95 降序，最慢的路由排最前");
});

test("慢请求 Top N 按耗时降序，来源分布分 web/shell", () => {
  const sandbox = makeSandbox();
  const ledger = sandbox.OwoDiagnosticsLedger;
  const top = ledger.slowest(SEED_RECORDS, 3);
  assert.deepEqual(
    top.map((r) => Number(r.duration_ms)),
    [1500, 900, 800],
  );
  // 沙箱内创建的对象原型与宿主不同，跨 realm 比较走 JSON 归一。
  assert.deepEqual(
    JSON.parse(JSON.stringify(ledger.sourceCounts(SEED_RECORDS))),
    { shell: 3, web: 5 },
  );
  const boot = ledger.lastBootstrap(SEED_RECORDS);
  assert.equal(boot.count, 1);
  assert.equal(boot.last_started_at, "2026-09-18T10:00:01.200Z");
});

test("渲染必须覆盖 §4.6 全部条目：数量、四类计数、慢请求、P50/P95、来源、重启、SSE、导出", () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: {
      state: "restarting",
      errorCode: "core/exited",
      pid: 4242,
      instanceId: "abcdef0123456789",
      logPath: "C:\\Users\\private\\AppData\\Local\\owo\\core.log",
      message: "process exited with Bearer sk-should-not-leak",
    },
    owoInvalidatorState: () => ({ state: "degraded", reconnectAttempts: 3 }),
  });
  const root = makeRoot();
  sandbox.OwoDiagnosticsLedger.render(root, {
    ledger: seedLedger(),
    overview: OVERVIEW,
    core: sandbox.OwoDiagnosticsLedger.readCoreSnapshot(),
    updated_at: "2026-09-18T10:00:09.000Z",
  });
  const html = root.innerHTML;
  for (const must of [
    "最近请求数量",
    "health 探测",
    "auth 引导",
    "events（SSE）",
    "业务请求",
    "慢请求 Top",
    "按路由模板聚合（P50 / P95）",
    "来源（x-owo-client）",
    "core 重启与引导",
    "SSE 事件流",
    "生成脱敏诊断包",
    "下载诊断包文件",
  ]) {
    assert.ok(html.includes(must), `页面缺少 §4.6 条目：${must}`);
  }
  assert.ok(html.includes("1500 ms"), "慢请求表要真的带耗时");
  assert.ok(html.includes("web（界面）") && html.includes("shell（桌面壳）"), "来源要给用户看得懂的口径");
  assert.ok(html.includes("当前连接数"), "SSE 当前连接数必须展示");
});

test("§4.6 禁止回显：本地绝对路径、Bearer、配对秘密、原始 query 都不得出现在页面或导出包", () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: {
      state: "failed",
      errorCode: "storage/not_writable",
      pid: 1,
      instanceId: "zeta-instance-id-0123456789",
      logPath: "D:\\secret-data-root\\logs\\core.log",
      message: "open failed: Authorization: Bearer tok-abc.def-123 at /home/ovo/private",
    },
    owoInvalidatorState: () => null,
  });
  const ledger = sandbox.OwoDiagnosticsLedger;
  const core = ledger.readCoreSnapshot();
  const root = makeRoot();
  ledger.render(root, { ledger: seedLedger(), overview: OVERVIEW, core: core, updated_at: "2026-09-18T10:00:09.000Z" });
  const html = root.innerHTML;
  for (const forbidden of [
    "secret-data-root",
    "core.log",
    "/home/ovo",
    "Bearer",
    "tok-abc",
    "zeta-instance-id-0123456789",
  ]) {
    assert.ok(!html.includes(forbidden), `页面不得出现 ${forbidden}`);
  }
  const bundle = JSON.stringify(ledger.buildExportBundle({ ledger: seedLedger(), overview: OVERVIEW, core: core }));
  for (const forbidden of ["secret-data-root", "core.log", "/home/ovo", "Bearer", "tok-abc", "zeta-instance-id-0123456789"]) {
    assert.ok(!bundle.includes(forbidden), `导出包不得出现 ${forbidden}`);
  }
  assert.equal(JSON.parse(bundle).schema, ledger.EXPORT_SCHEMA);
  assert.ok(ledger.maskLocalPath("C:\\a\\b") === "<本地路径已脱敏>", "盘符路径必须脱敏");
  assert.ok(ledger.maskLocalPath("\\\\nas\\share") === "<本地路径已脱敏>", "UNC 路径必须脱敏");
  assert.ok(ledger.maskLocalPath("core.log") === "core.log", "纯文件名可保留（非路径）");
  assert.ok(ledger.stripQuery("/sessions?limit=999") === "/sessions", "query 一律剥离");
});

test("load() 经 OwoApi 取两个端点并渲染；端点不可达时给出可读提示而非空白", async () => {
  const calls = [];
  const sandbox = makeSandbox({
    OwoApi: {
      get(path) {
        calls.push(path);
        if (path.startsWith("/diagnostics/requests")) return Promise.resolve(seedLedger());
        return Promise.resolve(OVERVIEW);
      },
    },
    __owoCoreDiagnostics: { state: "ready", pid: 7, instanceId: "ready-instance" },
  });
  const root = makeRoot();
  const data = await sandbox.OwoDiagnosticsLedger.load(root, { force: true });
  assert.deepEqual(calls, ["/diagnostics/requests?limit=200", "/metrics/overview"], "台账 = ledger + overview 两端点，且都走 OwoApi（不裸 fetch）");
  assert.equal(data.ledger.total, 900);
  assert.ok(root.innerHTML.includes("最近请求数量"));
  assert.ok(!/fetch\(|XMLHttpRequest/.test(viewSource), "视图内禁止裸 fetch：网络只经 core/api-client.js（§4.8）");

  const broken = makeSandbox({
    OwoApi: {
      get() {
        return Promise.reject(new Error("core 未就绪"));
      },
    },
  });
  const root2 = makeRoot();
  await broken.OwoDiagnosticsLedger.load(root2, { force: true });
  assert.ok(root2.innerHTML.includes("台账端点不可达"), "端点失败必须说明白，禁止静默 loading 骨架");
  assert.ok(root2.innerHTML.includes("刷新台账"), "失败时仍保留重试出口");
  assert.ok(!root2.innerHTML.includes("最近请求数量"), "失败态不得播报请求数量（那是拿不到的事实）");
});

test("骨架态不得播报假数字：0 ≠ 未知（真机误判回归）", () => {
  const sandbox = makeSandbox();
  const root = makeRoot();
  sandbox.OwoDiagnosticsLedger.render(root, { loading: true });
  assert.ok(!/环形容量 0/.test(root.innerHTML), "未取到数据前不得显示环形容量");
  assert.ok(!root.innerHTML.includes("最近请求数量"), "骨架态不给数量结论");
  assert.ok(root.innerHTML.includes("正在读取"), "骨架态必须说明在读取");
  assert.ok(root.innerHTML.includes("刷新台账"), "骨架态仍保留重试出口");
});

test("生成/复制/下载三个出口分工明确：一键生成不得写用户目录", async () => {
  const copied = [];
  let downloads = 0;
  const sandbox = makeSandbox({
    navigator: { clipboard: { writeText: (text) => Promise.resolve(copied.push(text)) } },
    __owoCoreDiagnostics: { state: "ready", logPath: "C:\\Users\\ovo\\core.log", instanceId: "inst-0123456789ab" },
    anchors: { created: 0 },
  });
  sandbox.URL.createObjectURL = () => "blob:stub";
  const realCreate = sandbox.document.createElement;
  sandbox.document.createElement = function (tag) {
    const el = realCreate(tag);
    if (String(tag).toLowerCase() === "a") {
      el.click = () => { downloads += 1; };
    }
    return el;
  };
  const root = makeRoot();
  sandbox.OwoDiagnosticsLedger.render(root, {
    ledger: seedLedger(),
    overview: OVERVIEW,
    core: sandbox.OwoDiagnosticsLedger.readCoreSnapshot(),
    updated_at: "2026-09-18T10:00:09.000Z",
  });
  root.querySelector("#owoLedgerDownload").click();
  assert.equal(downloads, 1, "§4.6「一键导出」= 单点下载即可拿到脱敏包（不要求先生成）");
  root.querySelector("#owoLedgerExport").click();
  assert.equal(downloads, 1, "「生成」只展开，不得再触发文件下载（副作用必须是显式动作）");
  const pre = root.querySelector("#owoLedgerBundle");
  assert.equal(pre.hidden, false, "生成后必须在页面展开脱敏包");
  assert.equal(root.querySelector("#owoLedgerUpdated").textContent.includes("已生成脱敏诊断包"), true);
  root.querySelector("#owoLedgerCopy").click();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(copied.length, 1, "复制走 clipboard.writeText 一次");
  const bundle = JSON.parse(copied[0]);
  assert.equal(bundle.schema, sandbox.OwoDiagnosticsLedger.EXPORT_SCHEMA);
  assert.equal(bundle.core.log_available, true, "只导出「日志可否打开」");
  assert.ok(!JSON.stringify(bundle).includes("Users"), "导出包不得含本地绝对路径");
  assert.equal(bundle.sse.server_active_connections, 2);
  assert.equal(bundle.ledger.buckets.events, 1);
});

test("接线：诊断台账是懒加载面板，首屏不读取诊断数据", () => {
  assert.match(indexHtml, /<script src="views\/diagnostics-ledger\.view\.js"><\/script>/,
    "诊断台账模块必须在应用装配前引入");
  assert.match(indexHtml, /<div id="panelRoot"><\/div>/, "工具面板必须复用统一挂载根节点");
  assert.match(appDomainSource, /"diagnostics-ledger"/, "诊断台账必须进入统一面板目录");
  assert.match(appDomainSource, /function mountPanel\(id, writeHash = true\)/, "面板必须复用统一挂载生命周期");
  assert.match(appDomainSource, /button\.addEventListener\("click", \(\) => openPanelById\(id\)\)/,
    "诊断请求须由用户显式打开面板触发");
  assert.match(viewSource, /mount\(root\)[\s\S]*load\(root, \{ force: true \}\)/,
    "进入面板时才发起诊断读取");
  const hydrate = /const BOOT_HYDRATE_TASKS = \[([\s\S]*?)\];/.exec(appSource);
  assert.ok(hydrate, "首屏水合任务清单应存在");
  assert.ok(!/DiagnosticsLedger|diagnostics\/requests|metrics\/overview/.test(hydrate[1]),
    "诊断数据不得进入首屏水合链路");
  assert.match(viewSource, /loadSequence !== sequence/, "切换或重载后迟到响应不得覆盖当前面板");
});
test("§4.6 重启口径：壳上报 generation/attempt 时用权威计数，不再拿引导次数近似", () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: {
      state: "restarting",
      attempt: 2,
      generation: 3,
      pid: 42,
      instanceId: "restart-instance-0123456789",
      logPath: "D:\\data\\logs\\core.log",
      message: "core exited unexpectedly",
    },
    owoInvalidatorState: () => null,
  });
  const ledger = sandbox.OwoDiagnosticsLedger;
  const core = ledger.readCoreSnapshot();
  assert.equal(core.generation, 3, "壳快照必须把 generation 透出来（api-client 归一后）");
  assert.equal(core.attempt, 2);
  const root = makeRoot();
  ledger.render(root, { ledger: seedLedger(), overview: OVERVIEW, core: core, updated_at: "2026-09-18T10:00:09.000Z" });
  const html = root.innerHTML;
  assert.ok(html.includes("最近一次 core 重启"), "§4.6 条目：重启事实必须在页面可见");
  assert.ok(html.includes("第 3 代"), "代际必须真渲染出来");
  assert.ok(html.includes("当代自动重启 2 次"), "同代崩溃自动重启与换代是两个事实，必须分开");
  assert.ok(!html.includes("以 /auth/token 引导次数近似"), "有权威计数时不得再挂近似口径");
  assert.ok(html.includes("取自壳侧权威快照"), "口径来源必须写明，避免与 auth 卡片互证");
  const bundle = ledger.buildExportBundle({ ledger: seedLedger(), overview: OVERVIEW, core: core });
  assert.equal(bundle.core.shell_generation, 3);
  assert.equal(bundle.core.shell_attempt, 2);
});

test("§4.6 旧壳未上报代际：显式说「壳未上报」并保留近似口径（不得用 0 冒充）", () => {
  const sandbox = makeSandbox({
    __owoCoreDiagnostics: { state: "ready", pid: 8, instanceId: "old-shell-instance-01" },
  });
  const ledger = sandbox.OwoDiagnosticsLedger;
  const core = ledger.readCoreSnapshot();
  assert.equal(core.generation, undefined, "旧壳没有该字段 → 不得凭空补 0");
  const root = makeRoot();
  ledger.render(root, { ledger: seedLedger(), overview: OVERVIEW, core: core, updated_at: "2026-09-18T10:00:09.000Z" });
  assert.ok(root.innerHTML.includes("壳未上报"), "缺失必须显式呈现为未知");
  assert.ok(root.innerHTML.includes("以 /auth/token 引导次数近似"), "无权威计数时才允许近似口径");
});

test("§4.6 auth 卡片口径修正：壳注入 token 后引导数为 0 不等于未引导", () => {
  const sandbox = makeSandbox({ __owoCoreDiagnostics: { state: "ready", generation: 1 } });
  const noAuth = {
    total: 3,
    returned: 2,
    cap: 512,
    aggregates: { health: 2, auth_token: 0, business: 0 },
    records: [
      { method: "GET", route_template: "/health", started_at: "2026-09-18T10:00:00.100Z", duration_ms: 3, status: 200, source: "shell" },
      { method: "GET", route_template: "/health", started_at: "2026-09-18T10:00:01.100Z", duration_ms: 4, status: 200, source: "shell" },
    ],
  };
  const root = makeRoot();
  sandbox.OwoDiagnosticsLedger.render(root, {
    ledger: noAuth,
    overview: OVERVIEW,
    core: sandbox.OwoDiagnosticsLedger.readCoreSnapshot(),
    updated_at: "2026-09-18T10:00:09.000Z",
  });
  assert.ok(root.innerHTML.includes("壳注入 token 时为 0，不等于未引导"), "§4 冷启动不再请求 /auth/token，提示语必须同步");
  assert.ok(!root.innerHTML.includes("每次 core 重启恰好一次"), "旧口径（每重启必有一次引导）已不成立");
  assert.ok(root.innerHTML.includes("窗口内未见 /auth/token"), "无引导记录时说明事实而不是显示 0 就完事");
});

