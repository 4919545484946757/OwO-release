/* 逐 hunk 变更审阅（桌面 Web）：把 /session/{id}/hunks 视图变成可操作的行。
 *
 * 纯逻辑模块（无 DOM、无网络）：序号解析、请求体、结果摘要都在这里，
 * 便于 node 测试；app-domain.js 只负责把行渲染成按钮。
 *
 * 序号码约定：视图里的 hunk.index 是 0 起（服务端契约），界面展示 1 起。
 */
(function (global) {
  "use strict";

  /** 1 起的序号串（"1,3" / "1 3" / "all"）→ 0 起序号数组。 */
  function parseIndices(spec, total) {
    const text = String(spec == null ? "" : spec).trim();
    if (!text) return { ok: false, error: "请填写 hunk 序号（1 起，例如 1,3）或 all" };
    if (text.toLowerCase() === "all") {
      if (!Number.isFinite(total) || total <= 0) return { ok: false, error: "当前文件没有可应用的 hunk" };
      return { ok: true, indices: Array.from({ length: total }, (_, index) => index) };
    }
    const indices = [];
    for (const token of text.split(/[,\s;]+/).filter(Boolean)) {
      const number = Number(token);
      if (!Number.isInteger(number)) return { ok: false, error: "无法解析 hunk 序号：" + token };
      if (number < 1) return { ok: false, error: "hunk 序号从 1 起（0 非法）" };
      if (Number.isFinite(total) && number > total) {
        return { ok: false, error: "hunk 序号越界：" + number + "（本文件共 " + total + " 个）" };
      }
      if (!indices.includes(number - 1)) indices.push(number - 1);
    }
    if (!indices.length) return { ok: false, error: "没有有效序号" };
    indices.sort((left, right) => left - right);
    return { ok: true, indices };
  }

  /** 请求体：只发用户指定的那一侧（两侧都发会被服务端判为语义冲突 422）。 */
  function applyBody(path, side, indices) {
    if (!path) return { ok: false, error: "缺少文件路径" };
    if (!Array.isArray(indices) || !indices.length) return { ok: false, error: "没有选择任何 hunk" };
    if (side !== "accept" && side !== "reject") return { ok: false, error: "side 只能是 accept 或 reject" };
    const body = { path: path };
    body[side] = indices;
    return { ok: true, body: body };
  }

  /** 会话 hunk 视图 → 渲染行（含冲突标记与不可审阅原因）。 */
  function buildRows(view) {
    const files = view && Array.isArray(view.files) ? view.files : [];
    const conflicts = view && Array.isArray(view.conflicts) ? view.conflicts : [];
    return files.map((file) => {
      const hunks = Array.isArray(file.hunks) ? file.hunks : [];
      return {
        path: file.path,
        kind: file.kind || "modified",
        conflict: conflicts.includes(file.path),
        reviewable: hunks.length > 0,
        reason: file.reason || (hunks.length ? "" : "不可逐 hunk 审阅"),
        hunks: hunks.map((hunk) => ({
          index: hunk.index,
          label:
            "#" +
            (Number(hunk.index) + 1) +
            " " +
            (hunk.kind || "modified") +
            " @" +
            (Number(hunk.old_start) + 1) +
            " → " +
            (Number(hunk.new_start) + 1) +
            "（-" +
            hunk.old_lines +
            " +" +
            hunk.new_lines +
            "）",
        })),
      };
    });
  }

  /** 应用结果 → 一行人类可读摘要（与服务端响应字段同形）。 */
  function summarizeApply(result) {
    if (!result || typeof result !== "object") return "（没有收到应用结果）";
    const accepted = Array.isArray(result.accepted) ? result.accepted.map((index) => index + 1) : [];
    const rejected = Array.isArray(result.rejected) ? result.rejected.map((index) => index + 1) : [];
    const remaining = Number.isFinite(Number(result.remaining)) ? Number(result.remaining) : 0;
    const changed = result.changed === false ? "（内容已一致，未写盘）" : "";
    return (
      "已应用 " +
      (result.path || "?") +
      "：接受 [" +
      accepted.join(",") +
      "] / 拒绝 [" +
      rejected.join(",") +
      "]；剩余改动 " +
      remaining +
      " 处" +
      changed
    );
  }

  global.OwoHunkReview = {
    parseIndices: parseIndices,
    applyBody: applyBody,
    buildRows: buildRows,
    summarizeApply: summarizeApply,
    _test: { parseIndices: parseIndices, applyBody: applyBody, buildRows: buildRows, summarizeApply: summarizeApply },
  };
})(typeof window !== "undefined" ? window : globalThis);
