import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const read = (relative) => readFileSync(join(here, relative), "utf8");

test("§5.1.4 用户界面不出现 API 路径（团队/产物页）", () => {
  for (const file of ["../panels/workswarm.panel.js", "../panels/action-center.panel.js"]) {
    const source = read(file);
    const hintSpans = source.match(/<span class="hint">[^<]*<\/span>/g) || [];
    for (const span of hintSpans) {
      assert.doesNotMatch(span, /(GET|POST|PUT|DELETE) \//, `${file} 的 hint 文案不得包含 API 路径：${span}`);
    }
    const titleAttrs = source.match(/title="[^"]*"/g) || [];
    for (const attr of titleAttrs) {
      assert.doesNotMatch(attr, /(GET|POST|PUT|DELETE) \//, `${file} 的 title 提示不得包含 API 路径：${attr}`);
    }
  }
  const actionCenter = read("../panels/action-center.panel.js");
  assert.ok(!actionCenter.includes("正式 Inbox（/human/inbox）"), "产物页不得用 API 路径当标题");
  const workswarm = read("../panels/workswarm.panel.js");
  assert.ok(!workswarm.includes("团队列表 GET /teams"), "错误文案不得包含 API 路径");
});

test("服务连接：启动快速探测、离线退避和重连入口", () => {
  const app = read("../app.js");
  const index = read("../index.html");
  assert.match(app, /const serviceWatch = \(\(\) =>/);
  assert.match(app, /const STARTUP_DEADLINE_MS = 10000/);
  assert.match(app, /const RETRY_BASE_MS = 3000/);
  assert.match(app, /const RETRY_MAX_MS = 15000/);
  assert.match(app, /function showBanner\(\)/);
  assert.match(app, /function notifyOffline\(\)/);
  assert.match(app, /await serviceWatch\.start\(\)/);
  assert.match(index, /id="serviceBannerRetry"/);
});
test("OpenAPI 文档从同源工作台入口打开", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  assert.match(index, /<a href="openapi\.json" target="_blank" rel="noopener">OpenAPI 3\.1<\/a>/);
  assert.match(app, /window\.open\("openapi\.json", "_blank", "noopener"\)/);
  assert.ok(!index.includes('id="openapiLink"'), "当前页面没有可动态拼接 API 地址的旧链接节点");
});
test("§4.6/§4.8 首次配置引导：NoWorkspace 分流 + 提供商选择", () => {
  const app = read("../app.js");
  assert.match(app, /async function needsSetup\(\)/, "必须提供 needsSetup 判定");
  // 首启配置独立于聊天路由：使用 #setupRoot，完成后重置壳连接并重载，
  // 以确保工作区/provider 快照重新取样；组件错误必须显示本地可操作退路。
  assert.match(app, /if \(await needsSetup\(\)\) \{\s*renderSetupGuide\(\);\s*return;\s*\}/,
    "boot 必须在 health/hydration 前分流首启配置");
  assert.match(app, /window\.renderOwoSetupGuide\(root, window\.__owoSetupDiagnostics \|\| \{\},/,
    "配置视图必须接收壳诊断和设置根节点");
  assert.match(app, /window\.OwoApi\.resetCoreConnection\(\);\s*window\.location\.reload\(\);/,
    "工作区/provider 更新后必须重取壳连接快照");
  assert.match(app, /showFallback\("配置页面组件未加载/);
  assert.match(app, /showFallback\("配置页面组件启动失败/);
  const guide = read("../views/setup-guide.view.js");
  assert.match(guide, /get_workspace/, "引导读取当前工作区");
  assert.match(guide, /set_workspace/, "引导可设置工作区");
  assert.match(guide, /get_provider_status/, "引导读取提供商状态");
  assert.match(guide, /set_provider/, "引导可保存提供商选择");
  assert.ok(!guide.includes("OPENAI_API_KEY") || guide.includes("不显示内容"), "密钥内容永不回前端（仅布尔/端点/模型名）");
  const index = read("../index.html");
  assert.match(index, /views\/setup-guide\.view\.js/, "引导视图必须加载");
  const css = read("../style.css");
  assert.match(css, /\.setup-card/, "引导卡片样式必须存在");
});

test("响应式工作台：窄窗口隐藏检查器并将会话侧栏改为抽屉", () => {
  const index = read("../index.html");
  const css = read("../style.css");
  const app = read("../app.js");
  assert.match(index, /id="mobileSidebarToggle"[^>]*aria-controls="sidebar"/);
  assert.match(index, /id="mobileSidebarBackdrop"/);
  assert.match(app, /matchMedia\("\(max-width: 860px\)"\)/);
  assert.match(app, /mobileSidebarBackdrop\.addEventListener\("click"/);
  assert.match(app, /!menuWasOpen && !modalWasOpen && document\.body\.classList\.contains\("mobile-sidebar-open"\)[\s\S]*?setMobileSidebarOpen\(false\);[\s\S]*?mobileSidebarToggle\.focus\(\)/,
    "Escape closes the drawer and restores focus when no overlay is open");
  assert.match(css, /@media \(max-width: 1180px\)[\s\S]*?#right, #rightResize[\s\S]*?display: none !important/);
  assert.match(css, /@media \(max-width: 860px\)[\s\S]*?grid-template-columns: minmax\(0, 1fr\)/);
  assert.match(css, /@media \(max-width: 700px\)[\s\S]*?mobile-sidebar-open[^\n]*#sidebar/);
});
test("§5.1.3 项目页：深度改三段选项，原始数值进高级折叠", () => {
  const panel = read("../panels/project-launcher.panel.js");
  assert.match(panel, /data-pl-depth="2"/);
  assert.match(panel, /data-pl-depth="4"/);
  assert.match(panel, /data-pl-depth="8"/);
  assert.match(panel, /快速 · 2 层/);
  assert.match(panel, /标准 · 4 层/);
  assert.match(panel, /深入 · 8 层/);
  assert.match(panel, /<details class="owo-pl-advanced"><summary>自定义深度（1–8）<\/summary>/, "原始深度输入必须收进高级折叠");
  assert.ok(!panel.includes("TeamRun 的 Agent Worker 将以该目录为工作区"), "内部执行参数不得出现在任务创建流程");
  assert.match(panel, /允许写入路径/, "受控写入约束标签必须保留");
});

test("§5.5 主审批卡：展示服务端解释与脱敏参数，并显式提交授权范围", () => {
  const domain = read("../app-domain.js");
  assert.match(domain, /function describeApproval\(payload\)/);
  assert.match(domain, /redacted_args|redactedArgs/);
  assert.match(domain, /explain\.action/);
  assert.match(domain, /explain\.undoable/);
  assert.match(domain, /always_readonly/);
  assert.match(domain, /工作区长期/);
  assert.match(domain, /scope: scope \|\| "once"/);
  assert.ok(!domain.includes("payload.args"), "审批卡不得读取原始参数");
  assert.ok(!domain.includes("JSON.stringify(item.args"), "审批卡不得序列化原始参数");
  const app = read("../app.js");
  assert.match(app, /querySelector\("\.approval-scope"\)/);
  assert.match(read("../style.css"), /\.approval-actions/);
});

test("§12-13 约束表单：自动化与 MCP 按类型显示有效字段", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  const domain = read("../app-domain.js");
  const automations = read("../panels/automations.panel.js");
  assert.match(automations, /id="owo-aut-interval" type="number" min="1" step="1"/);
  assert.match(automations, /id="owo-aut-daily" type="time"/);
  assert.match(automations, /id="owo-aut-oneshot" type="datetime-local"/);
  assert.match(automations, /function syncScheduleFields\(\)/);
  assert.match(automations, /function buildSchedule\(kind, value, offsetMinutes\)/);
  assert.match(index, /data-mcp-group="stdio"/);
  assert.match(index, /data-mcp-group="http" hidden/);
  assert.match(index, /id="mcpUrl" type="url"[^>]*disabled/);
  assert.match(domain, /function syncMcpFields\(\)/);
  assert.match(domain, /command: transport === "stdio" \? command : ""/);
  assert.match(domain, /url: transport === "http" && url \? url : null/);
  assert.match(app, /mcpTransport.*addEventListener\("change", syncMcpFields\)/);
  assert.match(app, /syncMcpFields\(\);/);
  assert.match(index, /id="providerBaseUrl"/);
  assert.match(index, /id="providerApiKey"(?: data-core-action)? type="password"/);
  assert.match(index, /id="providerSaveBtn"/);
});
// §5.1.5 模块边界守卫：同一函数不得同时在 app.js 和 app-domain.js 定义，
// 防止把已拆出的实现复制回巨型文件（经典脚本下同名声明会直接冲突或静默覆盖）。
test("模块边界守卫：app.js 与 app-domain.js 函数定义零交集", () => {
  const fnNames = (src) => new Set([...src.matchAll(/^\s*(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/gm)].map((m) => m[1]));
  const inApp = fnNames(read("../app.js"));
  const inDomain = fnNames(read("../app-domain.js"));
  const dup = [...inApp].filter((n) => inDomain.has(n));
  assert.deepEqual(dup, [], "重复定义的函数：" + dup.join(", "));
});

test("工具和设置导航：按功能组进入工具页、按分类切换设置页", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  const css = read("../style.css");
  const navTabs = index.match(/class="settings-nav-item[^"]*" data-settings-tab=/g) || [];
  const toolGroups = index.match(/data-group="(workspace|intelligence|automation|system)"/g) || [];
  assert.ok(navTabs.length >= 8, "设置页保留分类导航");
  assert.ok(toolGroups.length >= 8, "工具视图按类别组织内容");
  assert.match(index, /id="codexToolsEntry"/);
  assert.match(index, /id="codexSettingsEntry"/);
  assert.match(app, /function setSettingsTab\(name\)/);
  assert.match(app, /bindToolsEntry\("codexToolsEntry"\)/);
  assert.match(app, /setSettingsPageVisible\(true\)/);
  assert.match(css, /\.settings-layout/);
  assert.ok(!index.includes('id="toolsPanel"'), "工具页已由工作台视图状态机承载");
});
test("工具分组切换将专属滚动容器复位到分组起点", () => {
  const app = read("../app.js");
  const navLoop = 'for (const button of document.querySelectorAll("[data-jump]")) {';
  const clickStart = app.indexOf('button.addEventListener("click", () => {', app.indexOf("function syncToolsJump"));
  const handlerStart = app.lastIndexOf(navLoop, clickStart);
  const handlerEnd = app.indexOf("function refreshPluginSkillOverview()", handlerStart);
  assert.ok(handlerStart >= 0 && handlerEnd > handlerStart, "工具分组点击处理器必须存在");
  const handler = app.slice(handlerStart, handlerEnd);
  assert.match(handler, /sidebar\.scrollTop\s*=\s*0/, "切换分组必须重置 #sidebar 自身的滚动位置");
  assert.doesNotMatch(handler, /sidebar\.scrollIntoView/, "外层滚动不能替代工具面板滚动容器");
});

test("R10 侧栏：复杂表单受工具/设置视图状态门控，不常驻会话导航", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  const css = read("../style.css");
  const sidebarStart = index.indexOf('<aside id="sidebar">');
  const toolContentStart = index.indexOf('<div class="group-title tool-page-head"');
  assert.ok(sidebarStart >= 0 && toolContentStart > sidebarStart, "侧栏与工具页结构必须存在");
  const primaryShell = index.slice(sidebarStart, toolContentStart);
  for (const id of ["providerBaseUrl", "mcpForm", "evalRunBtn"]) {
    assert.ok(!primaryShell.includes('id="' + id + '"'), id + " 不得进入常驻会话导航");
  }
  assert.match(index, /id="codexToolsEntry"/);
  assert.match(index, /id="codexSettingsEntry"/);
  assert.match(index, /<section class="settings-section" data-group="settings">/);
  assert.match(css, /#sidebar > section \{ display: none; \}/);
  assert.match(css, /body\.tools-open #sidebar > section/);
  assert.match(css, /body\.settings-open #sidebar > section\.settings-section/);
  assert.match(app, /bindToolsEntry\("codexToolsEntry"\)/);
  assert.match(app, /\$\("codexSettingsEntry"\)\.addEventListener\("click"/);
});
test("R12 会话卡片：低频操作收进 ⋯ 弹层，卡片上不再堆按钮", () => {
  const domain = read("../app-domain.js");
  // 卡片主体点一下就是"打开会话"，不该再有一个常显的"继续"按钮。
  assert.ok(!/data-act="open"/.test(domain), "会话卡片不得再有常显的「继续」按钮");
  assert.match(domain, /class="owo-session-more"[\s\S]{0,120}data-act="menu"/, "必须有 ⋯ 菜单触发器");
  assert.match(domain, /class="owo-session-menu"[\s\S]*?role="menu"/, "必须有 role=menu 的弹层");
  for (const act of ["rename", "pin", "archive", "fork", "rewind", "redo"]) {
    assert.ok(domain.includes(`data-act="${act}"`), `弹层里必须有 ${act} 操作`);
  }
  // 弹层默认隐藏 + 点空白/Esc 关闭（否则会"粘"在界面上）。
  assert.match(domain, /class="owo-session-menu" role="menu" hidden/, "弹层必须默认 hidden");
  assert.ok(domain.includes('event.key !== "Escape"'), "Esc 必须能关掉弹层");
  for (const key of ["ArrowDown", "ArrowUp", "Home", "End"]) {
    assert.ok(domain.includes(`"${key}"`), `菜单焦点导航必须支持 ${key}`);
  }
  assert.match(domain, /trigger\.focus\(\)/, "关闭菜单后焦点应返回触发器");
  const css = read("../style.css");
  // 窄侧栏里中文被压成逐字竖排就是缺这几条：关键元素一律 nowrap + 省略。
  assert.match(css, /\.owo-session-menu button \{[\s\S]*?white-space: nowrap;/, "菜单项不得换行断字");
  assert.match(css, /\.owo-session-title \{[\s\S]*?text-overflow: ellipsis;/, "标题必须省略号截断而非换行");
});

test("R13 模型接入：端点、密钥和模型选择从活动设置表单保存", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  assert.match(index, /id="settingsModel"/);
  assert.match(index, /id="providerBaseUrl"(?: data-core-action)? type="text"/);
  assert.match(index, /id="providerApiKey"(?: data-core-action)? type="password"/);
  assert.match(index, /id="providerPreset"/);
  assert.match(index, /id="customModelAddBtn"/);
  assert.match(app, /async function saveProvider\(\)/);
  assert.match(app, /settings\.model = \$\("settingsModel"\)\.value/);
  assert.match(app, /base_url: baseUrl/);
  assert.match(app, /\.\.\.\(apiKey \? \{ api_key: apiKey \} : \{\}\)/);
  assert.match(app, /async function saveSettings\(/);
  assert.ok(!index.includes('id="settingsContextWindow"'), "旧设置模型面板字段不属于当前活动页面");
  assert.match(index, /id="modelOutputModel"/);
  assert.match(index, /id="modelOutputLimit"(?: data-core-action)? type="number" min="1" max="1000000"/);
  assert.ok(index.indexOf('<script src="core/model-output-budget.js"></script>') < index.indexOf('<script src="app.js"></script>'));
  assert.match(app, /window\.OwoModelOutputBudget\.MAX/);
  assert.match(index, /id="modelOutputSaveBtn"/);
  assert.match(index, /id="modelOutputResetBtn"/);
  assert.match(app, /function refreshModelOutputSettings\(\)/);
  assert.match(app, /function saveModelOutputSettings\(\)/);
  assert.match(app, /model_output_tokens: outputTokens/);
});
test("R10 模型操作：设置页配置默认模型，输入区可快速切换可用模型", () => {
  const index = read("../index.html");
  const app = read("../app.js");
  assert.match(index, /data-settings-tab="models"/);
  assert.match(index, /id="settingsModel"/);
  assert.match(index, /id="modelChip"/);
  assert.match(app, /openComposerMenu\(\$\("modelChip"\)/);
  assert.match(app, /function saveSettings\(/);
  assert.match(app, /\$\("settingsModel"\)\.addEventListener\("change", \(\) => saveSettings\(\)\)/);
  assert.match(index, /id="customModelAddBtn"/);
  assert.match(app, /custom_models/);
});