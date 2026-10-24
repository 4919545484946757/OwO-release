import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, "..");
const source = readFileSync(join(web, "core", "session-mode.js"), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(source, sandbox);
const mode = sandbox.window.OwoSessionMode;

test("模式解析与后端线格式一致", () => {
  assert.equal(mode.normalize("chat"), "chat");
  assert.equal(mode.normalize("CHAT"), "chat");
  assert.equal(mode.normalize("conversation"), "chat");
  assert.equal(mode.normalize("agent"), "agent");
  assert.equal(mode.normalize("default"), "agent");
  assert.equal(mode.normalize(""), "agent");
  assert.equal(mode.normalize(undefined), "agent");
  // 未知值不冒充合法值：parse 返回空串，normalize 才回退 agent。
  assert.equal(mode.parse("telepathy"), "");
  assert.equal(mode.isChat("chat"), true);
  assert.equal(mode.isChat("agent"), false);
});

test("纯对话模式必须给出「不碰本机文件」的可见提示", () => {
  assert.match(mode.notice("chat"), /不会读取或修改本机文件/);
  assert.equal(mode.notice("agent"), "");
  assert.match(mode.describe("chat").description, /不读写本地文件/);
  assert.match(mode.describe("agent").description, /读写工作区/);
  // 跨 realm 数组原型不同，deepEqual 会报"结构相同但非同一原型"，逐项断言。
  const options = mode.options();
  assert.equal(options.length, 2);
  assert.equal(options[0].id, "agent");
  assert.equal(options[1].id, "chat");
});

test("输入区模式入口与切换调用已接线", () => {
  const html = readFileSync(join(web, "index.html"), "utf8");
  assert.match(html, /id="modeChip"/, "缺少模式 chip");
  assert.match(html, /id="modeNotice"/, "缺少纯对话提示条");
  assert.match(html, /<script src="core\/session-mode.js"><\/script>/, "未注册脚本");

  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /function openModeMenu\(\)/, "缺少菜单函数");
  assert.match(app, /function setSessionMode\(mode\)/, "缺少切换函数");
  // 关键：必须先写服务端再改本地显示，失败时不能"看起来切了"。
  assert.match(
    app,
    /await api\(`\/session\/\$\{state\.sessionId\}\/mode`[\s\S]{0,120}state\.sessionMode = target/,
    "切换模式必须服务端先行",
  );
  assert.match(app, /sessionMode: "chat",/, "缺少默认对话模式");
  assert.match(html, /id="modeSwitch" class="mode-switch"/, "缺少顶部模式切换");
  // 偏好只影响新建会话：不得覆盖已有会话从服务端读回的模式。
  assert.match(app, /localStorage\.setItem\(MODE_PREF_KEY, mode\)/);
  // 命令面板可直接新建纯对话 / Agent 会话，且不改动当前会话模式。
  assert.match(app, /async function newSessionWithMode\(mode\)/);
  assert.match(app, /run: \(\) => newSessionWithMode\("chat"\)/);
  assert.match(app, /run: \(\) => newSessionWithMode\("agent"\)/);
  assert.match(app, /restoreSessionModePreference\(\);/);
  // 切会话先回显缓存，再以服务端为准：避免 chip 慢半拍。
  assert.match(app, /function restoreCachedSessionMeta/);
  assert.match(app, /cacheSessionMeta\(state\.sessionId, \{ mode: target \}\)/);
  const domain = readFileSync(join(web, "app-domain.js"), "utf8");
  // 切回会话要还原阅读位置，不能被甩到底部。
  assert.match(app, /sessionScroll: new Map\(\)/);
  assert.match(app, /function restoreSessionScroll\(sessionId\)/);
  assert.match(app, /state\.autoScroll = distance <= 48;/);
  assert.match(domain, /restoreSessionScroll\(id\)/);
  assert.match(domain, /restoreCachedSessionMeta\(id\)/);
  // 侧栏要能一眼区分纯对话会话。
  // 助手回答的整条复制入口（悬停操作）。
  assert.match(app, /aria-label", "复制回答"/);
  assert.match(app, /await copyText\(String\(text \|\| ""\)\)/);
  assert.match(domain, /class="mode-badge"/);
  assert.match(domain, /对话模式：不读写本地文件/);


  assert.match(domain, /state\.sessionMode = window\.OwoSessionMode/, "切会话未同步模式");
  // 参数已跨行（新增自定义提示词），逐项断言更稳。
  assert.match(domain, /buildCreateSessionRequest\(\s*workspace,/, "新建会话未带模式");
  assert.match(domain, /state\.pendingModelOverride,\s*state\.sessionMode,/, "新建会话未带模式");

  const routing = readFileSync(join(web, "core", "model-routing.js"), "utf8");
  assert.match(routing, /if \(normalizedMode === "chat"\) request\.mode = "chat";/, "建会话未带 chat");
});

test("纯对话模式不会假装禁用媒体入口", () => {
  // 媒体入口属于本地工具，纯对话下模型拿不到；前端只提示，不做假禁用。
  const app = readFileSync(join(web, "app.js"), "utf8");
  assert.match(app, /is-chat-mode/, "缺少纯对话视觉标记");
  assert.match(app, /classList\.toggle\("is-unavailable"/, "媒体入口缺少不可用态");
  // 输入框提示语跟随模式（问答 vs 执行）。
  assert.match(app, /问点什么…/);
  assert.match(app, /promptEl\.placeholder = mode === "chat"/);
  const css = readFileSync(join(web, "experience.css"), "utf8");
  assert.match(css, /\.mode-notice \{/, "缺少提示条样式");
  assert.match(css, /\.codex-model-chip\.is-chat-mode \{/, "缺少 chip 样式");
});
