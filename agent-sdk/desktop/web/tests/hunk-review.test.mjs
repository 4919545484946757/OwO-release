// ============================================================================
// 逐 hunk 变更审阅（桌面 Web）纯逻辑测试 —— tests/hunk-review.test.mjs
//
// 运行：node tests/hunk-review.test.mjs（在 agent-sdk/desktop/web 下）
// 依赖：仅 node:test / node:assert，无 DOM、无网络。
//
// 覆盖面：序号解析（1 起 / all / 越界 / 非法）、请求体只发一侧、
//         视图行构造（冲突与不可审阅原因）、应用结果摘要。
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const source = readFileSync(join(here, "../core/hunk-review.js"), "utf8");
const scope = {};
new Function("window", source)(scope);
const H = scope.OwoHunkReview._test;

test("parseIndices：1 起序号转 0 起并去重排序", () => {
  assert.deepEqual(H.parseIndices("1,3", 3), { ok: true, indices: [0, 2] });
  assert.deepEqual(H.parseIndices(" 3 1 ", 3), { ok: true, indices: [0, 2] });
  assert.deepEqual(H.parseIndices("2,2,1", 2), { ok: true, indices: [0, 1] });
});

test("parseIndices：all 需要总数，0 与越界都拒绝", () => {
  assert.deepEqual(H.parseIndices("all", 3), { ok: true, indices: [0, 1, 2] });
  assert.equal(H.parseIndices("all", 0).ok, false);
  assert.equal(H.parseIndices("", 2).ok, false);
  assert.equal(H.parseIndices("0", 2).ok, false);
  assert.equal(H.parseIndices("5", 2).ok, false);
  assert.equal(H.parseIndices("x", 2).ok, false);
});

test("applyBody：只发用户指定的一侧（两侧同发会 422）", () => {
  const accepted = H.applyBody("a.txt", "accept", [1]);
  assert.deepEqual(accepted, { ok: true, body: { path: "a.txt", accept: [1] } });
  const rejected = H.applyBody("a.txt", "reject", [0, 2]);
  assert.deepEqual(rejected, { ok: true, body: { path: "a.txt", reject: [0, 2] } });
  assert.equal(H.applyBody("", "accept", [0]).ok, false);
  assert.equal(H.applyBody("a.txt", "accept", []).ok, false);
  assert.equal(H.applyBody("a.txt", "either", [0]).ok, false);
});

test("buildRows：展示序号 1 起、标出冲突与不可审阅原因", () => {
  const rows = H.buildRows({
    files: [
      {
        path: "a.txt",
        kind: "modified",
        hunks: [
          { index: 0, kind: "modified", old_start: 2, old_lines: 1, new_start: 2, new_lines: 1 },
          { index: 1, kind: "added", old_start: 5, old_lines: 0, new_start: 5, new_lines: 2 },
        ],
      },
      { path: "new.txt", kind: "unavailable", hunks: [], reason: "新增文件没有写前快照" },
    ],
    conflicts: ["a.txt"],
  });
  assert.equal(rows.length, 2);
  assert.equal(rows[0].conflict, true);
  assert.equal(rows[0].reviewable, true);
  assert.equal(rows[0].hunks[0].label, "#1 modified @3 → 3（-1 +1）");
  assert.equal(rows[0].hunks[1].label, "#2 added @6 → 6（-0 +2）");
  assert.equal(rows[1].reviewable, false);
  assert.equal(rows[1].reason, "新增文件没有写前快照");
  assert.equal(H.buildRows(null).length, 0);
});

test("summarizeApply：序号回到 1 起并标注未写盘", () => {
  assert.equal(
    H.summarizeApply({ path: "a.txt", accepted: [1], rejected: [0], remaining: 1, changed: true }),
    "已应用 a.txt：接受 [2] / 拒绝 [1]；剩余改动 1 处",
  );
  assert.equal(
    H.summarizeApply({ path: "a.txt", accepted: [0, 1], rejected: [], remaining: 2, changed: false }),
    "已应用 a.txt：接受 [1,2] / 拒绝 []；剩余改动 2 处（内容已一致，未写盘）",
  );
  assert.equal(H.summarizeApply(null), "（没有收到应用结果）");
});
