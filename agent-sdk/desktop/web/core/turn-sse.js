/* Durable turn-stream parsing and replay helpers shared by the desktop shell. */
(function (global) {
  "use strict";

  function parseBlock(block) {
    let event = "message";
    let id = null;
    const data = [];
    for (const line of String(block || "").split(/\r?\n/)) {
      if (line.startsWith("event:")) event = line.slice(6).replace(/^ /, "");
      else if (line.startsWith("id:")) id = line.slice(3).replace(/^ /, "");
      else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
    }
    return { event, id, data: data.join("\n") };
  }

  function replayPath(sessionId, turnId, afterSeq, limit = 256) {
    const query = new URLSearchParams({
      turn_id: String(turnId),
      after_seq: String(afterSeq),
      limit: String(limit),
    });
    return `/session/${encodeURIComponent(sessionId)}/turn/events?${query}`;
  }

  function eventsAfterCursor(page, turnId, afterSeq) {
    const cursor = Number(afterSeq) || 0;
    return (page && Array.isArray(page.events) ? page.events : [])
      .filter((record) => record && record.turn_id === turnId && Number.isSafeInteger(Number(record.seq)) && Number(record.seq) > cursor)
      .sort((left, right) => Number(left.seq) - Number(right.seq));
  }


  const statuses = new Set(["response_complete", "candidate", "accepted", "unverified", "blocked", "aborted"]);
  function streamError(message, status = "unverified") {
    const error = new Error(message); error.name = "TurnStreamError"; error.completionStatus = status; return error;
  }
  function createCompletion() {
    let final = null, stats = null, failure = null;
    return {
      get terminal() { return Boolean(stats || failure); },
      observe(event, payload) {
        if (event === "final") final = payload;
        if (event === "turn_stats") {
          if (!statuses.has(payload.completion_status)) throw streamError("回合缺少有效的宿主完成状态");
          stats = payload;
        }
        if (event === "turn_failed") failure = payload;
      },
      finish() {
        if (failure) throw streamError(failure.message || "回合执行失败", failure.completion_status || "unverified");
        if (!stats) throw streamError("连接结束但缺少宿主完成记录；回答和改动尚未确认");
        if (!final) throw streamError("宿主已结束回合，但缺少最终回答");
        return { completionStatus: stats.completion_status, finalText: final.text || "", stats };
      },
    };
  }
  function delay(ms, signal) {
    return new Promise((resolve, reject) => {
      const abort = () => {
        clearTimeout(timer); signal?.removeEventListener("abort", abort);
        const error = new Error("Aborted"); error.name = "AbortError"; reject(error);
      };
      const timer = setTimeout(() => { signal?.removeEventListener("abort", abort); resolve(); }, ms);
      if (signal?.aborted) abort();
      else signal?.addEventListener("abort", abort, { once: true });
    });
  }
  const idleRead = Symbol("idle turn stream");
  function readWithDeadline(pending, signal, milliseconds) {
    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (value, error) => {
        if (settled) return; settled = true;
        clearTimeout(timer); signal?.removeEventListener("abort", abort);
        if (error) reject(error); else resolve(value);
      };
      const abort = () => { const error = new Error("Aborted"); error.name = "AbortError"; finish(null, error); };
      const timer = setTimeout(() => finish(idleRead), milliseconds);
      if (signal?.aborted) abort();
      else signal?.addEventListener("abort", abort, { once: true });
      pending.then(value => finish(value), error => finish(null, error));
    });
  }
  async function replayPage(replay, turnId, cursor, signal, timeout) {
    const controller = new AbortController();
    const abort = () => controller.abort();
    if (signal?.aborted) abort(); else signal?.addEventListener("abort", abort, { once: true });
    try {
      const page = await readWithDeadline(Promise.resolve().then(() => replay(turnId, cursor, controller.signal)), controller.signal, timeout);
      if (page === idleRead) throw streamError("turn/replay_timeout: completion remains unconfirmed");
      return page;
    } finally {
      signal?.removeEventListener("abort", abort);
      controller.abort();
    }
  }
  async function consumeResponse(response, { onEvent, replay, signal, wait = delay, idleTimeoutMs = 60000, replayTimeoutMs = 15000 }) {
    if (!response.body) throw streamError("服务未返回流式响应");
    const completion = createCompletion(), turnId = response.headers.get("x-owo-turn-id");
    let cursor = 0, buffer = "", streamEnded = false;
    const idleMs = Number.isFinite(idleTimeoutMs) && idleTimeoutMs > 0 ? idleTimeoutMs : 60000;
    let nextRecovery = Date.now() + idleMs;
    let pendingRead = null;
    const progressEvents = new Set(["token_delta", "reasoning_delta", "model_call", "tool_start", "tool_result", "permission_request", "user_question", "user_answered", "progress", "plan_update", "compaction", "final", "turn_stats", "turn_failed"]);
    const decoder = new TextDecoder(), encoder = new TextEncoder(), reader = response.body.getReader();
    const dispatch = (event, payload, id, replay = false) => {
      if (!payload || typeof payload !== "object") throw streamError("无效的回合事件");
      const sequence = Number(id);
      const hasSeq = id != null && Number.isSafeInteger(sequence) && sequence > 0;
      const type = event === "message" ? payload.type : event;
      // 思考增量按"内容"而不是"id"去重：旧版服务端把连续 reasoning_delta 合并进同一行并
      // 复用该行 seq，SSE id 因此重复；按 id 去重会把第一条之后的思考全部丢弃（只剩开头几个
      // 字符）。实时推送的 reasoning_delta 一定是纯增量，重复投递最多造成重复文本、不会丢内容；
      // 断线重放一侧仍由 eventsAfterCursor 按 seq 过滤，不会重复补投。
      const incrementalReasoning = type === "reasoning_delta";
      if (!replay && hasSeq && sequence <= cursor && !incrementalReasoning) return;
      completion.observe(type, payload);
      if (progressEvents.has(type)) nextRecovery = Date.now() + idleMs;
      onEvent(type, payload, { id: hasSeq ? sequence : null, replay });
      if (hasSeq && sequence > cursor) cursor = sequence;
    };
    const consume = (flush) => {
      const blocks = buffer.split(/\r?\n\r?\n/);
      buffer = flush ? "" : blocks.pop() || "";
      for (const block of blocks) {
        const { event, id, data } = parseBlock(block);
        if (!data || data === "[DONE]") continue;
        if (encoder.encode(data).length > 1024 * 1024) throw streamError("回合事件超过 1 MiB 上限");
        let payload;
        try { payload = JSON.parse(data); } catch (_) { throw streamError("回合事件格式无效"); }
        dispatch(event, payload, id);
      }
      if (buffer.length > 256 * 1024 && encoder.encode(buffer).length > 1024 * 1024) throw streamError("回合事件超过 1 MiB 上限");
    };
    try {
      while (true) {
        let chunk;
        try {
          pendingRead ||= reader.read();
          chunk = await readWithDeadline(pendingRead, signal, Math.max(1, nextRecovery - Date.now()));
        }
        catch (error) {
          if (signal?.aborted || error.name === "AbortError" || (!turnId && !completion.terminal)) throw error;
          break; // Recover transport failures only; protocol and callback errors propagate.
        }
        if (chunk === idleRead) {
          if (!turnId || !replay) throw streamError("turn/no_progress: stream stalled and durable recovery is unavailable");
          while (!completion.terminal) {
            const page = await replayPage(replay, turnId, cursor, signal, replayTimeoutMs);
            let advanced = false;
            for (const record of eventsAfterCursor(page, turnId, cursor)) {
              if (Number(record.seq) <= cursor) continue;
              dispatch(record.payload?.type, record.payload, record.seq, true); advanced = true;
            }
            if (completion.terminal) break;
            if (advanced) continue;
            if (!page?.active) throw streamError("turn/recovery_incomplete: host completion record is missing");
            break;
          }
          if (completion.terminal) break;
          nextRecovery = Date.now() + idleMs;
          continue; // Keep the same outstanding read and the active turn connection.
        }
        pendingRead = null;
        if (chunk.done) { streamEnded = true; buffer += decoder.decode(); consume(true); break; }
        buffer += decoder.decode(chunk.value, { stream: true }); consume(false);
        if (completion.terminal) break;
      }
    } finally {
      if (!streamEnded) await readWithDeadline(reader.cancel().catch(() => undefined), undefined, 1000);
      reader.releaseLock();
    }
    let emptyPages = 0;
    while (!completion.terminal && turnId && replay) {
      if (signal?.aborted) { const error = new Error("Aborted"); error.name = "AbortError"; throw error; }
      const page = await replayPage(replay, turnId, cursor, signal, replayTimeoutMs);
      let advanced = false;
      for (const record of eventsAfterCursor(page, turnId, cursor)) {
        if (Number(record.seq) <= cursor) continue;
        dispatch(record.payload?.type, record.payload, record.seq, true); advanced = true;
      }
      if (completion.terminal) break;
      if (advanced) { emptyPages = 0; continue; } // Drain completed turns spanning multiple pages.
      if (!page?.active) throw streamError(page?.state === "interrupted"
        ? "回合在写入完成结果前中断；已保留部分回答" : "重放结束但缺少宿主完成记录；结果尚未确认");
      await wait(Math.min(2500, 200 * (++emptyPages)), signal);
    }
    return completion.finish();
  }

  global.OwoTurnSse = Object.freeze({ parseBlock, replayPath, eventsAfterCursor, createCompletion, consumeResponse });
})(typeof window !== "undefined" ? window : globalThis);
