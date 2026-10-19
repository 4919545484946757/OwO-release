// ============================================================================
// 符号检索（桌面 Web）纯逻辑测试 —— tests/symbol-search.test.mjs
//
// 运行：node tests/symbol-search.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert，无 DOM、无网络。
//
// 覆盖面：查询串构造（必填校验、中文编码、session_id/kind/path/max_results）、
//         结果行、摘要（空结果 / 截断 / 文件数上限三种措辞）。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const source = readFileSync(join(here, "../core/symbol-search.js"), "utf8");
const scope = {};
new Function("window", source)(scope);
const S = scope.OwoSymbolSearch._test;

test("buildQueryUri：query 必填，中文与保留字符正确编码", () => {
  assert.equal(S.buildQueryUri("").ok, false);
  assert.equal(S.buildQueryUri("   ").ok, false);
  assert.deepEqual(S.buildQueryUri("parse config"), { ok: true, uri: "/symbols?query=parse%20config" });
  assert.deepEqual(S.buildQueryUri("符号"), { ok: true, uri: "/symbols?query=%E7%AC%A6%E5%8F%B7" });
  assert.deepEqual(S.buildQueryUri("a&b=c"), { ok: true, uri: "/symbols?query=a%26b%3Dc" });
});

test("buildQueryUri：session_id/kind/path/max_results 只在给出时拼上", () => {
  const full = S.buildQueryUri("parse", {
    kind: "struct",
    path: "src/lib",
    sessionId: "sess-1",
    maxResults: 5,
  });
  assert.deepEqual(full, {
    ok: true,
    uri: "/symbols?query=parse&kind=struct&path=src%2Flib&session_id=sess-1&max_results=5",
  });
  // max_results 非法值（0/NaN/负）不带参数；max 取整数。
  assert.equal(S.buildQueryUri("parse", { maxResults: 0 }).uri, "/symbols?query=parse");
  assert.equal(S.buildQueryUri("parse", { maxResults: "x" }).uri, "/symbols?query=parse");
  assert.equal(S.buildQueryUri("parse", { maxResults: 3.7 }).uri, "/symbols?query=parse&max_results=3");
});

test("buildRows：路径/行号/种类成行，缺字段有兜底", () => {
  const rows = S.buildRows({
    results: [
      { path: "src/lib.rs", line: 3, kind: "function", name: "parse", signature: "pub fn parse() {}" },
      { path: "web/a.py", line: 7 },
    ],
  });
  assert.equal(rows.length, 2);
  assert.equal(rows[0].label, "src/lib.rs:3  [function]");
  assert.equal(rows[0].signature, "pub fn parse() {}");
  assert.equal(rows[1].kind, "unknown");
  assert.equal(rows[1].label, "web/a.py:7  [unknown]");
  assert.equal(S.buildRows(null).length, 0);
});

test("summarize：空结果 / 截断 / 文件数上限三种措辞", () => {
  assert.equal(
    S.summarize({ query: "nope", results: [], scanned_files: 5 }),
    "（没有匹配“nope”的定义；已扫描 5 个源文件）",
  );
  assert.equal(
    S.summarize({ query: "nope", results: [], scanned_files: 2000, file_limit_reached: true }),
    "（没有匹配“nope”的定义；已扫描 2000 个源文件，且已达文件数上限（结果不完整））",
  );
  assert.equal(
    S.summarize({
      query: "parse",
      scanned_files: 12,
      results: [{ path: "a.rs", line: 1, kind: "function" }],
      results_truncated: true,
    }),
    "符号“parse”（1 条，扫描 12 个源文件，结果被截断）：",
  );
});
