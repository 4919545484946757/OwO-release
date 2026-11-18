import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { runInNewContext } from "node:vm";
const read = file => readFileSync(new URL("../" + file, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const BASELINE_HASH = "34794e3d983aa907ed4ee515a3d732ccc319250598cbf896569a3305bb4ea2bf";
const CURRENT_PREFIX_HASH = "5cfd5691d8c1b492c4529aebbb1db38bda04f8fe9ccd83a0affda27e306fd5fb";
// 2026-11-11 对齐 ChatGPT/Codex 参考的增量：标题栏前进/后退、输入框下上下文托盘、
// codex-align.css 精修层。主结构（侧栏/欢迎区/输入框 id 与 data-*）不变。
const ALIGN_PREFIX_HASH = "76904da89f3a20eedbaf3a2e92b1bb478e10baa794a5216585bbfb202b187e70";
// 2026-11-13 品牌清理：单一工作区 owo 横标、紧凑模型选择器，保留原有按钮与工作区状态节点。
// 2026-11-13 提示词优化：输入框工具栏新增闪电按钮，子代理任务框加同款角标；主结构不变。
// 2026-11-13 侧栏：owo 静态化、顶部工作区下拉、独立置顶区；主结构不变。
// 2026-11-13 增强提示词：✨ 改为点击直接回填，工具栏去独立推理 chip、改 data-tip 悬浮说明；主结构不变。
const BRAND_PREFIX_HASH = "bb42cc716c4bafbddbd17966458103cee38005ea358e47c7086b1ccf1a40a1b5";
// 2026-11-14 提示词增强：设置「配置」页新增「提示词增强模型」一行
// （跟随当前模型 / 指定云端 / 指定本地）；主结构（侧栏/欢迎区/输入框 id 与 data-*）不变。
const ENHANCE_MODEL_PREFIX_HASH = "105e9f105b09e7b7a8adf8e6fd71755bb2fa81efa1b6ae9c1e1ba9c0a202c75a";
// 2026-11-15 composer 托盘：运行环境下拉（本地 / 云端 / 工作树 / SSH / WSL），
// 项目选择改为搜索 + 新建 + 项目列表；主结构（侧栏/欢迎区/输入框 id 与 data-*）不变。
const ENV_TRAY_PREFIX_HASH = "fe37c03c84cb3cf5957630b119601138960fd08add7ea86717f2dede48db7d14";
// 2026-11-15 侧栏：去掉顶部「当前工作区」下拉，工作区入口统一走 composer 托盘；
// 置顶区与项目区左对齐。主结构（品牌/新建对话/会话列表）不变。
const CODE_SIDEBAR_PREFIX_HASH = "ff928b316582c870e82a8c3d1c067a20c9056d6aaf854af60547458207dac918";
// 这些哈希都对应同一套冻结结构/样式：后者只多了 2026-11-02 的 logo 与
// UI 精修增量（单独 CSS 文件、不改主结构）。哈希是回退保险，不是禁止增量。
const ALLOWED_PREFIX_HASHES = new Set([
  BASELINE_HASH,
  CURRENT_PREFIX_HASH,
  ALIGN_PREFIX_HASH,
  BRAND_PREFIX_HASH,
  ENHANCE_MODEL_PREFIX_HASH,
  ENV_TRAY_PREFIX_HASH,
  CODE_SIDEBAR_PREFIX_HASH,
]);
test("welcome, rail, top bar and composer keep the a01b1bf1 visual baseline", () => {
  const html = read("index.html");
  const rawPrefix = html.slice(0, html.indexOf('  <aside id="right"'));
  const prefix = rawPrefix.replace('  <link rel="stylesheet" href="task-details.css?v=20261101-restore">\n', '');
  const hash = createHash("sha256").update(prefix).digest("hex");
  assert.ok(ALLOWED_PREFIX_HASHES.has(hash), `unexpected frozen home prefix hash: ${hash}`);
  assert.doesNotMatch(prefix, /agent-workspace\.css|task-navigation|composerOptions|taskListHeading/);
  if (hash === CURRENT_PREFIX_HASH || hash === ALIGN_PREFIX_HASH) {
    assert.match(rawPrefix, /brand-logo\.css/);
    assert.match(rawPrefix, /ui-refine\.css/);
    assert.equal(rawPrefix.includes('src="assets/owo-logo-64.png?v=20261102-transparent"'), false);
  }
  if (hash === ALIGN_PREFIX_HASH || hash === BRAND_PREFIX_HASH) {
    assert.match(rawPrefix, /codex-align\.css/);
    assert.match(rawPrefix, /id="navBackBtn"/);
    assert.match(rawPrefix, /id="navForwardBtn"/);
    assert.match(rawPrefix, /id="composerContextTray"/);
  }
});
test("retained project filters classify real running and waiting tasks outside the welcome page", () => {
  const root = {};
  runInNewContext(read("core/task-workbench.js"), root);
  runInNewContext(read("panels/home.panel.js"), root);
  const filter = root.OwoPanels.home._test.filterSessions;
  const sessions = [{id:"run",facts:{outcome:"running"}},{id:"wait"},{id:"history"},{id:"chat",mode:"chat"}];
  const facts = {approvals:[{sessionId:"wait"}]};
  assert.equal(filter(sessions,"",facts), sessions);
  assert.deepEqual(Array.from(filter(sessions,"running",facts), s=>s.id), ["run"]);
  assert.deepEqual(Array.from(filter(sessions,"waiting",facts), s=>s.id), ["wait"]);
  assert.deepEqual(Array.from(filter(sessions,"history",facts), s=>s.id), ["history"]);
  assert.deepEqual(Array.from(filter(sessions,"chat",facts), s=>s.id), ["chat"]);
});

test("project filter changes neither source sessions nor the sidebar and is inert after leaving the panel", async () => {
  const root = { OwoSessionFacts:{fromDetail:()=>({outcome:"completed"})} };
  runInNewContext(read("core/task-workbench.js"), root);
  runInNewContext(read("panels/home.panel.js"), root);
  const panel = root.OwoPanels.home;
  const sessions = [{id:"task",title:"Task source"},{id:"chat",title:"Chat source",mode:"chat"}];
  const host={innerHTML:"",scrollTop:0,setAttribute(){},querySelectorAll:()=>[],contains:()=>false};
  let requests=0;
  await panel.mount(host,{esc:String,snapshot:()=>({sessionsKnown:true,sessions}),get:async path=>{
    requests++; return path==="/teams"?{teams:[]}:path==="/human/inbox"?{items:[]}:{};
  }});
  assert.match(host.innerHTML,/Task source/); assert.match(host.innerHTML,/Chat source/);
  const beforeRequests=requests, change=host.onchange;
  change({target:{matches:()=>true,value:"chat"}});
  assert.doesNotMatch(host.innerHTML,/Task source/); assert.match(host.innerHTML,/Chat source/);
  assert.equal(requests,beforeRequests); assert.equal(sessions.length,2);
  const filtered=host.innerHTML; panel.dispose();
  change({target:{matches:()=>true,value:""}});
  assert.equal(host.innerHTML,filtered); assert.equal(host.onchange,null);
});
