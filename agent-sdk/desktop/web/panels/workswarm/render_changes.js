// ============================================================================
// WorkSwarm 文件变更 / 变更集 / 交付清单渲染（HTML 视图构建）
//   desktop/web/panels/workswarm/render_changes.js
//
// 从 render.js 拆出的变更域纯 HTML 构建器簇：
//   变更状态徽标 / 变更列表 / 运行时变更视图（Git 记录）/ 变更集徽标 /
//   交付清单文本 / 评审历史。
//
// esc、format（fmtAbsTime/normCsStatus）、reviewBadgeHtml 与中文映射表由调用方
// 经 ctx 注入，保持浏览器（H.esc / 注入渲染器）与 Node 单测（defaultEsc）一致。
//
// 加载约定与 domain/format/render 相同：
//   - 浏览器由 index.html 预加载，挂 win.OwoWorkswarmRenderChanges；
//   - Node 单测 require 同目录模块（module.exports 导出）。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;

  // —— 自面板迁入：changeStateBadge ——
  function changeStateBadge(ctx, stateKey) {
    var k = String(stateKey || "").toLowerCase();
    return '<span class="owo-ws-badge ' + (ctx.CHG_STATE_CLS[k] || "") + '">' + ctx.esc(ctx.CHG_STATE_CN[k] || k || "—") + "</span>";
  }

  // —— 自面板迁入：changesListHtml ——
  function changesListHtml(ctx, changes) {
    var esc = ctx.esc;
    var list = (changes || []).filter(function (c) {
      return c && typeof c === "object";
    });
    if (!list.length) {
      return '<div class="hint">暂无文件变更（可写 Worker 执行前后采集 Git status/diff；只读任务无变更）。</div>';
    }
    var head = '<div class="hint">共 ' + list.length + " 个文件变更</div>";
    var rows = list
      .map(function (c) {
        var delta =
          c.added_lines == null && c.deleted_lines == null
            ? ""
            : '<span class="hint">+' + esc(String(c.added_lines == null ? 0 : c.added_lines)) + " / -" + esc(String(c.deleted_lines == null ? 0 : c.deleted_lines)) + "</span>";
        var diff =
          c.diff == null || c.diff === ""
            ? ""
            : '<details class="owo-ws-chg-diff"><summary>diff 预览</summary><pre class="owo-ws-diff">' + esc(String(c.diff)) + "</pre></details>";
        return (
          '<div class="owo-ws-chg-row">' +
          changeStateBadge(ctx, c.state) +
          "<b>" + esc(c.path || "—") + "</b>" +
          delta +
          "</div>" +
          diff
        );
      })
      .join("");
    return head + '<div class="owo-ws-chg-list">' + rows + "</div>";
  }

  // —— 自面板迁入：changesRemoteView ——
  function changesRemoteView(payload) {
    var p = payload && typeof payload === "object" ? payload : {};
    var records = (Array.isArray(p.records) ? p.records : []).map(function (r) {
      var rec = r && typeof r === "object" ? r : {};
      return {
        role: String(rec.role || "—"),
        step: String(rec.step || "unknown"),
        at: rec.at == null ? null : Number(rec.at),
        git: !!rec.git,
        changed_files: Array.isArray(rec.changed_files) ? rec.changed_files.map(String) : [],
        diff_summary: String(rec.diff_summary || ""),
        diff_ref: rec.diff_ref == null ? null : String(rec.diff_ref),
        violation: rec.violation == null ? null : String(rec.violation),
      };
    });
    return {
      team_id: p.team_id == null ? "" : String(p.team_id),
      git: !!p.git,
      changed_files: Array.isArray(p.changed_files) ? p.changed_files.map(String) : [],
      diff_summary: String(p.diff_summary || ""),
      has_violation: !!p.has_violation,
      records: records,
    };
  }

  // —— 自面板迁入：changeRecordsHtml ——
  function changeRecordsHtml(ctx, records) {
    var esc = ctx.esc;
    var list = records || [];
    if (!list.length) return "";
    var rows = list
      .map(function (r) {
        var badge = r.violation
          ? '<span class="owo-ws-badge rv-bad" title="' + esc(r.violation) + '">越界</span>'
          : '<span class="owo-ws-badge rv-ok">通过</span>';
        var files = r.changed_files.length
          ? '<div class="hint">' + r.changed_files.map(esc).join("、") + "</div>"
          : '<div class="hint">（本窗口无新增变更文件）</div>';
        var diff = r.diff_summary
          ? '<pre class="owo-ws-diff">' + esc(r.diff_summary) + "</pre>"
          : "";
        return (
          '<div class="owo-ws-chg-row">' +
          badge +
          "<b>" + esc(r.role) + "</b>" +
          '<span class="hint">步骤 ' + esc(r.step) + (r.at ? " · " + esc(ctx.fmt.fmtAbsTime(r.at)) : "") + (r.diff_ref ? " · patch " + esc(r.diff_ref) : "") + "</span>" +
          "</div>" +
          files +
          diff
        );
      })
      .join("");
    return '<div class="owo-ws-chg-list">' + rows + "</div>";
  }

  // —— 自面板迁入：changesRuntimeHtml ——
  function changesRuntimeHtml(ctx, remote, detailChanges) {
    var esc = ctx.esc;
    var parts = [];
    var r = remote && typeof remote === "object" ? remote : null;
    if (r && (r.records.length || r.has_violation || r.diff_summary)) {
      if (r.has_violation) {
        parts.push(
          '<div class="owo-pl-failures"><div class="hint err">⚠ 存在白名单越界写记录（scope_violation）：越界变更不会被登记为成功产物，请核对写角色行为。</div></div>'
        );
      }
      if (r.diff_summary) {
        parts.push('<div class="hint">最近 diff 摘要（git diff --stat）：</div><pre class="owo-ws-diff">' + esc(r.diff_summary) + "</pre>");
      }
      parts.push(changeRecordsHtml(ctx, r.records));
    }
    var files = detailChanges || [];
    if (files.length) parts.push(changesListHtml(ctx, files));
    if (!parts.length) return changesListHtml(ctx, []);
    return parts.join("");
  }

  // —— 自面板迁入：changeSetBadge ——
  function changeSetBadge(ctx, status) {
    var st = ctx.fmt.normCsStatus(status);
    var cls =
      st === "accepted"
        ? "rv-ok"
        : st === "conflicted"
          ? "rv-bad"
          : st === "pending_review"
            ? "rv-warn"
            : st === "superseded"
              ? "rv-off"
              : "";
    return '<span class="owo-ws-badge ' + cls + '">' + ctx.esc(ctx.CS_STATUS_CN[st] || st || "未知") + "</span>";
  }

  // —— 自面板迁入：deliveryManifestText ——
  function deliveryManifestText(d) {
    var payload = d && typeof d === "object" ? d : {};
    var all = Array.isArray(payload.manifest) ? payload.manifest : [];
    var items = all.filter(function (m) {
      return m && typeof m === "object";
    });
    var lines = [
      "project_id: " + String(payload.project_id || "—"),
      "generated_at: " + String(payload.generated_at || "—"),
      "artifacts: " + items.length,
      "",
    ];
    items.forEach(function (m) {
      lines.push(
        "- " + String(m.artifact_id || "?") +
          "  " + String(m.kind || "?") + "/" + String(m.format || "?") +
          "  v" + String(m.version == null ? "?" : m.version) +
          "  sha256:" + String(m.sha256 || "—") +
          "  " + String(m.size_bytes == null ? "?" : m.size_bytes) + "B" +
          "  " + (m.approved ? "已批准" : "未批准") +
          "  " + String(m.content_url || "")
      );
    });
    return lines.join("\n");
  }

  // —— 自面板迁入：artifactHistoryHtml ——
  function artifactHistoryHtml(ctx, records) {
    var esc = ctx.esc;
    if (!records || !records.length) return '<div class="hint">暂无评审记录</div>';
    return records
      .map(function (r) {
        var dec = String((r && r.decision) || "");
        var cn = { approve: "批准", request_changes: "要求修改", reject: "驳回" }[dec] || dec;
        var rid = r && (r.review_id || r.id) ? String(r.review_id || r.id) : "";
        return (
          '<div class="owo-ws-review-rec"' + (rid ? ' data-review-id="' + esc(rid) + '" data-review-decision="' + esc(dec) + '"' : "") + ">" +
          ctx.reviewBadgeHtml(dec === "approve" ? "approved" : dec === "request_changes" ? "changes_requested" : "rejected") +
          "<b>" + esc(r.reviewer || "—") + "</b>" +
          '<span class="owo-ws-ellip" title="' + esc(r.comment || "") + '">' + esc(r.comment || "（无评语）") + "</span>" +
          '<span class="hint">' + esc(r.created_at || "") + "</span>" +
          "</div>"
        );
      })
      .join("");
  }

  var api = {
    changeStateBadge: changeStateBadge,
    changesListHtml: changesListHtml,
    changesRemoteView: changesRemoteView,
    changeRecordsHtml: changeRecordsHtml,
    changesRuntimeHtml: changesRuntimeHtml,
    changeSetBadge: changeSetBadge,
    deliveryManifestText: deliveryManifestText,
    artifactHistoryHtml: artifactHistoryHtml,
  };

  win.OwoWorkswarmRenderChanges = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})();
