/* 统一任务事件存储（Agent V2 Phase 5）。
 *
 * 约束（方案 §9）：
 * - 事件只有两个来源：Task API 的历史补拉（after 游标）与 SSE 流（同一游标）；
 * - 按 sequence 去重、升序保存，断线重连带 retry-after 游标，不重复显示；
 * - 序列号是唯一的顺序依据，不用时间戳排序；
 * - 终态 `done` 帧到达后停止重连；页面关闭 / 切换任务时 stop() 取消流与定时器。
 *
 * 该模块不发请求：历史补拉走注入的 TaskApiClient，流式订阅走注入的 openStream
 * （app-domain 注入 window.OwoApi.openEventStream），便于 Node 契约测试。
 */
(function (root) {
  "use strict";

  const DEFAULTS = {
    reconnectBaseMs: 800,
    maxBackoffMs: 15000,
    pageLimit: 200,
    maxEvents: 4000,
  };

  function validEvent(raw) {
    return raw && typeof raw === "object" &&
      typeof raw.sequence === "number" && raw.sequence >= 1 &&
      typeof raw.event_type === "string" && raw.event_type;
  }

  /**
   * @param {{
   *   client: {events: Function},
   *   openStream: Function,
   *   reconnectBaseMs?: number,
   *   maxBackoffMs?: number,
   *   pageLimit?: number,
   *   maxEvents?: number,
   *   onChange?: (snapshot: object) => void,
   * }} options
   */
  function createTaskEventStore(options) {
    const opts = options || {};
    const client = opts.client;
    if (!client || typeof client.events !== "function") {
      throw new Error("TaskEventStore 需要 TaskApiClient");
    }
    if (typeof opts.openStream !== "function") {
      throw new Error("TaskEventStore 需要 openStream");
    }
    const reconnectBaseMs = Number(opts.reconnectBaseMs) > 0 ? Number(opts.reconnectBaseMs) : DEFAULTS.reconnectBaseMs;
    const maxBackoffMs = Number(opts.maxBackoffMs) > 0 ? Number(opts.maxBackoffMs) : DEFAULTS.maxBackoffMs;
    const pageLimit = Number(opts.pageLimit) > 0 ? Number(opts.pageLimit) : DEFAULTS.pageLimit;
    const maxEvents = Number(opts.maxEvents) > 0 ? Number(opts.maxEvents) : DEFAULTS.maxEvents;

    let taskId = null;
    let events = [];
    let cursor = 0;
    let state = "idle"; // idle | connecting | live | degraded | closed
    let generation = 0;
    let controller = null;
    let retryTimer = null;
    let backoffMs = reconnectBaseMs;
    let lastError = null;

    function snapshot() {
      return { taskId, cursor, state, events: events.slice(), error: lastError };
    }

    function notify() {
      if (typeof opts.onChange !== "function") return;
      try {
        opts.onChange(snapshot());
      } catch (_) {
        // 渲染失败不能打断事件接收。
      }
    }

    function setState(next, error) {
      state = next;
      lastError = error || null;
      notify();
    }

    function ingest(list) {
      let changed = false;
      const known = new Set(events.map((event) => event.sequence));
      for (const raw of list || []) {
        // 游标只用于补拉起点；已收到的序号按集合去重，迟到但未见过的事件仍保留。
        if (!validEvent(raw) || known.has(raw.sequence)) continue;
        events.push(raw);
        known.add(raw.sequence);
        if (raw.sequence > cursor) cursor = raw.sequence;
        changed = true;
      }
      if (!changed) return;
      events.sort((a, b) => a.sequence - b.sequence);
      if (events.length > maxEvents) events = events.slice(events.length - maxEvents);
      notify();
    }

    async function backfill() {
      const result = await client.events(taskId, { after: cursor, limit: pageLimit });
      ingest(result && result.events);
    }

    function clearRetry() {
      if (retryTimer != null) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
    }

    function scheduleReconnect() {
      clearRetry();
      const delay = backoffMs;
      backoffMs = Math.min(backoffMs * 2, maxBackoffMs);
      retryTimer = setTimeout(() => {
        retryTimer = null;
        connect();
      }, delay);
    }

    async function connect() {
      if (state === "closed" || taskId == null) return;
      const gen = generation;
      controller = new AbortController();
      setState("connecting");
      try {
        // 先补拉：断线或打开页面时可能已经错过了若干事件。
        await backfill();
        if (gen !== generation) return;
        setState("live");
        backoffMs = reconnectBaseMs;
        await opts.openStream(
          "/agent-tasks/" + encodeURIComponent(taskId) + "/events/stream?after=" + cursor,
          {
            signal: controller.signal,
            onOpen() {
              if (gen !== generation) return;
              if (state !== "live") setState("live");
              backoffMs = reconnectBaseMs;
            },
            onEvent(frame) {
              if (gen !== generation || !frame) return;
              if (frame.event === "done") {
                setState("closed");
                try { controller.abort(); } catch (_) { /* 已结束 */ }
                return;
              }
              if (frame.event !== "task-event") return;
              try {
                ingest([JSON.parse(frame.data)]);
              } catch (_) {
                // 单帧损坏不打断整条流；下次重连按游标补拉。
              }
            },
          }
        );
        // 流正常结束但没有 done：任务可能仍在运行，按游标重连。
        if (gen === generation && state !== "closed") {
          setState("degraded", new Error("事件流提前结束"));
          scheduleReconnect();
        }
      } catch (error) {
        if (gen !== generation) return;
        if (error && error.name === "AbortError") return;
        setState("degraded", error);
        scheduleReconnect();
      }
    }

    function start(id) {
      if (typeof id !== "string" || !id) throw new Error("TaskEventStore.start 需要任务 ID");
      stop();
      taskId = id;
      events = [];
      cursor = 0;
      backoffMs = reconnectBaseMs;
      generation += 1;
      state = "connecting";
      connect();
      return snapshot();
    }

    function stop() {
      generation += 1;
      clearRetry();
      if (controller) {
        try { controller.abort(); } catch (_) { /* 忽略 */ }
        controller = null;
      }
      if (taskId != null) {
        taskId = null;
        events = [];
        cursor = 0;
        state = "idle";
      }
    }

    return {
      start,
      stop,
      snapshot,
    };
  }

  const api = { createTaskEventStore };
  root.OwoTaskEvents = Object.freeze(api);
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
