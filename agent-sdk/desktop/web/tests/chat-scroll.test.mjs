// ============================================================================
// 粘性滚动决策测试 —— tests/chat-scroll.test.mjs
// 运行：node desktop/web/tests/chat-scroll.test.mjs
// ============================================================================
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const scope = {};
new Function("window", readFileSync(new URL("../core/chat-scroll.js", import.meta.url), "utf8"))(scope);
const S = scope.OwoChatScroll;

test("atBottom：阈值内的贴底判定", () => {
  assert.equal(S.atBottom({ scrollHeight: 1000, scrollTop: 600, clientHeight: 380 }), true);
  assert.equal(S.atBottom({ scrollHeight: 1000, scrollTop: 500, clientHeight: 380 }), false);
  assert.equal(S.atBottom(null), true, "容器缺失时视为贴底（不阻塞跟随）");
});

test("decideScroll：跟随中/强制/恢复/用户上滑四种情形", () => {
  const following = S.decideScroll({ autoScroll: true, scrollHeight: 1000, scrollTop: 600, clientHeight: 380 });
  assert.deepEqual(following, { scroll: true, autoScroll: true, reason: "following" });

  const forced = S.decideScroll({ autoScroll: false, force: true, scrollHeight: 1000, scrollTop: 0, clientHeight: 380 });
  assert.deepEqual(forced, { scroll: true, autoScroll: true, reason: "forced" });

  // 关键回归：用户曾上滑（autoScroll=false），但内容增长后已经贴底 → 自动恢复跟随。
  const recovered = S.decideScroll({ autoScroll: false, scrollHeight: 1200, scrollTop: 830, clientHeight: 380 });
  assert.deepEqual(recovered, { scroll: true, autoScroll: true, reason: "recovered" });

  // 用户明确停在半途：不抢滚动（可读历史）。
  const away = S.decideScroll({ autoScroll: false, scrollHeight: 1200, scrollTop: 200, clientHeight: 380 });
  assert.deepEqual(away, { scroll: false, autoScroll: false, reason: "user-scrolled-away" });
});

test("afterUserScroll：滚动事件后的跟随状态", () => {
  assert.equal(S.afterUserScroll({ scrollHeight: 900, scrollTop: 520, clientHeight: 380 }), true);
  assert.equal(S.afterUserScroll({ scrollHeight: 900, scrollTop: 100, clientHeight: 380 }), false);
});
