/* 符号检索（桌面 Web）：把 GET /symbols 的查询串与结果变成可用结构。
 *
 * 纯逻辑模块（无 DOM、无网络）；`session_id` 让检索落在**当前会话打开的项目**
 * 上（Daemon 自己的工作区未必是被打开的项目）。UI 只负责渲染。
 */
(function (global) {
  "use strict";

  /** 构造 /symbols 查询串；query 必填，其余可选。中文/空格走 encodeURIComponent。 */
  function buildQueryUri(query, options) {
    const text = String(query == null ? "" : query).trim();
    if (!text) return { ok: false, error: "请输入符号名或名字片段" };
    const opts = options || {};
    const parts = ["query=" + encodeURIComponent(text)];
    if (opts.kind) parts.push("kind=" + encodeURIComponent(String(opts.kind)));
    if (opts.path) parts.push("path=" + encodeURIComponent(String(opts.path)));
    if (opts.sessionId) parts.push("session_id=" + encodeURIComponent(String(opts.sessionId)));
    if (Number.isFinite(Number(opts.maxResults)) && Number(opts.maxResults) > 0) {
      parts.push("max_results=" + String(Math.floor(Number(opts.maxResults))));
    }
    return { ok: true, uri: "/symbols?" + parts.join("&") };
  }

  /** 结果行：`path:line`、种类与签名（截断由服务端完成）。 */
  function buildRows(payload) {
    const results = payload && Array.isArray(payload.results) ? payload.results : [];
    return results.map((row) => ({
      path: row.path,
      line: row.line,
      kind: row.kind || "unknown",
      name: row.name || "",
      signature: row.signature || "",
      label: (row.path || "?") + ":" + (row.line == null ? "?" : row.line) + "  [" + (row.kind || "unknown") + "]",
    }));
  }

  /** 结果摘要：命中数 / 扫描文件数 / 是否截断，空结果给明确文案。 */
  function summarize(payload) {
    const query = payload && payload.query ? payload.query : "";
    const rows = buildRows(payload);
    const scanned = payload && Number.isFinite(Number(payload.scanned_files)) ? Number(payload.scanned_files) : 0;
    if (!rows.length) {
      const suffix = payload && payload.file_limit_reached ? "，且已达文件数上限（结果不完整）" : "";
      return "（没有匹配“" + query + "”的定义；已扫描 " + scanned + " 个源文件" + suffix + "）";
    }
    const truncated = payload && (payload.results_truncated || payload.truncated) ? "，结果被截断" : "";
    return "符号“" + query + "”（" + rows.length + " 条，扫描 " + scanned + " 个源文件" + truncated + "）：";
  }

  global.OwoSymbolSearch = {
    buildQueryUri: buildQueryUri,
    buildRows: buildRows,
    summarize: summarize,
    _test: { buildQueryUri: buildQueryUri, buildRows: buildRows, summarize: summarize },
  };
})(typeof window !== "undefined" ? window : globalThis);
