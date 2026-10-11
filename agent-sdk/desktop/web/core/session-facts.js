(function (root) {
  "use strict";
  function fromDetail(detail) {
    const h = detail?.turn_history, active = detail?.active_turn_id;
    if (!h) return { outcome: active ? "running" : "", historyAvailable: false, lastExecutionModel: "", lastCompletionRecord: null, lastCompletionStatus: "" };
    const terminal = h.last_terminal, lastId = h.last_turn_id;
    const statsRecord = h.last_stats || (terminal?.payload?.type === "turn_stats" ? terminal : null);
    const stats = statsRecord?.turn_id === terminal?.turn_id ? statsRecord.payload : null;
    const parsed = root.OwoCompletionRecord.fromTurnStats(stats);
    const sameTurn = terminal?.turn_id === lastId;
    const payload = sameTurn ? terminal?.payload : null;
    const status = payload?.completion_record?.status || payload?.completion_status || "";
    let outcome = "";
    if (active) outcome = "running";
    else if (!sameTurn) outcome = "interrupted";
    else if (payload?.type === "turn_failed") outcome = status === "aborted" ? "cancelled" : "failed";
    else outcome = status === "aborted" ? "cancelled" : status === "blocked" ? "failed" : "completed";
    return { ...parsed, outcome, historyAvailable: true, turnId: lastId, latestSeq: h.latest_seq,
      lastCompletionRecord: sameTurn ? payload?.completion_record || null : null,
      lastCompletionStatus: status, terminal, stats, finishedAt: terminal?.created_at || "", activity: detail.activity || null };
  }
  root.OwoSessionFacts = Object.freeze({ fromDetail });
})(typeof window !== "undefined" ? window : globalThis);
