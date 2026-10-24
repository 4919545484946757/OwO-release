import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const source = readFileSync(fileURLToPath(new URL("../core/panel-runtime.js", import.meta.url)), "utf8");
const sandbox = { window: {} };
vm.runInNewContext(source, sandbox, { filename: "panel-runtime.js" });
const mount = sandbox.window.OwoPanelRuntime.mount;

test("同步面板异常被局部兜底，导航调用方保持可用", () => {
  const failure = new Error("mount failed");
  const errors = [];
  assert.equal(mount({ mount() { throw failure; } }, {}, {}, (error) => errors.push(error)), null);
  assert.deepEqual(errors, [failure]);
});

test("异步面板拒绝也变为局部失败，不泄漏未处理拒绝", async () => {
  const failure = new Error("async mount failed");
  const errors = [];
  const result = await mount({ mount() { return Promise.reject(failure); } }, {}, {}, (error) => errors.push(error));
  assert.equal(result, null);
  assert.deepEqual(errors, [failure]);
});

test("正常面板挂载的同步返回值保持原样", () => {
  const sentinel = { mounted: true };
  assert.equal(mount({ mount() { return sentinel; } }, {}, {}, () => assert.fail()), sentinel);
});

test("面板重挂载前 dispose 与挂载失败恢复均接入共享导航", () => {
  const domain = readFileSync(fileURLToPath(new URL("../app-domain.js", import.meta.url)), "utf8");
  const body = /function mountPanel\(id, writeHash = true\) \{([\s\S]*?)\n\}/.exec(domain);
  assert.ok(body);
  assert.match(body[1], /if \(currentPanel\) \{[\s\S]*?prev\.dispose\(\)/);
  assert.match(body[1], /window\.OwoPanelRuntime\.mount\(panel, root, panelHelpers\(\), showMountError\)/);
  assert.match(body[1], /重新加载此页面/);
});

test("分区标题后的主体与标题同宽，避免两列布局留下空半屏", () => {
  const make = (tagName, isHeading = false) => ({
    tagName,
    classList: { contains(name) { return isHeading && name === "sub"; } },
  });
  const blocks = [make("DIV", true), make("DIV"), make("DIV"), make("TABLE")];
  const flags = sandbox.window.OwoPanelRuntime.layoutFlags(blocks, [true, false, false, true]);
  assert.equal(Array.from(flags).join(","), "true,true,false,true");
});

test("layout flags module is wired before app-domain panel layout", () => {
  const index = readFileSync(fileURLToPath(new URL("../index.html", import.meta.url)), "utf8");
  const domain = readFileSync(fileURLToPath(new URL("../app-domain.js", import.meta.url)), "utf8");
  assert.ok(index.indexOf("core/panel-runtime.js") < index.indexOf("app-domain.js"));
  assert.match(domain, /OwoPanelRuntime\.layoutFlags\(children, wideFlags\)/);
});

test("Shared skills owns its tab layout instead of the generic panel heuristic", () => {
  const domain = readFileSync(fileURLToPath(new URL("../app-domain.js", import.meta.url)), "utf8");
  const team = readFileSync(fileURLToPath(new URL("../panels/team.panel.js", import.meta.url)), "utf8");
  assert.match(domain, /section\.dataset\.layout === "custom"\) return/);
  const context = { window: {} };
  vm.runInNewContext(team, context);
  const html = context.window.OwoPanels.team.nav();
  assert.match(html, /data-layout="custom" class="owo-team-panel\b/);
  assert.equal((html.match(/role="tab"/g) || []).length, 4);
  assert.equal((html.match(/role="tabpanel"/g) || []).length, 4);
  assert.match(html, /id="team-pane-import" role="tabpanel" aria-labelledby="team-tab-import">/);
  for (const id of ["export", "versions", "audit"]) {
    assert.match(html, new RegExp('id="team-pane-' + id + '" role="tabpanel"[^>]* hidden>'));
  }
});
