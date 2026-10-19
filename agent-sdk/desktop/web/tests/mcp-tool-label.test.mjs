import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

// toolLabel 在 app.js（DOM 密集文件）里，无法整体加载；按源码片段提取到 vm 中执行，
// 保证展示层与注册命名（core `mcp_tool_full_name`）不漂移：审查 N5 之后 MCP 工具名
// 是 `mcp__{server}__{tool}`，审批卡/时间线不能显示成 `mcp__owo_plugin_translate__translate`。
const source = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const start = source.indexOf("function toolLabel(tool) {");
assert.ok(start >= 0, "app.js 必须定义 toolLabel");
const tail = source.slice(start);
const end = tail.indexOf("\n}\n");
assert.ok(end > 0, "toolLabel 函数体必须能以顶层 `}` 结束");
const snippet = tail.slice(0, end + 2);
const sandbox = { TOOL_LABELS: { read_file: "读取文件" } };
vm.createContext(sandbox);
vm.runInContext(`${snippet}; globalThis.owoToolLabel = toolLabel;`, sandbox);
const toolLabel = sandbox.owoToolLabel;

test("MCP namespaced tool names render as MCP · server · tool", () => {
  assert.equal(
    toolLabel("mcp__owo_plugin_translate__translate"),
    "MCP · owo_plugin_translate · translate",
  );
  assert.equal(toolLabel("mcp__git__status"), "MCP · git · status");
  // 只有 server 段（资源/提示类没有 `__tool` 后缀）时不出现空尾段。
  assert.equal(toolLabel("mcp__owo_plugin_demo__"), "MCP · owo_plugin_demo");
  assert.equal(toolLabel("mcp__solo"), "MCP · solo");
});

test("builtin labels and legacy prefixes keep working", () => {
  assert.equal(toolLabel("read_file"), "读取文件");
  // 历史会话里的旧命名仍要可读（不是新注册名，但不能显示成乱码）。
  assert.equal(toolLabel("mcp_demo_lookup"), "MCP · demo_lookup");
  assert.equal(toolLabel("owo_plugin_translate_translate"), "插件 · translate_translate");
  assert.equal(toolLabel("run_command"), "run_command");
  assert.equal(toolLabel(""), "工具");
});
