// ============================================================================
// WorkSwarm 详情渲染（HTML 视图构建）—— desktop/web/panels/workswarm/render.js
//
// 从 workswarm.panel.js 拆出的纯 HTML 构建器簇（四/五/六/七/八期详情区）：
//   评价徽标 / 策略盒 / 角色指标卡 / diff / 版本链时间线 / 交付物箱 /
//   工作区（目录树 + Git 状态）/ 模板盒 / 失败徽标与摘要 / Worker 能力表 /
//   写租约盒 / 文件变更列表与运行时 / 变更集徽标 / 交付清单文本 / 评审历史。
//
// 与 domain.js / format.js 相同的加载约定：
//   - 浏览器由 index.html 预加载，挂 win.OwoWorkswarmRender；
//   - Node 单测 require 同目录模块（module.exports 导出）。
// esc/short 由面板注入（bindEsc/bindShort），保持浏览器（H.esc DOM 实现）
// 与 Node 测试（defaultEsc 正则实现）的行为完全一致；未注入时回退本模块
// 自带的 defaultEsc / short（与面板实现逐字节一致）。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  var fmt = win.OwoWorkswarmFormat;
  if (!fmt && typeof require !== "undefined") fmt = require("./format.js");
  var domain = win.OwoWorkswarmDomain;
  if (!domain && typeof require !== "undefined") domain = require("./domain.js");
  if (!domain) throw new Error("WorkSwarm domain helpers 未加载");

  if (!fmt) throw new Error("WorkSwarm format helpers 未加载");

  var convo = win.OwoWorkswarmRenderConversation;
  if (!convo && typeof require !== "undefined") convo = require("./render_conversation.js");
  if (!convo) throw new Error("WorkSwarm conversation render helpers 未加载");

  var changesMod = win.OwoWorkswarmRenderChanges;
  if (!changesMod && typeof require !== "undefined") changesMod = require("./render_changes.js");
  if (!changesMod) throw new Error("WorkSwarm changes render helpers 未加载");

  /// 变更域渲染上下文：esc 为可绑定实现，fmt/reviewBadgeHtml/中文映射表来自本模块。
  function changesCtx() {
    return {
      esc: esc,
      fmt: fmt,
      reviewBadgeHtml: reviewBadgeHtml,
      CHG_STATE_CN: CHG_STATE_CN,
      CHG_STATE_CLS: CHG_STATE_CLS,
      CS_STATUS_CN: CS_STATUS_CN,
    };
  }

  // ---------- esc / short（面板注入；回退实现与面板一致） ----------
  function defaultEsc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function defaultShort(s, n) {
    s = String(s == null ? "" : s);
    return s.length > n ? s.slice(0, n) + "…" : s;
  }
  var esc = defaultEsc;
  var short = defaultShort;
  function bindEsc(fn) { esc = fn || defaultEsc; }
  function bindShort(fn) { short = fn || defaultShort; }

  // ---------- format.js 视图函数（仅本模块用到的） ----------
  var normReviewState = fmt.normReviewState;
  var fmtMs = fmt.fmtMs;
  var diffLines = fmt.diffLines;
  var failureCodeLabel = fmt.failureCodeLabel;
  var fmtAbsTime = fmt.fmtAbsTime;
  var normCsStatus = fmt.normCsStatus;
  var fmtElapsed = fmt.fmtElapsed;
  var csStatusHint = fmt.csStatusHint;
  var roleOfProducer = fmt.roleOfProducer;
  var STEP_STATUS_CN = domain.STEP_STATUS_CN;
  var normStatus = domain.normStatus;
  var isDeadStatus = domain.isDeadStatus;
  var taskBlocked = domain.taskBlocked;
  var isRetryableStep = domain.isRetryableStep;
  var shouldShowRetry = domain.shouldShowRetry;


  // ---------- 本模块私有常量（随函数从面板迁入） ----------
  var REVIEW_CN = { draft: "草稿", pendingreview: "待评审", approved: "已批准", changesrequested: "要求修改", rejected: "已驳回", superseded: "已被取代" };
  var REVIEW_CLS = { draft: "off", pendingreview: "warn", approved: "ok", changesrequested: "warn", rejected: "bad", superseded: "off" };
  var CHG_STATE_CN = { added: "新增", modified: "修改", deleted: "删除" };
  var CHG_STATE_CLS = { added: "st-running", modified: "st-awaiting_human", deleted: "st-failed" };
  var CS_STATUS_CN = {
    pending_review: "待审批",
    accepted: "已接受",
    rejected: "已拒绝",
    reverted: "已撤销",
    conflicted: "冲突",
  };

  // ================= 迁移的详情渲染函数（原面板 workswarm.panel.js） =================
    // —— 自面板迁入：reviewBadgeHtml ——
    function reviewBadgeHtml(st) {
      var k = normReviewState(st);
      var cls = REVIEW_CLS[k] || "off";
      return '<span class="owo-ws-badge rv-' + cls + '" data-art-state="' + esc(k || "unknown") + '">' + esc(REVIEW_CN[k] || String(st || "—")) + "</span>";
    }

    // —— 自面板迁入：strategyBoxHtml ——
    function strategyBoxHtml(dec) {
      if (!dec) return '<div class="hint owo-ws-strategy-empty">策略判定将由「自动选择」模式在创建时给出（单 Agent / 组队 + 角色与预算 + 理由）。</div>';
      var html =
        '<div class="owo-ws-strategy" data-strategy-mode="' + esc(dec.mode) + '">' +
        '<span class="owo-ws-badge ' + (dec.mode === "single" ? "rv-off" : "rv-warn") + '">' +
        (dec.mode === "single" ? "单 Agent" : "多 Agent 团队") + "</span>";
      if (dec.roles.length) {
        html += '<span class="hint">角色：' + esc(dec.roles.map(function (r) { return r.role; }).join(" → ")) + "</span>";
      }
      if (dec.parallelism != null) html += '<span class="hint">并行度 ' + esc(dec.parallelism) + "</span>";
      if (dec.budgetPerRole != null) html += '<span class="hint">每角色调用预算 ' + esc(dec.budgetPerRole) + "</span>";
      if (dec.reasons.length) {
        html += "<ul>" + dec.reasons.map(function (r) { return "<li>" + esc(r) + "</li>"; }).join("") + "</ul>";
      }
      return html + "</div>";
    }

    // —— 自面板迁入：metricsCardsHtml ——
    function metricsCardsHtml(vm, budgetPerRole) {
      if (!vm) return '<div class="hint owo-ws-metrics-empty">暂无角色指标（指标由角色 Worker 执行时采集；服务端未提供时此区保持空态）。</div>';
      var s = vm.summary;
      var lifecycleStages = vm.lifecycleStages || [];
      var stageLabels = {
        registry_build: "注册表构建",
        phase_orchestration: "阶段编排与执行",
        delivery_finalize: "最终交付验收",
      };
      var executionEpochs = vm.executionEpochs || [];
      var epochHtml = executionEpochs.length
        ? '<div class="owo-ws-epochs"><b>执行代次（最近 ' + executionEpochs.length + (vm.executionEpochsTruncated ? '+': '') + '）</b>' + executionEpochs.slice().reverse().map(function (epoch) {
            var tokenText = epoch.promptTokens != null || epoch.completionTokens != null
              ? ' · token ' + (epoch.usageKnown ? "" : "≥") + (epoch.promptTokens == null ? "—" : epoch.promptTokens) + '/' + (epoch.completionTokens == null ? "—" : epoch.completionTokens)
              : "";
            var overlapText = epoch.overlapFactor != null ? ' · Worker时长/窗口 ' + epoch.overlapFactor.toFixed(2) + 'x' : "";
            return '<span><b>epoch ' + esc(epoch.phaseEpoch) + '</b>' +
              (epoch.phaseDurationMs != null ? ' · 编排 ' + fmtMs(epoch.phaseDurationMs) : '') +
              ' · Worker窗口 ' + fmtMs(epoch.workerWindowMs) +
              ' · Worker累计 ' + fmtMs(epoch.workerWallMsSum) +
              ' · 任务尝试 ' + (epoch.taskAttemptCount == null ? 0 : epoch.taskAttemptCount) +
              ' · 调用 ' + (epoch.modelCalls == null ? 0 : epoch.modelCalls) + tokenText +
              ' · 失败 ' + (epoch.failedSpans == null ? 0 : epoch.failedSpans) + overlapText + '</span>';
          }).join("") + '</div>'
        : "";
      var lifecycleHtml = lifecycleStages.length
        ? '<div class="owo-ws-lifecycle"><b>运行阶段耗时</b>' + lifecycleStages.map(function (stage) {
            var label = stageLabels[stage.stage] || stage.stage;
            return '<span title="次数 ' + (stage.count == null ? "—" : stage.count) + '，失败 ' + (stage.failures == null ? 0 : stage.failures) + '">' + esc(label) + ' <b>P50 ' + fmtMs(stage.durationMsP50) + ' / P90 ' + fmtMs(stage.durationMsP90) + '</b></span>';
          }).join("") + '</div>'
        : "";
      var contextAssembly = vm.contextAssembly || {};
      var contextRoles = contextAssembly.byRole || [];
      var contextEpochs = contextAssembly.byEpoch || [];
      var contextSamplingNote = contextAssembly.sampleTruncated || contextAssembly.auditScanTruncated
        ? ' · 最近样本，统计可能不完整'
        : "";
      var contextAssemblyVisible = contextAssembly.sampleCount > 0 || contextAssembly.attributionMissingCount > 0;
      var contextAssemblyHtml = contextAssemblyVisible
        ? '<div class="owo-ws-lifecycle"><b>上下文装配（Worker 内部耗时，不与墙钟相加）</b>' +
          '<span>样本 ' + (contextAssembly.sampleCount || 0) + contextSamplingNote +
          ' · P50 ' + fmtMs(contextAssembly.durationMsP50) +
          ' / P90 ' + fmtMs(contextAssembly.durationMsP90) +
          ' · 失败 ' + (contextAssembly.failures || 0) +
          ((contextAssembly.attributionMissingCount || 0) > 0 ? ' · 未归因 ' + contextAssembly.attributionMissingCount : '') + '</span>' +
          contextRoles.map(function (role) {
            return '<span title="样本 ' + (role.sampleCount || 0) + '，失败 ' + (role.failures || 0) +
              '，累计工作时长不代表墙钟">' + esc(role.role) + ' <b>P50 ' +
              fmtMs(role.durationMsP50) + ' / P90 ' + fmtMs(role.durationMsP90) + '</b></span>';
          }).join("") +
          contextEpochs.slice(-12).map(function (epoch) {
            return '<span title="样本 ' + (epoch.sampleCount || 0) + '，失败 ' + (epoch.failures || 0) +
              '">epoch ' + esc(epoch.phaseEpoch) + ' 上下文 P50 ' +
              fmtMs(epoch.durationMsP50) + ' / P90 ' + fmtMs(epoch.durationMsP90) + '</span>';
          }).join("") + '</div>'
        : "";
      var chips =
        '<div class="owo-ws-metrics-sum">' +
        '<span>总墙钟 <b>' + fmtMs(s.wallClockMs) + "</b></span>" +
        '<span>总调用 <b>' + (s.totalModelCalls == null ? "—" : s.totalModelCalls) + "</b></span>" +
        (s.totalBudgetReservationWaitMs != null ? '<span>预算预留等待 <b>' + fmtMs(s.totalBudgetReservationWaitMs) + "</b></span>" : "") +
        (s.totalTokensIn != null || s.totalTokensOut != null
          ? "<span>token <b>" + (s.totalTokensIn == null ? "—" : s.totalTokensIn) + " / " + (s.totalTokensOut == null ? "—" : s.totalTokensOut) + "</b></span>"
          : "") +
        (s.totalEstCost != null ? '<span>估算费用 <b>$' + s.totalEstCost.toFixed(4) + "</b></span>" : "") +
        (s.slowestWorker ? '<span>最慢 <b>' + esc(s.slowestWorker) + "</b></span>" : "") +
        (s.failures != null ? '<span>失败 <b class="' + (s.failures > 0 ? "bad" : "") + '">' + s.failures + "</b></span>" : "") +
        (s.reworks != null ? "<span>返工 <b>" + s.reworks + "</b></span>" : "") +
        (s.artifactVersions != null ? "<span>产物版本 <b>" + s.artifactVersions + "</b></span>" : "") +
        (s.budgetExhausted ? '<span class="owo-ws-budget-exhausted bad">预算耗尽' + (s.budgetReason ? "：" + esc(s.budgetReason) : "") + "</span>" : "") +
        "</div>";
      if (!vm.workers.length) return chips + epochHtml + lifecycleHtml + contextAssemblyHtml;
      var cards = vm.workers
        .map(function (w) {
          var remain = budgetPerRole != null && w.modelCalls != null ? budgetPerRole - w.modelCalls : null;
          return (
            '<div class="owo-ws-mcard' + (w.failureReason ? " bad" : "") + '">' +
            "<div><b>" + esc(w.role || w.worker) + "</b>" +
            (w.worker && w.role && w.worker !== w.role ? '<span class="hint">（' + esc(w.worker) + "）</span>" : "") +
            (w.terminal ? '<span class="owo-ws-badge ' + (w.failureReason || /fail|abort|cancel/i.test(String(w.terminal)) ? "rv-bad" : "rv-ok") + '">' + esc(w.terminal) + "</span>" : "") +
            "</div>" +
            '<div class="hint">' +
            "耗时 " + fmtMs(w.durationMs) +
            (w.budgetReservationWaitMs != null ? " · 预算预留等待 " + fmtMs(w.budgetReservationWaitMs) : "") +
            " · 调用 " + (w.modelCalls == null ? "—" : w.modelCalls) +
            (remain != null ? "（余 " + remain + "" : "") + (remain != null ? "）" : "") +
            (w.tokensIn != null || w.tokensOut != null ? " · token " + (w.tokensIn == null ? "—" : w.tokensIn) + "/" + (w.tokensOut == null ? "—" : w.tokensOut) : "") +
            (w.estCost != null ? " · $" + w.estCost.toFixed(4) : "") +
            (w.attempts != null ? " · 次数 " + w.attempts : "") +
            (w.taskId ? " · task " + esc(w.taskId) : "") +
            (w.attemptId ? " · attempt " + esc(w.attemptId) : "") +
            (w.phaseEpoch != null ? " · epoch " + esc(w.phaseEpoch) : "") +
            "</div>" +
            (w.failureReason ? '<div class="bad hint">失败：' + esc(short(w.failureReason, 120)) + "</div>" : "") +
            (w.artifactIds.length ? '<div class="hint">产物：' + esc(w.artifactIds.join("、")) + "</div>" : "") +
            "</div>"
          );
        })
        .join("");
      return chips + epochHtml + lifecycleHtml + contextAssemblyHtml + '<div class="owo-ws-mgrid">' + cards + "</div>";
    }

    // —— 自面板迁入：diffHtml ——
    function diffHtml(aText, bText, labelA, labelB) {
      var rows = diffLines(aText, bText)
        .map(function (r) {
          if (r.t === " ") return '<span class="dl-ctx">  ' + esc(r.s) + "\n</span>";
          if (r.t === "-") return '<span class="dl-del">- ' + esc(r.s) + "\n</span>";
          return '<span class="dl-add">+ ' + esc(r.s) + "\n</span>";
        })
        .join("");
      return (
        '<pre class="owo-ws-diff" data-diff-a="' + esc(labelA || "vA") + '" data-diff-b="' + esc(labelB || "vB") + '">' +
        '<span class="hint">差异 ' + esc(labelA || "vA") + " → " + esc(labelB || "vB") + "（- 删行 / + 增行）</span>\n" + rows + "</pre>"
      );
    }

    // —— 自面板迁入：artifactTimelineHtml ——
    function artifactTimelineHtml(chain) {
      if (!chain || !chain.length) return "";
      var nodes = chain
        .map(function (a) {
          var st = normReviewState(a.review_state);
          return (
            '<span class="owo-ws-tl-node" data-tl-state="' + esc(st) + '" title="' + esc(a.artifact_id) + '">' +
            "v" + (a.version == null ? "?" : a.version) +
            '<i class="owo-ws-badge ' + (REVIEW_CLS[st] || "rv-off") + '">' + (REVIEW_CN[st] || st || "—") + "</i></span>"
          );
        })
        .join('<span class="owo-ws-tl-arrow">→</span>');
      return '<div class="owo-ws-timeline" data-timeline-len="' + chain.length + '">' + nodes + "</div>";
    }

    // —— 自面板迁入：deliverablesBoxHtml ——
    function deliverablesBoxHtml(dl) {
      if (!dl) return '<div class="hint">交付物数据为空。</div>';
      var html = "";
      if (!dl.approved.length && !dl.pending.length && !dl.other.length) {
        return '<div class="hint">项目暂无交付物（尚未有产物通过评审或进入评审）。</div>';
      }
      html += '<div class="owo-ws-dlv-counts"><span class="owo-ws-badge rv-ok">已批准 ' + dl.approved.length + "</span>" +
        '<span class="owo-ws-badge rv-warn">待评审 ' + dl.pending.length + "</span>" +
        '<span class="owo-ws-badge rv-off">驳回/取代 ' + dl.other.length + "</span>" +
        (dl.complete ? '<span class="owo-ws-badge rv-ok">交付完成</span>' : "") +
        (dl.reworkCount != null && dl.reworkCount > 0 ? '<span class="owo-ws-badge rv-warn">返工中 ' + dl.reworkCount + "</span>" : "") +
        "</div>";
      if (dl.manifestRef) {
        html += '<div class="hint">交付清单：' + esc(dl.manifestRef) + "</div>";
      }
      if (dl.approved.length) {
        html +=
          '<div class="owo-ws-dlv-list">' +
          dl.approved
            .map(function (x) {
              return (
                '<div class="owo-ws-dlv-item" data-dlv-art="' + esc(x.artifactId) + '">' +
                '<span class="owo-ws-badge rv-ok">交付</span> <b>' + esc(x.kind || "artifact") + "</b> v" + (x.version == null ? "?" : x.version) +
                '<span class="hint">' + esc(x.artifactId) + " · " + esc(x.producer || "—") + "</span></div>"
              );
            })
            .join("") +
          "</div>";
      } else {
        html += '<div class="hint">尚无已批准版本：approved head 建立后将在此列出最终交付物。</div>';
      }
      return html;
    }

    // —— 自面板迁入：workspaceBoxHtml ——
    function workspaceBoxHtml(ws, view, viewKind, busy) {
      if (!ws) {
        return '<div class="hint">未绑定项目工作区（该团队使用服务端默认工作区）。新建项目任务面板可绑定真实目录。</div>';
      }
      var html = '<div class="owo-pl-wsbox">';
      html += "<div><b>工作区</b> <code>" + esc(ws.root) + "</code></div>";
      html += "<div><b>模式</b> " + (ws.readOnly ? "只读（默认）" : "受控写入") +
        (ws.writePaths && ws.writePaths.length ? '<span class="hint">（允许路径：' + esc(ws.writePaths.join("、")) + "）</span>" : ws.readOnly ? "" : '<span class="hint">（未列允许路径：写入将被拒绝）</span>') + "</div>";
      if (ws.treeDepth != null) html += "<div><b>目录树深度</b> " + esc(String(ws.treeDepth)) + "</div>";
      html += '<div class="owo-ws-inline">' +
        '<button class="owo-ws-mini" id="ws-ws-tree"' + (busy ? " disabled" : "") + ">目录树</button>" +
        '<button class="owo-ws-mini" id="ws-ws-git"' + (busy ? " disabled" : "") + ">Git 状态</button>" +
        "</div>";
      if (view && viewKind === "tree") html += workspaceTreeHtml(view);
      if (view && viewKind === "git") html += gitStatusHtml(view);
      html += "</div>";
      return html;
    }

    // —— 自面板迁入：workspaceTreeHtml ——
    function workspaceTreeHtml(payload) {
      if (!payload || typeof payload !== "object") return "";
      var entries = Array.isArray(payload.entries) ? payload.entries : [];
      if (!entries.length) return '<div class="hint">目录为空（或深度内无条目）。</div>';
      var rows = entries.map(function (e) {
        var path = String((e && e.path) || "");
        var isDir = (e && e.type) === "dir";
        var depth = path.split(/[\\/]/).length - 1;
        var pad = depth > 0 ? ' style="padding-left:' + Math.min(depth, 8) * 14 + 'px"' : "";
        return '<div' + pad + ">" + (isDir ? "📁" : "📄") + " " + esc(path.split(/[\\/]/).pop() || path) +
          (isDir ? '<span class="hint">/</span>' : (e && e.size != null ? '<span class="hint"> ' + esc(String(e.size)) + "B</span>" : "")) + "</div>";
      }).join("");
      return '<div class="owo-pl-tree">' + rows + "</div>";
    }

    // —— 自面板迁入：gitStatusHtml ——
    function gitStatusHtml(payload) {
      if (!payload || typeof payload !== "object") return "";
      if (payload.is_git_repo === false || payload.git === false)
        return '<div class="hint">该目录不是 Git 仓库。</div>';
      var rawEntries = Array.isArray(payload.entries) ? payload.entries : [];
      var rows = [];
      var count = 0;
      rawEntries.slice(0, 50).forEach(function (e) {
        var path, state;
        if (typeof e === "string") {
          // porcelain 行：" M path" / "?? path" / "MM path"
          var m = e.match(/^(\S+)\s+(.*)$/);
          state = m ? m[1] : "";
          path = m ? m[2] : e;
        } else {
          path = String((e && e.path) || "");
          state = String((e && e.state) || "");
        }
        if (!path) return;
        count++;
        rows.push('<div><code>' + esc(path) + '</code> <span class="owo-pl-badge">' + esc(state || "?") + "</span></div>");
      });
      var more = rawEntries.length > 50 ? '<div class="hint">…其余 ' + (rawEntries.length - 50) + " 项略</div>" : "";
      var branch = payload.branch ? "<div><b>分支</b> " + esc(String(payload.branch)) + "</div>" : "";
      return '<div class="owo-pl-tree">' + branch +
        "<div><b>状态</b> " + (count === 0 ? "干净（无未提交变更）" : "有变更 " + count + " 项") + "</div>" + rows.join("") + more + "</div>";
    }

    // —— 自面板迁入：templateBoxHtml ——
    function templateBoxHtml(team, templateInfo) {
      var tid = team && typeof team === "object" ? String(team.template_id || "") : "";
      if (!tid) {
        return '<div class="hint">动态组队（未使用模板）——角色由组队策略判定，理由见下方策略区。</div>';
      }
      var v = templateInfo && templateInfo.template_id === tid && templateInfo.version != null
        ? " v" + esc(String(templateInfo.version))
        : "";
      var title = templateInfo && templateInfo.template_id === tid && templateInfo.title
        ? esc(String(templateInfo.title)) + "（" + esc(tid) + "）"
        : esc(tid);
      // 交付门禁/写权限失败时，用户需要回到工作流本身去修配置（检查清单 + 一键修复）。
      // 这里给出直达入口，避免"报错在两处、修在哪不知道"。
      return (
        '<div><b>模板</b> ' + title + v +
        '<span class="hint">（按保存的自定义节点、依赖与预算组队）</span> ' +
        '<button type="button" class="owo-ws-mini" data-core-action data-open-workflows="' +
        esc(tid) + '" title="打开自定义工作流面板查看检查清单与一键修复">查看该工作流</button></div>'
      );
    }

    // —— 自面板迁入：failureBadgeHtml ——
    function failureBadgeHtml(task) {
      if (!task) return "";
      var st = String(task.status || "");
      if (st !== "Failed" && st !== "Aborted") return "";
      var code = String(task.failure_code || "");
      var label = failureCodeLabel(code);
      if (!label) {
        var err = String(task.error || "");
        if (/output_contract_invalid/i.test(err)) code = "output_contract_invalid";
        else if (/artifact_missing/i.test(err)) code = "artifact_missing";
        else if (/scope_violation/i.test(err)) code = "scope_violation";
        label = failureCodeLabel(code);
      }
      return label ? '<span class="owo-pl-badge bad" title="失败原因代码：' + esc(code) + '">' + esc(label) + "</span>" : "";
    }

    // —— 自面板迁入：failureSummaryHtml ——
    function failureSummaryHtml(tasks) {
      var list = (Array.isArray(tasks) ? tasks : []).filter(function (t) {
        return t && (t.status === "Failed" || t.status === "Aborted");
      });
      if (!list.length) return "";
      var rows = list.map(function (t) {
        return "<div>" + esc(String(t.task_id || "")) + " " + failureBadgeHtml(t) +
          (t.error ? '<span class="hint"> ' + esc(String(t.error).slice(0, 120)) + "</span>" : "") + "</div>";
      }).join("");
      return '<div class="owo-pl-failures"><b>失败原因</b>' + rows + "</div>";
    }

    // —— 自面板迁入：validationBadgeHtml ——
    function validationBadgeHtml(validation) {
      var v = validation && typeof validation === "object" ? validation : null;
      if (!v || v.valid == null) return "";
      if (v.valid) return '<span class="owo-ws-badge rv-ok" title="格式校验通过">校验通过</span>';
      return '<span class="owo-ws-badge rv-bad" title="' + esc(String(v.reason || "格式校验未通过")) + '">校验未通过</span>';
    }

    // —— 自面板迁入：workerProfilesTable ——
    function workerProfilesTable(profiles) {
      var list = (profiles || []).filter(function (p) {
        return p && typeof p === "object";
      });
      if (!list.length) {
        return '<div class="hint">暂无 WorkerProfile（权限/预算字段未下发或团队尚未生成角色配置）。</div>';
      }
      var rows = list
        .map(function (p) {
          var tools = Array.isArray(p.visible_tools) && p.visible_tools.length ? p.visible_tools.join("、") : "—";
          var paths = Array.isArray(p.write_allowed_paths) && p.write_allowed_paths.length
            ? p.write_allowed_paths.join("、")
            : p.read_only
              ? "—"
              : "未声明（写入将被拒绝）";
          return (
            "<tr>" +
            "<td><b>" + esc(p.role || "—") + "</b></td>" +
            "<td>" + (p.read_only ? "只读" : "可写") + "</td>" +
            '<td class="hint">' + esc(tools) + "</td>" +
            "<td>" + (p.can_run_command ? "✓" : "✗") + "</td>" +
            "<td>" + (p.can_use_browser ? "✓" : "✗") + "</td>" +
            "<td>" + esc(p.max_turns == null ? "—" : String(p.max_turns)) + "</td>" +
            '<td class="hint">' + esc(paths) + "</td>" +
            "</tr>"
          );
        })
        .join("");
      return (
        '<table class="owo-ws-table"><tr><th>角色</th><th>读写</th><th>可见工具</th><th>命令</th><th>浏览器</th><th>最大轮次（预算）</th><th>允许写路径</th></tr>' +
        rows +
        "</table>"
      );
    }

    // —— 自面板迁入：writeLeaseBox ——
    function writeLeaseBox(lease) {
      if (!lease || typeof lease !== "object") {
        return '<div class="hint">独立输出范围可并行写入；范围重叠时协调执行。</div>';
      }
      var head =
        '<span class="owo-ws-badge st-running">写租约持有中</span>' +
        "<b>" + esc(lease.holder_role || "—") + "</b>" +
        '<span class="hint">步骤 ' + esc(lease.holder_step_id || "—") + "</span>";
      if (lease.released_at_ms != null) {
        return '<div class="owo-ws-lease">' + head + '<span class="hint">已于 ' + esc(fmtAbsTime(lease.released_at_ms)) + " 释放</span></div>";
      }
      return (
        '<div class="owo-ws-lease">' +
        head +
        (lease.acquired_at_ms != null ? '<span class="hint">自 ' + esc(fmtAbsTime(lease.acquired_at_ms)) + " 起持有</span>" : "") +
        "</div>"
      );
    }

    // —— 自面板迁入：changeStateBadge（实现见 render_changes.js） ——
    function changeStateBadge(stateKey) {
      return changesMod.changeStateBadge(changesCtx(), stateKey);
    }

    // —— 自面板迁入：changesListHtml（实现见 render_changes.js） ——
    function changesListHtml(changes) {
      return changesMod.changesListHtml(changesCtx(), changes);
    }

    // —— 自面板迁入：changesRemoteView（实现见 render_changes.js） ——
    function changesRemoteView(payload) {
      return changesMod.changesRemoteView(payload);
    }

    // —— 自面板迁入：changeRecordsHtml（实现见 render_changes.js） ——
    function changeRecordsHtml(records) {
      return changesMod.changeRecordsHtml(changesCtx(), records);
    }

    // —— 自面板迁入：changesRuntimeHtml（实现见 render_changes.js） ——
    function changesRuntimeHtml(remote, detailChanges) {
      return changesMod.changesRuntimeHtml(changesCtx(), remote, detailChanges);
    }

    // —— 自面板迁入：changeSetBadge（实现见 render_changes.js） ——
    function changeSetBadge(status) {
      return changesMod.changeSetBadge(changesCtx(), status);
    }

    // —— 自面板迁入：deliveryManifestText（实现见 render_changes.js） ——
    function deliveryManifestText(d) {
      return changesMod.deliveryManifestText(d);
    }

    // —— 自面板迁入：artifactHistoryHtml（实现见 render_changes.js） ——
    function artifactHistoryHtml(records) {
      return changesMod.artifactHistoryHtml(changesCtx(), records);
    }
  // ---------- DAG / 实时进度渲染（第三轮自面板迁入） ----------

    function dagSvg(tasks, teamStatus) {
      var showRetry = shouldShowRetry(tasks, teamStatus || "");
      var byId = {};
      tasks.forEach(function (t) {
        byId[t.task_id] = t;
      });
      var levelCache = {};
      function level(id) {
        if (levelCache[id] != null) return levelCache[id];
        var t = byId[id];
        if (!t) return 0;
        levelCache[id] = 0; // 环保护
        var lv = 0;
        var deps = t.depends_on || [];
        for (var i = 0; i < deps.length; i++) {
          if (byId[deps[i]]) {
            var d = level(deps[i]) + 1;
            if (d > lv) lv = d;
          }
        }
        levelCache[id] = lv;
        return lv;
      }
      var maxL = 0;
      tasks.forEach(function (t) {
        var l = level(t.task_id);
        if (l > maxL) maxL = l;
      });
      var cols = {};
      tasks.forEach(function (t) {
        var l = level(t.task_id);
        (cols[l] = cols[l] || []).push(t);
      });
      var NW = 210;
      var NH = 100;
      var GX = 60;
      var GY = 40;
      var PAD = 12;
      var rows = 0;
      for (var l = 0; l <= maxL; l++) rows = Math.max(rows, (cols[l] || []).length);
      var W = PAD * 2 + (maxL + 1) * NW + maxL * GX;
      var Hh = PAD * 2 + Math.max(rows, 1) * NH + Math.max(rows - 1, 0) * GY;
      var pos = {};
      for (var l2 = 0; l2 <= maxL; l2++) {
        (cols[l2] || []).forEach(function (t, i) {
          pos[t.task_id] = { x: PAD + l2 * (NW + GX), y: PAD + i * (NH + GY) };
        });
      }
      var edges = "";
      tasks.forEach(function (t) {
        (t.depends_on || []).forEach(function (dep) {
          if (!byId[dep] || !pos[dep] || !pos[t.task_id]) return;
          var dead = isDeadStatus(byId[dep].status);
          var s = pos[dep];
          var e2 = pos[t.task_id];
          edges +=
            '<line x1="' + s.x + NW + '" y1="' + (s.y + NH / 2) + '" x2="' + (e2.x - 5) + '" y2="' + (e2.y + NH / 2) + '" marker-end="url(#ws-arrow' + (dead ? "-dead" : "") + ')"' + (dead ? ' class="dead"' : "") + "></line>";
        });
      });
      var nodes = "";
      tasks.forEach(function (t) {
        var p = pos[t.task_id];
        if (!p) return;
        var st = normStatus(t.status);
        var blocked = taskBlocked(t, byId);
        var inner =
          '<div class="owo-ws-node st-' + st + (blocked ? " blocked" : "") + '">' +
          '<div class="owo-ws-node-line"><b>' + esc(t.role || t.worker || t.task_id) + "</b>" +
          '<span class="chip">' + (STEP_STATUS_CN[st] || esc(t.status)) + (blocked ? " · 已阻塞" : "") + "</span></div>" +
          '<div class="hint">worker ' + esc(t.worker || "—") + (t.attempts ? " · 第 " + t.attempts + " 次" : "") + "</div>" +
          (t.error
            ? '<div class="owo-ws-node-err" title="' + esc(t.error) + '">' + esc(short(t.error, 70)) + "</div>"
            : '<div class="hint">&nbsp;</div>') +
          // R2 冻结契约入口：仅 Failed/Aborted 节点给出「重试此节点」；
          // 提交体由 buildRetryBody 构造，提交锁/去重在 handleRetryClick。
          (showRetry && isRetryableStep(t)
            ? '<div class="owo-ws-node-act"><button type="button" class="owo-ws-mini owo-ws-retry" data-ws-retry="' +
              esc(t.task_id) +
              '" aria-label="重试此节点 ' + esc(t.role || t.worker || t.task_id) + '">↻ 重试此节点</button></div>'
            : "") +
          "</div>";
        nodes +=
          '<foreignObject x="' + p.x + '" y="' + p.y + '" width="' + NW + '" height="' + NH + '">' + inner + "</foreignObject>";
      });
      return (
        '<svg class="owo-ws-dag" width="' + W + '" height="' + Hh + '">' +
        "<defs>" +
        '<marker id="ws-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="#8899aa"></path></marker>' +
        '<marker id="ws-arrow-dead" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="#e05555"></path></marker>' +
        "</defs>" +
        "<g>" + edges + "</g>" +
        nodes +
        "</svg>"
      );
    }

    function progressCountChip(label, n, cls) {
      return '<span class="owo-ws-prog-count' + (cls ? " " + cls : "") + '">' + esc(label) + " <b>" + esc(n) + "</b></span>";
    }

    function renderProgress(vm) {
      if (!vm) return '<div class="hint owo-ws-prog-empty">暂无实时进度事件（等待 progress 推送…）</div>';
      var c = vm.counts || {};
      var html =
        '<div class="owo-ws-prog-head">' +
        (vm.seq != null ? '<span class="owo-ws-prog-seq" title="最新 progress 事件序号（单调递增，断线恢复依据）">seq #' + esc(vm.seq) + "</span>" : "") +
        progressCountChip("等待", c.pending || 0, "off") +
        progressCountChip("运行", c.running || 0, "run") +
        progressCountChip("完成", c.succeeded || 0, "ok") +
        progressCountChip("失败", c.failed || 0, "bad") +
        (vm.cancelling ? '<span class="owo-ws-badge st-interrupted owo-ws-prog-cancelling">取消中…（已下发，等待执行器停止）</span>' : "") +
        "</div>";
      if (!vm.rows.length) {
        return html + '<div class="hint">当前无活动步骤</div>';
      }
      html += '<div class="owo-ws-prog-steps">';
      for (var i = 0; i < vm.rows.length; i++) {
        var r = vm.rows[i];
        html +=
          '<div class="owo-ws-prog-step' + (r.running ? " run" : "") + '">' +
          '<span class="owo-ws-mono owo-ws-ellip" title="' + esc(r.step_id) + '">' + esc(r.worker || r.step_id) + "</span>" +
          '<span class="owo-ws-badge">' + esc(r.statusCn) + "</span>" +
          '<span class="hint">第 ' + esc(r.attempts) + " 次尝试</span>" +
          '<span class="owo-ws-prog-elapsed" title="自 started_at 起的已运行时间">' +
          (r.running ? "已运行 " : "耗时 ") + esc(r.elapsedMs == null ? "—" : fmtElapsed(r.elapsedMs)) +
          "</span>" +
          "</div>";
      }
      html += "</div>";
      return html;
    }

    // ---------- ChangeSet 审批区 / Artifact 产物行（视图模型化，九期迁入） ----------
    // vm 参数由面板从 state 切片传入：vm = { results, busy, approvalBlock }（changeSetsHtml）
    // 与 vm = { reviewBusy, reworkBusy, reviewFlash, approvalBlock }（artifactRowHtml）。
    // 纯函数只依赖参数与 fmt/domain/esc/short，Node 测试可直接构造 vm 而不依赖面板 state。
    function approvalBlockBanner(approvalBlock) {
      if (!approvalBlock || !approvalBlock.blocked) return "";
      return (
        '<div class="hint" data-cs-approval-block="1">⚠ 批准被门控阻断：' +
        esc(approvalBlock.reason || "存在未处理的 ChangeSet（待审批/冲突）——先接受或拒绝后才能批准 Artifact") +
        "</div>"
      );
    }

    function changeSetsHtml(list, vm) {
      vm = vm || {};
      var results = vm.results || {};
      var busy = vm.busy || {};
      var approvalBlock = vm.approvalBlock || null;
      list = list || [];
      if (!list.length) {
        return '<div class="hint">暂无 ChangeSet（写角色执行后自动生成；端点未上线时本区保持空态）。</div>';
      }
      var rows = list
        .map(function (c) {
          var st = results[c.change_set_id];
          // 九期：conflicted 同样提供动作（恢复被拒绝后人工处理完可重试决定）。
          var actionable = c.status === "pending_review" || c.status === "conflicted";
          var actions = "";
          if (actionable) {
            actions = [
              ["accept", "接受"],
              ["reject", "拒绝"],
              ["revert", "撤销"],
            ]
              .map(function (pair) {
                var b = busy[c.change_set_id + ":" + pair[0]];
                return (
                  '<button type="button" class="owo-ws-mini" data-core-action data-cs-act="' + pair[0] +
                  '" data-cs-id="' + esc(c.change_set_id) + '"' + (b ? " disabled" : "") +
                  ">" + pair[1] + "</button>"
                );
              })
              .join("");
          }
          var statusHint = csStatusHint(c.status);
          return (
            '<div class="owo-ws-chg-row">' +
            changeSetBadge(c.status) +
            "<b><code>" + esc(c.change_set_id) + "</code></b>" +
            '<span class="hint">' + esc(c.role || "—") + " · 步骤 " + esc(c.step_id || "—") +
            (c.created_at ? " · " + esc(String(c.created_at).replace("T", " ").slice(0, 19)) : "") +
            (c.diff_ref ? " · patch " + esc(c.diff_ref) : "") + "</span>" +
            (statusHint ? '<span class="hint" data-cs-status-hint="' + esc(c.status) + '">（' + esc(statusHint) + "）</span>" : "") +
            "</div>" +
            (c.changed_files.length
              ? '<div class="hint">' + c.changed_files.map(esc).join("、") + "</div>"
              : '<div class="hint">（无变更文件清单）</div>') +
            (c.status === "conflicted" && c.conflicts.length
              ? '<div class="hint" data-cs-conflicts="' + esc(c.change_set_id) + '">⚠ 冲突文件（用户已修改，恢复未覆盖新内容）：' + c.conflicts.map(esc).join("、") + "</div>"
              : "") +
            (actions ? '<div class="owo-ac-actions">' + actions + "</div>" : "") +
            '<div class="owo-ac-result' + (st ? (st.ok ? " ok" : " bad") : "") +
            '" data-cs-result="' + esc(c.change_set_id) + '" aria-live="polite">' +
            (st ? esc(st.text) : "") +
            "</div>"
          );
        })
        .join("");
      // 九期：门控横幅置顶（blocked 时 Artifact 批准按钮同步禁用）。
      var banner = approvalBlockBanner(approvalBlock);
      return '<div class="owo-ws-chg-list">' + banner + rows + "</div>";
    }

    // 产物行（链内）：版本徽标 + 评审状态 + 产出者 + 取代关系 + 预览 + 评审表单 + 历史。
    // 视图模型 vm = { reviewBusy, reworkBusy, reviewFlash, approvalBlock }：
    // 评审表单出现在 PendingReview；返工表单出现在链内最新 Draft/Rejected。
    function artifactRowHtml(a, chain, vm) {
      vm = vm || {};
      var reviewBusy = vm.reviewBusy || {};
      var approvalBlock = vm.approvalBlock || null;
      var reworkBusy = vm.reworkBusy || {};
      var reviewFlash = vm.reviewFlash || null;
      var aid = String(a.artifact_id == null ? "" : a.artifact_id);
      var busy = !!reviewBusy[aid];
      var isHead = chain && chain.items[chain.items.length - 1] === a;
      var sup = a.supersedes_artifact_id == null ? "" : String(a.supersedes_artifact_id);
      var supVer = "";
      if (sup) {
        for (var i = 0; i < (chain ? chain.items : []).length; i++) {
          if (String(chain.items[i].artifact_id) === sup) supVer = "v" + chain.items[i].version;
        }
      }
      var formHtml = "";
      if (normReviewState(a.review_state) === "pendingreview") {
        // 九期：ChangeSet 未处理（pending_review/conflicted）时批准被门控阻断——
        // 仅禁用「批准」，要求修改/驳回不受影响（服务端 approve 同样拒绝并给原因）。
        var block = approvalBlock && approvalBlock.blocked;
        var blockReason = block ? String(approvalBlock.reason || "") : "";
        formHtml =
          '<details class="owo-ws-review"' + (busy ? ' data-busy="1"' : "") + ">" +
          '<summary>评审此版本（批准 / 要求修改 / 驳回）</summary>' +
          '<div class="owo-ws-review-form">' +
          '<input class="owo-ws-review-reviewer" placeholder="评审者：critic 或 human 用户名（生产者不能自行批准）">' +
          '<textarea class="owo-ws-review-comment" rows="2" placeholder="评语（随不可变评审记录保存）"></textarea>' +
          (block
            ? '<div class="hint" data-art-approve-blocked="' + esc(aid) + '">⚠ ChangeSet 未处理，批准暂不可用：' +
              esc(blockReason || "存在待审批/冲突的 ChangeSet，先在「ChangeSet 审批」区接受或拒绝") + "</div>"
            : "") +
          '<div class="owo-ws-review-actions">' +
          '<button type="button" class="owo-ws-review-act ok" data-core-action data-art-act="approve" data-art-id="' + esc(aid) + '"' +
          (busy || block ? " disabled" : "") +
          (block ? ' title="ChangeSet 未处理：批准被门控阻断"' : "") + ">批准</button>" +
          '<button type="button" class="owo-ws-review-act warn" data-core-action data-art-act="request_changes" data-art-id="' + esc(aid) + '"' + (busy ? " disabled" : "") + ">要求修改</button>" +
          '<button type="button" class="owo-ws-review-act bad" data-core-action data-art-act="reject" data-art-id="' + esc(aid) + '"' + (busy ? " disabled" : "") + ">驳回</button>" +
          "</div>" +
          "</div></details>";
      }
      // 五期：返工表单——链内最新版本且状态为 Draft（被要求修改后）或 Rejected 时提供
      // 「根据评审意见返工」；展开时懒加载评审历史预填 review_id 与指令。
      var reworkHtml = "";
      var rowState = normReviewState(a.review_state);
      if (isHead && (rowState === "draft" || rowState === "rejected")) {
        var rbusy = !!reworkBusy[aid];
        reworkHtml =
          '<details class="owo-ws-rework"' + (rbusy ? ' data-busy="1"' : "") + ">" +
          '<summary>根据评审意见返工（生成 v2 取代本版本）</summary>' +
          '<div class="owo-ws-rework-form" data-rework-form="' + esc(aid) + '">' +
          '<textarea class="owo-ws-rework-instruction" rows="2" placeholder="返工指令（展开时自动从最近一次「要求修改」评审意见预填，可修改）"></textarea>' +
          '<div class="owo-ws-inline"><button type="button" class="owo-ws-rework-go primary" data-core-action data-rework-go="' + esc(aid) + '"' + (rbusy ? " disabled" : "") + ">发起返工</button>" +
          '<span class="hint">POST /artifacts/{id}/rework —— 同一评审仅创建一个返工任务（重复提交幂等返回原任务）</span></div>' +
          "</div></details>";
      }
      // 评审结果行（行级，独立于表单）：状态迁移后表单可能消失，但 flash 提示仍在。
      var flash = reviewFlash && String(reviewFlash.artifactId) === aid ? reviewFlash : null;
      var resultHtml =
        '<div class="owo-ws-review-result sub' + (flash && !flash.ok ? " bad" : flash ? " ok" : "") + '" data-art-result="' + esc(aid) + '" aria-live="polite">' +
        (flash ? esc(flash.text) : "") +
        "</div>";
      return (
        '<div class="owo-ws-art-row' + (isHead ? " head" : "") + '" data-art-row="' + esc(aid) + '">' +
        '<div class="owo-ws-art-line">' +
        '<span class="owo-ws-mono">v' + esc(a.version) + (isHead ? "（最新）" : "") + "</span>" +
        reviewBadgeHtml(a.review_state) +
        validationBadgeHtml(a.validation) +
        '<span class="hint">产出者 ' + esc(roleOfProducer(a.producer)) + "</span>" +
        (supVer ? '<span class="hint">取代 ' + esc(supVer) + "</span>" : "") +
        (a.evidence_refs && a.evidence_refs.length
          ? '<span class="hint" title="证据引用：' + esc(a.evidence_refs.join("，")) + '">证据 ' + esc(String(a.evidence_refs.length)) + " 条</span>"
          : "") +
        (a.handoff ? '<span class="hint" title="该产物携带 Handoff 交接记录">含 Handoff</span>' : "") +
        (a.sha256
          ? '<span class="hint owo-ws-ellip" title="sha256: ' + esc(String(a.sha256)) + '">sha256 ' + esc(String(a.sha256).slice(0, 10)) + "…</span>"
          : "") +
        '<span class="hint owo-ws-ellip" title="' + esc(a.created_at || "") + '">' + esc(a.created_at || "") + "</span>" +
        "</div>" +
        (a.preview != null
          ? '<details class="owo-ws-art-preview"><summary>预览</summary><pre>' + esc(a.preview || "（空）") + "</pre></details>"
          : "") +
        formHtml +
        reworkHtml +
        resultHtml +
        '<div class="owo-ws-art-histline">' +
        '<button type="button" class="owo-ws-mini" data-art-dl="' + esc(aid) + '" title="GET /artifacts/{id}/content —— 以推断扩展名保存正文">下载</button>' +
        '<button type="button" class="owo-ws-mini" data-art-history="' + esc(aid) + '">评审历史</button>' +
        '<span class="owo-ws-art-history" data-art-history-box="' + esc(aid) + '"></span>' +
        "</div>" +
        "</div>"
      );
    }


  /// §8.1 引用选择器：实现见 render_conversation.js；这里注入当前 esc/short 绑定，
  /// 保证浏览器（H.esc）与 Node 单测（defaultEsc）行为一致。
  /// opts = {kind, rows:[string], options:[{value,label}], query, allowFree}
  function refPickerHtml(opts) {
    return convo.refPickerHtml({ esc: esc, short: short }, opts);
  }

  function teamConversationHtml(handoffs, members, markdown, errorText, replay, onlyRole) {
    var workerHtml = convo.workerConversationHtml({ esc: esc, short: short }, replay, markdown, onlyRole);
    var handoffHtml = convo.teamConversationHtml({ esc: esc, short: short }, handoffs, members, markdown, errorText, onlyRole);
    return workerHtml + (workerHtml ? '<details><summary>交接记录</summary>' + handoffHtml + '</details>' : handoffHtml);
  }

  var api = {
    bindEsc: bindEsc,
    bindShort: bindShort,
    REVIEW_CN: REVIEW_CN,
    REVIEW_CLS: REVIEW_CLS,
    CHG_STATE_CN: CHG_STATE_CN,
    CHG_STATE_CLS: CHG_STATE_CLS,
    CS_STATUS_CN: CS_STATUS_CN,
    reviewBadgeHtml: reviewBadgeHtml,
    strategyBoxHtml: strategyBoxHtml,
    metricsCardsHtml: metricsCardsHtml,
    diffHtml: diffHtml,
    artifactTimelineHtml: artifactTimelineHtml,
    deliverablesBoxHtml: deliverablesBoxHtml,
    workspaceBoxHtml: workspaceBoxHtml,
    workspaceTreeHtml: workspaceTreeHtml,
    gitStatusHtml: gitStatusHtml,
    templateBoxHtml: templateBoxHtml,
    failureBadgeHtml: failureBadgeHtml,
    failureSummaryHtml: failureSummaryHtml,
    validationBadgeHtml: validationBadgeHtml,
    workerProfilesTable: workerProfilesTable,
    writeLeaseBox: writeLeaseBox,
    changeStateBadge: changeStateBadge,
    changesListHtml: changesListHtml,
    changesRemoteView: changesRemoteView,
    changeRecordsHtml: changeRecordsHtml,
    changesRuntimeHtml: changesRuntimeHtml,
    changeSetBadge: changeSetBadge,
    deliveryManifestText: deliveryManifestText,
    artifactHistoryHtml: artifactHistoryHtml,
    dagSvg: dagSvg,
    progressCountChip: progressCountChip,
    renderProgress: renderProgress,
    approvalBlockBanner: approvalBlockBanner,
    changeSetsHtml: changeSetsHtml,
    artifactRowHtml: artifactRowHtml,
    refPickerHtml: refPickerHtml,
    teamConversationHtml: teamConversationHtml,
  };

  win.OwoWorkswarmRender = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})();
