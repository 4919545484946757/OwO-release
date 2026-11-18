import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";

const read = (file) => readFileSync(new URL("../" + file, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const app = read("app.js");
const domain = read("app-domain.js");
const css = read("frontend-visual-system.css");

test("owo 品牌静态展示，工作区入口统一走 composer 托盘", () => {
  assert.match(html, /id="sidebarWorkspaceBtn"[^>]*class="codex-workspace-switcher codex-workspace-brand"[^>]*role="img"/);
  assert.doesNotMatch(html, /id="sidebarWorkspaceBtn"[^>]*aria-haspopup/);
  assert.doesNotMatch(html, /id="sidebarWorkspaceBtn"[^>]*aria-expanded/);
  assert.doesNotMatch(html, /id="sidebarWorkspaceSelect"/);
  assert.doesNotMatch(app, /\$\("sidebarWorkspaceSelect"\)\.addEventListener\("click"/);
  assert.doesNotMatch(app, /\$\("sidebarWorkspaceBtn"\)\.addEventListener\("click"/);
  assert.doesNotMatch(app, /sidebarBrandBtn"\)\?\.addEventListener\("click"/);
  assert.match(app, /openWorkspaceMenu\(\$\("trayProjectBtn"\)\)/);
});

test("没有工作区时不从历史会话猜一个目录，空状态引导到顶部入口", () => {
  assert.match(domain, /if \(!currentWorkspace\) return;/);
  assert.match(domain, /请先在顶部文件夹入口选择项目工作区/);
  assert.match(app, /function clearWorkspaceSelection\(\)/);
  assert.match(app, /localStorage\.removeItem\("owo\.workspace"\)/);
  assert.match(app, /data-project-action="clear"/);
  assert.match(app, /在顶部选择工作区/);
});

test("置顶会话单独成区，不再只依赖红色贴图纸", () => {
  assert.match(domain, /codex-pinned-area/);
  assert.match(domain, /codex-pinned-header/);
  assert.match(domain, /codex-pinned-list/);
  assert.doesNotMatch(domain, /badges\.push\("📌"\)/);
  assert.match(domain, /options\.pinnedArea[\s\S]{0,120}?group\.roots\.filter\(\(session\) => !session\.pinned\)[\s\S]{0,80}?: group\.roots;/);
  assert.match(domain, /项目区保留全部任务/);
  assert.match(css, /#sessionList \.codex-pinned-area \{/);
  assert.match(css, /#sessionList \.codex-pinned-list > li \{/);
});

test("项目和单个对话可分别置顶，置顶顺序只由置顶操作决定", () => {
  for (const marker of [
    "owoReadPinnedProjects", "owoSetProjectPinned", "owoReadPinnedSessionOrder",
    "owoRememberPinnedSession", "codex-project-pin", "owo-session-pin",
  ]) {
    assert.match(domain, new RegExp(marker));
  }
  assert.match(domain, /pinnedRoots[\s\S]*?\.sort\(/);
  assert.match(css, /#sessionList \.codex-project-pin/);
  assert.match(css, /#sessionList \.owo-session-pin/);
  // 取消置顶后从本地置顶顺序移除，下一次刷新回到项目/最近列表。
  const store = new Map();
  const localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => store.set(key, String(value)),
  };
  const source = domain.slice(
    domain.indexOf("const OWO_PINNED_PROJECTS_KEY"),
    domain.indexOf("function owoPinIcon"),
  );
  const helpers = runInNewContext(
    source + "\n({ owoReadPinnedProjects, owoSetProjectPinned, owoReadPinnedSessionOrder, owoRememberPinnedSession });",
    {
      localStorage,
      workspacePathKey: (p) => String(p || "").replace(/[\\/]+$/, "").replace(/\//g, "\\").toLowerCase(),
    },
  );
  helpers.owoSetProjectPinned("D:\\A", true);
  helpers.owoSetProjectPinned("D:\\B", true);
  assert.deepEqual(Array.from(helpers.owoReadPinnedProjects()), ["D:\\B", "D:\\A"]);
  helpers.owoSetProjectPinned("D:\\A", false);
  assert.deepEqual(Array.from(helpers.owoReadPinnedProjects()), ["D:\\B"]);
  helpers.owoRememberPinnedSession("s1", true);
  helpers.owoRememberPinnedSession("s2", true);
  helpers.owoRememberPinnedSession("s1", false);
  assert.deepEqual(Array.from(helpers.owoReadPinnedSessionOrder()), ["s2"]);
});

test("悬浮直接显示内容说明；推理档位与模型合并为同一控件", () => {
  // 悬浮不再弹出需要二次点击的菜单，而是约 90ms 直接显示深色说明气泡。
  assert.doesNotMatch(app, /bindHoverMenu/);
  assert.doesNotMatch(app, /dataset\.hoverAuto/);
  assert.match(app, /window\.OwoTooltip = \{/);
  assert.match(app, /\}, 90\);/);
  assert.match(app, /悬浮只负责「直接显示内容说明」/);
  // 独立「推理」chip 收起，思考强度仍留在模型菜单里。
  assert.match(html, /id="effortChip"[^>]*hidden/);
  assert.match(css, /#chatForm#chatForm #effortChip \{\s*display: none !important;/);
  assert.match(app, /composer-model-effort/);
});

test("侧栏文字比图标更深一级，分组与会话标题可读性更高", () => {
  assert.match(css, /#sessionList \.codex-session-group \{\s*color: var\(--text\) !important;/);
  assert.match(css, /#sessionList \.codex-session-group \.codex-group-label::before \{\s*background: var\(--text-3\) !important;/);
  assert.match(css, /#sessionList \.owo-session-title strong \{\s*color: var\(--text\) !important;/);
  assert.match(css, /#sidebar \.workbench-shortcuts > button \.nav-icon \{\s*color: var\(--text-3\) !important;/);
});
