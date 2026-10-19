// ============================================================================
// 桌面符号检索入口测试 —— tests/symbol-search-entry.test.mjs
//
// 运行：node tests/symbol-search-entry.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert / node:vm，无 DOM、无网络。
//
// 守卫：入口必须把检索落在**当前会话工作区**（session_id），并把空查询挡在
// 请求之前；结果行与摘要取自 core/symbol-search.js（同一模块，不另写一套）。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const read = (name) => readFileSync(join(here, "..", name), "utf8");

function loadEntry({ api, sessionId }) {
  const domain = read("app-domain.js");
  const source = /async function searchSymbols\(query, options\) \{[\s\S]*?\n\}/.exec(domain)?.[0];
  assert.ok(source, "app-domain.js 必须定义 searchSymbols");
  const symbolModule = {};
  new Function("window", read("core/symbol-search.js"))(symbolModule);
  const window = symbolModule;
  const state = { sessionId };
  const entry = runInNewContext(source + "\nsearchSymbols;", { window, state, api, Object, String, Number, Boolean });
  return entry;
}

test("入口把 session_id 带进 /symbols，并返回行与摘要", async () => {
  const calls = [];
  const entry = loadEntry({
    sessionId: "sess-42",
    api: async (uri) => {
      calls.push(uri);
      return {
        query: "parse",
        scanned_files: 3,
        results: [{ path: "src/lib.rs", line: 3, kind: "function", name: "parse", signature: "pub fn parse() {}" }],
      };
    },
  });
  const result = await entry("parse", {});
  assert.equal(calls.length, 1);
  assert.equal(calls[0], "/symbols?query=parse&session_id=sess-42");
  assert.equal(result.ok, true);
  assert.equal(result.rows.length, 1);
  assert.equal(result.rows[0].label, "src/lib.rs:3  [function]");
  assert.equal(result.summary, "符号“parse”（1 条，扫描 3 个源文件）：");
});

test("入口支持 kind/path/max_results，并可与 session_id 共存", async () => {
  const calls = [];
  const entry = loadEntry({
    sessionId: "sess-7",
    api: async (uri) => {
      calls.push(uri);
      return { query: "a", scanned_files: 1, results: [] };
    },
  });
  const result = await entry("a", { kind: "struct", path: "src", maxResults: 5 });
  assert.equal(calls[0], "/symbols?query=a&kind=struct&path=src&session_id=sess-7&max_results=5");
  assert.equal(result.rows.length, 0);
  assert.equal(result.summary, "（没有匹配“a”的定义；已扫描 1 个源文件）");
});

test("空查询在请求之前就被挡住", async () => {
  let called = false;
  const entry = loadEntry({
    sessionId: "sess-1",
    api: async () => {
      called = true;
      return {};
    },
  });
  const result = await entry("   ", {});
  assert.equal(result.ok, false);
  assert.match(result.error, /请输入符号名/);
  assert.equal(called, false, "空查询不得发出请求");
});

test("请求失败时返回可读错误而不是抛出", async () => {
  const entry = loadEntry({
    sessionId: "sess-1",
    api: async () => {
      throw new Error("HTTP 400 缺少 query");
    },
  });
  const result = await entry("parse", {});
  assert.equal(result.ok, false);
  assert.match(result.error, /符号检索失败：HTTP 400/);
});
