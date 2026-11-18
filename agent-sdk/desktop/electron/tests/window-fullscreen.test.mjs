import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");

test("startup fills the primary display work area without entering kiosk fullscreen", () => {
  const readyToShow = source.slice(source.indexOf('mainWindow.once("ready-to-show"'), source.indexOf('// 工作台里的外链交给系统浏览器'));
  assert.match(readyToShow, /if \(!mainWindow\.isMaximized\(\)\) mainWindow\.maximize\(\);/);
  assert.ok(readyToShow.indexOf("mainWindow.maximize()") < readyToShow.indexOf("raiseMainWindow()"));
  assert.doesNotMatch(source, /setFullScreen\(true\)/);
  assert.match(source, /width: 1360,/);
  assert.match(source, /height: 880,/);
});
