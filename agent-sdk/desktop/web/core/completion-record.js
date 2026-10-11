(function (root) {
  "use strict";
  const statusLabels = {
    response_complete: "回答完成",
    candidate: "候选待验收",
    accepted: "验收通过",
    unverified: "未验证",
    blocked: "存在阻断",
    aborted: "已取消",
  };
  const short = (value, limit) => Array.from(String(value || "")).slice(0, limit).join("");
  function summarize(record) {
    if (!record || typeof record !== "object") return null;
    const taskId = typeof record.task_id === "string" ? record.task_id : "";
    const attemptId = typeof record.attempt_id === "string" ? record.attempt_id : "";
    const receipts = Array.isArray(record.evidence_receipt_ids)
      ? record.evidence_receipt_ids.filter((id) => typeof id === "string" && id.length > 0)
      : [];
    const candidate = typeof record.candidate_version_sha256 === "string"
      ? record.candidate_version_sha256
      : "";
    const status = statusLabels[record.status] || "状态未知";
    const details = [];
    if (taskId) details.push(["任务", taskId]);
    if (attemptId) details.push(["尝试", attemptId]);
    const shownReceipts = receipts.slice(0, 8);
    const receiptText = shownReceipts.join("、") + (receipts.length > shownReceipts.length ? ` 等 ${receipts.length} 项` : "");
    details.push(["验收回执", receiptText || "无"]);
    if (candidate) details.push(["候选 SHA-256", candidate]);
    const compact = [
      `完成记录：${status}`,
      `回执 ${receipts.length} 项`,
      candidate ? `候选 ${short(candidate, 12)}…` : "",
    ].filter(Boolean).join(" · ");
    return { status, compact, details };
  }
  function fromTurnStats(stats) {
    const value = stats && typeof stats === "object" ? stats : {};
    const calls = Array.isArray(value.model_calls) ? value.model_calls : [];
    const reported = calls.filter(call => call && typeof call.model === "string" && call.model.trim());
    return { lastExecutionModel: reported.length ? reported[reported.length - 1].model.trim() : "",
      lastCompletionStatus: typeof value.completion_status === "string" ? value.completion_status : "",
      lastCompletionRecord: value.completion_record && typeof value.completion_record === "object" ? value.completion_record : null };
  }
  function usageFromTurnStats(stats) {
    const calls = Array.isArray(stats?.model_calls) ? stats.model_calls : [];
    if (!calls.length) return null;
    const known = calls.filter(call => call?.usage && call.usage.total_tokens != null && Number.isFinite(Number(call.usage.total_tokens)));
    if (!known.length) return { text: "用量未知", known: 0, total: calls.length };
    const tokens = known.reduce((sum, call) => sum + Number(call.usage.total_tokens), 0);
    return { text: "已知 " + tokens + " Tokens · 用量覆盖 " + known.length + "/" + calls.length, known: known.length, total: calls.length };
  }
  root.OwoCompletionRecord = Object.freeze({ summarize, fromTurnStats, usageFromTurnStats });
})(globalThis);
