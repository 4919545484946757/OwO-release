// 2026-10-31 桌宠 / 托盘 / 模型菜单口径契约。
//
// 用户要求：托盘只保留「显示桌面 / 退出」；桌宠置顶、可聊天、拖动不再"卡住"、
// 能从桌宠打开工作区；用户菜单里有桌宠开关。这些行为横跨 main / preload / pet / app
// 四处，行为测试之外再加一层接线守卫，避免下一次改 UI 时悄悄丢回来。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const electron = join(here, "..", "src", "main");
const main = readFileSync(join(electron, "main.js"), "utf8");
const preload = readFileSync(join(electron, "preload.js"), "utf8");
const petFeed = readFileSync(join(electron, "pet-feed.js"), "utf8");
const web = join(here, "..", "..", "web");
const app = readFileSync(join(web, "app.js"), "utf8");
const pet = readFileSync(join(web, "pet", "pet.js"), "utf8");
const petHtml = readFileSync(join(web, "pet", "index.html"), "utf8");
const theme = readFileSync(join(web, "chatgpt-theme.css"), "utf8");
const refine = readFileSync(join(web, "codex-refine.css"), "utf8");
const visual = readFileSync(join(web, "frontend-visual-system.css"), "utf8");
const petCss = readFileSync(join(web, "pet", "pet.css"), "utf8");

test("托盘菜单只剩显示桌面与退出", () => {
  const block = /function refreshTrayMenu\(\) \{[\s\S]*?\n\}/.exec(main)?.[0];
  assert.ok(block, "找不到 refreshTrayMenu");
  assert.match(block, /label: "显示桌面"/);
  assert.match(block, /label: "退出"/);
  assert.doesNotMatch(block, /显示 \/ 隐藏桌宠|打开配置目录|开机自启|label: stateText/);
  // 状态仍保留在 tooltip，不占菜单项。
  assert.match(block, /setToolTip\(`OwO Agent · \$\{stateText\}`\)/);
});

test("桌宠窗口置顶且不抢焦点", () => {
  const windows = [...main.matchAll(/alwaysOnTop: ?["']?([^,'"\n}]+)["']?/g)].map((m) => m[1].trim());
  assert.ok(windows.length >= 2, "主桌宠与角色桌宠都要显式声明置顶");
  assert.ok(windows.every((value) => value === "screen-saver"), `置顶档位必须一致：${windows.join(", ")}`);
  assert.match(main, /focusable: false/);
});

test("聊天期间窗口才可聚焦，关闭后交还键盘", () => {
  // 常驻不可聚焦是桌宠不抢键盘的前提；但聊天输入框必须拿到系统焦点才能打字。
  assert.match(petFeed, /pet:actor:chat-focus/);
  assert.match(petFeed, /w\.setFocusable\(true\)[\s\S]{0,80}w\.focus\(\)/);
  assert.match(petFeed, /w\.blur\(\);w\.setFocusable\(false\)/);
  assert.match(preload, /setChatFocus: open => ipcRenderer\.invoke\("pet:actor:chat-focus"/);
  assert.match(pet, /shellBridge\.setChatFocus\(open\)/);
  assert.match(petCss, /\.chat-panel header button \{[^}]*width: 28px/);
  assert.match(pet, /event\.key !== "Escape"/);
});

test("命中区判定必须在主进程：穿透后的窗口收不到 mousemove，页面无法自救", () => {
  // 2026-11-13 的真实故障：页面把窗口切成穿透后，Windows 把鼠标消息发给下面的窗口，
  // 页面再也收不到 mousemove，于是永远等不到"把穿透关掉"的那次事件——桌宠活着、
  // 看得见，但右键、拖动、摸摸头全部失效（用户看到的是"卡在右上角拖不动"）。
  // 因此判定只能由主进程轮询指针位置来做，页面只报几何。
  assert.match(main, /screen\.getCursorScreenPoint\(\)/);
  const watch = /function startPetPointerWatch\(\) \{[\s\S]*?\n\}/.exec(main)?.[0];
  assert.ok(watch, "找不到 startPetPointerWatch");
  assert.match(watch, /petDragging\.has\(win\)/, "拖动期间不得收回输入权");
  const apply = /function setPetPointerInside\(win, inside\) \{[\s\S]*?\n\}/.exec(main)?.[0];
  assert.ok(apply, "找不到 setPetPointerInside");
  assert.match(apply, /setIgnoreMouseEvents\(!inside, \{ forward: true \}\)/);
  // WS_EX_NOACTIVATE 下 Chromium 只收到 mouseup，命中时必须临时可聚焦，但不能抢焦点。
  assert.match(apply, /win\.setFocusable\(true\)/);
  assert.doesNotMatch(apply, /win\.focus\(\)/, "悬停不得主动抢焦点");
  // 页面只报几何，不再自己改穿透（否则两个真相源又打架）。
  assert.match(preload, /setRegion: rect => ipcRenderer\.invoke\("pet:actor:region"/);
  assert.match(petFeed, /pet:actor:region/);
  assert.match(pet, /shellBridge\.setRegion\(region\)/);
  assert.doesNotMatch(pet, /setIgnoreMouseEvents/, "页面不得再自行切换穿透");
  assert.doesNotMatch(preload, /pet:actor:ignore-mouse/, "决定穿透的 IPC 必须只剩主进程内部使用");
  // 拖动期间要保持输入权，否则 pointer capture 随穿透一起失效，窗口粘住不动。
  assert.match(petFeed, /pet:actor:dragging/);
  assert.match(preload, /setDragging: value => ipcRenderer\.invoke\("pet:actor:dragging"/);
  assert.match(pet, /shellBridge\.setDragging\(true\)/);
  assert.match(pet, /shellBridge\.setDragging\(false\)/);
  // 点击会激活窗口，互动一结束必须把键盘交还；聊天期间由主进程忽略。
  assert.match(petFeed, /pet:actor:blur/);
  assert.match(preload, /releaseFocus: \(\) => ipcRenderer\.invoke\("pet:actor:blur"\)/);
  assert.match(pet, /function releaseKeyboard\(\)/);
  assert.match(pet, /releaseKeyboard\(\)/);
});

test("桌宠打开工作台时带上当前工作区，并订阅聚焦事件", () => {
  assert.match(main, /ipcMain\.handle\("pet:workbench"[\s\S]{0,500}pet:focus-workspace/);
  assert.match(preload, /onPetFocusWorkspace/);
  assert.match(preload, /ipcRenderer\.on\("pet:focus-workspace"/);
  assert.match(app, /onPetFocusWorkspace/);
  assert.match(app, /openWorkspaceMenu\(\$\("composerProjectBtn"\)\)/);
});

test("用户菜单提供桌宠开关并复用现有切换逻辑", () => {
  assert.match(app, /label: "桌面桌宠"/);
  assert.match(app, /toggleDesktopPet\(\)/);
  // 只应有一份实现：用户菜单调用现有切换函数，而不是另写显隐逻辑。
  assert.equal([...app.matchAll(/function toggleDesktopPet\(\)/g)].length, 1);
});

test("桌宠聊天复用统一 API、流式回合和完成确认，不另起运行时", () => {
  assert.match(petHtml, /\/core\/turn-sse\.js/);
  assert.match(pet, /OwoTurnSse\.consumeResponse/);
  assert.match(pet, /client\.stream\("\/session\/"/);
  assert.match(pet, /api\("\/session", \{ method: "POST"/);
  assert.doesNotMatch(pet, /new Agent\(|createAgent\(|createSessionRuntime/);
});

test("拖动默认移动窗口，Shift 拖动才旋转 3D 形象", () => {
  const move = /pet\.addEventListener\("pointermove"[\s\S]*?\n\}\);/.exec(pet)?.[0];
  assert.ok(move);
  assert.match(move, /down\.shift/);
  assert.match(move, /moveBy/);
  // 默认分支必须是移动窗口，旋转只在 Shift 分支里。
  assert.match(move, /if \(down\.shift[\s\S]{0,200}rotate[\s\S]{0,300}else if[\s\S]{0,100}moveBy/);
});

test("模型菜单默认收起详情，窄窗口仍能打开能力和思考强度", () => {
  assert.match(visual, /body > \.menu-popover\.composer-menu\.composer-menu-model \{[^}]*width: min\(320px/);
  assert.match(visual, /\.composer-menu-model\[data-detail-side="inline"\] \.composer-model-details \{[^}]*position: static/);
  assert.match(app, /composer-model-browser/);
  assert.match(app, /data-model-effort-panel[^>]* hidden>/);
  assert.match(app, /data-model-info aria-expanded="false"/);
  assert.match(app, /data-model-preview/);
  assert.match(app, /data-effort-segment/);
});

test("桌宠聊天带当前自定义模型连接，但拿不到密钥", () => {
  assert.match(preload, /chatConnection: model => ipcRenderer\.invoke\("pet:chat:connection"/);
  const handler = /ipcMain\.handle\("pet:chat:connection"[\s\S]*?\n\}\);/.exec(main)?.[0];
  assert.ok(handler);
  assert.match(handler, /api_key_env/);
  assert.doesNotMatch(handler, /sealed|secret\(/);
  assert.match(pet, /shellBridge\?\.chatConnection\?\.\(model\)/);
  assert.match(pet, /model_connection: connection/);
});

test("桌宠聊天面板占满小窗口，形象退场且标题栏可拖动", () => {
  assert.match(petCss, /\.chat-panel \{[^}]*inset: 6px/);
  assert.match(petCss, /body\.chatting #pet/);
  assert.match(pet, /document\.body\.classList\.toggle\("chatting", open\)/);
  assert.match(pet, /chatHeader\?\.addEventListener\("pointermove"/);
});

test("桌宠整窗留在工作区内，透明留白不再溢出屏幕边缘", () => {
  // 旧版为了「模型贴边」允许窗口右/下边缘溢出到屏幕外，DPI 或缩放一变溢出的
  // 就是模型本身，屏幕上只剩右边一小条。现在整窗留安全边距。
  assert.match(main, /const PET_EDGE_MARGIN = 16/);
  assert.match(main, /const PET_CONTENT_INSETS = \{ left: 76, right: 74, top: 116, bottom: 54 \}/);
  assert.doesNotMatch(main, /PET_SIZE\.width \+ PET_CONTENT_INSETS\.right/, "不得再允许窗口溢出工作区");
  assert.match(main, /area\.x \+ area\.width - PET_EDGE_MARGIN - inset\(insets\.right\) - contentWidth/);
  assert.match(main, /area\.y \+ area\.height - PET_EDGE_MARGIN - inset\(insets\.bottom\) - contentHeight/);
  assert.match(main, /Math\.max\(minX, area\.x \+ area\.width - PET_EDGE_MARGIN - inset\(insets\.right\) - contentWidth\)/, "小屏时上下限不能倒挂");
  assert.match(main, /petContentInsets/);
  // 初始必须是整窗穿透；接管与否交给指针轮询（startPetPointerWatch）。
  assert.match(main, /setIgnoreMouseEvents\(true, \{ forward: true \}\)/);
  assert.match(main, /startPetPointerWatch\(\)/);
  assert.match(main, /const PET_HITBOX = \{ x: 43, y: 78, width: 120, height: 120 \}/);
  assert.match(petHtml, /id="pet-hitbox"/);
  assert.match(petCss, /\.pet-hitbox \{/);
  // 命中区几何由页面量、由主进程用：窗口内坐标 + 拖动期间常驻可交互。
  assert.match(pet, /petHitbox\.getBoundingClientRect\(\)/);
  assert.match(pet, /contentInsets:/);
  assert.match(pet, /requestAnimationFrame\(\(\) => reportPetRegion\(\)\)/);
  assert.match(pet, /reportPetRegion\(open\)/);
  // 页面报的是 CSS 像素，窗口吃的是 DIP：命中区与拖动位移都要折算页面缩放，
  // 否则桌宠页跟着工作台的界面缩放一起变成 95%，命中区整体偏移、拖动快 5%。
  assert.match(main, /function petZoomFactor\(win\)/);
  assert.match(main, /normalizePetRegion\(rect, petZoomFactor\(win\)\)/);
  assert.match(main, /dx\*=zoom;dy\*=zoom;/);
});

test("右键菜单由系统弹出且形象收进子菜单，不再使用窗口内 DOM 菜单", () => {
  assert.match(preload, /pet:actor:menu/);
  assert.match(pet, /label: "更换形象"/);
  assert.match(pet, /submenu: skinIndex\.skins\.map/);
  assert.doesNotMatch(petHtml, /id="skin-menu"/);
  assert.doesNotMatch(petCss, /\.skin-menu/);
});

test("工作台全屏盖满桌宠时保持置顶，不让桌宠凭空消失", () => {
  // 工作台最大化时桌宠整窗都在它里面，「压到窗口后面」= 完全看不见（或只在屏幕
  // 右边露一条边）。只有桌宠还有一部分露在外面时才允许让位。
  assert.match(main, /const fullyCovered = content\.x >= app\.x && content\.y >= app\.y/);
  assert.match(main, /focused && overlaps && !fullyCovered/);
  assert.match(main, /const content = \{ x: r\.x \+ insets\.left/);
  // 层级只在真的翻转时才重设：透明窗口每次 setAlwaysOnTop 都会被 Windows 重新合成，
  // 而本函数挂在 focus/blur/move/resize 上，拖工作台时一秒几十次 → 桌宠闪烁 + 黑底。
  assert.match(main, /if \(petTopmost\.get\(win\) === next\) continue;/);
  assert.match(main, /const petTopmost = new WeakMap\(\)/);
  assert.match(main, /const overlaps = content\.x/);
  assert.match(main, /const fullyCovered = content\.x/);
});

test("界面放大时把桌宠页缩放钳回 100%，模型不被小窗口裁掉", () => {
  // 桌宠页与工作台同源会继承 host 缩放；125%/150% 时 CSS 视口比形象还小，
  // 固定布局会被 270×290 的窗口裁掉，屏幕上只剩一角。
  assert.match(main, /"did-finish-load", \(\) => \{/);
  assert.match(main, /petZoomFactor\(petWindow\) > 1/);
  assert.match(main, /petZoomFactor\(win\) > 1/);
  assert.match(main, /setZoomFactor\(1\)/);
});
