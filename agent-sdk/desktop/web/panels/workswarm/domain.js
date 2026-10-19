// WorkSwarm domain helpers shared by the browser panel and Node regression tests.
// Keep this module DOM-free: it owns state normalization and run-summary rules only.
(function () {
  "use strict";

  var TEAM_STATUS_CN = {
    created: "已创建",
    running: "运行中",
    awaiting_human: "等待人节点",
    succeeded: "已完成",
    failed: "失败",
    cancelled: "已取消",
    stopping: "正在停止",
    stopped: "已停止",
  };
  var STEP_STATUS_CN = {
    pending: "等待",
    ready: "就绪",
    running: "运行中",
    succeeded: "完成",
    failed: "失败",
    aborted: "中止",
  };
  var MODE_CN = { single: "单节点", team: "接力（默认）", swarmflow: "DAG 流程" };
  var HEALTH_CN = { active: "正常", degraded: "降级", offline: "离线", fused: "熔断" };
  var PROPOSAL_STATUS_CN = { proposed: "待采纳", adopted: "已采纳", rejected: "已拒绝" };
  var TERMINAL = { succeeded: true, failed: true, cancelled: true };
  var STEP_TERMINAL = { succeeded: true, failed: true, aborted: true };
  var STEP_DEAD = { failed: true, aborted: true };
  var STEP_RETRYABLE = { failed: true, aborted: true };

  function normStatus(status) {
    var normalized = String(status || "").toLowerCase();
    return normalized === "awaitinghuman" ? "awaiting_human" : normalized;
  }

  function isDeadStatus(status) {
    return !!STEP_DEAD[normStatus(status)];
  }

  function taskBlocked(task, byId) {
    if (!task || STEP_TERMINAL[normStatus(task.status)]) return false;
    return (task.depends_on || []).some(function (dependency) {
      return byId[dependency] && isDeadStatus(byId[dependency].status);
    });
  }

  function isTerminalTeam(status) {
    return !!TERMINAL[normStatus(status)];
  }

  function isRetryableStep(task) {
    return !!task && !!STEP_RETRYABLE[normStatus(task.status)];
  }

  function retryableTasks(tasks) {
    return (tasks || []).filter(isRetryableStep);
  }

  function shouldShowRetry(tasks, teamStatus) {
    var status = normStatus(teamStatus);
    return status !== "succeeded" && status !== "cancelled" && retryableTasks(tasks).length > 0;
  }

  function buildRetryBody(stepId, note) {
    var normalizedId = String(stepId == null ? "" : stepId).trim();
    var normalizedNote = note == null ? "" : String(note).trim();
    return {
      command: "retry",
      step_id: normalizedId,
      note: normalizedNote || ("重试此节点：" + normalizedId),
    };
  }

  function computeRunSummary(input) {
    var team = (input && input.team) || {};
    var tasks = (input && input.tasks) || [];
    var byId = {};
    tasks.forEach(function (task) { byId[task.task_id] = task; });
    var counts = { total: tasks.length, succeeded: 0, failed: 0, waiting: 0, running: 0, blocked: 0 };
    var totalAttempts = 0;
    var failedStep = null;
    var runningStep = null;
    tasks.forEach(function (task) {
      var status = normStatus(task.status);
      var attempts = Number(task.attempts || 0);
      totalAttempts += attempts;
      if (status === "succeeded") counts.succeeded++;
      else if (status === "failed" || status === "aborted") {
        counts.failed++;
        if (!failedStep) failedStep = {
          task_id: task.task_id,
          role: task.role || task.worker || task.task_id,
          attempts: attempts,
          error: task.error || "",
        };
      } else if (status === "running") {
        counts.running++;
        if (!runningStep) runningStep = task;
      } else counts.waiting++;
      if (taskBlocked(task, byId)) counts.blocked++;
    });
    var interrupted = !!(input && input.interrupted);
    var rawStatus = team.status != null ? String(team.status) : "";
    var status = normStatus(rawStatus);
    var interruptedView = interrupted && !isTerminalTeam(status);
    var phase;
    if (interruptedView) phase = "已中断，可恢复";
    else if (status === "awaiting_human") phase = "等待人节点结果";
    else if (runningStep) phase = "执行：" + (runningStep.role || runningStep.worker || runningStep.task_id);
    else if (status === "succeeded") phase = "全部步骤已完成";
    else if (counts.failed > 0) phase = "已停止：存在失败步骤";
    else if (status === "running") phase = counts.total ? "调度中" : "等待任务图生成";
    else if (status === "created") phase = counts.total ? "准备启动" : "尚未开始";
    else if (status === "cancelled") phase = "已取消";
    else if (status === "stopping") phase = "正在停止（Worker 退出中）";
    else if (status === "stopped") phase = "已停止";
    else if (status === "failed") phase = "已失败";
    else phase = "—";
    return {
      statusKey: interruptedView ? "interrupted" : status,
      statusLabel: interruptedView ? "已中断（可恢复）" : TEAM_STATUS_CN[status] || rawStatus || "未知",
      phase: phase,
      counts: counts,
      totalAttempts: totalAttempts,
      failedStep: failedStep,
      artifactCount: input && input.artifactCount != null ? input.artifactCount : null,
      canRetry: shouldShowRetry(tasks, status),
    };
  }

  // ---------- §8.1 引用选择器（可搜索 + 可增删结构化行；纯函数，Node 可测） ----------

  /// 添加引用行：trim、拒空、按值去重；返回新数组（不改变入参）。
  function addRefRow(rows, value) {
    var v = String(value == null ? "" : value).trim();
    var out = (Array.isArray(rows) ? rows : []).slice();
    if (!v) return out;
    if (out.some(function (r) { return String(r) === v; })) return out;
    out.push(v);
    return out;
  }

  /// 移除指定值引用行；返回新数组。
  function removeRefRow(rows, value) {
    var v = String(value == null ? "" : value);
    return (Array.isArray(rows) ? rows : []).filter(function (r) {
      return String(r) !== v;
    });
  }

  /// 候选过滤：value/label 子串不区分大小写；query 为空返回前 max 项。
  function filterRefOptions(options, query, max) {
    var q = String(query == null ? "" : query).trim().toLowerCase();
    var list = Array.isArray(options) ? options : [];
    var cap = Math.max(1, Number(max) || 8);
    var hit = q
      ? list.filter(function (o) {
          return (
            String((o && o.value) || "").toLowerCase().indexOf(q) >= 0 ||
            String((o && o.label) || "").toLowerCase().indexOf(q) >= 0
          );
        })
      : list.slice();
    return hit.slice(0, cap);
  }

  // ---------- §8.1 自定义 worker/role 门禁（选“自定义”才出现输入框；提交前校验） ----------

  /// 平台已知 worker 集合（与创建页提示一致：agent/echo/sleep/fail）。
  var KNOWN_WORKERS = ["agent", "echo", "sleep", "fail"];
  /// 自定义下拉哨兵值：选中后才显示自由输入框。
  var CUSTOM_WORKER = "__custom__";

  /// 换员 worker 提交前解析：known → 原值；自定义 → 必填校验；空 → 不变更。
  /// 返回 {ok, worker?, error?}（ok=false 时 error 为用户文案）。
  function resolveReplaceWorker(selectValue, customValue) {
    var v = String(selectValue == null ? "" : selectValue);
    if (!v) return { ok: true, worker: "" }; // 不变更
    if (v === CUSTOM_WORKER) {
      var c = String(customValue == null ? "" : customValue).trim();
      if (!c) return { ok: false, error: "已选择自定义 worker：请填写 worker 标识" };
      return { ok: true, worker: c };
    }
    if (KNOWN_WORKERS.indexOf(v) < 0) return { ok: false, error: "未知 worker：" + v };
    return { ok: true, worker: v };
  }

  /// 角色规格行 worker 初值解析（模板/既有规格回填）：known → 原值；其他（含空）→ 自定义。
  function roleWorkerPlan(workerValue) {
    var v = String(workerValue == null ? "" : workerValue).trim();
    var known = !!v && KNOWN_WORKERS.indexOf(v) >= 0;
    return {
      select: known ? v : CUSTOM_WORKER,
      custom: known ? "" : v,
      isKnown: known,
    };
  }

  /// 角色规格行提交前校验：自定义 worker 必填（assignee=human 时 worker 为用户 id，同样必填）。
  /// rows = [{role, workerSelected, worker, assignee}]；返回错误文案数组（空 = 通过）。
  function validateRoleRows(rows) {
    var errs = [];
    (Array.isArray(rows) ? rows : []).forEach(function (r) {
      var role = String((r && r.role) || "").trim();
      if (!role) return; // collectRoles 静默跳过空角色行
      var selected = String((r && r.workerSelected) || "");
      var worker = String((r && r.worker) || "").trim();
      if (selected === CUSTOM_WORKER && !worker) {
        errs.push("角色 " + role + "：已选择自定义 worker，请填写 worker 标识");
      }
    });
    return errs;
  }

  /// 清洗角色规格为请求体形状：剔除校验标记，丢弃空角色行。
  /// workerSelected === "agent"（known 默认项）视为未显式选择，不发 worker 字段
  /// （与历史行为一致：空输入 = 交由服务端默认）；显式选 echo/sleep/fail/自定义照发。
  function sanitizeRoles(rows) {
    return (Array.isArray(rows) ? rows : []).filter(function (r) {
      return r && String(r.role || "").trim();
    }).map(function (r) {
      var item = { role: String(r.role).trim(), assignee: r.assignee };
      ["model", "tools", "write_paths", "handoff_contract", "capabilities", "extra_input", "verify"].forEach(function (key) {
        if (r[key] !== undefined) item[key] = r[key];
      });
      if (r.worker && r.workerSelected !== "agent") item.worker = r.worker;
      if (r.depends_on && r.depends_on.length) item.depends_on = r.depends_on;
      return item;
    });
  }

  var api = {
    TEAM_STATUS_CN: TEAM_STATUS_CN,
    STEP_STATUS_CN: STEP_STATUS_CN,
    MODE_CN: MODE_CN,
    HEALTH_CN: HEALTH_CN,
    PROPOSAL_STATUS_CN: PROPOSAL_STATUS_CN,
    STEP_TERMINAL: STEP_TERMINAL,
    KNOWN_WORKERS: KNOWN_WORKERS,
    CUSTOM_WORKER: CUSTOM_WORKER,
    normStatus: normStatus,
    isDeadStatus: isDeadStatus,
    taskBlocked: taskBlocked,
    isTerminalTeam: isTerminalTeam,
    isRetryableStep: isRetryableStep,
    retryableTasks: retryableTasks,
    shouldShowRetry: shouldShowRetry,
    buildRetryBody: buildRetryBody,
    computeRunSummary: computeRunSummary,
    addRefRow: addRefRow,
    removeRefRow: removeRefRow,
    filterRefOptions: filterRefOptions,
    resolveReplaceWorker: resolveReplaceWorker,
    roleWorkerPlan: roleWorkerPlan,
    validateRoleRows: validateRoleRows,
    sanitizeRoles: sanitizeRoles,
  };
  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoWorkswarmDomain = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})();
