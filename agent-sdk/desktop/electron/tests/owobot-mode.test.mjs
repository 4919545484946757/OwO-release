// OwOBot 不再由壳持有独立窗口：整窗模式完全在工作台页面内完成（见
// desktop/web/tests/owobot-mode.test.mjs）。主进程/preload 不得残留第二业务窗口
// 的机制，否则旧 IPC 会把页面导回已废弃的 ?surface=owobot 流程。
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const main = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");
const preload = readFileSync(new URL("../src/main/preload.js", import.meta.url), "utf8");

test("主进程不再创建 OwOBot 窗口，也不注册 owobot:open handler", () => {
  assert.doesNotMatch(main, /owobotWindow|owobotUiPort|owobotTask/);
  assert.doesNotMatch(main, /owobot:open|owobot:task/);
  assert.doesNotMatch(main, /openOwobotWindow|syncOwobotWindow|owobotUrl|normalizeOwobotTask/);
  assert.doesNotMatch(main, /surface=owobot|"surface", "owobot"/);
});

test("preload 不再暴露独立窗口桥，页面与浏览器行为一致", () => {
  assert.doesNotMatch(preload, /openOwobot|onOwobotTask|owobot:/);
});

test("notifyState 只服务主窗口与桌宠，核心重启导航不带 OwOBot 分支", () => {
  const match = /function notifyState\(\) \{[\s\S]*?\n\}/.exec(main);
  assert.ok(match);
  assert.doesNotMatch(match[0], /owobot/i);
  assert.match(match[0], /mainWindow\.loadURL\(webUiUrl\(coreState\.port\)\);/);
});