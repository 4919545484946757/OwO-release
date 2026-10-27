// WorkSwarm view-model helpers (纯函数簇)：指标/交付物/ChangeSet/
// 工作区/产物文件名等归一化与格式化规则，独立于 DOM、state 与传输层。
// 与 domain.js 同款模式：浏览器经 index.html 预加载全局对象；Node 测试直接 require。
// 本模块不含 esc/state/H/rootEl 依赖——所有函数只消费入参与模块内常量。
// 注意：本文件由 scripts 级 codemod 从 workswarm.panel.js 原样抽取生成，
// 修改函数逻辑请改面板源文件（或本文件后保持面板与格式命名一致）。
(function () {
  "use strict";

    // 评审状态归一：大小写/分隔符无关。
    function normReviewState(s) {
      return String(s == null ? "" : s).toLowerCase().replace(/[_\s-]/g, "");
    }
    // 版本链分组（纯函数）：supersedes_artifact_id 指向链内既有产物则续链；链内按 version 升序；链头（items 末位）为最新版本。
    function groupArtifactChain(arts) {
      var list = (arts || []).filter(function (a) {
        return a && a.artifact_id != null;
      });
      var byId = {};
      list.forEach(function (a) {
        byId[String(a.artifact_id)] = a;
      });
      list.sort(function (x, y) {
        return String(x.created_at).localeCompare(String(y.created_at));
      });
      var chains = [];
      var chainOf = {};
      list.forEach(function (a) {
        var id = String(a.artifact_id);
        var sup = a.supersedes_artifact_id == null ? "" : String(a.supersedes_artifact_id);
        if (sup && byId[sup] && chainOf[sup] != null) {
          var c = chains[chainOf[sup]];
          c.items.push(a);
          chainOf[id] = chainOf[sup];
        } else {
          chainOf[id] = chains.length;
          chains.push({ items: [a] });
        }
      });
      chains.forEach(function (c) {
        c.items.sort(function (x, y) {
          return (Number(x.version) || 0) - (Number(y.version) || 0) || String(x.created_at).localeCompare(String(y.created_at));
        });
        var approved = null;
        c.items.forEach(function (a) {
          if (normReviewState(a.review_state) === "approved") approved = a; // 取最高版本（链已升序）
        });
        c.approvedHead = approved;
      });
      return chains;
    }
    // 组队策略判定归一（detail.strategy_decision | detail.strategy）。
    function strategyDecisionOf(detail) {
      if (!detail || typeof detail !== "object") return null;
      var raw = detail.strategy_decision || detail.strategy;
      if (!raw || typeof raw !== "object") return null;
      var mode = String(raw.mode || raw.decision || "").toLowerCase();
      if (mode !== "single" && mode !== "team") return null;
      var reasons = raw.reasons || raw.reason || raw.why || [];
      if (typeof reasons === "string") reasons = [reasons];
      if (!Array.isArray(reasons)) reasons = [];
      var roles = raw.roles || [];
      if (!Array.isArray(roles)) roles = [];
      var budget = Number(raw.budget_per_role);
      var resolved = raw.resolved_roles;
      if (!Array.isArray(resolved)) resolved = [];
      return {
        mode: mode,
        roles: roles.map(function (r) {
          if (typeof r === "string") return { role: r, duty: "" };
          return { role: String((r && r.role) || ""), duty: String((r && r.duty) || (r && r.responsibility) || "") };
        }).filter(function (r) { return r.role; }),
        // Actual runnable topology after template/adaptive/auto-reviewer
        // resolution; budget_calls is the per-role cap the run will use.
        resolvedRoles: resolved.map(function (r) {
          var n = Number(r && r.budget_calls);
          return {
            role: String((r && r.role) || ""),
            assignee: String((r && r.assignee) || ""),
            budgetCalls: isFinite(n) && n > 0 ? n : null,
            writePaths: Array.isArray(r && r.write_paths) ? r.write_paths.map(String) : [],
          };
        }).filter(function (r) { return r.role; }),
        parallelism: raw.parallelism == null ? null : Number(raw.parallelism),
        budgetPerRole: isFinite(budget) && budget > 0 ? budget : null,
        reasons: reasons.map(String).filter(Boolean),
      };
    }
    function pickNum(o, keys) {
      for (var i = 0; i < keys.length; i++) {
        var v = o && o[keys[i]];
        if (v != null && isFinite(Number(v))) return Number(v);
      }
      return null;
    }
    function pickStr(o, keys) {
      for (var i = 0; i < keys.length; i++) {
        var v = o && o[keys[i]];
        if (v != null && v !== "") return String(v);
      }
      return "";
    }
    // 角色指标归一（GET /teams/{id}/metrics 容错：workers/roles/items 多形状）。
    function metricsFromPayload(d) {
      if (!d || typeof d !== "object") return null;
      var rawWorkers = d.workers || d.roles || d.items || [];
      if (!Array.isArray(rawWorkers)) rawWorkers = [];
      var workers = rawWorkers.map(function (w) {
        var arts = (w && (w.artifact_ids || w.artifacts || w.output_artifacts)) || [];
        if (!Array.isArray(arts)) arts = [];
        // 服务端 worker 行可为 {artifact:{artifact_id,...}} 单对象（五期实弹形状）。
        var one = (w && w.artifact) || null;
        if (one && typeof one === "object") {
          var oid = one.artifact_id || one.id;
          if (oid) arts.push(oid);
        } else if (one) {
          arts.push(one);
        }
        return {
          worker: pickStr(w, ["worker", "worker_id", "member_id", "name", "role"]),
          role: pickStr(w, ["role", "worker_role"]),
          taskId: pickStr(w, ["task_id", "taskId"]),
          attemptId: pickStr(w, ["attempt_id", "attemptId"]),
          phaseEpoch: pickNum(w, ["phase_epoch", "phaseEpoch"]),
          startedAt: pickStr(w, ["started_at", "startedAt", "start"]),
          endedAt: pickStr(w, ["ended_at", "endedAt", "end", "finished_at"]),
          durationMs: pickNum(w, ["duration_ms", "durationMs", "wall_ms", "wall_ms_sum", "elapsed_ms"]),
          budgetReservationWaitMs: pickNum(w, ["budget_reservation_wait_ms"]),
          modelCalls: pickNum(w, ["model_calls", "calls", "model_call_count"]),
          tokensIn: pickNum(w, ["tokens_in", "input_tokens", "prompt_tokens"]),
          tokensOut: pickNum(w, ["tokens_out", "output_tokens", "completion_tokens"]),
          estCost: pickNum(w, ["est_cost", "cost_usd", "cost"]),
          attempts: pickNum(w, ["attempts", "try_count", "attempt", "spans"]),
          terminal: pickStr(w, ["terminal", "outcome", "status", "final_status"]),
          failureReason: pickStr(w, ["failure_reason", "error", "fail_reason"]),
          artifactIds: arts.map(function (x) { return typeof x === "string" ? x : String(x && (x.artifact_id || x.id) || ""); }).filter(Boolean),
        };
      });
      var s = d.summary || d.totals || {};
    // 五期实弹：slowest_worker 可为 {role,span_id,step_id} 对象——提取 role。
      var slowest = s.slowest_worker;
      if (slowest && typeof slowest === "object") slowest = slowest.role || slowest.worker || slowest.span_id || "";
      var budget = (d && d.budget) || {};
      var rawLifecycle = (d && d.lifecycle) || {};
      var lifecycleStages = Array.isArray(rawLifecycle.stages) ? rawLifecycle.stages.map(function (stage) {
        return {
          stage: pickStr(stage, ["stage"]),
          count: pickNum(stage, ["count"]),
          failures: pickNum(stage, ["failures"]),
          durationMsSum: pickNum(stage, ["duration_ms_sum"]),
          durationMsP50: pickNum(stage, ["duration_ms_p50"]),
          durationMsP90: pickNum(stage, ["duration_ms_p90"]),
          durationMsMax: pickNum(stage, ["duration_ms_max"]),
        };
      }).filter(function (stage) { return stage.stage; }) : [];
      var lifecycleEpochs = {};
      (Array.isArray(rawLifecycle.epochs) ? rawLifecycle.epochs : []).forEach(function (epoch) {
        var id = pickNum(epoch, ["phase_epoch"]);
        if (id != null) lifecycleEpochs[String(id)] = pickNum(epoch, ["phase_duration_ms_sum"]);
      });
      var rawContextAssembly = (d && d.context_assembly) || {};
      var contextAssemblyRoles = Array.isArray(rawContextAssembly.by_role)
        ? rawContextAssembly.by_role.map(function (role) {
            return {
              role: pickStr(role, ["role"]),
              sampleCount: pickNum(role, ["sample_count"]),
              failures: pickNum(role, ["failures"]),
              durationMsSum: pickNum(role, ["duration_ms_sum"]),
              durationMsP50: pickNum(role, ["duration_ms_p50"]),
              durationMsP90: pickNum(role, ["duration_ms_p90"]),
            };
          }).filter(function (role) { return role.role; })
        : [];
      var contextAssemblyEpochs = Array.isArray(rawContextAssembly.by_epoch)
        ? rawContextAssembly.by_epoch.map(function (epoch) {
            return {
              phaseEpoch: pickNum(epoch, ["phase_epoch"]),
              sampleCount: pickNum(epoch, ["sample_count"]),
              failures: pickNum(epoch, ["failures"]),
              durationMsSum: pickNum(epoch, ["duration_ms_sum"]),
              durationMsP50: pickNum(epoch, ["duration_ms_p50"]),
              durationMsP90: pickNum(epoch, ["duration_ms_p90"]),
            };
          }).filter(function (epoch) { return epoch.phaseEpoch != null; })
        : [];
      var contextAssembly = {
        sampleCount: pickNum(rawContextAssembly, ["sample_count"]),
        attributionMissingCount: pickNum(rawContextAssembly, ["attribution_missing_count"]),
        sampleCap: pickNum(rawContextAssembly, ["sample_cap"]),
        sampleTruncated: !!rawContextAssembly.sample_truncated,
        auditScanTruncated: !!rawContextAssembly.audit_scan_truncated,
        invalidRecords: pickNum(rawContextAssembly, ["invalid_records"]),
        failures: pickNum(rawContextAssembly, ["failures"]),
        durationMsSum: pickNum(rawContextAssembly, ["duration_ms_sum"]),
        durationMsP50: pickNum(rawContextAssembly, ["duration_ms_p50"]),
        durationMsP90: pickNum(rawContextAssembly, ["duration_ms_p90"]),
        byRole: contextAssemblyRoles,
        byEpoch: contextAssemblyEpochs,
      };
      var rawEpochSummary = (d && d.execution_epochs) || {};
      var executionEpochs = Array.isArray(rawEpochSummary.items) ? rawEpochSummary.items.map(function (epoch) {
        var id = pickNum(epoch, ["phase_epoch"]);
        return {
          phaseEpoch: id,
          workerSpanCount: pickNum(epoch, ["worker_span_count"]),
          taskAttemptCount: pickNum(epoch, ["task_attempt_count"]),
          failedSpans: pickNum(epoch, ["failed_spans"]),
          modelCalls: pickNum(epoch, ["model_calls"]),
          promptTokens: pickNum(epoch, ["prompt_tokens"]),
          completionTokens: pickNum(epoch, ["completion_tokens"]),
          totalTokens: pickNum(epoch, ["total_tokens"]),
          usageKnown: !!epoch.usage_known,
          costUsd: pickNum(epoch, ["cost_usd"]),
          costKnown: !!epoch.cost_known,
          workerWallMsSum: pickNum(epoch, ["worker_wall_ms_sum"]),
          workerWindowMs: pickNum(epoch, ["worker_window_ms"]),
          overlapFactor: pickNum(epoch, ["parallel_overlap_factor"]),
          providerWaitMsSum: pickNum(epoch, ["provider_wait_ms_sum"]),
          budgetWaitMsSum: pickNum(epoch, ["budget_reservation_wait_ms_sum"]),
          leaseWaitMsSum: pickNum(epoch, ["lease_wait_ms_sum"]),
          phaseDurationMs: id == null ? null : lifecycleEpochs[String(id)],
        };
      }).filter(function (epoch) { return epoch.phaseEpoch != null; }) : [];
      var summary = {
        wallClockMs: pickNum(s, ["wall_clock_ms", "wallClockMs", "wall_window_ms", "wall_ms", "total_wall_ms"]),
        totalModelCalls: pickNum(s, ["total_model_calls", "total_calls", "model_calls"]),
        totalTokensIn: pickNum(s, ["total_tokens_in", "tokens_in", "prompt_tokens"]),
        totalTokensOut: pickNum(s, ["total_tokens_out", "tokens_out", "completion_tokens"]),
        totalEstCost: pickNum(s, ["total_est_cost", "total_cost_usd", "total_cost", "cost_usd", "est_cost"]),
        totalBudgetReservationWaitMs: pickNum(s, ["budget_reservation_wait_ms_sum"]),
        slowestWorker: typeof slowest === "string" ? slowest : pickStr(s, ["slowest_worker", "slowest"]),
        failures: pickNum(s, ["failures", "failed_spans", "failure_count"]),
        reworks: pickNum(s, ["reworks", "rework_count"]),
        artifactVersions: pickNum(s, ["artifact_versions", "artifact_count"]),
        budgetExhausted: !!(s.budget_exhausted || s.budgetExhausted || budget.exceeded),
        budgetReason: (function () {
          var r = pickStr(s, ["budget_reason", "budget_exhausted_reason"]);
          if (r) return r;
          var br = budget.reason;
          return typeof br === "string" ? br : "";
        })(),
      };
      if (!workers.length && summary.wallClockMs == null && summary.totalModelCalls == null && !lifecycleStages.length && !executionEpochs.length && !(contextAssembly.sampleCount || contextAssembly.attributionMissingCount)) return null;
      return {
        workers: workers,
        summary: summary,
        lifecycleStages: lifecycleStages,
        contextAssembly: contextAssembly,
        executionEpochs: executionEpochs,
        executionEpochCount: pickNum(rawEpochSummary, ["epoch_count"]),
        executionEpochsTruncated: !!rawEpochSummary.epochs_truncated,
      };
    }
    function fmtMs(ms) {
      if (ms == null || !isFinite(ms)) return "—";
      if (ms < 1000) return Math.round(ms) + "ms";
      var s = ms / 1000;
      if (s < 60) return s.toFixed(1) + "s";
      var m = Math.floor(s / 60);
      return m + "m" + Math.round(s - m * 60) + "s";
    }
    function fmtElapsed(ms) {
      if (!isFinite(ms) || ms == null || ms < 0) return "—";
      var s = Math.floor(ms / 1000);
      if (s < 60) return s + "s";
      var m = Math.floor(s / 60);
      var rs = s % 60;
      if (m < 60) return m + "m" + (rs ? rs + "s" : "");
      var h = Math.floor(m / 60);
      return h + "h" + (m % 60) + "m";
    }
    function diffLines(aText, bText) {
      var a = String(aText == null ? "" : aText).split("\n").slice(0, 400);
      var b = String(bText == null ? "" : bText).split("\n").slice(0, 400);
      var n = a.length, m = b.length;
      var dp = [];
      for (var i = 0; i <= n; i++) { dp.push(new Array(m + 1).fill(0)); }
      for (i = n - 1; i >= 0; i--) {
        for (var j = m - 1; j >= 0; j--) {
          dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
        }
      }
      var out = [];
      i = 0; j = 0;
      while (i < n && j < m) {
        if (a[i] === b[j]) { out.push({ t: " ", s: a[i] }); i++; j++; }
        else if (dp[i + 1][j] >= dp[i][j + 1]) { out.push({ t: "-", s: a[i] }); i++; }
        else { out.push({ t: "+", s: b[j] }); j++; }
      }
      while (i < n) { out.push({ t: "-", s: a[i] }); i++; }
      while (j < m) { out.push({ t: "+", s: b[j] }); j++; }
      return out;
    }
    // 交付物三桶归一（approved/pending/other + 概览字段；多形状容错）。
    function deliverablesFromPayload(d) {
      if (!d || typeof d !== "object") return null;
      var all = d.approved || d.deliverables || d.items || d.artifacts || [];
    // 服务端实弹键为 pending_review；契约草案为 pending——两者皆容错。
      var pending = d.pending || d.pending_review || [];
      var other = d.rejected_or_superseded || d.rejected || d.superseded || [];
      var isArr = Array.isArray(all);
      if (!isArr && typeof all === "object" && all != null) all = [];
      if (!Array.isArray(pending)) pending = [];
      if (!Array.isArray(other)) other = []; // 实弹可为 null
      var norm = function (x) {
        return {
          artifactId: String((x && x.artifact_id) || ""),
          kind: String((x && x.kind) || ""),
          version: x && x.version != null ? x.version : null,
          producer: String((x && x.producer) || ""),
          reviewState: normReviewState(x && x.review_state),
        };
      };
      var approved, pendingOut, otherOut;
      if (d.approved || d.deliverables) {
        approved = (isArr ? all : []).map(norm);
        pendingOut = pending.map(norm);
        otherOut = other.map(norm);
      } else {
        // 单列表形状：按 review_state 分桶
        var src = (d.items || d.artifacts || []);
        approved = []; pendingOut = []; otherOut = [];
        src.map(norm).forEach(function (x) {
          if (x.reviewState === "approved") approved.push(x);
          else if (x.reviewState === "pendingreview") pendingOut.push(x);
          else otherOut.push(x);
        });
      }
    // manifest/complete/rework 概览（实弹字段；缺省无害）
      var manifestRef = typeof d.delivery_manifest_ref === "string" ? d.delivery_manifest_ref : "";
      return {
        approved: approved,
        pending: pendingOut,
        other: otherOut,
        complete: !!d.complete,
        manifestRef: manifestRef,
        reworkCount: Array.isArray(d.rework_tasks) ? d.rework_tasks.length : null,
      };
    }
    // TeamRun.workspace（六期冻结契约）容错归一：字段缺失/旧记录均安全。
    function workspaceFromTeam(team) {
      var ws = team && typeof team === "object" ? team.workspace : null;
      if (!ws || typeof ws !== "object") return null;
      var root = typeof ws.root === "string" ? ws.root : "";
      if (!root) return null; // 未绑定工作区的旧团队
      var paths = Array.isArray(ws.write_allowed_paths) ? ws.write_allowed_paths.map(String) : [];
      var readOnly = ws.read_only != null ? !!ws.read_only : true;
      var depth = ws.tree_depth != null ? Number(ws.tree_depth) : null;
      return {
        root: root,
        readOnly: readOnly,
        writePaths: paths,
        treeDepth: isFinite(depth) ? depth : null,
      };
    }
    // 失败原因代码 → 中文标签（六期输出契约失败原因）。
    function failureCodeLabel(code) {
      var map = {
        output_contract_invalid: "输出契约无效",
        artifact_missing: "缺少交付物",
        scope_violation: "越权访问",
      };
      return map[String(code || "")] || "";
    }
    // 下载文件名：artifact_id + 按格式推断的扩展名（未知格式回退原串/txt）。
    var FORMAT_EXT = { json: "json", csv: "csv", markdown: "md", research: "md" };
    function artifactFileName(a) {
      var fmt = String((a && a.format) || "").toLowerCase();
      var ext = FORMAT_EXT[fmt] || (fmt || "txt");
      return String((a && a.artifact_id) || "artifact") + "." + ext;
    }
    // 绝对时间格式化（toLocaleString；空值返回空串）。
    function fmtAbsTime(ms) {
      var n = Number(ms);
      if (!isFinite(n) || n <= 0) return "";
      try {
        return new Date(n).toLocaleString();
      } catch (e) {
        return String(ms);
      }
    }
    // ChangeSet 状态归一：camelCase → snake + 小写 + 分隔符统一。
    function normCsStatus(s) {
      return String(s == null ? "" : s)
        .replace(/([a-z0-9])([A-Z])/g, "$1_$2") // camelCase → snake（serde Debug 形式容错）
        .toLowerCase()
        .replace(/[\s-]+/g, "_");
    }
    // ChangeSet 列表归一（双形状：payload.change_sets 或裸数组；全字段容错）。
    function changeSetsView(payload) {
      return ((payload && Array.isArray(payload.change_sets) ? payload.change_sets : []) || [])
        .map(function (c) {
          var x = c && typeof c === "object" ? c : {};
          return {
            change_set_id: String(x.change_set_id || ""),
            team_id: x.team_id == null ? "" : String(x.team_id),
            step_id: x.step_id == null ? "" : String(x.step_id),
            role: x.role == null ? "" : String(x.role),
            changed_files: Array.isArray(x.changed_files) ? x.changed_files.map(String) : [],
            conflicts: Array.isArray(x.conflicts) ? x.conflicts.map(String) : [],
            diff_ref: x.diff_ref == null ? null : String(x.diff_ref),
            status: normCsStatus(x.status) || "pending_review",
            created_at: x.created_at == null ? "" : String(x.created_at),
            resolved_at: x.resolved_at == null ? "" : String(x.resolved_at),
          };
        })
        .filter(function (x) {
          return !!x.change_set_id;
        });
    }
    function csStatusHint(status) {
      var st = normCsStatus(status);
      if (st === "pending_review") return "等待接受或拒绝";
      if (st === "conflicted") return "存在冲突，禁止批准（处理后可重试接受/拒绝）";
      return "";
    }
    function approvalBlockView(payload) {
      return {
        blocked: !!(payload && payload.approval_blocked),
        reason: String((payload && payload.approval_block_reason) || ""),
      };
    }
    function roleOfProducer(p) {
      return String(p || "").replace(/^m-/, "");
    }
    // ChangeSet 动作幂等键：每次点击生成新键，提交锁防双击重复请求。
    var csIdemSeq = 0;
    function csIdemKey(csId, action) {
      csIdemSeq += 1;
      return ["workswarm-cs", String(csId || ""), String(action || ""), Date.now(), csIdemSeq].join(":");
    }
    // 评审/返工幂等键：不同意图生成新键；同键重复提交由服务端保证零副作用。
    var aidemSeq = 0;
    function aidemKey(a, decision, reviewer) {
      aidemSeq += 1;
      return [String((a && a.artifact_id) || ""), (a && a.version) || 0, decision, reviewer, Date.now(), aidemSeq].join(":");
    }

  var api = {
    normReviewState: normReviewState,
    groupArtifactChain: groupArtifactChain,
    strategyDecisionOf: strategyDecisionOf,
    pickNum: pickNum,
    pickStr: pickStr,
    metricsFromPayload: metricsFromPayload,
    fmtMs: fmtMs,
    fmtElapsed: fmtElapsed,
    diffLines: diffLines,
    deliverablesFromPayload: deliverablesFromPayload,
    workspaceFromTeam: workspaceFromTeam,
    failureCodeLabel: failureCodeLabel,
    artifactFileName: artifactFileName,
    fmtAbsTime: fmtAbsTime,
    normCsStatus: normCsStatus,
    changeSetsView: changeSetsView,
    csStatusHint: csStatusHint,
    approvalBlockView: approvalBlockView,
    roleOfProducer: roleOfProducer,
    csIdemKey: csIdemKey,
    aidemKey: aidemKey,
  };
  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoWorkswarmFormat = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})();
