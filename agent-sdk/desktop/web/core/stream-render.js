/**
 * 流式回答的渲染合并器。
 *
 * 模型每个 token 都会推一条 `token_delta`；如果每次都把整段 Markdown 重新渲染进 DOM，
 * 长回答会在每个 token 上重排整段内容（列表、表格、代码块），出现输入延迟与滚动抖动。
 * 这里把增量按固定间隔合并：一帧内多次 delta 只渲染最后一次文本，终态再 `flush()` 立即落盘，
 * 保证首字延迟不变、结尾不丢内容。
 */
(function (root) {
  "use strict";

  const DEFAULT_INTERVAL_MS = 80;

  function createStreamRenderer(render, options) {
    const opts = options || {};
    const schedule = opts.schedule || ((fn, ms) => setTimeout(fn, ms));
    const cancel = opts.cancel || ((id) => clearTimeout(id));
    const interval = Number(opts.interval) > 0 ? Number(opts.interval) : DEFAULT_INTERVAL_MS;
    const onPaint = typeof opts.onPaint === "function" ? opts.onPaint : null;
    let timer = null;
    let bubble = null;
    let latest = "";
    let dirty = false;

    const paint = () => {
      timer = null;
      if (!dirty) return;
      dirty = false;
      const target = bubble;
      if (!target || typeof render !== "function") return;
      target.innerHTML = render(latest);
      if (onPaint) onPaint(target);
    };

    return {
      /// 记录最新文本；第一次 update 会排一次帧，后续 update 只替换文本。
      update(nextBubble, text) {
        bubble = nextBubble || bubble;
        latest = String(text == null ? "" : text);
        dirty = true;
        if (timer === null) timer = schedule(paint, interval);
      },
      /// 立即渲染尚未落盘的文本（终态、错误、回合结束前调用）。
      flush() {
        if (timer !== null) {
          cancel(timer);
          timer = null;
        }
        paint();
      },
      /// 丢弃挂起的帧（气泡已被替换或回合已取消）。
      reset() {
        if (timer !== null) {
          cancel(timer);
          timer = null;
        }
        bubble = null;
        latest = "";
        dirty = false;
      },
      get pending() {
        return dirty;
      },
    };
  }

  root.OwoStreamRender = Object.freeze({ createStreamRenderer, DEFAULT_INTERVAL_MS });
})(typeof window !== "undefined" ? window : globalThis);
