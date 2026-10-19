// ============================================================================
// 会话粘性滚动决策（纯函数）—— desktop/web/core/chat-scroll.js
//
// 运行：node tests/chat-scroll.test.mjs
// 依赖：无（IIFE + 可选 module.exports），可在浏览器与 node 测试里同源使用。
//
// 背景（用户反馈）：流式输出时页面不自动下滑、要手动下拉。两个真因：
//   1) 内容增长只改 scrollHeight，不触发 scroll 事件 → 上滑一次后 autoScroll
//      永远停在 false，之后所有输出都不再跟随；
//   2) 用户手动点「回到底部」或滚回底部后，应当立刻恢复跟随。
// 这里把"是否跟随/是否恢复跟随"变成纯函数，便于回归测试。
// ============================================================================
(function (root) {
  "use strict";

  var BOTTOM_THRESHOLD_PX = 40;

  /// 判断滚动容器是否（接近）底部。
  function atBottom(metrics, threshold) {
    if (!metrics) return true;
    var limit = typeof threshold === "number" ? threshold : BOTTOM_THRESHOLD_PX;
    var scrollHeight = Number(metrics.scrollHeight) || 0;
    var scrollTop = Number(metrics.scrollTop) || 0;
    var clientHeight = Number(metrics.clientHeight) || 0;
    return scrollHeight - scrollTop - clientHeight < limit;
  }

  /// 决定这一次要不要滚动，以及滚动后 autoScroll 的值。
  /// - force：用户主动点「回到底部」→ 一定滚；
  /// - 已贴底（含内容增长后被动贴底）→ 自动恢复跟随并滚；
  /// - 用户上滑离开底部 → 不滚，保持 autoScroll=false。
  function decideScroll(input) {
    var state = input || {};
    var force = state.force === true;
    var autoScroll = state.autoScroll === true;
    var pinned = atBottom(
      {
        scrollHeight: state.scrollHeight,
        scrollTop: state.scrollTop,
        clientHeight: state.clientHeight,
      },
      state.threshold
    );
    if (force) return { scroll: true, autoScroll: true, reason: "forced" };
    if (autoScroll) return { scroll: true, autoScroll: true, reason: "following" };
    if (pinned) return { scroll: true, autoScroll: true, reason: "recovered" };
    return { scroll: false, autoScroll: false, reason: "user-scrolled-away" };
  }

  /// 用户滚动事件后 autoScroll 的新值（贴底=跟随，否则停住）。
  function afterUserScroll(metrics, threshold) {
    return atBottom(metrics, threshold);
  }

  var api = {
    BOTTOM_THRESHOLD_PX: BOTTOM_THRESHOLD_PX,
    atBottom: atBottom,
    decideScroll: decideScroll,
    afterUserScroll: afterUserScroll,
  };
  root.OwoChatScroll = Object.freeze(api);
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
