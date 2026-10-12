// ============================================================================
// WorkSwarm（团队编排）面板 —— desktop/web/panels/workswarm.panel.js
//
// 纯脚本 IIFE，注册 window.OwoPanels.workswarm。
// app.js 的 panelHelpers() 缺失时自建 get/post/esc/friendlyError 回退
// （与 fleet / goal 面板一致的防御式写法）。
//
// API 以 owo-agent-server（workswarm_api.rs）实际路由为准，不发明字段：
//   POST /teams                          创建团队 {objective, mode?, template_id?, roles[]} → 202
//   GET  /teams                          列表 {teams:[TeamRun + active]}
//   GET  /teams/{id}                     详情 {team, tasks:task_view[], audit_tail[]}
//   GET  /teams/{id}/tasks               {team_id, tasks:task_view[]}
//   GET  /teams/{id}/events              SSE：open / state{status,active} / audit{ts,event,detail}；
//                                        ?format=json 一次性快照 {team_id,status,active,audit}
//   POST /teams/{id}/steer               {command: continue|steer|replace|cancel, step_id?, new_input?, note?, role?, new_worker?, new_user_id?}
//   GET  /projects/{pid}/artifacts       产物 {project_id, artifacts[]}
//   POST /tasks/{id}/handoff             交接记录 {team_id, from_member, to_member?, completed_summary, ...}
//   POST /tasks/{id}/human-result        人节点结果 {team_id, result} → {artifact}
//   GET  /teams/templates                已采纳模板 {templates[]}
//   GET  /teams/templates/proposals      模板提案 {proposals[]}
//   POST /teams/templates/proposals/{pid}/adopt    采纳 → {template}
//   POST /teams/templates/proposals/{pid}/reject   拒绝 → {proposal_id, status}
//
// 说明：/teams/* 为受保护路由，仅识别 Authorization: Bearer 头；浏览器
// EventSource 无法携带请求头，SSE 若出错（如 401）本面板自动降级为 2.5s 轮询
// （GET /teams/{id}/events?format=json + GET /teams/{id}），审计事件按
// ts|event|detail 去重，两种通道可无缝拼接。
//
// 修复/体验记录：
// - el() 同时接受裸 ID 与 "#selector"。历史版本对 "#id" 再拼一次 "#"，生成
//   非法选择器 "##id"，querySelector 抛 SyntaxError 使首次挂载中断 → 面板空白。
// - 写操作按钮（创建/steer/cancel/换员/人节点提交/模板采纳拒绝/各刷新）
//   均带提交锁（data-busy + disabled + 文案变化），杜绝双击重复提交。
// - 团队终态时禁用 continue/steer/replace/cancel 与人节点结果提交并给出
//   门控横幅；离开详情视图立即停止 SSE/轮询，不留幽灵请求。
// - 列表/详情/成员/任务 DAG/产物/模板/提案具备「加载中 / 空 / 失败+重试」三态。
// - 结果区与实时通道标注 aria-live；键盘焦点可见；审计区独立限高滚动；
//   状态徽标 / 阻塞计数 / 实时通道使用分级视觉层级。
//
// 二轮（V1-R2 联动，2026-08-27）：
// - 团队详情顶部新增「运行摘要」：状态/活动阶段、成功/失败/等待/运行/阻塞计数、
//   当前失败步骤（含尝试次数与错误摘要）、累计尝试、产物数量。
// - Failed/Aborted 节点与摘要行提供「重试此节点」，走第三路冻结契约
//   POST /teams/{id}/steer {"command":"retry","step_id":...,"note":...}；
//   提交锁防双击重复请求，终态成功/取消团队不渲染重试入口。
// - 重启遗留中断态（服务端 interrupted 标记，列表/详情/SSE/轮询四通道一致接线）
//   显示「运行已中断，可恢复」，不再呈现为正常执行中。
// - 视觉层级与长文本治理：摘要统计卡、成员/DAG/产物/审计省略+完整 tooltip、
//   表格横向滚动包裹；防溢出与重试/中断样式在 style.css 第 16 节（.owo-ws-* 作用域）。
// - Node 兼容：window 回退 globalThis + module.exports 导出 _test 纯逻辑挂钩，
//   供 tests/workswarm.panel.test.mjs 做结构/行为断言（浏览器零差异）。
// ============================================================================
(function () {
  "use strict";

  // Node 测试环境兼容：window 未定义时回退 globalThis（浏览器行为不变）。
  // 文件末尾按需导出 module.exports，供 tests/workswarm.panel.test.mjs 使用。
  var win = typeof window !== "undefined" ? window : globalThis;
  // 领域规则与状态归一化独立于 DOM，便于复用和单测；Node 直接 require 本模块时
  // 主动加载同目录模块，浏览器则由 index.html 预加载全局对象。
  var domain = win.OwoWorkswarmDomain;
  if (!domain && typeof require !== "undefined") domain = require("./workswarm/domain.js");
  var fmt = win.OwoWorkswarmFormat;
  if (!fmt && typeof require !== "undefined") fmt = require("./workswarm/format.js");
  var render = win.OwoWorkswarmRender;
  if (!render && typeof require !== "undefined") render = require("./workswarm/render.js");
  if (!render) throw new Error("WorkSwarm render helpers 未加载");
  if (!fmt) throw new Error("WorkSwarm format helpers 未加载");

  if (!domain) throw new Error("WorkSwarm domain helpers 未加载");

  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels.workswarm = (function () {
    var ID = "workswarm";

    // ---------- helpers（优先 app.js 注入，缺失时自建回退） ----------
    var H = {};
    var rootEl = null;
    var deliveryView = null;
    function defaultGet(path, options) {
      return window.OwoApi.get(path, options);
    }

    function defaultPost(path, body) {
      return window.OwoApi.post(path, body || {});
    }

    function defaultEsc(s) {
      return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
        return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
      });
    }

    function esc(s) {
      return H.esc ? H.esc(s) : defaultEsc(s);
    }

    function confirmAction(title, message, confirmText) {
      if (H.confirm) {
        return Promise.resolve(H.confirm({
          title: title,
          message: message,
          confirmText: confirmText || "继续",
          kind: "danger",
        }));
      }
      return Promise.resolve(win.confirm(message));
    }

    // ---------- 状态 ----------
    var state = {
      view: "list", // create | list | templates | detail
      teams: [],
      current: null, // 当前详情 team_id
      team: null,
      tasks: [],
      audit: [], // 最新在前
      auditKeys: {},
      active: false,
      teamStatus: "", // 最新状态（可能是 Debug 形式，展示前统一归一化）
      interrupted: false, // 服务端中断标记：磁盘 Running 但无活动运行（R2）
      artifactCount: null, // 产物数量（loadArtifacts 成功后回填；null=未知）
      streamNote: "",
      liveMode: "off", // sse | poll | off
      es: null,
      pollTimer: null,
      pollRequest: null,
      pollNoteShown: false,
      refreshDebounce: null,
      templates: [],
      proposals: [],
      humanTask: "",
      // —— 四期：实时进度（第二路 progress 事件）——
      progress: null, // 最新 progress 事件 {seq,status,active,current_steps[],counts{},updated_at}
      lastProgressSeq: 0, // 单调递增守卫：断线重连/轮询快照里的旧 seq 一律跳过（不重置于重连）
      progressTimer: null, // 1s tick：刷新"已运行时间"（仅在存在 Running 步骤时活跃）
      cancelling: false, // 用户点了取消：立即置位（不等服务端），终态/确认后清除
      reviewBusy: {}, // artifact_id -> true（评审提交进行中，按钮锁定）
      artifacts: [], // 最近一次 loadArtifacts 的产物数组（findArtifactById / 评审提交依赖）
      artifactLoadError: "",
      reviewResult: null, // 最近一次评审提交结果（测试与 DOM 提示共用）
      reviewFlash: null, // {artifactId, ok, text} 评审结果闪存：产物区重载后仍显示
      // —— 五期：自适应组队 / 角色指标 / 版本链返工 / 交付物 / 诊断 ——
      strategyDecision: null, // strategyDecisionOf(detail) 归一结果（auto/single/team + 理由）
      metrics: null, // metricsFromPayload 归一结果（角色指标卡）
      deliverables: null, // deliverablesFromPayload 归一结果（最终交付物三桶）
      deliverablesOpen: false, // 交付物区是否展开（懒加载）
      reworkBusy: {}, // artifact_id -> true（返工提交进行中）
      reworkResult: null, // 最近一次返工提交结果
      historyReviews: {}, // artifact_id -> 最近一次评审历史记录（rework 预填 review_id 用）
      diagnostic: null, // 最近一次下载的诊断 JSON（已脱敏）
      handoffs: null, // 按需从脱敏诊断端点加载的持久化交接消息
      conversationError: "",
      conversationRequest: null,
      conversationSeq: 0,
      // —— 六期：工作区绑定 / 模板信息 / 输出契约失败原因 ——
      workspace: null, // workspaceFromTeam(team) 归一（root/read_only/write_paths/tree_depth）
      workspaceView: null, // 最近一次目录树/git-status 载荷（workspaceViewKind 标记类型）
      workspaceViewKind: "", // "tree" | "git" | ""
      workspaceBusy: false, // 目录树/git-status 拉取中
      templateInfo: null, // {template_id, version, title}（目录懒加载解析版本）
      // —— 七期：Worker 能力 / 写租约 / 文件变更（详情字段缺失时全部容错为空） ——
      workerProfiles: [], // WorkerProfile[]：visible_tools/read_only/max_turns/write_allowed_paths/...
      writeLease: null, // 单写租约（null=未持有；released_at_ms 非空=已释放）
      changes: [], // 工作区文件变更 [{path,state,diff?,added_lines?,deleted_lines?}]
      changesRemote: null, // 二路 changes 端点归一视图（changesRemoteView）；null=未拉取/404 容错
      // —— 八期：ChangeSet 审批闭环（二路交接；端点未上线时全部容错为空态） ——
      changeSets: null, // GET /teams/{id}/change-sets 归一列表（changeSetsView）；null=未拉取/404
      csApprovalBlock: null, // 九期：{blocked, reason} 批准门控（blocked 时禁用 Artifact 批准）
    // §8.1 引用选择器：交接表单的关联产物/证据引用结构化行（不再手输 CSV）。
    handoffRefs: { arts: [], evid: [] },
    handoffRefQuery: { arts: "", evid: "" },
    handoffArtsFetched: false, // 交接选择器产物候选懒加载标记（仅成功后置位）
    handoffArtsLoading: false,
    handoffArtsError: "",
    handoffArtsRequest: null,
      csBusy: {}, // key(change_set_id:action) -> true（accept/reject/revert 提交锁）
      csResults: {}, // change_set_id -> { ok, text }（动作结果行）
    };

    // ---------- 常量与领域规则（见 panels/workswarm/domain.js） ----------
    var TEAM_STATUS_CN = domain.TEAM_STATUS_CN;
    var STEP_STATUS_CN = domain.STEP_STATUS_CN;
    var MODE_CN = domain.MODE_CN;
    var HEALTH_CN = domain.HEALTH_CN;
    var PROPOSAL_STATUS_CN = domain.PROPOSAL_STATUS_CN;
    var STEP_TERMINAL = domain.STEP_TERMINAL;
    var isDeadStatus = domain.isDeadStatus;
    var taskBlocked = domain.taskBlocked;
    var normStatus = domain.normStatus;
    var isTerminalTeam = domain.isTerminalTeam;
    var isRetryableStep = domain.isRetryableStep;
    var retryableTasks = domain.retryableTasks;
    var shouldShowRetry = domain.shouldShowRetry;
    var buildRetryBody = domain.buildRetryBody;
    var computeRunSummary = domain.computeRunSummary;

    // ---------- view-model 纯函数簇（见 panels/workswarm/format.js） ----------
    var normReviewState = fmt.normReviewState;
    var groupArtifactChain = fmt.groupArtifactChain;
    var strategyDecisionOf = fmt.strategyDecisionOf;
    var pickNum = fmt.pickNum;
    var pickStr = fmt.pickStr;
    var metricsFromPayload = fmt.metricsFromPayload;
    var fmtMs = fmt.fmtMs;
    var fmtElapsed = fmt.fmtElapsed;
    var diffLines = fmt.diffLines;
    var deliverablesFromPayload = fmt.deliverablesFromPayload;
    var workspaceFromTeam = fmt.workspaceFromTeam;
    var failureCodeLabel = fmt.failureCodeLabel;
    var artifactFileName = fmt.artifactFileName;
    var fmtAbsTime = fmt.fmtAbsTime;
    var normCsStatus = fmt.normCsStatus;
    var changeSetsView = fmt.changeSetsView;
    var csStatusHint = fmt.csStatusHint;
    var approvalBlockView = fmt.approvalBlockView;
    var roleOfProducer = fmt.roleOfProducer;
    var csIdemKey = fmt.csIdemKey;
    var aidemKey = fmt.aidemKey;

    // ---------- 详情渲染 HTML 簇（见 panels/workswarm/render.js） ----------
    var reviewBadgeHtml = render.reviewBadgeHtml;
    var strategyBoxHtml = render.strategyBoxHtml;
    var metricsCardsHtml = render.metricsCardsHtml;
    var diffHtml = render.diffHtml;
    var artifactTimelineHtml = render.artifactTimelineHtml;
    var deliverablesBoxHtml = render.deliverablesBoxHtml;
    var workspaceBoxHtml = render.workspaceBoxHtml;
    var workspaceTreeHtml = render.workspaceTreeHtml;
    var gitStatusHtml = render.gitStatusHtml;
    var templateBoxHtml = render.templateBoxHtml;
    var failureBadgeHtml = render.failureBadgeHtml;
    var failureSummaryHtml = render.failureSummaryHtml;
    var validationBadgeHtml = render.validationBadgeHtml;
    var workerProfilesTable = render.workerProfilesTable;
    var writeLeaseBox = render.writeLeaseBox;
    var changeStateBadge = render.changeStateBadge;
    var changesListHtml = render.changesListHtml;
    var changesRemoteView = render.changesRemoteView;
    var changeRecordsHtml = render.changeRecordsHtml;
    var changesRuntimeHtml = render.changesRuntimeHtml;
    var changeSetBadge = render.changeSetBadge;
    var deliveryManifestText = render.deliveryManifestText;
    var artifactHistoryHtml = render.artifactHistoryHtml;
    // 注入 esc / short：浏览器走 H.esc（DOM 实现），Node 测试回退 defaultEsc
    render.bindEsc(esc);
    render.bindShort(short);
    var dagSvg = render.dagSvg;
    var progressCountChip = render.progressCountChip;
    var renderProgress = render.renderProgress;



    // ---------- 三态（加载/空/失败）、提交锁、终态门控、重试委托 ----------
    // 生成统一的加载中 / 失败状态片段；retryKind 注册到重试委托。
    function stateBox(kind, msg, retryKind) {
      var cls = kind === "error" ? "hint err" : kind === "loading" ? "hint owo-ws-loading" : "hint";
      var icon = kind === "error" ? "⚠ " : kind === "loading" ? "⏳ " : "";
      var role = kind === "error" ? ' role="alert"' : kind === "loading" ? ' role="status" aria-live="polite"' : "";
      var html = '<div class="' + cls + '"' + role + ">" + icon + esc(msg) + "</div>";
      if (retryKind)
        html +=
          '<div style="margin-top:4px"><button type="button" class="owo-ws-mini" data-retry="' +
          esc(retryKind) +
          '">↻ 重试</button></div>';
      return html;
    }

    function retryAction(kind) {
      if (kind === "teams") return loadTeams();
      if (kind === "templates") return loadTemplates();
      if (kind === "proposals") return loadProposals();
      if (kind === "artifacts") return loadArtifacts();
      if (kind === "tasks") return refreshTasksOnly();
      if (kind === "deliverables") return loadDeliverables();
      if (kind.indexOf("history-") === 0) return loadArtifactHistory(kind.slice("history-".length));
      if (kind === "detail" && state.current) return loadDetail(state.current);
      return Promise.resolve();
    }

    // —— 节点重试（第三路冻结契约） ——
    // submitRetry 返回 Promise（成功/失败都展示结果）；终态成功/取消团队直接拒绝。
    function submitRetry(stepId) {
      if (!state.current) return Promise.resolve();
      var st = normStatus(currentTeamStatus());
      if (st === "succeeded" || st === "cancelled") {
        show(
          "ws-act-result",
          "err",
          "团队已进入终态（" + (TEAM_STATUS_CN[st] || st) + "），重试不可用；如需重跑请新建团队"
        );
        return Promise.resolve();
      }
      var body = buildRetryBody(stepId);
      return H.post("/teams/" + encodeURIComponent(state.current) + "/steer", body)
        .then(function (d) {
          state.interrupted = !!(d && d.interrupted); // retry 恢复会清除中断标记
          var dst = normStatus(d && d.status);
          show(
            "ws-act-result",
            "ok",
            "[重试] " +
              stepId +
              " 已受理：仅重置该节点及其未完成下游（已成功步骤、Artifact、Handoff、DecisionRecord 保持不变），运行循环已重启。团队状态：" +
              (TEAM_STATUS_CN[dst] || (d && d.status) || "—")
          );
          syncDetail();
          refreshTasksOnly();
        })
        .catch(function (e) {
          show("ws-act-result", "err", explainError(e, "重试节点 " + stepId));
        });
    }

    // 重试按钮点击入口（事件委托调用；测试挂钩直接调用）：
    // 入口处同步置提交锁——快速双击的第二击读到 data-busy=1 即被丢弃。
    function handleRetryClick(btn) {
      if (!btn || btn.disabled || btn.getAttribute("data-busy") === "1") return false;
      var stepId = btn.getAttribute("data-ws-retry");
      if (stepId == null || String(stepId) === "") return false;
      lockBtn(btn, "重试中…");
      var p = submitRetry(String(stepId));
      if (p && typeof p.then === "function") {
        p.then(
          function () {
            unlockBtn(btn);
          },
          function () {
            unlockBtn(btn);
          }
        );
      } else {
        unlockBtn(btn);
      }
      return true;
    }

    // 重试按钮走事件委托：绑定在面板 section 上（随 innerHTML 重建一起销毁），
    // 绝不能绑在 rootEl 上——app.js 用同一个 #panelRoot 反复挂载各面板，
    // 挂在根上的监听器会在多次切换后累积并污染其他面板。
    function bindDelegates(section) {
      if (!section || !section.addEventListener) return;
      section.addEventListener("click", function (ev) {
        var t = ev.target;
        if (!t || !t.getAttribute) return;
        var r = t.getAttribute("data-retry");
        if (r != null && !t.disabled) {
          ev.preventDefault();
          retryAction(String(r));
          return;
        }
        var rt = t.getAttribute("data-ws-retry");
        if (rt != null) {
          ev.preventDefault();
          handleRetryClick(t); // 忙/禁用守卫与提交锁在入口内
        }
      });
    }

    function lockBtn(btn, busyLabel) {
      if (!btn) return;
      btn.setAttribute("data-busy", "1");
      btn.setAttribute("aria-busy", "true");
      btn.__idleText = btn.textContent;
      btn.disabled = true;
      if (busyLabel) btn.textContent = busyLabel;
    }

    function unlockBtn(btn) {
      if (!btn) return;
      btn.removeAttribute("data-busy");
      btn.removeAttribute("aria-busy");
      btn.disabled = false;
      if (btn.__idleText != null) btn.textContent = btn.__idleText;
      applyGating(); // 解锁后按最新状态重新应用终态门控
    }

    // 绑定带提交锁的按钮：makePromise 返回待执行的 Promise；
    // 返回空值表示校验未通过或用户取消（不进入提交态）。双击/连点被丢弃。
    function bindLockedButton(btn, makePromise, busyLabel) {
      if (!btn) return;
      btn.addEventListener("click", function () {
        if (btn.disabled || btn.getAttribute("data-busy") === "1") return;
        var p;
        try {
          p = makePromise();
        } catch (e) {
          // 校验函数内部已展示错误，这里兜底避免未捕获异常
          if (typeof console !== "undefined" && console.error) console.error("[workswarm] 按钮处理异常:", e);
          return;
        }
        if (!p || typeof p.then !== "function") return;
        lockBtn(btn, busyLabel);
        p.then(
          function () {
            unlockBtn(btn);
          },
          function () {
            unlockBtn(btn);
          }
        );
      });
    }

    // 终态团队不再允许的操作按钮；终态时禁用并解释原因。
    var TEAM_ACTION_BTNS = ["ws-act-continue", "ws-act-cancel", "ws-act-steer-go", "ws-act-replace-go"];

    function currentTeamStatus() {
      return state.teamStatus || ((state.team && state.team.status) || "");
    }

    // 七期：停止中/已停止与终态同样不可操作（cancel 已受理，等待 Worker 退出）。
    function isGatedTeam(s) {
      var st = normStatus(s);
      return isTerminalTeam(st) || st === "stopping" || st === "stopped";
    }

    function applyGating() {
      var st = normStatus(currentTeamStatus());
      var term = isTerminalTeam(st);
      var gated = isGatedTeam(st);
      TEAM_ACTION_BTNS.forEach(function (id) {
        var b = el(id);
        if (!b) return;
        if (b.getAttribute("data-busy") === "1") return; // 提交中的锁优先
        b.disabled = gated;
        b.title = term
          ? "团队已进入终态，该操作不可用"
          : gated
            ? "团队正在停止/已停止，该操作不可用"
            : "";
      });
      var hGo = el("ws-h-go"); // 人节点结果提交：终态/停止禁用；没有未完成的人节点任务也禁用
      if (hGo && hGo.getAttribute("data-busy") !== "1") {
        var noHuman = humanTasks().length === 0;
        hGo.disabled = gated || noHuman;
        hGo.title = term
          ? "团队已进入终态，人节点结果提交不可用"
          : gated
            ? "团队正在停止/已停止，人节点结果提交不可用"
            : noHuman
              ? "当前没有未完成的人节点任务"
              : "";
      }
      var xGo = el("ws-x-go"); // 任务交接：终态/停止禁用
      if (xGo && xGo.getAttribute("data-busy") !== "1") {
        xGo.disabled = gated;
        xGo.title = term ? "团队已进入终态，交接不可用" : gated ? "团队正在停止/已停止，交接不可用" : "";
      }
      // 节点重试按钮：仅 succeeded / cancelled / stopping / stopped 隐藏为禁用
      //（failed 终态正是 retry 的合法场景）；提交中的锁优先。渲染层已按
      // shouldShowRetry 控制可见性。
      var noRetry = (function () {
        var s = normStatus(currentTeamStatus());
        return s === "succeeded" || s === "cancelled" || s === "stopping" || s === "stopped";
      })();
      var rbtns = rootEl ? rootEl.querySelectorAll("[data-ws-retry]") : [];
      for (var ri = 0; ri < rbtns.length; ri++) {
        var rb = rbtns[ri];
        if (rb.getAttribute("data-busy") === "1") continue;
        rb.disabled = noRetry;
        rb.title = noRetry
          ? "团队已成功/已取消/已停止，无可重试节点"
          : "重置此节点及其未完成下游；已成功步骤、产物与交接保持不变";
      }
      var gate = el("ws-d-gate");
      if (gate) {
        // 七期：停止中/已停止给出专门文案（其余沿用静态终态文案，不覆盖）。
        if (st === "stopping") {
          gate.textContent = "⏳ 正在停止：取消指令已受理，等待运行中的 Worker 退出（通常数秒内完成）…";
        } else if (st === "stopped") {
          gate.textContent = "⛔ 团队已停止：运行操作已停用（任务进度、审计与产物保留，可继续查看）。";
        }
        gate.style.display = gated ? "" : "none";
      }
    }

    // 结果区与实时通道标注 aria-live，供读屏器播报状态变化。
    function markLiveRegions() {
      ["ws-create-result", "ws-act-result", "ws-h-result", "ws-x-result"].forEach(function (id) {
        var n = el(id);
        if (n) {
          n.setAttribute("aria-live", "polite");
          n.setAttribute("role", "status");
        }
      });
      var s = el("ws-d-stream");
      if (s) s.setAttribute("aria-live", "polite");
      var dErr = el("ws-d-error");
      if (dErr) dErr.setAttribute("aria-live", "assertive");
      var badge = el("ws-d-status");
      if (badge) badge.setAttribute("aria-live", "polite");
    }

    // 同时接受裸 ID（"ws-main"）与 "#selector"（"#ws-main"）。
    // 历史 bug：调用方传 "#id" 时这里会拼出 "##id"，querySelector 直接抛
    // SyntaxError，renderView()/mount() 随之中断，面板首次打开即空白。
    // 统一归一化后查询；对残留的非法选择器兜底返回 null 而不是抛异常。
    function el(sel) {
      if (!rootEl || sel == null) return null;
      var q = String(sel);
      if (!q) return null;
      if (q.charAt(0) !== "#") q = "#" + q;
      try {
        return rootEl.querySelector(q);
      } catch (e) {
        return null;
      }
    }

    function short(s, n) {
      s = String(s == null ? "" : s);
      return s.length > n ? s.slice(0, n) + "…" : s;
    }

    function splitCsv(input) {
      var v = input && input.value;
      if (!v) return [];
      return v
        .split(",")
        .map(function (x) {
          return x.trim();
        })
        .filter(Boolean);
    }

    // show() 的 id 约定为裸 ID（如 "ws-act-result"），交由 el() 补 "#"，
    // 避免出现 "##id"；保留元素原有的 "sub" 修饰类不被覆盖。
    function show(id, kind, text) {
      var box = el(id);
      if (!box) return;
      box.textContent = text || "—";
      box.className =
        "owo-ws-result" +
        (box.classList.contains("sub") ? " sub" : "") +
        (kind === "ok" ? " ok" : kind === "err" ? " err" : "");
    }

    // 可操作的错误文案：解析 api() 抛出的 "NNN: body"，4xx 给出具体指引。
    // 原始响应体超长时截断（保留 full 参数回传），避免整段 HTML/日志刷屏。
    function explainError(err, ctx) {
      var msg = String((err && err.message) || err || "");
      var m = msg.match(/^(\d{3}):/);
      var extra = ctx ? "（" + ctx + "）" : "";
      if (m) {
        var st = Number(m[1]);
        var raw = msg.slice(m[0].length).trim();
        var detail = "";
        try {
          var o = JSON.parse(raw);
          if (o && (o.error || o.message)) detail = String(o.error || o.message);
        } catch (e) {
          detail = raw;
        }
        if (/OPENAI_API_KEY/i.test(detail)) {
          detail += "。提示：请为 owo-agent-server 设置 OPENAI_API_KEY 环境变量后重启服务";
        }
        detail = short(detail || raw, 240);
        switch (st) {
          case 400:
            return "请求被拒绝（400）：" + (detail || "参数不合法") + extra;
          case 401:
            return "认证失败（401）：访问令牌可能已失效，请刷新页面重新获取后重试" + extra;
          case 403:
            return "权限不足（403）：" + (detail || "当前令牌无权执行该操作，请联系管理员或检查角色配置") + extra;
          case 404:
            return "资源不存在（404）：" + (detail || "") + "。请检查团队/任务 ID 是否正确，或刷新团队列表" + extra;
          case 409:
            return "状态冲突（409）：" + (detail || "") + "。团队可能已终结或正在运行/等待人工，无法重复该操作；可先刷新状态再决定" + extra;
          case 500:
            return "服务内部错误（500）：" + (detail || "") + "。请稍后重试；若持续出现请查看 owo-agent-server 日志" + extra;
          case 502:
          case 503:
          case 504:
            return "服务暂不可用（HTTP " + st + "）：" + (detail || "") + "。请确认服务进程存活、端口未被占用后再重试" + extra;
          default:
            return "服务端错误（HTTP " + st + "）：" + (detail || "") + extra;
        }
      }
      if (msg === "Failed to fetch" || /network|CORS|load failed/i.test(msg)) {
        return "无法连接 owo-agent-server（" + (H.baseUrl || "http://127.0.0.1:4096") + "）：请确认服务已启动（如 cd agent-sdk && cargo run -p owo-agent-server），并检查端口与防火墙设置" + extra;
      }
      if (H.friendlyError) {
        try {
          return H.friendlyError(err, { resource: true });
        } catch (e2) {
          /* 回退原文 */
        }
      }
      // 原写法 msg || "未知错误" + extra 有优先级问题：有 msg 时会丢掉上下文。
      return (msg || "未知错误") + extra;
    }

    // ---------- 审计事件（去重） ----------
    function addAudit(entry) {
      if (!entry || !entry.event) return;
      var key = [entry.ts, entry.event, entry.detail].join("|");
      if (state.auditKeys[key]) return;
      state.auditKeys[key] = true;
      state.audit.unshift(entry);
      if (state.audit.length > 300) {
        var dropped = state.audit.pop();
        delete state.auditKeys[[dropped.ts, dropped.event, dropped.detail].join("|")];
      }
    }

    function paintAudit() {
      var box = el("#ws-d-audit");
      if (!box) return;
      if (!state.audit.length) {
        box.innerHTML = '<div class="hint">暂无审计事件</div>';
        return;
      }
      box.innerHTML = state.audit
        .map(function (e) {
          return (
            '<div class="owo-ws-audit-row">' +
            '<span class="owo-ws-audit-ts">' + esc(e.ts) + "</span>" +
            '<span class="owo-ws-audit-ev">' + esc(e.event) + "</span>" +
            '<span class="owo-ws-audit-det" title="' + esc(e.detail) + '">' + esc(short(e.detail, 200)) + "</span>" +
            "</div>"
          );
        })
        .join("");
    }

    // ==================== 四期：实时进度（第二路 progress 事件） ====================
    // progress 事件形状（协作计划冻结）：
    // { seq, team_id, status, active, current_steps:[{step_id,worker,status,attempts,started_at}],
    //   counts:{pending,running,succeeded,failed}, updated_at }
    var PROGRESS_STEP_CN = {
      pending: "等待",
      ready: "就绪",
      running: "运行中",
      succeeded: "完成",
      failed: "失败",
      aborted: "中止",
      cancelled: "已取消",
    };

    // 应用一条 progress 事件：seq 单调递增守卫 —— 断线重连后的重复推送、
    // 轮询快照里的旧事件一律跳过（返回 false），保证"重复事件不得重复渲染"。
    // lastProgressSeq 不因重连/轮询切换而重置；仅在切换团队（loadDetail）时清零。
    function applyProgress(evt) {
      if (!evt || typeof evt !== "object") return false;
      var seq = Number(evt.seq);
      if (!isFinite(seq)) return false; // 无有效 seq 的形状一律不采纳
      if (seq <= state.lastProgressSeq) return false; // 旧/重复事件
      state.lastProgressSeq = seq;
      var steps = Array.isArray(evt.current_steps) ? evt.current_steps : [];
      state.progress = {
        seq: seq,
        status: String(evt.status || ""),
        active: !!evt.active,
        current_steps: steps
          .map(function (s) {
            s = s || {};
            return {
              step_id: String(s.step_id || ""),
              worker: String(s.worker || ""),
              status: String(s.status || ""),
              attempts: Number(s.attempts) || 0,
              started_at: String(s.started_at || ""),
            };
          })
          .filter(function (s) {
            return s.step_id;
          }),
        counts: {
          pending: Number(evt.counts && evt.counts.pending) || 0,
          running: Number(evt.counts && evt.counts.running) || 0,
          succeeded: Number(evt.counts && evt.counts.succeeded) || 0,
          failed: Number(evt.counts && evt.counts.failed) || 0,
        },
        updated_at: String(evt.updated_at || ""),
      };
      if (state.progress.status) {
        state.teamStatus = state.progress.status; // progress 是最新状态源
        state.active = state.progress.active;
      }
      return true;
    }


    // 进度视图模型（纯函数）：耗时基于 started_at 与 nowMs；rows 只收有 step_id 的步骤。
    // 无 progress 快照但用户已点取消：仍返回最小视图，让"取消中"徽标立即可见。
    function computeProgressView(nowMs) {
      var p = state.progress;
      if (!p) {
        if (!state.cancelling) return null;
        return { seq: null, counts: { pending: 0, running: 0, succeeded: 0, failed: 0 }, updated_at: "", rows: [], cancelling: true, active: false };
      }
      var now = typeof nowMs === "number" && isFinite(nowMs) ? nowMs : Date.now();
      var rows = (p.current_steps || []).map(function (s) {
        var started = Date.parse(s.started_at);
        var elapsed = isFinite(started) ? Math.max(0, now - started) : null;
        var st = normStatus(s.status);
        return {
          step_id: s.step_id,
          worker: s.worker,
          status: st,
          statusCn: PROGRESS_STEP_CN[st] || s.status,
          attempts: s.attempts,
          elapsedMs: elapsed,
          running: st === "running",
        };
      });
      return {
        seq: p.seq,
        counts: p.counts,
        updated_at: p.updated_at,
        rows: rows,
        cancelling: state.cancelling,
        active: p.active,
      };
    }

    // 进度计数片段（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 实时进度视图（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    function paintProgress() {
      var box = el("#ws-d-progress");
      if (!box) return;
      var vm = computeProgressView(Date.now());
      box.innerHTML = renderProgress(vm);
      var hasRunning = vm && vm.rows.some(function (r) {
        return r.running;
      });
      if (hasRunning) ensureProgressTimer();
      else stopProgressTimer();
    }

    // 1s tick：仅在存在 Running 步骤时刷新"已运行时间"；无活动步骤自动停止。
    function ensureProgressTimer() {
      if (state.progressTimer) return;
      state.progressTimer = setInterval(function () {
        if (el("#ws-d-progress")) paintProgress();
        else stopProgressTimer(); // 视图已切走
      }, 1000);
    }

    function stopProgressTimer() {
      if (state.progressTimer) {
        clearInterval(state.progressTimer);
        state.progressTimer = null;
      }
    }

    // ==================== 四期：Artifact 版本链与评审（第三路 review API） ====================
    // 评审接口（协作计划冻结）：
    //   POST /artifacts/{id}/review  body {team_id, decision, reviewer, comment, expected_version, idempotency_key}
    //   GET  /artifacts/{id}/history → 不可变 ArtifactReviewRecord 列表
    // 评审状态文案常量（随渲染簇迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）
    // 评审状态配色常量（随渲染簇迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）
    var REVIEW_DECISIONS = ["approve", "request_changes", "reject"];
    var DECISION_CN = { approve: "批准", request_changes: "要求修改", reject: "驳回" };


    // 评审状态徽标（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    function isReviewable(a) {
      return !!a && normReviewState(a.review_state) === "pendingreview" && !state.reviewBusy[String(a.artifact_id)];
    }

    // 版本链分组（纯函数）：supersedes_artifact_id 指向链内既有产物则续链；
    // 链内按 version 升序；链头（items 末位）为最新版本。

    // 评审请求体（纯函数）：expected_version 乐观并发控制 + 幂等键；生产者禁止自行批准。
    function buildReviewBody(input) {
      var a = input.artifact || {};
      var decision = String(input.decision || "");
      if (REVIEW_DECISIONS.indexOf(decision) < 0) throw new Error("未知评审动作：" + decision);
      var reviewer = String(input.reviewer || "").trim();
      if (!reviewer) throw new Error("请先填写评审者（critic 或 human 用户名）");
      if (decision === "approve") {
        var producer = String(a.producer || "");
        var producerRole = producer.replace(/^m-/, "");
        if (reviewer === producer || reviewer === producerRole) {
          throw new Error("生产者不能自行批准自己的产物（approve 需 Human 策略授权的 critic/human 执行）");
        }
      }
      return {
        team_id: input.teamId || state.current || "",
        decision: decision,
        reviewer: reviewer,
        comment: String(input.comment == null ? "" : input.comment),
        expected_version: Number(a.version) || 0,
        idempotency_key: input.idempotencyKey || aidemKey(a, decision, reviewer),
      };
    }

    // 幂等键：不同意图的提交生成新键（时间戳 + 进程内序号，同毫秒两次提交也互异）；
    // 同键重复提交由服务端保证零副作用。

    // 评审错误文案：409/403 用计划规定的可操作提示，其余退回 explainError。
    // 传输层把状态码嵌在 message 头部（"409: {...}"），此处一并识别。
    function explainReviewError(err) {
      var st = err && err.status;
      if (!st && err && typeof err.message === "string") {
        var m = /^(\d{3}):/.exec(err.message);
        if (m) st = Number(m[1]);
      }
      if (st === 409) return "版本已更新，请刷新后重试（你提交的 expected_version 已过期，可能有更新的评审或版本）";
      if (st === 403) return "无评审权限（403）：生产者不能自行批准自己的产物；请使用已授权的 critic/human 身份重试";
      return explainError(err, "评审提交");
    }

    // 提交评审（transport 可注入，Node 测试直接调用）。结果与错误同时记录到
    // state.reviewResult，供 DOM 层与测试读取；按钮锁定由 reviewBusy 保证。
    function submitArtifactReview(opts) {
      var aid = String((opts && opts.artifactId) || "");
      var a = findArtifactById(aid);
      if (!a) return Promise.reject(new Error("产物不存在或列表已刷新，请刷新后重试"));
      var body;
      try {
        body = buildReviewBody({
          artifact: a,
          decision: opts && opts.decision,
          reviewer: opts && opts.reviewer,
          comment: opts && opts.comment,
          idempotencyKey: opts && opts.idempotencyKey,
          teamId: opts && opts.teamId,
        });
      } catch (e) {
        return Promise.reject(e);
      }
      if (state.reviewBusy[aid]) return Promise.reject(new Error("该产物已有评审提交进行中，请等待完成"));
      state.reviewBusy[aid] = true;
      return H.post("/artifacts/" + encodeURIComponent(aid) + "/review", body).then(
        function (resp) {
          delete state.reviewBusy[aid];
          state.reviewResult = { ok: true, artifactId: aid, decision: body.decision, reviewer: body.reviewer, resp: resp || null };
          state.reviewFlash = { artifactId: aid, ok: true, text: "评审已提交：" + (DECISION_CN[body.decision] || body.decision) + "（记录不可变）" };
          return resp;
        },
        function (e) {
          delete state.reviewBusy[aid];
          state.reviewResult = { ok: false, artifactId: aid, decision: body.decision, reviewer: body.reviewer, error: explainReviewError(e) };
          state.reviewFlash = { artifactId: aid, ok: false, text: explainReviewError(e) };
          throw e;
        }
      );
    }

    function findArtifactById(aid) {
      var list = state.artifacts || [];
      for (var i = 0; i < list.length; i++) {
        if (String(list[i].artifact_id) === String(aid)) return list[i];
      }
      return null;
    }

    // ---------- 五期：组队策略判定展示 ----------
    // 容错读取团队详情里的策略判定（第一路 TeamStrategyEngine 落地前后均可工作）。
    // 接受 detail.strategy_decision 或 detail.strategy；reasons 兼容 reason/reasons/why。

    // 自适应组队策略盒（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // ---------- 五期：角色指标（GET /teams/{id}/metrics 容错归一） ----------


    // 角色指标卡片（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // ---------- 五期：版本时间线与 v1/v2 差异 ----------
    // 简单 LCS 行差异（预览文本短，O(n·m) 可接受；超长截断保护）。

    // 行级 diff 视图（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 产物版本链时间线（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // ---------- 五期：根据评审意见返工（POST /artifacts/{id}/rework） ----------
    function buildReworkBody(input) {
      var aid = String((input && input.artifactId) || "");
      if (!aid) throw new Error("缺少 artifactId：无法定位要返工的产物");
      var teamId = String((input && input.teamId) || state.current || "");
      if (!teamId) throw new Error("缺少 team_id：请先打开团队详情再发起返工");
      var reviewId = String((input && input.reviewId) || "");
      if (!reviewId) throw new Error("缺少 review_id：请先加载该产物的评审历史（要求修改的评审记录）");
      var instruction = String((input && input.instruction) || "").trim();
      if (!instruction) throw new Error("返工指令不能为空：请填写要修正的内容（可从评审意见预填）");
      return {
        team_id: teamId,
        review_id: reviewId,
        instruction: instruction,
        idempotency_key: (input && input.idempotencyKey) || aidemKey({ artifact_id: aid }, "rework", "rework"),
      };
    }

    function explainReworkError(err) {
      var st = err && err.status;
      if (!st && err && typeof err.message === "string") {
        var m = /^(\d{3}):/.exec(err.message);
        if (m) st = Number(m[1]);
      }
      if (st === 409) return "该评审已创建过返工任务（409 幂等冲突）；请刷新产物列表查看返工进度，不要重复创建";
      if (st === 404) return "产物或评审记录不存在（404）：可能已被刷新移除，请刷新后重试";
      return explainError(err, "返工提交");
    }

    function submitRework(opts) {
      var aid = String((opts && opts.artifactId) || "");
      var body;
      try {
        body = buildReworkBody({
          artifactId: aid,
          teamId: (opts && opts.teamId) || state.current,
          reviewId: opts && opts.reviewId,
          instruction: opts && opts.instruction,
          idempotencyKey: opts && opts.idempotencyKey,
        });
      } catch (e) {
        return Promise.reject(e);
      }
      if (state.reworkBusy[aid]) return Promise.reject(new Error("该产物已有返工提交进行中，请等待完成"));
      state.reworkBusy[aid] = true;
      return H.post("/artifacts/" + encodeURIComponent(aid) + "/rework", body).then(
        function (resp) {
          delete state.reworkBusy[aid];
          state.reworkResult = { ok: true, artifactId: aid, resp: resp || null };
          state.reviewFlash = { artifactId: aid, ok: true, text: "返工任务已受理（" + ((resp && resp.replayed) ? "幂等重放既有任务" : "新任务") + "）；v2 生成后自动进入评审" };
          return resp;
        },
        function (e) {
          delete state.reworkBusy[aid];
          state.reworkResult = { ok: false, artifactId: aid, error: explainReworkError(e) };
          state.reviewFlash = { artifactId: aid, ok: false, text: explainReworkError(e) };
          throw e;
        }
      );
    }

    // ---------- 五期：最终交付物（GET /projects/{id}/deliverables 容错三桶） ----------

    // 交付物清单盒（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // ---------- 六期：工作区绑定 / 模板信息 / 输出契约失败原因 ----------
    /// TeamRun.workspace（六期冻结契约）容错归一：字段缺失/旧记录均安全。

    /// 工作区框 HTML（root/读写模式/允许范围/深度 + 目录树与 Git 状态按钮）。
    // 项目工作区盒（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// 目录树载荷 HTML（扁平列表按层级缩进渲染）。
    // 工作区目录树（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// Git 状态载荷 HTML（双形状：porcelain 行字符串数组 / 冻结契约对象数组）。
    // 工作区 Git 状态（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// 模板信息框（使用的模板及版本；版本经目录懒加载解析）。
    // 模板信息盒（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// 失败原因代码 → 中文标签（六期输出契约失败原因）。

    /// 单个失败任务的失败原因徽章（failure_code 优先，error 前缀兜底）。
    // 失败状态徽标（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// 详情失败原因汇总（所有失败步骤的代码列表；无失败 → 空串）。
    // 失败摘要（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    /// 拉取工作区绑定（/projects/{pid}/workspace；路由未接线时静默降级）。
    function loadWorkspace() {
      var tid = state.current;
      if (!tid) return Promise.resolve();
      return H.get("/projects/proj-" + encodeURIComponent(tid) + "/workspace").then(function (d) {
        // 双形状：{workspace:{...}} 包装（二路实现）或直接绑定对象（冻结契约）。
        var ws = d && d.workspace && typeof d.workspace === "object" ? d.workspace : d;
        state.workspace = ws && ws.root
          ? workspaceFromTeam({ workspace: ws }) || {
              root: String(ws.root),
              readOnly: ws.read_only != null ? !!ws.read_only : true,
              writePaths: Array.isArray(ws.write_allowed_paths) ? ws.write_allowed_paths.map(String) : [],
              treeDepth: ws.tree_depth != null ? Number(ws.tree_depth) : null,
            }
          : null;
        paintWorkspace();
      }).catch(function () {
        /* 404（未绑定/未接线）→ 保持 team.workspace 或空态，不打扰 */
        paintWorkspace();
      });
    }

    function loadWorkspaceView(kind) {
      var tid = state.current;
      if (!tid || state.workspaceBusy) return Promise.resolve();
      state.workspaceBusy = true;
      var path = kind === "git"
        ? "/projects/proj-" + encodeURIComponent(tid) + "/workspace/git-status"
        : "/projects/proj-" + encodeURIComponent(tid) + "/workspace/tree" + (state.workspace && state.workspace.treeDepth ? "?depth=" + encodeURIComponent(String(state.workspace.treeDepth)) : "");
      paintWorkspace();
      return H.get(path).then(function (d) {
        state.workspaceView = d || null;
        state.workspaceViewKind = kind;
      }).catch(function (e) {
        state.workspaceView = { error: friendly(e) };
        state.workspaceViewKind = kind;
      }).then(function () {
        state.workspaceBusy = false;
        paintWorkspace();
      });
    }

    function paintWorkspace() {
      var box = el("#ws-d-workspace");
      if (box)
        box.innerHTML =
          templateBoxHtml(state.team, state.templateInfo) +
          workspaceBoxHtml(state.workspace, state.workspaceView, state.workspaceViewKind, state.workspaceBusy);
      var bind = el("#ws-ws-tree");
      if (bind)
        bind.onclick = function () {
          loadWorkspaceView("tree");
        };
      var git = el("#ws-ws-git");
      if (git)
        git.onclick = function () {
          loadWorkspaceView("git");
        };
      var refresh = el("#ws-ws-refresh");
      if (refresh)
        refresh.onclick = function () {
          state.workspaceView = null;
          state.workspaceViewKind = "";
          loadWorkspace();
        };
      var fails = el("#ws-d-failures");
      if (fails) fails.innerHTML = failureSummaryHtml(state.tasks);
    }

    /// 模板版本解析（经模板目录；目录路由不可用时静默）。
    function loadTemplateInfo() {
      var tid = state.team && state.team.template_id;
      if (!tid || (state.templateInfo && state.templateInfo.template_id === tid)) return Promise.resolve();
      return H.get("/teams/templates/catalog").then(function (d) {
        var list = (d && d.catalog) || [];
        for (var i = 0; i < list.length; i++) {
          var e = list[i];
          if (!e || typeof e !== "object") continue;
          // 双形状：冻结契约顶层 / 三路实现嵌套 template{}
          var eid = String(e.template_id || (e.template && e.template.template_id) || "");
          if (eid === tid) {
            state.templateInfo = {
              template_id: tid,
              version: e.version != null ? e.version : null,
              title: String(e.title || (e.template && e.template.name) || ""),
            };
            break;
          }
        }
        var box = el("#ws-d-workspace");
        if (box) {
          // 模板行与工作区同区，重画一次。
          box.innerHTML = workspaceBoxHtml(state.workspace, state.workspaceView, state.workspaceViewKind, state.workspaceBusy);
          paintWorkspace();
        }
      }).catch(function () { /* 目录不可用 → 仅显示 template_id */ });
    }

    function paintTeamConversation() {
      var box = el("#ws-d-conversation-messages");
      if (!box) return;
      box.innerHTML = render.teamConversationHtml(
        state.handoffs,
        (state.team && state.team.members) || [],
        typeof win.renderMarkdown === "function" ? win.renderMarkdown : null,
        state.conversationError
      );
    }

    // Show only persisted, server-redacted handoffs; the API does not retain private
    // per-turn model messages, so this view must not imply a complete transcript.
    function loadTeamConversation(teamId, force) {
      var tid = String(teamId || state.current || "");
      if (!tid) return Promise.resolve(null);
      if (!force && state.conversationRequest) return state.conversationRequest;
      if (!force && state.handoffs !== null) return Promise.resolve(state.handoffs);
      var requestSeq = ++state.conversationSeq;
      var detailVersion = detailSeq;
      state.handoffs = null;
      state.conversationError = "";
      paintTeamConversation();
      var request = H.get("/teams/" + encodeURIComponent(tid) + "/diagnostic")
        .then(function (d) {
          if (requestSeq !== state.conversationSeq || tid !== state.current || detailVersion !== detailSeq) return null;
          state.handoffs = Array.isArray(d && d.handoffs) ? d.handoffs : [];
          state.conversationError = "";
          paintTeamConversation();
          return state.handoffs;
        })
        .catch(function (error) {
          if (requestSeq !== state.conversationSeq || tid !== state.current || detailVersion !== detailSeq) return null;
          state.handoffs = [];
          state.conversationError = explainError(error, "团队交接对话");
          paintTeamConversation();
          return null;
        })
        .finally(function () {
          if (requestSeq === state.conversationSeq) state.conversationRequest = null;
        });
      state.conversationRequest = request;
      return request;
    }

    // ---------- 五期：下载脱敏诊断（GET /teams/{id}/diagnostic） ----------
    function downloadDiagnostic(teamId) {
      var tid = String(teamId || state.current || "");
      if (!tid) return Promise.reject(new Error("缺少 team_id：请先打开团队详情"));
      return H.get("/teams/" + encodeURIComponent(tid) + "/diagnostic").then(function (d) {
        state.diagnostic = d || null;
        if (typeof Blob === "function" && typeof win.URL !== "undefined" && win.URL.createObjectURL) {
          var blob = new Blob([JSON.stringify(d, null, 2)], { type: "application/json" });
          var a = win.document.createElement("a");
          a.href = win.URL.createObjectURL(blob);
          a.download = "diagnostic-" + tid + ".json";
          win.document.body.appendChild(a);
          a.click();
          setTimeout(function () {
            win.URL.revokeObjectURL(a.href);
            a.remove();
          }, 400);
        }
        return d;
      });
    }

    // ---------- 七期：Worker 能力 / 写租约 / 文件变更 / 产物校验与下载交付 ----------

    // 下载文件名：artifact_id + 按格式推断的扩展名（未知格式回退原串/txt）。

    // 绝对时间格式化（fmtMs 是时长格式化器，写租约时间戳另用）。

    // 产物格式校验徽标：validation 为七期可选字段——缺失/未校验时不渲染（旧产物兼容）。
    // 产物校验徽标（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // WorkerProfile 表：角色 × 实际工具权限 × 调用预算（max_turns）。全字段容错。
    // Worker 能力表（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 单写租约状态盒：write_lease 归一后传入（null=未持有；released_at_ms 非空=已释放）。
    // 单写租约状态盒（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 文件变更列表 + diff 预览（changes[].state: added|modified|deleted；白名单外变更
    // 服务端会判 scope_violation，不会出现在成功登记的变更集中）。
    // 文件变更状态文案（随渲染簇迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）
    // 文件变更状态配色（随渲染簇迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 文件变更状态徽标（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 文件变更列表（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 七期（二路交接）：GET /projects/{pid}/workspace/changes 归一视图。
    // 响应 {team_id, git, changed_files[], diff_summary, has_violation, records[]}；
    // 记录元素 {role, step, at, git, changed_files[], diff_summary, diff_ref?, violation?}。
    // 全字段容错：非对象/缺键 → 缺省（空串/空数组/false/null）。
    // 远程 changes 归一视图（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 逐步骤变更记录行：越界红徽标 / 通过静默；diff_summary 以差异容器呈现。
    // 写越界记录列表（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 变更区组装：二路端点数据（越界警示 + diff 摘要 + 逐步骤记录）优先；
    // 详情 changes[] 文件清单补充在后；两者皆空 → 既有空态文案。
    // 文件变更运行时视图（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 七期详情区一次性重绘（容器仅在详情视图存在；缺容器时静默）。
    function paintSevenRuntime() {
      var pb = el("#ws-d-profiles");
      if (pb) pb.innerHTML = workerProfilesTable(state.workerProfiles);
      var lb = el("#ws-d-lease");
      if (lb) lb.innerHTML = writeLeaseBox(state.writeLease);
      var cb = el("#ws-d-changes");
      if (cb) cb.innerHTML = changesRuntimeHtml(state.changesRemote, state.changes);
      paintChangeSets();
    }

    // 二路 changes 端点拉取（详情打开时一次；404/失败容错为 null，不影响详情 changes[] 回退）。
    function loadWorkspaceChanges() {
      var pid = projectIdOfTeam();
      if (!pid) return Promise.resolve();
      return H.get("/projects/" + encodeURIComponent(pid) + "/workspace/changes")
        .then(function (d) {
          state.changesRemote = changesRemoteView(d);
          paintSevenRuntime();
        })
        .catch(function () {
          state.changesRemote = null; // 端点未上线/项目未绑定 → 详情 changes[] 兜底
        });
    }

    // ===========================================================================
    // 八期（二路交接）：ChangeSet 审批闭环 —— 列表 / 状态徽标 / accept-reject-revert。
    // 口径（AGENTS-COORD 八期冻结②）：
    //   GET  /teams/{id}/change-sets        {team_id, change_sets:[ChangeSet]}
    //   POST /change-sets/{id}/accept|reject|revert   200 {change_set, replayed?}
    // ChangeSet: {change_set_id, team_id, step_id, role?, base_hashes{}, result_hashes{},
    //   changed_files[], diff_ref?, status: pending_review|accepted|rejected|reverted|
    //   conflicted, decisions?[], created_at, resolved_at?}（全字段容错）。
    // ===========================================================================

    // 变更集状态文案（随渲染簇迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）
    var CS_ACTION_CN = { accept: "接受", reject: "拒绝", revert: "撤销" };



    // 变更集状态徽标（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 九期：状态行明确口径（与审批门控语义一致，服务端 approval_block_reason 同源）。

    // 九期：批准门控视图（GET /teams/{id}/change-sets 的 approval_blocked/reason）。

    // 九期：批准门控横幅——ChangeSet 未处理（pending_review/conflicted）时，该团队
    // 代码 Artifact 可评审但不能成为最终 approved head（批准按钮同时被禁用）。
    // 门控横幅（视图模型化四迁）：纯逻辑在 render.js，本层只切片 state。
    // 批准被门控阻断：ChangeSet 未处理（待审批/冲突）时先接受/拒绝才能批准 Artifact。
    function approvalBlockBanner() {
      return render.approvalBlockBanner(state.csApprovalBlock);
    }

    function changeSetsHtml(list) {
      return render.changeSetsHtml(list, {
        results: state.csResults,
        busy: state.csBusy,
        approvalBlock: state.csApprovalBlock,
      });
    }

    // 重绘 + 事件委托（容器标记防重复绑定；detail 重建后容器为新元素、标记自然清零）。
    function paintChangeSets() {
      var sb = el("#ws-d-csets");
      if (!sb) return;
      sb.innerHTML = changeSetsHtml(state.changeSets);
      if (!sb.dataset.csBound) {
        sb.dataset.csBound = "1";
        sb.addEventListener("click", function (ev) {
          var btn = ev.target && ev.target.closest ? ev.target.closest("[data-cs-act]") : null;
          if (btn) {
            startChangeSetAction(btn.getAttribute("data-cs-id") || "", btn.getAttribute("data-cs-act") || "");
          }
        });
      }
    }

    // 拉取（详情打开时一次）：404/失败容错为 null（端点未上线 → 空态，不阻塞详情）。
    // 九期：同时捕获批准门控（approval_blocked/approval_block_reason）供横幅与
    // Artifact 批准按钮禁用使用。
    function loadChangeSets() {
      var tid = state.current;
      if (!tid) return Promise.resolve();
      return H.get("/teams/" + encodeURIComponent(tid) + "/change-sets")
        .then(function (d) {
          state.changeSets = changeSetsView(d);
          state.csApprovalBlock = approvalBlockView(d);
          paintChangeSets();
        })
        .catch(function () {
          state.changeSets = null;
          state.csApprovalBlock = null;
        });
    }

    function paintChangeSetResult(csId) {
      var sb = el("#ws-d-csets");
      if (!sb) return;
      var res = state.csResults[csId];
      var div = sb.querySelector('[data-cs-result="' + String(csId).replace(/"/g, '\\"') + '"]');
      if (div) {
        div.textContent = res ? res.text : "";
        div.className = "owo-ac-result" + (res ? (res.ok ? " ok" : " bad") : "");
      }
    }

    // 幂等键（九期修复）：服务端要求请求体携带 idempotency_key（缺失 → 422，
    // 八期 UI 一直发空体属隐性缺陷）。每次点击生成新键：提交锁保证双击只发一次；
    // 失败后再次点击是新一轮真实决定（conflicted 处理完后重试恢复正是期望行为）。

    // 九期：跨面板刷新「待我处理」（Action Center 已挂载时；未挂载/失败静默，
    // 不阻塞本面板——accept/reject 后待办应立即从 Inbox 消失）。
    function refreshInboxPanel() {
      var ac = win.OwoPanels && win.OwoPanels["action-center"];
      if (ac && typeof ac.refreshInbox === "function") {
        try {
          ac.refreshInbox();
        } catch (e) {
          /* 跨面板刷新失败不阻塞本面板 */
        }
      }
    }

    // accept/reject/revert：幂等重放（replayed）与 409 冲突（文件被用户再次修改、
    // 恢复被拒绝且不覆盖新内容）文案如实提示；动作完成后同步重拉 ChangeSet 列表、
    // 产物/评审区（门控状态变化）并跨面板刷新 Inbox。
    function startChangeSetAction(csId, action) {
      var id = String(csId || "");
      var act = String(action || "");
      if (!id || ["accept", "reject", "revert"].indexOf(act) < 0) return Promise.resolve();
      var key = id + ":" + act;
      if (state.csBusy[key]) return Promise.resolve();
      state.csBusy[key] = true;
      paintChangeSets();
      return H.post("/change-sets/" + encodeURIComponent(id) + "/" + act, {
        idempotency_key: csIdemKey(id, act),
      })
        .then(function (resp) {
          var replayed = !!(resp && resp.replayed);
          var cs = resp && resp.change_set && resp.change_set.status ? normCsStatus(resp.change_set.status) : "";
          state.csResults[id] = {
            ok: true,
            text: replayed
              ? "已" + (CS_ACTION_CN[act] || act) + "（幂等重放，无重复副作用）。"
              : cs === "conflicted"
                ? "检测到冲突：文件已被用户再次修改，未覆盖新内容，ChangeSet 置为 conflicted。"
                : "已" + (CS_ACTION_CN[act] || act) + "。",
          };
        })
        .catch(function (e) {
          var msg = String((e && e.message) || e || "");
          state.csResults[id] = {
            ok: false,
            text: /^409/.test(msg)
              ? "冲突（409）：文件已被用户再次修改，恢复被拒绝、未覆盖新内容。"
              : "操作失败：" + msg,
          };
        })
        .then(function () {
          state.csBusy[key] = false;
          refreshInboxPanel();
          return loadChangeSets().then(function () {
            // 门控解除/保持都影响评审区（批准按钮可用性），一并重拉产物。
            return loadArtifacts();
          });
        });
    }

    // 浏览器文本下载（Node 测试环境无 Blob/document → 返回 false 不抛错）。
    function saveTextFile(name, text, mime) {
      if (typeof Blob !== "function" || !win.document || !win.URL || !win.URL.createObjectURL) return false;
      var blob = new Blob([String(text == null ? "" : text)], { type: mime || "text/plain;charset=utf-8" });
      var a = win.document.createElement("a");
      a.href = win.URL.createObjectURL(blob);
      a.download = String(name || "download.txt");
      win.document.body.appendChild(a);
      a.click();
      setTimeout(function () {
        win.URL.revokeObjectURL(a.href);
        a.remove();
      }, 400);
      return true;
    }

    // Artifact 内容下载（GET /artifacts/{id}/content；路由未上线/404 → 行级闪存提示）。
    function downloadArtifact(aid) {
      var id = String(aid == null ? "" : aid);
      if (!id) return Promise.resolve();
      var a = findArtifactById(id) || { artifact_id: id };
      return H.get("/artifacts/" + encodeURIComponent(id) + "/content")
        .then(function (d) {
          var info = d && typeof d === "object" ? d : {};
          saveTextFile(
            artifactFileName({ artifact_id: info.artifact_id || a.artifact_id, format: info.format || a.format }),
            info.content || ""
          );
          return d;
        })
        .catch(function (e) {
          state.reviewFlash = { artifactId: id, ok: false, text: "下载失败：" + explainError(e, "产物内容下载") };
          loadArtifacts(); // 重绘产物区以显示行级提示（reviewFlash 跨重绘保留）
        });
    }

    // 交付清单文本化（容错缺字段）：project/generated_at/逐项 版本·哈希·大小·批准态·content_url。
    // 交付清单文本（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    // 交付清单下载（GET /projects/{pid}/delivery-manifest；路由未上线 → ws-act-result 提示）。
    function downloadDeliveryManifest() {
      var t = state.team;
      var pid = (t && t.project_space_id) || (state.current ? "proj-" + state.current : "");
      if (!pid) return Promise.reject(new Error("缺少项目空间 id：请先打开团队详情"));
      return H.get("/projects/" + encodeURIComponent(pid) + "/delivery-manifest")
        .then(function (d) {
          saveTextFile("delivery-manifest-" + pid + ".txt", deliveryManifestText(d));
          return d;
        })
        .catch(function (e) {
          show("ws-act-result", "err", "交付清单下载失败：" + explainError(e, "交付清单下载"));
          throw e; // 交由 bindLockedButton 解锁
        });
    }

    // 评审历史（懒加载）：GET /artifacts/{id}/history → 不可变记录列表。
    // 评审历史记录（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）

    function loadArtifactHistory(aid) {
      var box = el("#ws-d-artifacts");
      if (!box) return Promise.resolve();
      var target = null;
      var boxes = box.querySelectorAll("[data-art-history-box]");
      for (var i = 0; i < boxes.length; i++) {
        if (boxes[i].getAttribute("data-art-history-box") === String(aid)) target = boxes[i];
      }
      if (!target) return Promise.resolve();
      target.innerHTML = '<div class="hint">加载评审历史…</div>';
      return H.get("/artifacts/" + encodeURIComponent(aid) + "/history")
        .then(function (d) {
          // 第三路响应：reviews[]（评审记录）+ supersedes/superseded_by/approved_head
          var recs = (d && (d.reviews || d.records || d.history)) || [];
          state.historyReviews[String(aid)] = recs; // 返工预填 review_id 用
          target.innerHTML = artifactHistoryHtml(recs);
        })
        .catch(function (e) {
          target.innerHTML = stateBox("error", explainError(e, "评审历史"), "history-" + aid);
        });
    }

    // 取某产物最近一次 request_changes 评审（返工依据）：优先用已缓存历史。
    function latestChangesReviewId(aid) {
      var recs = state.historyReviews[String(aid)] || [];
      for (var i = 0; i < recs.length; i++) {
        if (String((recs[i] && recs[i].decision) || "") === "request_changes") {
          return String((recs[i] && (recs[i].review_id || recs[i].id)) || "");
        }
      }
      return "";
    }

    // 产物行（链内）：版本徽标 + 评审状态 + 产出者 + 取代关系 + 预览 + 评审表单 + 历史。
    // 视图模型化五迁：纯 HTML 在 render.js（vm = {reviewBusy, reworkBusy, reviewFlash, approvalBlock}），
    // 本层只把 state 切片传入，保证 TEST_API 签名（a, chain）不变。
    function artifactRowHtml(a, chain) {
      return render.artifactRowHtml(a, chain, {
        reviewBusy: state.reviewBusy,
        reworkBusy: state.reworkBusy,
        reviewFlash: state.reviewFlash,
        approvalBlock: state.csApprovalBlock,
      });
    }


    function renderArtifactsChains(chains) {
      if (!chains || !chains.length) return "";
      return chains
        .map(function (c) {
          var kind = (c.items[0] && c.items[0].kind) || "—";
          var headHtml = c.approvedHead
            ? '当前 approved head：<span class="owo-ws-badge rv-ok">v' + esc(c.approvedHead.version) + "</span>"
            : '<span class="hint">无已批准版本（approved head 未建立）</span>';
          // 版本时间线（v1→v2 状态流转）+ 多版本时提供 v1/v2 差异入口
          var timeline = artifactTimelineHtml(c.items);
          var diffBtn =
            c.items.length >= 2
              ? '<button type="button" class="owo-ws-mini" data-chain-diff="' + esc((c.items[0] && c.items[0].artifact_id) || "") + '">查看首末版本差异</button>'
              : "";
          return (
            '<div class="owo-ws-art-chain">' +
            '<div class="owo-ws-art-chainhead"><b>' + esc(kind) + "</b> 版本链（" + c.items.length + " 个版本）· " + headHtml + " " + diffBtn + "</div>" +
            timeline +
            '<div class="owo-ws-chain-diffbox" data-chain-diffbox="' + esc((c.items[0] && c.items[0].artifact_id) || "") + '" hidden></div>' +
            c.items
              .map(function (a) {
                return artifactRowHtml(a, c);
              })
              .join("") +
            "</div>"
          );
        })
        .join("");
    }

    // ---------- 成员 / 任务 ----------
    function membersByMId() {
      var m = {};
      ((state.team && state.team.members) || []).forEach(function (x) {
        if (x.member_id) m[x.member_id] = x;
      });
      return m;
    }

    function findTask(id) {
      for (var i = 0; i < state.tasks.length; i++) {
        if (state.tasks[i].task_id === id) return state.tasks[i];
      }
      return null;
    }

    function paintMembers() {
      var box = el("#ws-d-members");
      if (!box) return;
      var members = (state.team && state.team.members) || [];
      if (!members.length) {
        box.innerHTML = '<div class="hint">暂无成员记录（团队可能尚未开始运行）</div>';
        return;
      }
      box.innerHTML = members
        .map(function (m) {
          var b = m.runtime_binding || {};
          var kind = b.kind || "";
          var who =
            kind === "agent"
              ? "Agent " + (b.agent_id || "")
              : kind === "human"
                ? "人工 " + (b.user_id || "")
                : "Worker " + (b.worker_name || "");
          var caps = (m.capabilities || []).join(", ");
          var extra =
            m.handoff_contract != null ||
            m.budget != null ||
            (m.read_scope || []).length ||
            (m.write_scope || []).length;
          return (
            '<div class="owo-ws-member">' +
            '<div class="owo-ws-member-head"><b>' + esc(m.role) + "</b>" +
            '<span class="chip c-' + esc(kind) + '">' + esc(kind) + "</span>" +
            '<span class="chip h-' + esc(m.health) + '">' + (HEALTH_CN[m.health] || esc(m.health)) + "</span>" +
            "</div>" +
            '<div class="hint owo-ws-ellip" title="' + esc(m.member_id + " · " + who) + '">' +
            '<span class="owo-ws-mono">' + esc(m.member_id) + "</span> · " + esc(who) + "</div>" +
            (m.handoff_contract
              ? '<div class="hint owo-ws-ellip" title="交接契约：' + esc(m.handoff_contract) + '">交接契约：' + esc(m.handoff_contract) + "</div>"
              : "") +
            (caps ? '<div class="hint owo-ws-ellip" title="能力：' + esc(caps) + '">能力：' + esc(caps) + "</div>" : "") +
            (extra
              ? "<details class=\"hint\"><summary>范围 / 预算</summary><pre class=\"hint\">" +
                esc(
                  JSON.stringify({
                    read_scope: m.read_scope || [],
                    write_scope: m.write_scope || [],
                    budget: m.budget != null ? m.budget : null,
                  })
                ) +
                "</pre></details>"
              : "") +
            "</div>"
          );
        })
        .join("");
    }

    // 任务 DAG：按依赖层级分列，SVG + foreignObject 渲染节点。
    // teamStatus 用于节点级「重试此节点」按钮的可见性（succeeded/cancelled 终态不出按钮）。
    // DAG SVG 构建（迁入 render.js）（模块化拆分后不再内联，见 panels/workswarm/render.js）
    function paintDag() {
      var box = el("ws-d-dag");
      if (!box) return;
      var tasks = state.tasks || [];
      if (!tasks.length) {
        box.innerHTML = '<div class="hint">暂无任务记录（团队可能尚未开始运行，或任务图尚未生成）</div>';
        return;
      }
      var byId = {};
      tasks.forEach(function (t) {
        byId[t.task_id] = t;
      });
      var doneN = 0;
      var failN = 0;
      var blockedN = 0;
      var runningN = 0;
      tasks.forEach(function (t) {
        var st = normStatus(t.status);
        if (st === "succeeded") doneN++;
        else if (st === "failed" || st === "aborted") failN++;
        else if (st === "running") runningN++;
        if (taskBlocked(t, byId)) blockedN++;
      });
      var sum =
        '<div class="owo-ws-dagsum">共 ' + tasks.length +
        " 项 · 完成 " + doneN +
        (runningN ? ' · <b class="run">运行中 ' + runningN + "</b>" : "") +
        (failN ? ' · <b class="blocked">失败/中止 ' + failN + "</b>" : "") +
        (blockedN ? ' · <b class="blocked">阻塞 ' + blockedN + "</b>" : "") +
        "</div>";
      try {
        box.innerHTML =
          '<div class="hint" style="margin:0 0 6px">图例：虚线框=未开始 · 实线框=进行中/已终结 · 红色边框=已阻塞（上游失败/中止，不会再运行）· 红色边=上游已死 · 琥珀按钮=可重试此节点（仅重置自身与未完成下游）</div>' +
          sum +
          '<div class="owo-ws-dagwrap">' + dagSvg(tasks, currentTeamStatus()) + "</div>";
      } catch (e) {
        var rows = tasks
          .map(function (t) {
            return (
              "<tr>" +
              "<td>" + esc(t.task_id) + "</td><td>" + esc(t.role || "") + "</td><td>" + esc(t.worker || "") + "</td>" +
              '<td class="hint">' + esc((t.depends_on || []).join(", ")) + "</td><td>" + esc(t.status) + "</td>" +
              "<td>" + esc(t.attempts) + "</td><td class=\"hint\">" + esc(t.error || "") + "</td></tr>"
            );
          })
          .join("");
        box.innerHTML =
          '<div class="hint">DAG 渲染失败（' + esc(e && e.message) + "），回退为列表：</div>" +
          '<div class="owo-ws-tablewrap"><table class="owo-ws-table"><tr><th>task_id</th><th>角色</th><th>worker</th><th>依赖</th><th>状态</th><th>尝试</th><th>错误</th></tr>' +
          rows +
          "</table></div>";
      }
    }


    // 中断态显示：服务端 interrupted 标记（磁盘 Running 但无活动运行）。
    // 需求红线——中断的团队不得继续显示为正常执行中。
    function interruptedView() {
      return !!state.interrupted && !isTerminalTeam(currentTeamStatus());
    }

    function paintInterrupted() {
      var box = el("ws-d-interrupted");
      if (box) box.style.display = interruptedView() ? "" : "none";
    }

    // 运行摘要（详情页顶部）：状态/阶段、成功/失败/等待/运行/阻塞、失败步骤、
    // 累计尝试、产物数；失败步骤行内联「重试此节点」。
    function paintRunSummary() {
      var box = el("ws-d-summary");
      if (!box) return;
      if (!state.team) {
        box.innerHTML = stateBox("loading", "正在汇总运行状态…");
        return;
      }
      var s = computeRunSummary({
        team: state.team,
        tasks: state.tasks,
        artifactCount: state.artifactCount,
        interrupted: state.interrupted,
        active: state.active,
      });
      var badge =
        '<span class="owo-ws-badge st-' + esc(s.statusKey) + '">' + esc(s.statusLabel) + "</span>" +
        '<span class="chip">' + esc(s.phase) + "</span>";
      var stats = [
        { label: "完成步骤", v: s.counts.succeeded, k: s.counts.succeeded ? "ok" : "" },
        { label: "失败/中止", v: s.counts.failed, k: s.counts.failed ? "bad" : "" },
        { label: "等待", v: s.counts.waiting, k: "" },
        { label: "运行中", v: s.counts.running, k: s.counts.running ? "run" : "" },
        { label: "阻塞", v: s.counts.blocked, k: s.counts.blocked ? "bad" : "" },
        { label: "执行记录", v: s.artifactCount == null ? "—" : s.artifactCount, k: "" },
      ];
      var cells = stats
        .filter(function(x){return x.v!==0 || x.label==="完成步骤" || x.label==="执行记录";})
        .map(function (x) {
          return (
            '<div class="owo-ws-stat"><small>' + esc(x.label) + '</small><b class="' + x.k + '">' + esc(x.v) + "</b></div>"
          );
        })
        .join("");
      var line = "";
      if (s.failedStep) {
        var head =
          "当前失败步骤：<b>" + esc(s.failedStep.role) + "</b>（" + esc(s.failedStep.task_id) +
          " · 已尝试 " + s.failedStep.attempts + " 次）";
        var err = s.failedStep.error
          ? '<span class="owo-ws-node-err owo-ws-ellip" title="' + esc(s.failedStep.error) + '">' +
            esc(short(s.failedStep.error, 80)) + "</span>"
          : "";
        if (s.canRetry) {
          line =
            '<div class="owo-ws-sumline">' + head + err +
            '<button type="button" class="owo-ws-mini owo-ws-retry" data-ws-retry="' + esc(s.failedStep.task_id) +
            '" aria-label="重试此节点 ' + esc(s.failedStep.role) + '">↻ 重试此节点</button></div>';
        } else {
          line = '<div class="owo-ws-sumline">' + head + err + '<span class="hint">团队已终态，重试不可用</span></div>';
        }
      } else {
        line = '<div class="owo-ws-sumline hint">当前无失败步骤' + (s.counts.total ? "" : "（任务图尚未生成）") + "</div>";
      }
      box.innerHTML = '<div class="owo-ws-sumline owo-ws-sum-head">' + badge + "</div>" +
        '<div class="owo-ws-statgrid">' + cells + "</div>" + line;
    }

    function paintStatusLive() {
      var t = state.team || {};
      var raw = state.teamStatus || (t.status != null ? t.status : "");
      var st = normStatus(raw);
      var iv = interruptedView();
      var badge = el("#ws-d-status");
      if (badge) {
        if (iv) {
          badge.className = "owo-ws-badge st-interrupted";
          badge.textContent = "已中断（可恢复）";
          badge.title = "进程重启遗留的运行态：进度已保留，可用 continue / retry 显式恢复";
        } else {
          badge.className = "owo-ws-badge st-" + (st || "unknown");
          badge.textContent = TEAM_STATUS_CN[st] || raw || "未知";
          badge.title = "";
        }
      }
      var act = el("#ws-d-active");
      if (act) {
        act.textContent = iv
          ? "◦ 意外中断，可恢复"
          : state.active
            ? "● 运行中"
            : st === "succeeded"
              ? "✓ 已完成"
              : st === "created"
                ? "○ 等待启动"
                : st === "awaiting_human"
                  ? "◷ 等待你处理"
                  : "○ 已暂停";
        act.className = "owo-ws-active" + (iv ? " interrupted" : state.active ? " on" : "");
      }
      var meta = el("#ws-d-meta");
      if (meta && t && t.team_id) {
        var metaText =
          "模式 " + (MODE_CN[normStatus(t.mode)] || t.mode || "—") +
          " · 项目空间 " + (t.project_space_id || "—") +
          " · 模板 " + (t.template_id || "—") +
          " · 创建 " + (t.created_at || "—") +
          " · 更新 " + (t.updated_at || "—");
        meta.textContent = metaText;
        meta.title = metaText; // 长路径/长 ID：完整 tooltip
      }
      // 状态变化（含 SSE/轮询推送）实时重算终态门控。
      applyGating();
    }

    function paintDetailLive() {
      var status=normStatus(currentTeamStatus());
      var continueButton=el('#ws-act-continue'),cancelButton=el('#ws-act-cancel');
      if(continueButton)continueButton.style.display=interruptedView()||status==='paused'?'':'none';
      if(cancelButton)cancelButton.style.display=isTerminalTeam(status)?'none':'';
      var manual=el('[data-team-section="调整任务与人工协作"]');
      if(manual && status==='awaiting_human')manual.open=true;
      if(deliveryView && state.current)deliveryView.load(state.current, currentTeamStatus()+":"+(state.tasks||[]).map(function(t){return t.status+":"+t.attempts;}).join(","));
      paintStatusLive();
      paintRunSummary();
      paintInterrupted();
      paintMembers();
      paintDag();
      paintAudit();
      paintProgress();
      paintHumanSelect();
      paintHandoffSelect();
      paintFromSelect();
      paintSevenRuntime(); // 七期：Worker 能力/写租约/文件变更
    }

    // ---------- 人节点 / 交接 下拉 ----------
    function humanTasks() {
      var mBy = membersByMId();
      var out = [];
      (state.tasks || []).forEach(function (t) {
        var m = mBy[t.worker];
        if (m && m.runtime_binding && m.runtime_binding.kind === "human" && !STEP_TERMINAL[normStatus(t.status)]) {
          out.push(t);
        }
      });
      return out;
    }

    function taskStatusLabel(t) {
      var status = t && t.status;
      return esc(STEP_STATUS_CN[normStatus(status)] || status || "");
    }

    function taskOptionHtml(t) {
      return (
        '<option value="' + esc(t && t.task_id) + '">' +
        esc((t && t.role) || (t && t.task_id)) +
        "（" +
        taskStatusLabel(t) +
        "）</option>"
      );
    }

    function paintHumanSelect() {
      var sel = el("#ws-h-task");
      if (!sel) return;
      var prev = state.humanTask || "";
      var list = humanTasks();
      sel.innerHTML = list.length
        ? list.map(taskOptionHtml).join("")
        : '<option value="">暂无未完成的人节点任务</option>';
      if (list.some(function (t) {
        return t.task_id === prev;
      }))
        sel.value = prev;
      state.humanTask = sel.value;
    }

    function paintHandoffSelect() {
      var sel = el("#ws-x-task");
      if (!sel) return;
      var prev = sel.value;
      var tasks = state.tasks || [];
      sel.innerHTML = tasks.length
        ? tasks.map(taskOptionHtml).join("")
        : '<option value="">暂无任务</option>';
      if (tasks.some(function (t) {
        return t.task_id === prev;
      }))
        sel.value = prev;
    }

    function paintFromSelect() {
      var xTask = el("#ws-x-task");
      var xFrom = el("#ws-x-from");
      if (!xTask || !xFrom) return;
      var t = findTask(xTask.value);
      xFrom.innerHTML = t
        ? '<option value="' + esc(t.worker) + '">' + esc(t.worker) + "（任务执行者）</option>"
        : '<option value="">—</option>';
      paintToSelect();
      paintHandoffRefPickers();
    }

    /// §8.1：to_member 可搜索选择器（成员下拉 + 自动缺省；替代自由输入 m-<角色>）。
    function paintToSelect() {
      var xTo = el("#ws-x-to");
      if (!xTo) return;
      var prev = xTo.value;
      var ms = (state.team && state.team.members) || [];
      xTo.innerHTML =
        '<option value="">自动（交由团队/下游决定）</option>' +
        ms.map(function (m) {
          var label = (m.role ? m.role + " · " : "") + (m.member_id || "");
          return '<option value="' + esc(m.member_id || "") + '">' + esc(label) + "</option>";
        }).join("");
      if (ms.some(function (m) { return m.member_id === prev; })) xTo.value = prev;
      else xTo.value = "";
    }

    /// §8.1：关联产物/证据引用选择器（已选结构化行 + 搜索候选；替代手输 CSV）。
    function artifactRefOptions() {
      return (state.artifacts || []).map(function (a) {
        var name = artifactFileName(a) || a.artifact_id || "";
        var label = a && a.version != null ? name + " · v" + a.version : name;
        return { value: String(a.artifact_id || ""), label: label };
      }).filter(function (o) { return o.value; });
    }

    function handoffArtifactsStatusHtml() {
      if (state.handoffArtsLoading) {
        return '<div class="hint owo-ws-loading" role="status">正在读取关联产物候选…</div>';
      }
      if (state.handoffArtsError) {
        return '<div class="hint err" role="alert">关联产物候选加载失败：' + esc(state.handoffArtsError) +
          '。仍可手动输入引用。 <button type="button" class="owo-ws-mini" data-ws-ref-retry-artifacts>重试</button></div>';
      }
      return "";
    }

    function loadHandoffArtifacts(force) {
      if (state.handoffArtsLoading) return state.handoffArtsRequest || Promise.resolve();
      if (state.handoffArtsFetched && !state.handoffArtsError && !force) return Promise.resolve();
      var teamId = state.current;
      state.handoffArtsLoading = true;
      state.handoffArtsError = "";
      state.artifactLoadError = "";
      paintHandoffRefPickers();
      var request = loadArtifacts().then(function () {
        if (state.current !== teamId) return;
        state.handoffArtsLoading = false;
        state.handoffArtsError = state.artifactLoadError || "";
        state.handoffArtsFetched = !state.handoffArtsError;
        state.handoffArtsRequest = null;
        paintHandoffRefPickers();
      });
      state.handoffArtsRequest = request;
      return request;
    }

    function paintHandoffRefPickers() {
      // 首次加载和失败后的显式重试共用同一请求锁；失败时保留自由输入能力。
      if (!state.handoffArtsFetched && !state.handoffArtsLoading && !state.handoffArtsError) {
        loadHandoffArtifacts();
      }
      var artsBox = el("#ws-x-arts-pick");
      var evidBox = el("#ws-x-evid-pick");
      var statusHtml = handoffArtifactsStatusHtml();
      if (artsBox) {
        artsBox.innerHTML = statusHtml + render.refPickerHtml({
          kind: "arts",
          rows: state.handoffRefs.arts,
          options: domain.filterRefOptions(artifactRefOptions(), state.handoffRefQuery.arts, 8),
          query: state.handoffRefQuery.arts,
          allowFree: true,
        });
      }
      if (evidBox) {
        evidBox.innerHTML = render.refPickerHtml({
          kind: "evid",
          rows: state.handoffRefs.evid,
          options: domain.filterRefOptions(artifactRefOptions(), state.handoffRefQuery.evid, 8),
          query: state.handoffRefQuery.evid,
          allowFree: true,
        });
      }
    }

    /// 选择器容器事件委托（添加/移除/搜索/自由添加）；renderDetail 挂载后调用一次。
    function bindHandoffRefPickers() {
      ["#ws-x-arts-pick", "#ws-x-evid-pick"].forEach(function (selId) {
        var box = el(selId);
        if (!box) return;
        box.addEventListener("click", function (ev) {
          var t = ev.target;
          if (t && t.hasAttribute && t.hasAttribute("data-ws-ref-retry-artifacts")) {
            loadHandoffArtifacts(true);
            return;
          }
          var add = t && t.getAttribute && t.getAttribute("data-ws-ref-add");
          if (add) {
            var ci = add.indexOf(":");
            var kind = add.slice(0, ci);
            var value = add.slice(ci + 1);
            state.handoffRefs[kind] = domain.addRefRow(state.handoffRefs[kind], value);
            paintHandoffRefPickers();
            return;
          }
          var del = t && t.getAttribute && t.getAttribute("data-ws-ref-del");
          if (del) {
            var di = del.indexOf(":");
            var dkind = del.slice(0, di);
            var dvalue = del.slice(di + 1);
            state.handoffRefs[dkind] = domain.removeRefRow(state.handoffRefs[dkind], dvalue);
            paintHandoffRefPickers();
            return;
          }
          var free = t && t.getAttribute && t.getAttribute("data-ws-ref-free");
          if (free) {
            var q = box.querySelector("[data-ws-ref-q]");
            var v = q ? q.value : "";
            state.handoffRefs[free] = domain.addRefRow(state.handoffRefs[free], v);
            state.handoffRefQuery[free] = "";
            paintHandoffRefPickers();
          }
        });
        box.addEventListener("input", function (ev) {
          var t = ev.target;
          var kind = t && t.getAttribute && t.getAttribute("data-ws-ref-q");
          if (!kind) return;
          state.handoffRefQuery[kind] = t.value || "";
          paintHandoffRefPickers();
          var again = box.querySelector("[data-ws-ref-q]");
          if (again) again.focus();
        });
      });
    }

    function populateSteerSelect() {
      var sel = el("#ws-act-steer-step");
      if (!sel) return;
      var tasks = state.tasks || [];
      sel.innerHTML =
        '<option value="">全部未完成任务（默认）</option>' +
        tasks
          .map(function (t) {
            return (
              '<option value="' + esc(t.task_id) + '">' +
              esc(t.role || t.task_id) +
              " · " +
              taskStatusLabel(t) +
              "</option>"
            );
          })
          .join("");
    }

    function populateReplaceRole() {
      var sel = el("#ws-act-replace-role");
      if (!sel) return;
      var ms = (state.team && state.team.members) || [];
      sel.innerHTML = ms.length
        ? ms
            .map(function (m) {
              return '<option value="' + esc(m.role) + '">' + esc(m.role) + "（" + esc(m.member_id) + "）</option>";
            })
            .join("")
        : '<option value="">（暂无成员）</option>';
    }

    function populateSelects() {
      populateSteerSelect();
      populateReplaceRole();
      paintHumanSelect();
      paintHandoffSelect();
      paintFromSelect();
    }

    // ---------- 产物 ----------
    // ---------- 五期：角色指标与交付物加载 ----------
    function projectIdOfTeam() {
      var t = state.team;
      return (t && t.project_space_id) || (state.current ? "proj-" + state.current : "");
    }

    function loadMetrics() {
      if (!state.current) return Promise.resolve();
      return H.get("/teams/" + encodeURIComponent(state.current) + "/metrics")
        .then(function (d) {
          state.metrics = metricsFromPayload(d);
          paintMetrics();
        })
        .catch(function () {
          // 指标接口未上线/404 时保持空态，不干扰详情其它区域
          state.metrics = metricsFromPayload(null);
          paintMetrics();
        });
    }

    function paintMetrics() {
      var box = el("#ws-d-metrics");
      if (!box) return;
      box.innerHTML = metricsCardsHtml(state.metrics, state.strategyDecision && state.strategyDecision.budgetPerRole);
    }

    function paintStrategy() {
      var box = el("#ws-d-strategy");
      if (!box) return;
      box.innerHTML = strategyBoxHtml(state.strategyDecision);
    }

    function loadDeliverables() {
      var pid = projectIdOfTeam();
      var box = el("#ws-dlv-box");
      if (!box) return Promise.resolve();
      if (!pid) {
        box.innerHTML = '<div class="hint">项目空间尚不可用（交付物按项目聚合）。</div>';
        return Promise.resolve();
      }
      box.innerHTML = stateBox("loading", "正在加载最终交付物…");
      return H.get("/projects/" + encodeURIComponent(pid) + "/deliverables")
        .then(function (d) {
          state.deliverables = deliverablesFromPayload(d);
          if (state.deliverablesOpen && box.hidden === false) box.innerHTML = deliverablesBoxHtml(state.deliverables);
        })
        .catch(function (e) {
          if (box.hidden === false) box.innerHTML = stateBox("error", explainError(e, "交付物加载"), "deliverables");
        });
    }

    // ---------- 产物 ----------
    function loadArtifacts() {
      var box = el("#ws-d-artifacts");
      if (!box) return Promise.resolve();
      var t = state.team;
      var pid = (t && t.project_space_id) || (state.current ? "proj-" + state.current : "");
      if (!pid) {
        box.innerHTML = '<div class="hint">项目空间尚不可用</div>';
        return Promise.resolve();
      }
      state.artifactLoadError = "";
      box.innerHTML = '<div class="hint">project_id：' + esc(pid) + "</div>" + stateBox("loading", "正在加载产物…");
      return H.get("/projects/" + encodeURIComponent(pid) + "/artifacts")
        .then(function (d) {
          if (el("#ws-d-artifacts") !== box) return; // 视图已切走
          var arts = (d && d.artifacts) || [];
          state.artifactLoadError = "";
          state.artifacts = arts;
          state.artifactCount = arts.length; // 运行摘要「产物」计数
          paintRunSummary();
          if (!arts.length) {
            box.innerHTML = '<div class="hint">project_id：' + esc(pid) + '</div><div class="hint">暂无产物（任务产出后会出现在这里）</div>';
            if (state.handoffArtsFetched) paintHandoffRefPickers();
            return;
          }
          var chains = groupArtifactChain(arts);
          box.innerHTML =
            '<div class="hint">project_id：' + esc(pid) + " · 共 " + arts.length + " 个产物 · " + chains.length + " 条版本链</div>" +
            renderArtifactsChains(chains);
          bindArtifactReviewHandlers(box);
          if (state.handoffArtsFetched) paintHandoffRefPickers();
        })
        .catch(function (e) {
          if (el("#ws-d-artifacts") !== box) return;
          state.artifactLoadError = explainError(e, "产物加载");
          if (state.handoffArtsFetched) {
            state.handoffArtsFetched = false;
            state.handoffArtsError = state.artifactLoadError;
            paintHandoffRefPickers();
          }
          box.innerHTML =
            '<div class="hint">project_id：' + esc(pid) + "</div>" +
            stateBox("error", state.artifactLoadError, "artifacts");
        });
    }

    // 产物区事件委托：评审动作 / 评审历史懒加载。box 每次 loadArtifacts 只换
    // innerHTML，委托绑定标记在 box 元素自身上，元素随 renderDetail 重建时重绑。
    function bindArtifactReviewHandlers(box) {
      if (!box || box.getAttribute("data-review-bound") === "1") return;
      box.setAttribute("data-review-bound", "1");
      box.addEventListener("click", function (ev) {
        var target = ev.target;
        if (!target || !target.closest) return;
        // 七期：产物内容下载（GET /artifacts/{id}/content，404 容错为行级提示）。
        var dlBtn = target.closest("[data-art-dl]");
        if (dlBtn) {
          downloadArtifact(dlBtn.getAttribute("data-art-dl") || "");
          return;
        }
        var actBtn = target.closest("[data-art-act]");
        if (actBtn) {
          var aid = actBtn.getAttribute("data-art-id") || "";
          var act = actBtn.getAttribute("data-art-act") || "";
          var form = actBtn.closest(".owo-ws-review-form");
          var reviewerEl = form ? form.querySelector(".owo-ws-review-reviewer") : null;
          var commentEl = form ? form.querySelector(".owo-ws-review-comment") : null;
          var resultEl = form ? form.querySelector("[data-art-result]") : null;
          // 锁定该产物全部评审按钮（提交期间防重复）
          var allBtns = box.querySelectorAll('[data-art-act][data-art-id="' + aid.replace(/"/g, '\\"') + '"]');
          for (var i = 0; i < allBtns.length; i++) allBtns[i].disabled = true;
          if (actBtn.setAttribute) actBtn.setAttribute("data-busy", "1");
          submitArtifactReview({
            artifactId: aid,
            decision: act,
            reviewer: reviewerEl ? reviewerEl.value : "",
            comment: commentEl ? commentEl.value : "",
          }).then(
            function () {
              if (resultEl) resultEl.textContent = "评审已提交：" + (DECISION_CN[act] || act) + "（记录不可变）";
              loadArtifacts(); // 重取产物/链头状态
            },
            function (e) {
              for (var j = 0; j < allBtns.length; j++) allBtns[j].disabled = false;
              if (actBtn.removeAttribute) actBtn.removeAttribute("data-busy");
              if (resultEl) {
                resultEl.textContent = explainReviewError(e);
                resultEl.className = "owo-ws-review-result sub bad";
              }
            }
          );
          return;
        }
        var histBtn = target.closest("[data-art-history]");
        if (histBtn) {
          loadArtifactHistory(histBtn.getAttribute("data-art-history"));
          return;
        }
        // 五期：链内首末版本差异（懒计算预览行差异）
        var diffBtn = target.closest("[data-chain-diff]");
        if (diffBtn) {
          var headId = diffBtn.getAttribute("data-chain-diff") || "";
          var chainEl = diffBtn.closest(".owo-ws-art-chain");
          var diffBox = chainEl ? chainEl.querySelector("[data-chain-diffbox]") : null;
          if (diffBox) {
            if (!diffBox.hidden) {
              diffBox.hidden = true;
              diffBox.innerHTML = "";
              return;
            }
            // 找到与 headId 同链的产物序列（按 supersedes 关系重建链序）
            var chainsNow = groupArtifactChain(state.artifacts || []);
            var chain = null;
            for (var ci = 0; ci < chainsNow.length; ci++) {
              var first = chainsNow[ci].items[0];
              if (first && String(first.artifact_id) === String(headId)) { chain = chainsNow[ci]; break; }
            }
            if (!chain || chain.items.length < 2) {
              diffBox.hidden = false;
              diffBox.innerHTML = '<div class="hint">该链不足两个版本，暂无可对比内容。</div>';
              return;
            }
            var vA = chain.items[0];
            var vB = chain.items[chain.items.length - 1];
            diffBox.hidden = false;
            diffBox.innerHTML =
              vA.preview == null || vB.preview == null
                ? '<div class="hint">缺少预览内容（产物未携带 preview），无法生成差异。</div>'
                : diffHtml(vA.preview, vB.preview, "v" + vA.version, "v" + vB.version);
          }
          return;
        }
        // 五期：返工表单展开（懒加载评审历史并预填）与提交
        var reworkDetails = target.closest(".owo-ws-rework");
        if (reworkDetails && target.tagName === "SUMMARY") {
          var form = reworkDetails.querySelector("[data-rework-form]");
          if (form && !form.getAttribute("data-prefilled")) {
            form.setAttribute("data-prefilled", "1");
            var raid = form.getAttribute("data-rework-form") || "";
            var ensure = latestChangesReviewId(raid)
              ? Promise.resolve()
              : loadArtifactHistory(raid);
            ensure.then(function () {
              var rid = latestChangesReviewId(raid);
              if (rid) form.setAttribute("data-review-id", rid);
              var ta = form.querySelector(".owo-ws-rework-instruction");
              if (ta && !ta.value.trim()) {
                var recs = state.historyReviews[String(raid)] || [];
                for (var k = recs.length - 1; k >= 0; k--) {
                  if (String((recs[k] && recs[k].decision) || "") === "request_changes") {
                    ta.value = String((recs[k] && recs[k].comment) || "");
                    break;
                  }
                }
              }
            });
          }
          return;
        }
        var reworkBtn = target.closest("[data-rework-go]");
        if (reworkBtn) {
          var rAid = reworkBtn.getAttribute("data-rework-go") || "";
          var rForm = reworkBtn.closest("[data-rework-form]");
          var rTa = rForm ? rForm.querySelector(".owo-ws-rework-instruction") : null;
          var rid2 = rForm ? rForm.getAttribute("data-review-id") || latestChangesReviewId(rAid) : "";
          reworkBtn.disabled = true;
          submitRework({
            artifactId: rAid,
            reviewId: rid2,
            instruction: rTa ? rTa.value : "",
          }).then(
            function () {
              loadArtifacts();
            },
            function (e) {
              reworkBtn.disabled = false;
              if (resultEl || true) {
                var box2 = el("#ws-d-artifacts");
                if (box2) {
                  var rowEl = box2.querySelector('[data-art-result="' + rAid.replace(/"/g, '\\"') + '"]');
                  if (rowEl) {
                    rowEl.textContent = explainReworkError(e);
                    rowEl.className = "owo-ws-review-result sub bad";
                  }
                }
              }
            }
          );
          return;
        }
      });
    }

    // ---------- 详情数据 ----------
    var detailSeq = 0; // 每次进入/重载详情递增；用于丢弃陈旧响应

    function loadDetail(teamId) {
      state.current = teamId;
      // §8.1：切换团队时重置交接引用选择器（结构化行不跨团队携带）。
      state.handoffRefs = { arts: [], evid: [] };
      state.handoffRefQuery = { arts: "", evid: "" };
      state.handoffArtsFetched = false;
      state.handoffArtsLoading = false;
      state.handoffArtsError = "";
      state.handoffArtsRequest = null;
      state.artifactLoadError = "";
      state.artifacts = [];
      var seq = ++detailSeq;
      var idEl = el("#ws-d-id");
      if (idEl) idEl.textContent = teamId;
      var errBox = el("#ws-d-error");
      if (errBox) errBox.innerHTML = "";
      H.get("/teams/" + encodeURIComponent(teamId))
        .then(function (d) {
          if (seq !== detailSeq || state.current !== teamId || state.view !== "detail") return; // 已切走
          state.team = d.team || null;
          state.tasks = d.tasks || [];
          state.interrupted = !!d.interrupted;
          state.handoffs = null;
          state.conversationError = "";
          state.conversationRequest = null;
          state.conversationSeq += 1;
          // 七期：Worker 能力/写租约/文件变更——顶层或 team 对象内双路径容错读取。
          state.workerProfiles = d.worker_profiles || (d.team && d.team.worker_profiles) || [];
          state.writeLease = d.write_lease !== undefined ? d.write_lease : (d.team && d.team.write_lease) || null;
          state.changes = d.changes || (d.team && d.team.changes) || [];
          state.changesRemote = null; // 二路 changes 端点随详情重拉
          state.audit = [];
          state.auditKeys = {};
          state.progress = null; // 换团队：进度状态清零（seq 守卫随之重置）
          state.lastProgressSeq = 0;
          state.cancelling = false;
          state.artifacts = [];
          // Reviews may still be in flight while this team's detail refreshes.
          state.reviewResult = null;
          state.reviewFlash = null;
          state.strategyDecision = strategyDecisionOf(d.team || d); // 五期：auto 判定理由
          state.metrics = null;
          state.deliverables = null;
          state.deliverablesOpen = false;
          state.reworkResult = null;
          state.historyReviews = {};
          state.diagnostic = null;
          // 六期：工作区绑定（team.workspace 优先，补 GET /projects/{pid}/workspace）
          state.workspace = workspaceFromTeam(d.team || null);
          state.workspaceView = null;
          state.workspaceViewKind = "";
          state.workspaceBusy = false;
          stopProgressTimer();
          var tail = (d.audit_tail || []).slice();
          tail.sort(function (a, b) {
            return String(a.ts).localeCompare(String(b.ts));
          });
          tail.forEach(addAudit);
          populateSelects();
          paintDetailLive();
          paintStrategy();
          paintWorkspace();
          loadWorkspace();
          loadWorkspaceChanges(); // 七期（二路）：变更追踪端点（容错 404 → 详情 changes[] 兜底）
          loadChangeSets(); // 八期（二路交接）：ChangeSet 审批列表（容错 404 → 空态）
          loadTemplateInfo();
          loadMetrics();
          loadArtifacts();
          connectEvents(teamId);
        })
        .catch(function (e) {
          if (seq !== detailSeq || state.current !== teamId || state.view !== "detail") return;
          var eb = el("#ws-d-error");
          if (eb) eb.innerHTML = stateBox("error", explainError(e, "团队详情加载"), "detail");
        });
    }

    function syncDetail() {
      if (!state.current) return Promise.resolve();
      var seq = detailSeq;
      // 五期：角色指标随轮询轻量刷新（接口未上线时静默保持空态）
      if (state.active || !isTerminalTeam(state.teamStatus)) loadMetrics();
      return H.get("/teams/" + encodeURIComponent(state.current))
        .then(function (d) {
          if (seq !== detailSeq) return;
          if (d.team && d.team.team_id !== state.current) return; // 已切到其他团队
          state.team = d.team || state.team;
          state.tasks = d.tasks || [];
          state.interrupted = !!d.interrupted;
          // 七期：轮询同步同样刷新 Worker 能力/写租约/文件变更（容错同上）。
          state.workerProfiles = d.worker_profiles || (state.team && state.team.worker_profiles) || [];
          state.writeLease = d.write_lease !== undefined ? d.write_lease : (state.team && state.team.write_lease) || null;
          state.changes = d.changes || (state.team && state.team.changes) || [];
          (d.audit_tail || []).forEach(addAudit);
          populateSelects();
          paintDetailLive();
        })
        .catch(function () {
          /* 轮询会兜底 */
        });
    }

    function refreshTasksOnly() {
      if (!state.current) return Promise.resolve();
      var seq = detailSeq;
      return H.get("/teams/" + encodeURIComponent(state.current) + "/tasks")
        .then(function (d) {
          if (seq !== detailSeq) return;
          state.tasks = d.tasks || [];
          paintDag();
          paintRunSummary();
          populateSteerSelect();
          paintHumanSelect();
          paintHandoffSelect();
          paintFromSelect();
        })
        .catch(function (e) {
          if (seq !== detailSeq) return;
          var box = el("#ws-d-dag");
          if (box) box.innerHTML = stateBox("error", explainError(e, "任务刷新"), "tasks");
        });
    }

    // ---------- 实时通道：SSE + 轮询兜底 ----------
    // 通道指示：SSE=绿、轮询降级=琥珀、停止=灰；文字本身承载详细原因。
    function paintStreamNote() {
      var box = el("#ws-d-stream");
      if (!box) return;
      box.textContent = state.streamNote || "";
      var cls =
        state.liveMode === "sse" ? "owo-ws-live on" : state.liveMode === "poll" ? "owo-ws-live poll" : "owo-ws-live off";
      box.className = cls;
      box.title = state.streamNote || "";
    }

    function abortPollRequest() {
      var request = state.pollRequest;
      state.pollRequest = null;
      if (request && request.controller) {
        try {
          request.controller.abort();
        } catch (e) {
          /* ignore */
        }
      }
    }

    function stopLive(note) {
      abortPollRequest();
      if (state.es) {
        try {
          if (typeof state.es.abort === "function") state.es.abort(); // fetch 流式 SSE 的 AbortController
          else state.es.close();
        } catch (e) {
          /* ignore */
        }
        state.es = null;
      }
      if (state.pollTimer) {
        clearInterval(state.pollTimer);
        state.pollTimer = null;
      }
      if (state.refreshDebounce) {
        clearTimeout(state.refreshDebounce);
        state.refreshDebounce = null;
      }
      stopProgressTimer();
      state.pollNoteShown = false;
      state.liveMode = "off";
      if (note) {
        state.streamNote = note;
        paintStreamNote();
      }
    }

    function pollOnce() {
      var id = state.current;
      if (!id) return Promise.resolve();
      if (state.pollRequest) return state.pollRequest.promise;
      var seq = detailSeq;
      var controller = new AbortController();
      var request = { controller: controller, promise: null };
      state.pollRequest = request;
      var options = { signal: controller.signal };
      function isCurrent() {
        return state.pollRequest === request && !controller.signal.aborted &&
          state.current === id && seq === detailSeq;
      }
      var events = H.get("/teams/" + encodeURIComponent(id) + "/events?format=json", options)
        .then(function (snap) {
          if (!isCurrent()) return;
          state.active = !!snap.active;
          state.teamStatus = snap.status;
          state.interrupted = !!snap.interrupted;
          (snap.audit || []).forEach(addAudit);
          if (snap.progress && applyProgress(snap.progress)) {
            paintStatusLive();
            paintProgress();
          }
          paintStatusLive();
          paintAudit();
          if (isTerminalTeam(snap.status)) {
            state.cancelling = false; // 终态确认：清除"取消中"
            stopLive("已结束：团队进入终态（" + String(snap.status) + "）");
          }
        })
        .catch(function (e) {
          if (!isCurrent()) return;
          if (!state.pollNoteShown) {
            state.pollNoteShown = true;
            state.streamNote = "轮询模式（服务暂不可达：" + short(String(e.message || e), 60) + "，将继续重试）";
            paintStreamNote();
          }
        });
      var detail = H.get("/teams/" + encodeURIComponent(id), options)
        .then(function (d) {
          if (!isCurrent()) return;
          state.team = d.team || state.team;
          state.tasks = d.tasks || [];
          (d.audit_tail || []).forEach(addAudit);
          paintDetailLive();
        })
        .catch(function () {
          /* 事件快照已给出状态提示 */
        });
      request.promise = Promise.all([events, detail]).finally(function () {
        if (state.pollRequest === request) state.pollRequest = null;
      });
      return request.promise;
    }

    function startPolling(note) {
      if (state.es) {
        try {
          state.es.close();
        } catch (e) {
          /* ignore */
        }
        state.es = null;
      }
      abortPollRequest();
      if (state.pollTimer) clearInterval(state.pollTimer);
      state.liveMode = "poll";
      state.streamNote = note;
      paintStreamNote();
      state.pollTimer = setInterval(pollOnce, 2500);
      pollOnce();
    }

    function scheduleTasks() {
      if (state.refreshDebounce) clearTimeout(state.refreshDebounce);
      state.refreshDebounce = setTimeout(function () {
        state.refreshDebounce = null;
        refreshTasksOnly();
      }, 800);
    }

    // 事件帧统一分发（EventSource 与 fetch 流式 SSE 共用）。progress 帧兼容两种
    // 形状：嵌套 {type:"progress", progress:{...}}（第二路实现）与扁平（计划原形）。
    function handleEventFrame(f) {
      if (!f || !f.type) return;
      if (f.type === "audit") {
        addAudit(f);
        paintAudit();
        scheduleTasks();
      } else if (f.type === "progress") {
        var payload = f.progress && typeof f.progress === "object" ? f.progress : f;
        if (applyProgress(payload)) {
          paintStatusLive();
          paintRunSummary();
          paintProgress();
          scheduleTasks(); // 步骤状态变化 → 防抖刷新 DAG/任务表
        }
      } else if (f.type === "state") {
        state.active = !!f.active;
        state.teamStatus = f.status;
        state.interrupted = !!f.interrupted;
        paintStatusLive();
        paintRunSummary();
        paintInterrupted();
        if (isTerminalTeam(f.status)) {
          state.cancelling = false; // 终态确认：清除"取消中"
          stopLive("已结束：团队进入终态（" + String(f.status) + "）");
        }
      }
    }

    // fetch 流式 SSE：与 EventSource 等价的 text/event-stream 解析，但可携带
    // Authorization 头（受保护路由必需）。逐帧解析 data: 行；多行 data 按规范拼接。
    function connectFetchSse(teamId, note) {
      var ctrl = typeof AbortController !== "undefined" ? new AbortController() : null;
      state.es = ctrl;
      window.OwoApi.stream("/teams/" + encodeURIComponent(teamId) + "/events", {
        headers: { "Accept": "text/event-stream" },
        signal: ctrl ? ctrl.signal : undefined,
      })
        .then(function (resp) {
          if (state.es !== ctrl) return null; // 已被取代
            if (!resp.ok || !resp.body || typeof resp.body.getReader !== "function") {
              throw new Error("SSE HTTP " + resp.status);
            }
            state.liveMode = "sse";
            state.streamNote = note || "SSE 已连接（fetch 流式，实时推送）";
            paintStreamNote();
            var reader = resp.body.getReader();
            var decoder = new TextDecoder();
            var buf = "";
            function pump() {
              return reader.read().then(function (chunk) {
                if (state.es !== ctrl) return; // 已被取代/停止
                if (chunk.done) {
                  startPolling("SSE 流结束，已切换 2.5s 轮询");
                  return;
                }
                buf += decoder.decode(chunk.value, { stream: true });
                var idx;
                while ((idx = buf.indexOf("\n\n")) >= 0) {
                  var frame = buf.slice(0, idx);
                  buf = buf.slice(idx + 2);
                  var dataLines = frame.split("\n").filter(function (l) {
                    return l.indexOf("data:") === 0;
                  });
                  if (!dataLines.length) continue;
                  var payload = dataLines.map(function (l) { return l.slice(5).replace(/^ /, ""); }).join("\n");
                  var f;
                  try {
                    f = JSON.parse(payload);
                  } catch (e) {
                    continue;
                  }
                  handleEventFrame(f);
                }
                return pump();
              });
            }
            return pump();
        })
        .catch(function (e) {
          if (state.es !== ctrl) return; // 已被新连接/停流取代
          startPolling("SSE 不可用（" + short(String((e && e.message) || e), 60) + "），已切换 2.5s 轮询");
        });
    }

    function connectEvents(teamId) {
      stopLive();
      state.pollNoteShown = false;
      // 首选 fetch 流式 SSE（可带 Bearer，progress/audit/state 实时推送）；
      // 不支持 ReadableStream 的环境退回裸 EventSource（大概率 401 → 轮询兜底）。
      if (typeof fetch === "function" && win.ReadableStream !== undefined) {
        state.liveMode = "sse";
        state.streamNote = "正在连接 SSE（fetch 流式）…";
        paintStreamNote();
        connectFetchSse(teamId, "SSE 已连接（fetch 流式，实时推送）");
        return;
      }
      startPolling("当前环境不支持带鉴权的 SSE，已切换 2.5s 轮询");
    }

    // ---------- 视图：创建团队 ----------
    var KNOWN_WORKERS = domain.KNOWN_WORKERS;
    var CUSTOM_WORKER = domain.CUSTOM_WORKER;

    function roleRowHtml(role) {
      role = role || {};
      // §8.1：worker 为下拉（known 集合 + 自定义…）；仅选“自定义…”才显示输入框。
      var plan = domain.roleWorkerPlan(role.worker || "agent");
      var workerLabels = { agent: "Agent（模型执行）", echo: "测试回显", sleep: "测试等待", fail: "测试失败" };
      var workerSel =
        '<select class="owo-ws-role-worker-sel" aria-label="Worker 类型">' +
        KNOWN_WORKERS.map(function (k) {
          return '<option value="' + k + '"' + (plan.select === k ? " selected" : "") + ">" +
            esc(workerLabels[k] || k) + "</option>";
        }).join("") +
        '<option value="' + CUSTOM_WORKER + '"' + (plan.select === CUSTOM_WORKER ? " selected" : "") + ">自定义…</option>" +
        "</select>";
      return (
        '<div class="owo-ws-role-row">' +
        '<input class="owo-ws-role-name" aria-label="角色名称" placeholder="角色名称，如 planner" size="12" value="' + esc(role.role || "") + '">' +
        '<select class="owo-ws-role-assignee" aria-label="执行方">' +
        '<option value="agent"' + (role.assignee === "agent" ? " selected" : "") + ">Agent 执行</option>" +
        '<option value="human"' + (role.assignee === "human" ? " selected" : "") + ">人工承接</option>" +
        '<option value="worker"' + (role.assignee === "worker" ? " selected" : "") + ">Worker 执行</option>" +
        "</select>" +
        workerSel +
        '<input class="owo-ws-role-worker" aria-label="自定义 Worker 名称或用户 ID" placeholder="自定义 Worker 名称或用户 ID" size="18" value="' + esc(plan.custom || "") + '"' + (plan.select === CUSTOM_WORKER ? "" : " hidden") + ">" +
        '<input class="owo-ws-role-deps" aria-label="依赖的上游角色" placeholder="依赖的上游角色（逗号分隔）" size="16" value="' + esc((role.depends_on || []).join(",")) + '">' +
        '<button class="owo-ws-role-del" type="button" aria-label="删除角色" title="删除该角色">✕</button>' +
        "</div>"
      );
    }

    function addRoleRow(role) {
      var box = el("#ws-roles");
      if (!box) return;
      box.insertAdjacentHTML("beforeend", roleRowHtml(role || {}));
    }

    /// DOM → 原始角色行（含 workerSelected 标记，供提交前校验；不直接进请求体）。
    function collectRoleRows() {
      var box = el("#ws-roles");
      if (!box) return [];
      var rows = box.querySelectorAll(".owo-ws-role-row");
      var out = [];
      for (var i = 0; i < rows.length; i++) {
        var r = rows[i];
        var role = r.querySelector(".owo-ws-role-name").value.trim();
        var sel = r.querySelector(".owo-ws-role-worker-sel");
        var wInput = r.querySelector(".owo-ws-role-worker");
        var selected = sel ? sel.value : "";
        var worker = selected === CUSTOM_WORKER ? (wInput ? wInput.value.trim() : "") : selected;
        out.push({
          role: role,
          assignee: r.querySelector(".owo-ws-role-assignee").value,
          workerSelected: selected,
          worker: worker,
          depsText: r.querySelector(".owo-ws-role-deps").value,
        });
      }
      return out;
    }

    /// §8.1 提交前校验：自定义 worker 行必填；返回错误文案数组。
    function roleRowErrors() {
      return domain.validateRoleRows(collectRoleRows());
    }

    function collectRoles() {
      return domain.sanitizeRoles(
        collectRoleRows().map(function (r) {
          var deps = r.depsText
            .split(",")
            .map(function (x) {
              return x.trim();
            })
            .filter(Boolean);
          return {
            role: r.role,
            assignee: r.assignee,
            worker: r.worker,
            depends_on: deps,
          };
        })
      );
    }

    function fillCreateTemplateSelect() {
      var sel = el("#ws-template");
      if (!sel) return;
      var prev = sel.value;
      sel.innerHTML =
        '<option value="">（动态组队：按上方角色规格）</option>' +
        state.templates
          .map(function (t) {
            return '<option value="' + esc(t.template_id) + '">' + esc(t.name) + "（" + esc(normStatus(t.mode)) + "）</option>";
          })
          .join("");
      if (state.templates.some(function (t) {
        return t.template_id === prev;
      }))
        sel.value = prev;
    }

    function loadCreateTemplates() {
      var retry = el("#ws-create-template-retry");
      var status = el("#ws-create-template-status");
      if (retry) {
        retry.disabled = true;
        retry.textContent = "正在加载…";
      }
      if (status) status.textContent = "正在加载任务模板…";
      return H.get("/teams/templates")
        .then(function (d) {
          state.templates = Array.isArray(d && d.templates) ? d.templates : [];
          fillCreateTemplateSelect();
          if (retry) retry.hidden = true;
          if (status) status.textContent = state.templates.length
            ? "已加载 " + state.templates.length + " 个模板；也可以直接填写角色规格。"
            : "当前没有可用模板；可以直接填写角色规格创建团队。";
          return state.templates;
        })
        .catch(function (e) {
          state.templates = [];
          fillCreateTemplateSelect();
          if (retry) retry.hidden = false;
          if (status) status.textContent =
            "模板暂不可用：" + short(explainError(e, "模板列表"), 120) +
            " 仍可直接填写角色规格创建团队。";
          return null;
        })
        .then(function (result) {
          if (retry) {
            retry.disabled = false;
            retry.textContent = "重试加载";
          }
          return result;
        });
    }

    function prefillFromTemplate(tplId) {
      var t = null;
      for (var i = 0; i < state.templates.length; i++) {
        if (state.templates[i].template_id === tplId) t = state.templates[i];
      }
      if (!t) return;
      var modeSel = el("#ws-mode");
      if (modeSel) modeSel.value = normStatus(t.mode);
      var box = el("#ws-roles");
      if (!box) return;
      box.innerHTML = "";
      (t.roles || []).forEach(function (r) {
        addRoleRow(r);
      });
      if (!(t.roles || []).length) addRoleRow();
    }

    // 返回 Promise（成功/失败都在内部展示结果并 resolve）；校验失败返回 null，
    // 由 bindLockedButton 据此决定是否进入提交锁。
    function doCreate() {
      if (!state || state.view !== "create") return null;
      var objEl = el("#ws-obj");
      var objective = objEl ? objEl.value.trim() : "";
      if (!objective) {
        show("ws-create-result", "err", "objective 不能为空：请填写团队目标");
        return null;
      }
      var modeEl = el("#ws-mode");
      var tplEl = el("#ws-template");
      var mode = modeEl ? modeEl.value : "team";
      var tpl = tplEl ? tplEl.value : "";
      var body = { objective: objective, mode: mode };
      // 五期：组队策略（auto 为缺省值；现服务端忽略未知字段，第一路落地后生效）
      var stratEl = el("#ws-strategy");
      var strategy = stratEl ? String(stratEl.value || "auto") : "auto";
      body.strategy = strategy === "auto" ? "auto" : strategy;
      if (tpl && mode !== "single") body.template_id = tpl;
      // §8.1：提交前校验角色规格（自定义 worker 行必填）。
      var roleErrs = roleRowErrors();
      if (roleErrs.length) {
        show("ws-create-result", "err", roleErrs.join("；"));
        return null;
      }
      var roles = collectRoles();
      if (roles.length) body.roles = roles;
      show("ws-create-result", "", "创建中…");
      return H.post("/teams", body)
        .then(function (d) {
          show(
            "ws-create-result",
            "ok",
            "团队已创建：" +
              (d.team_id || "（响应缺少 team_id，请到团队列表查看）") +
              "（状态 " +
              (TEAM_STATUS_CN[normStatus(d.status)] || d.status || "—") +
              "，HTTP 202 已 spawn 运行）。正在打开团队详情…"
          );
          if (!d.team_id) return; // 缺 team_id 不做跳转，避免打开 /teams/undefined
          // 跳转前的提示窗口内保持按钮禁用，避免双击重复创建团队
          var goBtn = el("#ws-create-go");
          if (goBtn) {
            goBtn.setAttribute("data-busy", "1");
            goBtn.disabled = true;
          }
          setTimeout(function () {
            unlockBtn(goBtn);
            openTeam(d.team_id);
          }, 400);
        })
        .catch(function (e) {
          show("ws-create-result", "err", explainError(e, "创建团队"));
        });
    }

    function renderCreate() {
      return (
        '<div class="owo-ws-sec">' +
        '<h3>创建团队 <span class="hint">创建后立即启动运行</span></h3>' +
        '<label class="owo-ws-create-label" for="ws-obj">团队目标 <span class="hint">必填</span></label>' +
        '<textarea id="ws-obj" rows="3" spellcheck="false" placeholder="例如：设计并实现一个文本 diff 的 CLI 工具，并给出单元测试"></textarea>' +
        '<div class="owo-ws-create-options">' +
        '<label class="owo-ws-create-field"><span>运行模式</span>' +
        '<select id="ws-mode">' +
        '<option value="team">多 Agent 接力（规划、实现、评审）</option>' +
        '<option value="single">单 Agent 执行</option>' +
        '<option value="swarmflow">DAG 工作流（按角色依赖执行）</option>' +
        '</select></label>' +
        '<label class="owo-ws-create-field"><span>组队策略</span>' +
        '<select id="ws-strategy">' +
        '<option value="auto">自动判断（默认）</option>' +
        '<option value="single">强制单 Agent</option>' +
        '<option value="team">强制多 Agent</option>' +
        '</select></label>' +
        '<label class="owo-ws-create-field"><span>任务模板</span>' +
        '<span class="owo-ws-template-controls"><select id="ws-template"><option value="">（按上方角色规格创建）</option></select>' +
        '<button id="ws-create-template-retry" class="owo-ws-mini" type="button" hidden>重试</button></span>' +
        '</label>' +
        '</div>' +
        '<div id="ws-create-template-status" class="hint owo-ws-template-status" role="status" aria-live="polite"></div>' +
        '<div id="ws-roles-block"><label class="owo-ws-create-label" for="ws-roles">角色规格 <span class="hint">可选；留空时使用当前模式的默认流程</span></label>' +
        '<div id="ws-roles" class="owo-ws-role-list"></div>' +
        '<div class="owo-ws-inline owo-ws-create-role-actions"><button id="ws-role-add" class="owo-ws-mini" type="button">＋ 添加角色</button><span class="hint">DAG 模式可设置依赖；人工承接时请填写用户 ID。</span></div></div>' +
        '<div class="owo-ws-inline owo-ws-create-submit"><button id="ws-create-go" data-core-action class="primary" type="button">创建并启动</button><span class="hint">Agent 使用已连接的模型服务；echo、sleep、fail 可用于流程测试。</span></div>' +
        '<pre class="owo-ws-result sub" id="ws-create-result" role="status" aria-live="polite">—</pre>' +
        '</div>'
      );
    }

    /// §8.1：角色规格编辑器仅在非 single 模式显示（single 忽略角色规格）。
    function paintRolesVisibility() {
      var m = el("#ws-mode");
      var blk = el("#ws-roles-block");
      if (m && blk) blk.hidden = normStatus(m.value) === "single";
    }

    function bindCreate() {
      var addBtn = el("#ws-role-add");
      if (addBtn) addBtn.onclick = function () {
        addRoleRow();
      };
      var rolesBox = el("#ws-roles");
      if (rolesBox)
        rolesBox.addEventListener("click", function (ev) {
          if (ev.target && ev.target.classList.contains("owo-ws-role-del")) ev.target.closest(".owo-ws-role-row").remove();
        });
      if (rolesBox)
        // §8.1 门禁：worker 选“自定义…”才显示输入框；assignee=human 自动切自定义（填用户 id）。
        rolesBox.addEventListener("change", function (ev) {
          var t = ev.target;
          if (!t || !t.classList) return;
          var row = t.closest(".owo-ws-role-row");
          if (!row) return;
          var sel = row.querySelector(".owo-ws-role-worker-sel");
          var input = row.querySelector(".owo-ws-role-worker");
          if (t.classList.contains("owo-ws-role-worker-sel")) {
            var custom = t.value === CUSTOM_WORKER;
            if (input) {
              input.hidden = !custom;
              input.placeholder = "自定义 worker";
              if (custom) input.focus();
              if (!custom) input.value = "";
            }
          } else if (t.classList.contains("owo-ws-role-assignee")) {
            if (t.value === "human" && sel && input) {
              sel.value = CUSTOM_WORKER;
              input.hidden = false;
              input.placeholder = "承接用户 id";
              input.focus();
            } else if (input) {
              input.placeholder = "自定义 worker";
            }
          }
        });
      var modeSel = el("#ws-mode");
      if (modeSel) modeSel.onchange = paintRolesVisibility;
      paintRolesVisibility();
      var tpl = el("#ws-template");
      if (tpl)
        tpl.onchange = function () {
          prefillFromTemplate(tpl.value);
        };
      bindLockedButton(el("#ws-create-go"), doCreate, "创建中…");
      addRoleRow();
      var retryTemplates = el("#ws-create-template-retry");
      if (retryTemplates) retryTemplates.onclick = loadCreateTemplates;
      loadCreateTemplates();
    }

    // ---------- 视图：团队列表 ----------
    function paintTeams() {
      var body = el("#ws-list-body");
      if (!body) return;
      var t = state.teams;
      if (!t.length) {
        body.innerHTML = '<div class="hint">暂无团队。切换到"创建团队"页签创建第一个团队。</div>';
        return;
      }
      body.innerHTML =
        '<table class="owo-ws-table"><tr><th>team_id</th><th>模式</th><th>状态</th><th>执行</th><th>成员</th><th>项目空间</th><th>创建</th><th>更新</th><th></th></tr>' +
        t
          .map(function (x) {
            var st = normStatus(x.status);
            var listBadge =
              x.interrupted && !isTerminalTeam(st)
                ? '<span class="owo-ws-badge st-interrupted" title="进程重启遗留的运行态：进度已保留，可用 continue / retry 显式恢复">已中断（可恢复）</span>'
                : '<span class="owo-ws-badge st-' + st + '">' + (TEAM_STATUS_CN[st] || esc(x.status)) + "</span>";
            return (
              "<tr>" +
              '<td class="hint">' + esc(x.team_id) + "</td>" +
              "<td>" + esc(normStatus(x.mode)) + "</td>" +
              "<td>" + listBadge + "</td>" +
              '<td class="' + (x.active ? "owo-ws-active on" : "hint") + '">' + (x.active ? "●" : "○") + "</td>" +
              "<td>" + ((x.members || []).length) + "</td>" +
              '<td class="hint">' + esc(x.project_space_id || "—") + "</td>" +
              '<td class="hint">' + esc(x.created_at) + "</td>" +
              '<td class="hint">' + esc(x.updated_at) + "</td>" +
              '<td><button class="owo-ws-mini" data-team="' + esc(x.team_id) + '">详情</button></td>' +
              "</tr>"
            );
          })
          .join("") +
        "</table>";
      var rows = body.querySelectorAll("button[data-team]");
      for (var i = 0; i < rows.length; i++) {
        rows[i].onclick = function () {
          openTeam(this.getAttribute("data-team"));
        };
      }
    }

    function loadTeams() {
      var body = el("#ws-list-body");
      if (body && !body.querySelector(".owo-ws-table")) {
        body.innerHTML = stateBox("loading", "正在加载团队列表…");
      }
      return H.get("/teams")
        .then(function (d) {
          state.teams = (d && d.teams) || [];
          paintTeams();
          // 空态文案由 paintTeams 输出；这里无需额外处理
        })
        .catch(function (e) {
          var b2 = el("#ws-list-body");
          if (b2) b2.innerHTML = stateBox("error", explainError(e, "团队列表"), "teams");
        });
    }

    function renderList() {
      return (
        '<div class="owo-ws-sec">' +
        '<h3>团队列表 <span class="hint">来自本地核心服务</span> <button class="owo-ws-mini" id="ws-list-refresh">刷新</button></h3>' +
        '<div id="ws-list-body">' + stateBox("loading", "正在加载团队列表…") + "</div>" +
        "</div>"
      );
    }

    function bindList() {
      bindLockedButton(el("#ws-list-refresh"), loadTeams, "刷新中…");
      loadTeams();
    }

    // ---------- 视图：模板与提案 ----------
    function roleChips(roles) {
      if (!roles || !roles.length) return '<span class="hint">（无角色）</span>';
      return roles
        .map(function (r) {
          return (
            '<span class="chip">' +
            esc(r.role) +
            "（" +
            esc(r.assignee) +
            (r.worker ? ":" + esc(r.worker) : "") +
            (r.depends_on && r.depends_on.length ? " ← " + esc(r.depends_on.join(",")) : "") +
            "）</span>"
          );
        })
        .join(" ");
    }

    function tplCard(t) {
      return (
        '<div class="owo-ws-tpl-card">' +
        '<div class="owo-ws-member-head"><b>' + esc(t.name) + "</b>" +
        '<span class="chip">' + esc(normStatus(t.mode)) + "</span>" +
        '<span class="hint">' + esc(t.template_id) + "</span></div>" +
        '<div class="hint">适用条件：' + esc(t.applicability || "—") + "</div>" +
        '<div class="hint">' + roleChips(t.roles) + "</div>" +
        '<div class="hint">创建 ' + esc(t.created_at || "—") + (t.source_team_id ? " · 来源团队 " + esc(t.source_team_id) : "") + "</div>" +
        '<div class="owo-ws-inline"><button class="owo-ws-mini" data-usetpl="' + esc(t.template_id) + '">用此模板创建团队</button></div>' +
        "</div>"
      );
    }

    function propCard(p) {
      return (
        '<div class="owo-ws-tpl-card">' +
        '<div class="owo-ws-member-head"><b>' + esc(p.template.name) + "</b>" +
        '<span class="chip">' + esc(normStatus(p.template.mode)) + "</span>" +
        '<span class="chip owo-ws-prop-' + esc(p.status) + '">' +
        (PROPOSAL_STATUS_CN[p.status] || esc(p.status)) +
        "</span></div>" +
        '<div class="hint">提案 ' + esc(p.proposal_id) + " · 来源团队 " + esc(p.source_team_id) + " · 生成 " + esc(p.created_at) + "</div>" +
        '<div class="hint">证据：' +
        (p.evidence && p.evidence.length
          ? p.evidence
              .map(function (x) {
                return '<span class="chip">' + esc(x) + "</span>";
              })
              .join(" ")
          : "—") +
        "</div>" +
        '<div class="hint">' + roleChips(p.template.roles) + "</div>" +
        (p.status === "proposed"
          ? '<div class="owo-ws-inline"><button class="primary" data-core-action data-adopt="' + esc(p.proposal_id) + '">采纳</button><button class="danger" data-core-action data-reject="' + esc(p.proposal_id) + '">拒绝</button></div>'
          : "") +
        '<pre class="owo-ws-result sub" data-proj-result="' + esc(p.proposal_id) + '">—</pre>' +
        "</div>"
      );
    }

    function paintTemplates() {
      var box = el("#ws-tpl-list");
      if (!box) return;
      box.innerHTML = state.templates.length
        ? state.templates.map(tplCard).join("")
        : '<div class="hint">暂无已采纳模板。先运行并验证一个团队，待系统生成模板提案后在右侧"采纳"。</div>';
      var btns = box.querySelectorAll("button[data-usetpl]");
      for (var i = 0; i < btns.length; i++) {
        btns[i].onclick = function () {
          var tplId = this.getAttribute("data-usetpl");
          state.view = "create";
          renderView();
          var sel = el("#ws-template");
          if (sel) sel.value = tplId;
          prefillFromTemplate(tplId);
        };
      }
    }

    function paintProposals() {
      var box = el("#ws-prop-list");
      if (!box) return;
      box.innerHTML = state.proposals.length
        ? state.proposals.map(propCard).join("")
        : '<div class="hint">暂无模板提案。团队成功完成一次运行后，系统会据此生成模板提案（只提案，不自动启用）。</div>';
      var btns = box.querySelectorAll("button[data-adopt], button[data-reject]");
      for (var i = 0; i < btns.length; i++) {
        (function (btn) {
          btn.onclick = function () {
            if (btn.disabled || btn.getAttribute("data-busy") === "1") return;
            var adopt = btn.hasAttribute("data-adopt");
            var attr = adopt ? "data-adopt" : "data-reject";
            var pid = btn.getAttribute(attr);
            var pre = box.querySelector('pre[data-proj-result="' + pid + '"]');
            // 提交期间同时锁定同一张卡的「采纳/拒绝」两个按钮，防重复与并发操作
            var mate = box.querySelector((adopt ? 'button[data-reject="' : 'button[data-adopt="') + pid + '"]');
            var p = adopt ? adoptProposal(pid, pre) : rejectProposal(pid, pre); // 确认取消 → null
            if (!p || typeof p.then !== "function") return;
            lockBtn(btn, adopt ? "采纳中…" : "拒绝中…");
            if (mate) mate.disabled = true;
            p.then(
              function () {
                unlockBtn(btn);
                if (mate && !mate.getAttribute("data-busy")) mate.disabled = false;
              },
              function () {
                unlockBtn(btn);
                if (mate && !mate.getAttribute("data-busy")) mate.disabled = false;
              }
            );
          };
        })(btns[i]);
      }
    }

    function loadTemplates() {
      var box = el("#ws-tpl-list");
      if (box && !box.querySelector(".owo-ws-tpl-card")) {
        box.innerHTML = stateBox("loading", "正在加载已采纳模板…");
      }
      return H.get("/teams/templates")
        .then(function (d) {
          state.templates = (d && d.templates) || [];
          paintTemplates();
          fillCreateTemplateSelect();
        })
        .catch(function (e) {
          var b2 = el("#ws-tpl-list");
          if (b2) b2.innerHTML = stateBox("error", explainError(e, "模板列表"), "templates");
        });
    }

    function loadProposals() {
      var box = el("#ws-prop-list");
      if (box && !box.querySelector(".owo-ws-tpl-card")) {
        box.innerHTML = stateBox("loading", "正在加载模板提案…");
      }
      return H.get("/teams/templates/proposals")
        .then(function (d) {
          state.proposals = (d && d.proposals) || [];
          paintProposals();
        })
        .catch(function (e) {
          var b2 = el("#ws-prop-list");
          if (b2) b2.innerHTML = stateBox("error", explainError(e, "提案列表"), "proposals");
        });
    }

    // 采纳/拒绝：成功与失败都在提案卡内的结果区展示；返回 Promise 供提交锁。
    function adoptProposal(pid, pre) {
      return H.post("/teams/templates/proposals/" + encodeURIComponent(pid) + "/adopt", {})
        .then(function (d) {
          if (pre) {
            pre.textContent = "已采纳 → 模板 " + (d.template ? d.template.template_id + " · " + d.template.name : "（见左侧列表）");
            pre.className = "owo-ws-result ok";
          }
          loadTemplates();
          loadProposals();
        })
        .catch(function (e) {
          if (pre) {
            pre.textContent = explainError(e, "采纳提案 " + pid);
            pre.className = "owo-ws-result err";
          }
        });
    }

    function rejectProposal(pid, pre) {
      return confirmAction(
        "拒绝模板提案",
        "拒绝该模板提案？提案记录会保留以供审计，且无法再被采纳。",
        "拒绝提案"
      ).then(function (confirmed) {
        if (!confirmed) return null;
        return H.post("/teams/templates/proposals/" + encodeURIComponent(pid) + "/reject", {})
          .then(function (d) {
            if (pre) {
              pre.textContent = "已拒绝提案 " + (d.proposal_id || pid) + "（状态 " + (d.status || "rejected") + "，记录保留可审计）";
              pre.className = "owo-ws-result ok";
            }
            loadProposals();
          })
          .catch(function (e) {
            if (pre) {
              pre.textContent = explainError(e, "拒绝提案 " + pid);
              pre.className = "owo-ws-result err";
            }
          });
      });
    }

    function renderTemplates() {
      return (
        '<div class="owo-ws-sec">' +
        '<h3>已采纳模板 <span class="hint">采纳后可在创建团队时选用</span> <button class="owo-ws-mini" id="ws-tpl-refresh">刷新</button></h3>' +
        '<div class="owo-ws-tpl-grid" id="ws-tpl-list">' + stateBox("loading", "正在加载已采纳模板…") + "</div>" +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>模板提案 <span class="hint">只提案不自动启用，需人工采纳/拒绝</span> <button class="owo-ws-mini" id="ws-prop-refresh">刷新</button></h3>' +
        '<div class="owo-ws-tpl-grid" id="ws-prop-list">' + stateBox("loading", "正在加载模板提案…") + "</div>" +
        "</div>"
      );
    }

    function bindTemplates() {
      bindLockedButton(el("#ws-tpl-refresh"), loadTemplates, "刷新中…");
      bindLockedButton(el("#ws-prop-refresh"), loadProposals, "刷新中…");
      loadTemplates();
      loadProposals();
    }

    // ---------- 视图：团队详情 ----------
    function toggleForm(id, btnId) {
      var f = el(id); // 调用方传裸 ID，避免 "##id"
      var b = el(btnId);
      if (!f) return;
      f.style.display = f.style.display === "none" ? "" : "none";
      if (b) b.textContent = b.textContent.replace(/[▴▾]$/, f.style.display === "none" ? "▾" : "▴");
    }

    // 返回 Promise；校验失败/被终态拦截返回 null。
    function doSteerPost(body, label) {
      if (!state.current) return null;
      if (isTerminalTeam(currentTeamStatus())) {
        show("ws-act-result", "err", "团队已进入终态（" + (TEAM_STATUS_CN[normStatus(currentTeamStatus())] || currentTeamStatus()) + "），操作已停用。如需重新运行请新建团队");
        return null;
      }
      return H.post("/teams/" + encodeURIComponent(state.current) + "/steer", body)
        .then(function (d) {
          var st = normStatus(d && d.status);
          show("ws-act-result", "ok", "[" + label + "] 已提交，团队状态：" + (TEAM_STATUS_CN[st] || (d && d.status) || "—"));
          syncDetail();
        })
        .catch(function (e) {
          show("ws-act-result", "err", explainError(e, label));
        });
    }

    function doSteer() {
      if (!state || state.view !== "detail") return null;
      var stepEl = el("#ws-act-steer-step");
      var noteEl = el("#ws-act-steer-note");
      var rawEl = el("#ws-act-steer-input");
      var step = stepEl ? stepEl.value : "";
      var note = noteEl ? noteEl.value.trim() : "";
      if (!note) {
        show("ws-act-result", "err", "steer 需要 note（转向指令内容），请填写");
        return null;
      }
      var rawIn = rawEl ? rawEl.value.trim() : "";
      var body = { command: "steer", note: note };
      if (step) body.step_id = step;
      if (rawIn) {
        var v;
        try {
          v = JSON.parse(rawIn);
        } catch (e) {
          show("ws-act-result", "err", 'new_input 必须是合法 JSON（例如 {"prompt": "..."}）');
          return null;
        }
        body.new_input = v;
      }
      return doSteerPost(body, "转向");
    }

    function doReplace() {
      if (!state || state.view !== "detail") return null;
      var roleEl = el("#ws-act-replace-role");
      var role = roleEl ? roleEl.value : "";
      if (!role) {
        show("ws-act-result", "err", "换员前请先选择角色");
        return null;
      }
      var body = { command: "replace", role: role };
      // §8.1：worker 门禁——known 下拉直选；“自定义…”须填写标识；空 = 不变更。
      var wSel = el("#ws-act-replace-worker");
      var wCustom = el("#ws-act-replace-worker-custom");
      var resolved = domain.resolveReplaceWorker(wSel ? wSel.value : "", wCustom ? wCustom.value : "");
      if (!resolved.ok) {
        show("ws-act-result", "err", resolved.error);
        return null;
      }
      if (resolved.worker) body.new_worker = resolved.worker;
      var u = el("#ws-act-replace-user");
      var n = el("#ws-act-replace-note");
      if (u && u.value.trim()) body.new_user_id = u.value.trim();
      if (n && n.value.trim()) body.note = n.value.trim();
      return doSteerPost(body, "换员");
    }

    function doHumanResult() {
      if (!state || state.view !== "detail" || !state.current) return null;
      var tidEl = el("#ws-h-task");
      var textEl = el("#ws-h-text");
      var tid = tidEl ? tidEl.value : "";
      var result = textEl ? textEl.value.trim() : "";
      if (isTerminalTeam(currentTeamStatus())) {
        show("ws-h-result", "err", "团队已进入终态，人节点结果提交已停用");
        return null;
      }
      if (!tid) {
        show("ws-h-result", "err", "请选择一个未完成的人节点任务（当前没有可提交的任务）");
        return null;
      }
      if (!result) {
        show("ws-h-result", "err", "人节点结果不能为空：请填写结果内容");
        return null;
      }
      return H.post("/tasks/" + encodeURIComponent(tid) + "/human-result", { team_id: state.current, result: result })
        .then(function (d) {
          show(
            "ws-h-result",
            "ok",
            "人节点结果已提交 → 产物 " +
              (d.artifact && d.artifact.artifact_id ? d.artifact.artifact_id : "（见审计）") +
              "（评审状态 " +
              (d.artifact && d.artifact.review_state ? d.artifact.review_state : "—") +
              "），下游任务将自动继续。"
          );
          if (textEl) textEl.value = "";
          syncDetail();
        })
        .catch(function (e) {
          show("ws-h-result", "err", explainError(e, "提交人节点结果"));
        });
    }

    function doHandoff() {
      if (!state || state.view !== "detail" || !state.current) return null;
      var tidEl = el("#ws-x-task");
      var fromEl = el("#ws-x-from");
      var summaryEl = el("#ws-x-summary");
      var toEl = el("#ws-x-to");
      var tid = tidEl ? tidEl.value : "";
      var from = fromEl ? fromEl.value : "";
      var summary = summaryEl ? summaryEl.value.trim() : "";
      if (isTerminalTeam(currentTeamStatus())) {
        show("ws-x-result", "err", "团队已进入终态，任务交接已停用");
        return null;
      }
      if (!tid) {
        show("ws-x-result", "err", "请选择要交接的任务");
        return null;
      }
      if (!from) {
        show("ws-x-result", "err", "交出成员不能为空（必须为任务执行者）");
        return null;
      }
      if (!summary) {
        show("ws-x-result", "err", "完成摘要不能为空：请描述已完成的内容");
        return null;
      }
      var body = {
        team_id: state.current,
        from_member: from,
        completed_summary: summary,
        open_issues: splitCsv(el("#ws-x-issues")),
        // §8.1：结构化引用行（可搜索选择器产出），不再手输 CSV。
        output_artifact_refs: (state.handoffRefs.arts || []).slice(),
        evidence_refs: (state.handoffRefs.evid || []).slice(),
        suggested_next_actions: splitCsv(el("#ws-x-next")),
        known_risks: splitCsv(el("#ws-x-risks")),
      };
      var to = toEl ? toEl.value.trim() : "";
      if (to) body.to_member = to;
      return H.post("/tasks/" + encodeURIComponent(tid) + "/handoff", body)
        .then(function (d) {
          show(
            "ws-x-result",
            "ok",
            "交接已提交 → 交接记录 " +
              (d.handoff && d.handoff.handoff_id ? d.handoff.handoff_id : "（见审计）") +
              "。下游任务可据此继续。"
          );
          if (summaryEl) summaryEl.value = "";
          state.handoffRefs = { arts: [], evid: [] };
          state.handoffRefQuery = { arts: "", evid: "" };
          paintHandoffRefPickers();
          loadTeamConversation(state.current, true);
          syncDetail();
        })
        .catch(function (e) {
          show("ws-x-result", "err", explainError(e, "提交交接结果"));
        });
    }

    function renderDetail() {
      return (
        '<div class="owo-ws-d-head">' +
        '<button class="owo-ws-mini" id="ws-d-back">← 团队列表</button>' +
        '<span class="owo-ws-d-id" id="ws-d-id">…</span>' +
        '<span class="owo-ws-badge" id="ws-d-status">加载中</span>' +
        '<span class="owo-ws-active" id="ws-d-active"></span>' +
        '<span class="hint" id="ws-d-meta">正在加载团队详情…</span>' +
        '<button class="owo-ws-mini" id="ws-d-diag" title="导出 TeamRun 诊断（凭据与敏感输入已由服务端脱敏）">下载诊断信息（已脱敏）</button>' +
        "</div>" +
        '<div id="ws-d-main" style="display:flex;flex-direction:column;gap:10px">' +
        '<pre class="owo-ws-result sub" id="ws-d-error"></pre>' +
        '<section class="team-delivery" id="ws-d-delivery" aria-label="本次任务的生成文件"></section>' +
        '<div class="owo-ws-sec">' +
        '<h3>运行摘要 <span class="hint">状态 / 活动阶段 / 任务计数 / 失败步骤 / 尝试 / 产物</span></h3>' +
        '<div id="ws-d-summary" class="owo-ws-sumbox" aria-live="polite">' + stateBox("loading", "正在汇总运行状态…") + "</div>" +
        "</div>" +
        '<details class="owo-ws-sec owo-ws-conversation" id="ws-d-conversation">' +
        '<summary>团队交接对话 <span class="hint">按发送者查看已保存的交接消息；不代表完整内部对话记录</span></summary>' +
        '<div id="ws-d-conversation-messages" class="owo-ws-chat-wrap"><div class="hint">展开后加载交接记录…</div></div>' +
        '<button type="button" class="owo-ws-mini" id="ws-d-conversation-refresh">刷新交接消息</button>' +
        "</details>" +
        '<div class="owo-ws-interrupted" id="ws-d-interrupted" role="alert">⚠ 运行已中断，可恢复' +
        '<span class="owo-ws-int-detail">检测到上次进程未正常收尾遗留的运行态：任务进度已保留，未自动重放任何写操作。可用「继续（continue）」恢复运行，或对失败/被中断节点点「重试此节点」。</span></div>' +
        '<div class="owo-ws-gate" id="ws-d-gate">⛔ 团队已进入终态：运行操作与人节点结果提交已停用（仍可查看任务、产物与审计）。</div>' +
        '<div class="owo-ws-sec">' +
        '<h3>运行操作 <span class="hint">继续 / 重试失败节点 / 转向 / 更换成员 / 取消</span></h3>' +
        '<div class="owo-ws-inline">' +
        '<button id="ws-act-continue" data-core-action class="owo-ws-mini">继续（continue）</button>' +
        '<button id="ws-act-steer-toggle" class="owo-ws-mini">转向（steer）▾</button>' +
        '<button id="ws-act-replace-toggle" class="owo-ws-mini">换员（replace）▾</button>' +
        '<button id="ws-act-cancel" data-core-action class="danger">取消（cancel）</button>' +
        "</div>" +
        '<div class="owo-ws-form" id="ws-act-steer-form" style="display:none">' +
        '<div class="owo-ws-inline"><label>目标任务 step_id</label><select id="ws-act-steer-step"></select>' +
        '<label>指令 note</label><input id="ws-act-steer-note" placeholder="转向指令（必填）" size="32"></div>' +
        '<label class="hint">新输入 new_input（可选 JSON，覆盖该任务输入）</label>' +
        '<textarea id="ws-act-steer-input" rows="2" spellcheck="false" placeholder="{&quot;prompt&quot;: &quot;改为…&quot;}"></textarea>' +
        '<div class="owo-ws-inline"><button id="ws-act-steer-go" data-core-action class="primary">发送转向</button></div>' +
        "</div>" +
        '<div class="owo-ws-form" id="ws-act-replace-form" style="display:none">' +
        '<div class="owo-ws-inline"><label>角色 role</label><select id="ws-act-replace-role"></select>' +
        '<label>新 worker</label><select id="ws-act-replace-worker">' +
        '<option value="">（不变更 worker）</option>' +
        '<option value="agent">agent — 模型驱动</option>' +
        '<option value="echo">echo — 回显测试</option>' +
        '<option value="sleep">sleep — 模拟耗时</option>' +
        '<option value="fail">fail — 模拟失败</option>' +
        '<option value="__custom__">自定义…</option>' +
        "</select>" +
        '<input id="ws-act-replace-worker-custom" placeholder="自定义 worker 标识" size="14" hidden>' +
        '<label>承接用户（可选）</label><input id="ws-act-replace-user" placeholder="人节点换人时填写" size="12">' +
        '<label>note</label><input id="ws-act-replace-note" placeholder="换员原因（可选）" size="20"></div>' +
        '<div class="owo-ws-inline"><button id="ws-act-replace-go" data-core-action class="primary">提交换员</button></div>' +
        '<p class="hint" style="margin:0">新执行方式自该角色任务的下一轮尝试生效；人节点可指定承接用户。</p>' +
        "</div>" +
        '<pre class="owo-ws-result sub" id="ws-act-result">—</pre>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>成员与职责 <span class="hint">成员 / 运行绑定 / 交接契约</span></h3>' +
        '<div class="owo-ws-member-grid" id="ws-d-members">' + stateBox("loading", "正在加载成员与职责…") + "</div>" +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>任务 DAG <span class="hint">红色边框 = 已阻塞（上游失败/中止，不会再运行）</span> <button class="owo-ws-mini" id="ws-dag-refresh">刷新任务</button></h3>' +
        '<div id="ws-d-dag">' + stateBox("loading", "正在加载任务…") + "</div>" +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>人节点结果 <span class="hint">团队处于"等待人节点"时由人工提交结果，下游任务自动继续</span></h3>' +
        '<div class="owo-ws-inline">' +
        "<label>人节点任务</label><select id=\"ws-h-task\"></select>" +
        "</div>" +
        '<label class="hint">结果内容 result（必填；将存为产物并注入下游任务输入）</label>' +
        '<textarea id="ws-h-text" rows="3" spellcheck="false" placeholder="人工完成的产物/结论，例如：验收意见 + 修订要求"></textarea>' +
        '<div class="owo-ws-inline"><button id="ws-h-go" data-core-action class="primary">提交结果</button><span class="hint" id="ws-h-gate-note"></span></div>' +
        '<pre class="owo-ws-result sub" id="ws-h-result">—</pre>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>任务交接 <span class="hint">记录"谁完成了什么、留下什么问题、下一步建议"</span></h3>' +
        '<div class="owo-ws-inline">' +
        "<label>任务</label><select id=\"ws-x-task\"></select>" +
        "<label>交出成员（任务执行者）</label><select id=\"ws-x-from\"></select>" +
        '<label>接收成员（可选）</label><select id="ws-x-to"></select>' +
        "</div>" +
        '<label title="已完成内容与结论，下游任务据此继续">完成摘要（必填）</label>' +
        '<textarea id="ws-x-summary" rows="2" spellcheck="false"></textarea>' +
        '<details class="hint"><summary>可选明细（逗号分隔多项）</summary>' +
        '<div style="display:flex;flex-direction:column;gap:4px;margin:6px 0">' +
        '<label title="未解决、需要下游知晓或跟进的事项">遗留问题 <input id="ws-x-issues" size="60"></label>' +
        '<label>关联产物（可搜索选择，可增删）</label><div id="ws-x-arts-pick"></div>' +
        '<label>证据引用（可搜索选择或自由填写，可增删）</label><div id="ws-x-evid-pick"></div>' +
        '<label title="建议下游执行的后续动作">下一步建议 <input id="ws-x-next" size="60"></label>' +
        '<label title="已知的风险与注意事项">已知风险 <input id="ws-x-risks" size="60"></label>' +
        "</div></details>" +
        '<div class="owo-ws-inline"><button id="ws-x-go" data-core-action class="primary">提交交接</button></div>' +
        '<pre class="owo-ws-result sub" id="ws-x-result">—</pre>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>实时进度 <span class="hint">progress 事件（步骤开始/完成/失败/取消，seq 单调递增；断线恢复后旧 seq 自动跳过）</span></h3>' +
        '<div id="ws-d-progress" class="owo-ws-prog" aria-live="polite"><div class="hint">等待进度数据…</div></div>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>工作区与模板 <span class="hint">六期：绑定目录 / 读写范围 / 模板及版本 / 失败原因代码</span>' +
        '<button class="owo-ws-mini" id="ws-ws-refresh">刷新工作区</button></h3>' +
        '<div id="ws-d-workspace"></div>' +
        '<div id="ws-d-failures"></div>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>Worker 能力与执行详情 <span class="hint">七期：WorkerProfile 实际工具权限与调用预算 / 单写租约 / 文件变更与 diff；八期：ChangeSet 审批（接受/拒绝/撤销）</span></h3>' +
        '<div id="ws-d-profiles"></div>' +
        '<div id="ws-d-lease"></div>' +
        '<div id="ws-d-changes"></div>' +
        '<div id="ws-d-csets"></div>' +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>组队策略与角色指标 <span class="hint">auto 判定理由 —— 耗时·调用·token·费用·预算余量</span>' +
        '<button class="owo-ws-mini" id="ws-metrics-refresh">刷新指标</button></h3>' +
        '<div id="ws-d-strategy" class="owo-ws-strategybox">' + strategyBoxHtml(state.strategyDecision) + "</div>" +
        '<div id="ws-d-metrics">' + metricsCardsHtml(state.metrics, state.strategyDecision && state.strategyDecision.budgetPerRole) + "</div>" +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>产物 <span class="hint">CAS 引用，版本链 + 评审闭环</span>' +
        '<button class="owo-ws-mini" id="ws-art-refresh">刷新产物</button>' +
        '<button class="owo-ws-mini" id="ws-dlv-toggle">最终交付物</button>' +
        '<button class="owo-ws-mini" id="ws-dlv-dl" title="版本/哈希/校验/证据汇总文本">下载交付清单</button></h3>' +
        '<div id="ws-dlv-box" hidden></div>' +
        '<div id="ws-d-artifacts">' + stateBox("loading", "正在加载产物…") + "</div>" +
        "</div>" +
        '<div class="owo-ws-sec">' +
        '<h3>审计事件（实时） <span class="hint">团队事件流</span> <span class="owo-ws-live off" id="ws-d-stream"></span></h3>' +
        '<div class="owo-ws-audit" id="ws-d-audit" role="log" aria-label="审计事件流（可聚焦后用方向键滚动）" tabindex="0">' + stateBox("loading", "正在连接审计通道…") + "</div>" +
        "</div>" +
        "</div>"
      );
    }

    function simplifyTeamDetail() {
      var main=el('#ws-d-main');
      if(!main || !main.children || !win.document) return;
      var groups={};
      ['执行过程','调整任务与人工协作','记录与诊断'].forEach(function(label){
        var details=win.document.createElement('details');details.className='team-more';details.dataset.teamSection=label;
        var summary=win.document.createElement('summary');summary.textContent=label;details.appendChild(summary);groups[label]=details;
      });
      Array.from(main.children).forEach(function(section){
        if(!section.classList.contains('owo-ws-sec'))return;
        var heading=section.querySelector('h3,summary');
        var text=heading?heading.textContent:'';
        if(text.indexOf('运行摘要')===0){section.classList.add('team-progress');heading.firstChild.textContent='执行进度 ';return;}
        var group=/^(成员与职责|任务 DAG|实时进度|团队交接对话)/.test(text)?'执行过程':/^(运行操作|人节点结果|任务交接)/.test(text)?'调整任务与人工协作':'记录与诊断';
        groups[group].appendChild(section);
      });
      var metadata=win.document.createElement('div');metadata.className='team-run-meta';
      ['ws-d-id','ws-d-meta','ws-d-diag'].forEach(function(id){var element=el('#'+id);if(element)metadata.appendChild(element);});
      groups['记录与诊断'].insertBefore(metadata,groups['记录与诊断'].children[1]||null);
      Object.keys(groups).forEach(function(key){main.appendChild(groups[key]);});
      var header=el('.owo-ws-d-head');
      [['ws-act-continue','继续任务'],['ws-act-cancel','停止任务']].forEach(function(item){var button=el('#'+item[0]);if(button&&header){button.textContent=item[1];header.appendChild(button);}});
      var box=el('#ws-d-delivery');
      if(box && win.OwoTeamDelivery)deliveryView=win.OwoTeamDelivery.create({box:box,bridge:win.owo&&win.owo.teamFiles});
    }

    function bindDetail() {
      simplifyTeamDetail();
      var back = el("#ws-d-back");
      if (back)
        back.onclick = function () {
          stopLive();
          state.view = "list";
          renderView();
        };
      // 运行操作：continue / steer / replace / cancel 全部带提交锁；
      // toggle 按钮仅展开表单，不涉及请求，不锁。
      bindLockedButton(el("#ws-act-continue"), function () {
        return doSteerPost({ command: "continue", note: "继续" }, "继续");
      }, "提交中…");
      var stT = el("#ws-act-steer-toggle");
      if (stT)
        stT.onclick = function () {
          toggleForm("ws-act-steer-form", "ws-act-steer-toggle");
        };
      bindLockedButton(el("#ws-act-steer-go"), doSteer, "发送中…");
      var rpT = el("#ws-act-replace-toggle");
      if (rpT)
        rpT.onclick = function () {
          toggleForm("ws-act-replace-form", "ws-act-replace-toggle");
        };
      bindLockedButton(el("#ws-act-replace-go"), doReplace, "提交中…");
      // §8.1：worker 选“自定义…”才显示标识输入框；选其他值时清空并隐藏。
      var wSel = el("#ws-act-replace-worker");
      var wCustom = el("#ws-act-replace-worker-custom");
      if (wSel && wCustom)
        wSel.onchange = function () {
          var custom = wSel.value === domain.CUSTOM_WORKER;
          wCustom.hidden = !custom;
          if (custom) wCustom.focus();
          else wCustom.value = "";
        };
      bindLockedButton(el("#ws-act-cancel"), function () {
        return confirmAction(
          "取消团队运行",
          "取消该团队运行？未完成的任务将被中止，运行无法恢复。",
          "取消运行"
        ).then(function (confirmed) {
          if (!confirmed) return null;
          state.cancelling = true; // 立即反馈"取消中"，不等服务端往返
          paintProgress();
          var p = doSteerPost({ command: "cancel", note: "用户取消" }, "取消");
          // 下发失败（网络/权限）：撤销"取消中"标记，让用户可重试
          if (p && typeof p.catch === "function") {
            p = p.catch(function (e) {
              state.cancelling = false;
              paintProgress();
              throw e;
            });
          }
          return p;
        });
      }, "取消中…");
      bindLockedButton(el("#ws-h-go"), doHumanResult, "提交中…");
      bindLockedButton(el("#ws-x-go"), doHandoff, "提交中…");
      bindHandoffRefPickers();
      var xTask = el("#ws-x-task");
      if (xTask)
        xTask.onchange = function () {
          paintFromSelect();
        };
      bindLockedButton(el("#ws-dag-refresh"), refreshTasksOnly, "刷新中…");
      bindLockedButton(el("#ws-art-refresh"), function () {
        if (state.handoffArtsFetched || state.handoffArtsError || state.handoffArtsLoading) {
          return loadHandoffArtifacts(true);
        }
        return loadArtifacts();
      }, "刷新中…");
      var conversation = el("#ws-d-conversation");
      if (conversation) conversation.addEventListener("toggle", function () {
        if (conversation.open) loadTeamConversation(state.current, false);
      });
      var conversationRefresh = el("#ws-d-conversation-refresh");
      if (conversationRefresh) conversationRefresh.onclick = function () {
        loadTeamConversation(state.current, true);
      };
      bindLockedButton(el("#ws-metrics-refresh"), loadMetrics, "刷新中…");
      var diagBtn = el("#ws-d-diag");
      if (diagBtn)
        bindLockedButton(diagBtn, function () {
          return downloadDiagnostic(state.current).then(
            function (d) {
              show("ws-act-result", "ok", "诊断信息已下载（服务端已脱敏：不含凭据/Authorization/完整敏感输入）。");
              return d;
            },
            function (e) {
              show("ws-act-result", "err", explainError(e, "诊断导出"));
              throw e;
            }
          );
        }, "导出中…");
      var dlvBtn = el("#ws-dlv-toggle");
      if (dlvBtn)
        dlvBtn.onclick = function () {
          state.deliverablesOpen = !state.deliverablesOpen;
          var box = el("#ws-dlv-box");
          if (!box) return;
          box.hidden = !state.deliverablesOpen;
          dlvBtn.setAttribute("data-open", state.deliverablesOpen ? "1" : "0");
          if (state.deliverablesOpen) loadDeliverables();
        };
      // 七期：交付清单下载（路由未上线/404 → ws-act-result 提示，不阻塞页面）。
      bindLockedButton(el("#ws-dlv-dl"), function () {
        return downloadDeliveryManifest().then(function (d) {
          show("ws-act-result", "ok", "交付清单已下载（版本 / sha256 / 大小 / 批准态 / 证据引用）。");
          return d;
        });
      }, "生成中…");
      markLiveRegions();
      applyGating(); // 以当前已知状态初始化按钮可用性（数据到达后再刷新）
    }

    function openTeam(id) {
      if(rootEl)rootEl.scrollTop=0;
      if (!id || typeof id !== "string") return; // 防 /teams/undefined
      stopLive();
      state.view = "detail";
      state.current = id;
      state.team = null;
      state.tasks = [];
      state.audit = [];
      state.auditKeys = {};
      state.active = false;
      state.teamStatus = "";
      state.interrupted = false;
      state.artifactCount = null;
      state.handoffs = null;
      state.conversationError = "";
      state.conversationRequest = null;
      state.conversationSeq += 1;
      state.humanTask = "";
      state.progress = null;
      state.lastProgressSeq = 0;
      state.cancelling = false;
      state.artifacts = [];
      // Artifact requests continue across team navigation; keep per-artifact locks until settled.
      state.reviewResult = null;
      state.reviewFlash = null;
      state.strategyDecision = null;
      state.metrics = null;
      state.deliverables = null;
      state.deliverablesOpen = false;
      state.reworkResult = null;
      state.historyReviews = {};
      state.diagnostic = null;
      // 六期：工作区/模板/失败原因重置
      state.workspace = null;
      state.workspaceView = null;
      state.workspaceViewKind = "";
      state.workspaceBusy = false;
      state.templateInfo = null;
      stopProgressTimer();
      renderView();
    }

    // ---------- 视图调度 ----------
    function paintTabs() {
      if (!rootEl) return;
      var activeView = state.view === "detail" ? "list" : state.view;
      var tabs = rootEl.querySelectorAll(".owo-ws-tab");
      for (var i = 0; i < tabs.length; i++) {
        var b = tabs[i];
        var on = b.getAttribute("data-view") === activeView;
        if (on && !b.classList.contains("active")) b.classList.add("active");
        if (!on && b.classList.contains("active")) b.classList.remove("active");
      }
    }

    function renderView() {
      var main = el("#ws-main");
      if (!main) return;
      try {
        if(deliveryView){deliveryView.dispose();deliveryView=null;}
        paintTabs();
        // 离开详情视图必须停掉 SSE/轮询，避免在创建页/列表页留下幽灵请求
        if (state.view !== "detail") stopLive();
        if (state.view === "create") {
          main.innerHTML = renderCreate();
          bindCreate();
        } else if (state.view === "templates") {
          main.innerHTML = renderTemplates();
          bindTemplates();
        } else if (state.view === "detail" && state.current) {
          main.innerHTML = renderDetail();
          bindDetail();
          loadDetail(state.current);
        } else {
          // detail 但没有 current：回退列表，防止渲染 /teams/undefined 页面
          if (state.view === "detail") state.view = "list";
          paintTabs();
          main.innerHTML = renderList();
          bindList();
        }
        markLiveRegions();
      } catch (e) {
        // 渲染兜底：宁可显示错误也不要整块空白
        if (typeof console !== "undefined" && console.error) console.error("[workswarm] 视图渲染异常:", e);
        main.innerHTML =
          '<div class="owo-ws-sec"><h3>面板渲染异常</h3><div class="hint err">' +
          esc((e && e.message) || String(e)) +
          '</div><div class="hint" style="margin-top:6px">请切换到其他页签（如"团队列表"）后重试；若持续出现，请联系维护者并提供控制台报错。</div></div>';
      }
    }

    function bindTabs() {
      var tabs = rootEl.querySelectorAll(".owo-ws-tab");
      for (var i = 0; i < tabs.length; i++) {
        (function (b) {
          b.addEventListener("click", function () {
            state.view = b.getAttribute("data-view");
            renderView();
          });
        })(tabs[i]);
      }
    }

    // ---------- 样式（全部 .owo-ws-* 前缀） ----------
    // 注意：<style> 注入是全局生效的。旧版这里存在裸选择器（.hint/.chip/
    // button.primary/select,input,textarea…），会把外壳的全局表单样式整个
    // 覆盖掉 —— 现已全部收敛到 .owo-ws-panel 作用域下。
    // 颜色一律走 style.css 的设计令牌（--surface/--muted/--green…），
    // 深浅主题自动跟随外壳切换；fallback 仅保证面板被单独打开时可用。
    var CSS =
      "section.owo-ws-panel{display:flex;flex-direction:column;gap:10px;padding:4px;min-width:0;}" +
      ".owo-ws-tabs{display:flex;gap:6px;flex-wrap:wrap;}" +
      ".owo-ws-tab{background:transparent;border:1px solid var(--border,#e2e6ee);color:var(--muted,#5b6577);padding:5px 13px;border-radius:999px;cursor:pointer;font-size:13px;font-family:inherit;" +
      "transition:background-color .15s var(--ease-out,ease-out),color .15s var(--ease-out,ease-out),border-color .15s var(--ease-out,ease-out);}" +
      "@media(hover:hover){.owo-ws-tab:hover:not(.active){background:var(--hover,#ebEEF5);color:var(--text,#1d2433);}}" +
      // 激活页签与外壳 Rail 一致的着色药丸语言
      ".owo-ws-tab.active{background:var(--active-tint,#e7eeff);border-color:transparent;color:var(--accent,#2563eb);font-weight:600;}" +
      ".owo-ws-main{display:flex;flex-direction:column;gap:10px;min-width:0;}" +
      ".owo-ws-sec{border:1px solid var(--border,#e2e6ee);border-radius:12px;background:var(--surface,#fff);box-shadow:var(--shadow-sm,0 1px 2px rgba(18,25,40,.05));padding:12px 14px;display:flex;flex-direction:column;gap:8px;min-width:0;}" +
      ".owo-ws-sec h3{margin:0;font-size:14px;display:flex;gap:8px;align-items:center;flex-wrap:wrap;}" +
      // 以下通用类均限定在面板作用域内，不再污染全局
      ".owo-ws-panel .hint{font-size:12px;color:var(--muted,#5b6577);}" +
      ".owo-ws-panel .hint.err{color:var(--red,#cd3f35);}" +
      "pre.owo-ws-result{white-space:pre-wrap;word-break:break-word;font-size:12px;margin:0;max-height:180px;overflow:auto;font-family:inherit;border:1px solid var(--soft-border,#edf0f5);border-radius:8px;background:var(--raised,#f1f3f8);padding:7px 9px;}" +
      ".owo-ws-result.ok{color:var(--green,#14855a);border-color:var(--green-line,rgba(20,133,90,.3));}" +
      ".owo-ws-result.err{color:var(--red,#cd3f35);border-color:var(--red-line,rgba(207,63,53,.32));background:var(--red-soft,rgba(207,63,53,.1));}" +
      ".owo-ws-result.sub{color:var(--text,#1d2433);background:var(--surface,#fff);}" +
      ".owo-ws-panel button.owo-ws-mini{background:transparent;border:1px solid var(--border,#e2e6ee);color:var(--text,#1d2433);padding:3px 9px;border-radius:7px;cursor:pointer;font-size:12px;font-family:inherit;" +
      "transition:border-color .15s var(--ease-out,ease-out),background-color .15s var(--ease-out,ease-out),transform .15s var(--ease-out,ease-out);}" +
      "@media(hover:hover){.owo-ws-panel button.owo-ws-mini:hover:not([disabled]){border-color:var(--accent,#2563eb);background:var(--accent-soft,rgba(37,99,235,.09));}}" +
      ".owo-ws-panel button.primary{background:var(--accent,#2563eb);border:1px solid var(--accent,#2563eb);color:var(--accent-ink,#fff);padding:5px 13px;border-radius:8px;cursor:pointer;font-size:12px;font-family:inherit;font-weight:600;}" +
      "@media(hover:hover){.owo-ws-panel button.primary:hover:not([disabled]){background:var(--accent-hover,#1e56cf);border-color:var(--accent-hover,#1e56cf);}}" +
      ".owo-ws-panel button.danger{background:transparent;border:1px solid var(--red-line,rgba(207,63,53,.32));color:var(--red,#cd3f35);padding:5px 13px;border-radius:8px;cursor:pointer;font-size:12px;font-family:inherit;}" +
      "@media(hover:hover){.owo-ws-panel button.danger:hover:not([disabled]){background:var(--red-soft,rgba(207,63,53,.1));}}" +
      ".owo-ws-panel select,.owo-ws-panel input,.owo-ws-panel textarea{width:auto;background:var(--surface,#fff);border:1px solid var(--border,#e2e6ee);color:inherit;border-radius:7px;padding:4px 8px;font-size:12px;font-family:inherit;max-width:100%;}" +
      ".owo-ws-table{width:100%;border-collapse:collapse;font-size:12px;}" +
      ".owo-ws-table th,.owo-ws-table td{border-bottom:1px solid var(--soft-border,#edf0f5);padding:5px 7px;text-align:left;vertical-align:top;}" +
      ".owo-ws-table th{color:var(--muted,#5b6577);font-weight:600;white-space:nowrap;}" +
      ".owo-ws-badge{display:inline-block;padding:1px 9px;border-radius:10px;font-size:12px;background:var(--raised,#f1f3f8);color:var(--muted,#5b6577);font-variant-numeric:tabular-nums;}" +
      // 团队状态徽标：语义色平铺到柔和底色上，深浅主题同源
      ".owo-ws-badge.st-created{background:var(--active-tint,#e7eeff);color:var(--accent,#2563eb);}" +
      ".owo-ws-badge.st-running,.owo-ws-badge.st-succeeded{background:var(--green-soft,rgba(20,133,90,.11));color:var(--green,#14855a);}" +
      ".owo-ws-badge.st-awaiting_human{background:var(--yellow-soft,rgba(199,138,26,.13));color:var(--yellow,#a06a04);}" +
      ".owo-ws-badge.st-failed{background:var(--red-soft,rgba(207,63,53,.1));color:var(--red,#cd3f35);}" +
      ".owo-ws-active{font-size:12px;color:var(--faint,#8a93a5);font-variant-numeric:tabular-nums;}" +
      ".owo-ws-active.on{color:var(--green,#14855a);font-weight:600;}" +
      ".owo-ws-member-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(270px,1fr));gap:8px;}" +
      ".owo-ws-member{border:1px solid var(--border,#e2e6ee);border-radius:10px;padding:9px;display:flex;flex-direction:column;gap:4px;font-size:12px;background:var(--surface,#fff);}" +
      ".owo-ws-member-head{display:flex;gap:6px;align-items:center;flex-wrap:wrap;}" +
      ".owo-ws-panel .chip{display:inline-block;padding:0 8px;border-radius:9px;font-size:12px;background:var(--raised,#f1f3f8);color:var(--muted,#5b6577);}" +
      ".owo-ws-panel .chip.h-active{color:var(--green,#14855a);}" +
      ".owo-ws-panel .chip.h-degraded{color:var(--yellow,#a06a04);}" +
      ".owo-ws-panel .chip.h-offline,.owo-ws-panel .chip.h-fused{color:var(--red,#cd3f35);}" +
      // 提案状态徽标（替代旧的行内 style 三元硬编码）
      ".owo-ws-panel .chip.owo-ws-prop-proposed{color:var(--yellow,#a06a04);background:var(--yellow-soft,rgba(199,138,26,.13));}" +
      ".owo-ws-panel .chip.owo-ws-prop-adopted{color:var(--green,#14855a);background:var(--green-soft,rgba(20,133,90,.11));}" +
      ".owo-ws-panel .chip.owo-ws-prop-rejected{color:var(--red,#cd3f35);background:var(--red-soft,rgba(207,63,53,.1));}" +
      ".owo-ws-dagwrap{overflow:auto;border:1px solid var(--soft-border,#edf0f5);border-radius:10px;max-height:560px;background:var(--raised,#f1f3f8);}" +
      ".owo-ws-dag{display:block;}" +
      ".owo-ws-dag line{stroke:var(--scroll-thumb,rgba(122,132,152,.38));stroke-width:1.5;}" +
      ".owo-ws-dag line.dead{stroke:var(--red,#cd3f35);}" +
      ".owo-ws-node{box-sizing:border-box;width:100%;height:100%;border:1px solid var(--border,#e2e6ee);border-radius:9px;padding:6px 8px;font-size:12px;background:var(--surface,#fff);display:flex;flex-direction:column;gap:2px;overflow:hidden;}" +
      ".owo-ws-node.st-running{border-color:var(--accent,#2563eb);box-shadow:0 0 0 1px var(--accent-soft,rgba(37,99,235,.09)) inset;}" +
      ".owo-ws-node.st-succeeded{border-color:var(--green-line,rgba(20,133,90,.3));}" +
      ".owo-ws-node.st-failed,.owo-ws-node.st-aborted{border-color:var(--red-line,rgba(207,63,53,.32));}" +
      ".owo-ws-node.st-pending,.owo-ws-node.st-ready{border-style:dashed;}" +
      ".owo-ws-node.blocked{border:2px solid var(--red,#cd3f35);}" +
      ".owo-ws-node-line{display:flex;justify-content:space-between;gap:6px;align-items:center;}" +
      ".owo-ws-node-line b{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}" +
      ".owo-ws-node-err{color:var(--red,#cd3f35);font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}" +
      ".owo-ws-audit{max-height:340px;overflow:auto;border:1px solid var(--soft-border,#edf0f5);border-radius:10px;padding:6px 10px;font-size:12px;background:var(--raised,#f1f3f8);}" +
      ".owo-ws-audit-row{display:flex;gap:8px;padding:2px 0;border-bottom:1px solid var(--soft-border,#edf0f5);}" +
      ".owo-ws-audit-ts{color:var(--faint,#8a93a5);white-space:nowrap;font-family:Consolas,ui-monospace,monospace;font-variant-numeric:tabular-nums;}" +
      ".owo-ws-audit-ev{color:var(--accent,#2563eb);white-space:nowrap;min-width:110px;}" +
      ".owo-ws-audit-det{color:var(--text,#1d2433);word-break:break-all;opacity:.85;}" +
      ".owo-ws-role-row{display:flex;gap:6px;flex-wrap:wrap;align-items:center;}" +
      ".owo-ws-role-del{background:transparent;border:1px solid var(--red-line,rgba(207,63,53,.32));color:var(--red,#cd3f35);border-radius:6px;cursor:pointer;font-size:12px;padding:1px 7px;font-family:inherit;}" +
      ".owo-ws-d-head{display:flex;gap:10px;align-items:center;flex-wrap:wrap;}" +
      ".owo-ws-d-id{font-family:Consolas,ui-monospace,monospace;font-size:14px;font-weight:700;}" +
      ".owo-ws-tpl-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(340px,1fr));gap:10px;}" +
      ".owo-ws-tpl-card{border:1px solid var(--border,#e2e6ee);border-radius:11px;padding:11px;display:flex;flex-direction:column;gap:6px;font-size:12px;background:var(--surface,#fff);}" +
      ".owo-ws-form{border:1px dashed var(--border,#e2e6ee);border-radius:8px;padding:8px;display:flex;flex-direction:column;gap:6px;}" +
      // 终态门控横幅：琥珀语义色的柔和应用
      ".owo-ws-gate{display:none;border:1px solid var(--yellow-line,rgba(160,106,4,.34));background:var(--yellow-soft,rgba(199,138,26,.13));color:var(--yellow,#a06a04);border-radius:9px;padding:7px 11px;font-size:12px;font-weight:600;}" +
      // 实时通道指示：SSE=绿 / 轮询降级=琥珀 / 停止=灰
      ".owo-ws-live{font-size:12px;color:var(--faint,#8a93a5);}" +
      ".owo-ws-live.on{color:var(--green,#14855a);}" +
      ".owo-ws-live.poll{color:var(--yellow,#a06a04);}" +
      ".owo-ws-live.off{color:var(--faint,#8a93a5);}" +
      // 任务统计摘要行（阻塞标红，运行中标强调色）
      ".owo-ws-dagsum{font-size:12px;color:var(--muted,#5b6577);margin:0 0 6px;font-variant-numeric:tabular-nums;}" +
      ".owo-ws-dagsum b.blocked{color:var(--red,#cd3f35);}" +
      ".owo-ws-dagsum b.run{color:var(--accent,#2563eb);}" +
      // 加载中状态轻微弱化，便于与空态区分（焦点/禁用反馈由全局原语统一提供）
      ".owo-ws-loading{opacity:.75;}" +
      // 七期：停止中/已停止状态徽标（cancel 接入 Worker 取消令牌后的过渡/停止态）
      ".owo-ws-badge.st-stopping{background:var(--yellow-soft,rgba(199,138,26,.13));color:var(--yellow,#a06a04);}" +
      ".owo-ws-badge.st-stopped{background:var(--red-soft,rgba(207,63,53,.1));color:var(--red,#cd3f35);}" +
      // 七期：评审语义徽标补全（REVIEW_CLS 一直输出 rv-* 类，此处补齐配色）
      ".owo-ws-badge.rv-ok{background:var(--green-soft,rgba(20,133,90,.11));color:var(--green,#14855a);}" +
      ".owo-ws-badge.rv-warn{background:var(--yellow-soft,rgba(199,138,26,.13));color:var(--yellow,#a06a04);}" +
      ".owo-ws-badge.rv-bad{background:var(--red-soft,rgba(207,63,53,.1));color:var(--red,#cd3f35);}" +
      // 七期：单写租约状态盒
      ".owo-ws-lease{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:12px;min-width:0;}" +
      // 七期：文件变更列表 + diff 预览（复用 .owo-ws-diff 的差异配色）
      ".owo-ws-chg-list{display:flex;flex-direction:column;gap:4px;margin-top:4px;min-width:0;}" +
      ".owo-ws-chg-row{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:12px;border:1px solid var(--soft-border,#edf0f5);border-radius:8px;padding:4px 8px;min-width:0;}" +
      ".owo-ws-chg-row b{overflow-wrap:anywhere;}" +
      ".owo-ws-chg-diff{margin-top:3px;min-width:0;}" +
      ".owo-ws-chg-diff > summary{cursor:pointer;font-size:12px;color:var(--accent,#2563eb);}" +
      ".owo-ws-chg-diff > summary::marker{content:\"▸ \";}" +
      "";

    function nav() {
      return (
        '<section data-panel="workswarm" class="owo-ws-panel">' +
        "<style>" + CSS + "</style>" +
        '<div class="owo-ws-tabs">' +
        '<button class="owo-ws-tab" data-view="create">创建团队</button>' +
        '<button class="owo-ws-tab" data-view="list">团队列表</button>' +
        '<button class="owo-ws-tab" data-view="templates">模板与提案</button>' +
        "</div>" +
        '<div class="owo-ws-main" id="ws-main"></div>' +
        "</section>"
      );
    }

    function mount(root, helpers) {
      rootEl = root;
      H = helpers || {};
      H.baseUrl = H.baseUrl || (win.OwoPanels && win.OwoPanels.baseUrl) || "";
      H.get = H.get || defaultGet;
      H.post = H.post || defaultPost;
      H.esc = H.esc || defaultEsc;
      stopLive();
      detailSeq++; // 上一次详情流水的响应全部作废
      state.view = "list";
      state.current = null;
      root.innerHTML = nav();
      // 重试按钮的事件委托绑在本次渲染出的 section 上：
      // innerHTML 重建时旧监听器随之销毁，重复挂载不会累积监听器。
      var section = root.querySelector("section.owo-ws-panel");
      if (section) bindDelegates(section);
      bindTabs();
      renderView();
    }

    function refresh() {
      if (!rootEl) return;
      renderView();
    }

    // —— 测试挂钩（tests/workswarm.panel.test.mjs 专用；浏览器运行时不读取） ——
    // 只暴露纯逻辑与字符串构建器；state 引用供测试预置输入（不触发 DOM）。
    var TEST_API = {
      normStatus: normStatus,
      taskStatusLabel: taskStatusLabel,
      taskOptionHtml: taskOptionHtml,
      isTerminalTeam: isTerminalTeam,
      taskBlocked: taskBlocked,
      isRetryableStep: isRetryableStep,
      retryableTasks: retryableTasks,
      shouldShowRetry: shouldShowRetry,
      buildRetryBody: buildRetryBody,
      computeRunSummary: computeRunSummary,
      explainError: explainError,
      stateBox: stateBox,
      dagSvg: dagSvg,
      renderDetail: renderDetail,
      renderList: renderList,
      renderCreate: renderCreate,
      loadCreateTemplates: loadCreateTemplates,
      handleRetryClick: handleRetryClick,
      retryAction: retryAction,
      submitRetry: submitRetry,
      syncDetail: syncDetail,
      // —— 四期挂钩：实时进度 + Artifact 评审 ——
      applyProgress: applyProgress,
      handleEventFrame: handleEventFrame,
      computeProgressView: computeProgressView,
      renderProgress: renderProgress,
      fmtElapsed: fmtElapsed,
      paintProgress: paintProgress,
      stopProgressTimer: stopProgressTimer,
      pollOnce: pollOnce,
      stopLive: stopLive,
      normReviewState: normReviewState,
      reviewBadgeHtml: reviewBadgeHtml,
      isReviewable: isReviewable,
      groupArtifactChain: groupArtifactChain,
      buildReviewBody: buildReviewBody,
      explainReviewError: explainReviewError,
      submitArtifactReview: submitArtifactReview,
      artifactHistoryHtml: artifactHistoryHtml,
      artifactRowHtml: artifactRowHtml,
      renderArtifactsChains: renderArtifactsChains,
      loadArtifactHistory: loadArtifactHistory,
      loadArtifacts: loadArtifacts,
      findArtifactById: findArtifactById,
      // —— §8.1 挂钩：交接引用选择器（可搜索 + 可增删结构化行） ——
      artifactRefOptions: artifactRefOptions,
      handoffArtifactsStatusHtml: handoffArtifactsStatusHtml,
      loadHandoffArtifacts: loadHandoffArtifacts,
      paintHandoffRefPickers: paintHandoffRefPickers,
      bindHandoffRefPickers: bindHandoffRefPickers,
      addRefRow: domain.addRefRow,
      removeRefRow: domain.removeRefRow,
      filterRefOptions: domain.filterRefOptions,
      // —— §8.1 挂钩：自定义 worker/role 门禁 ——
      KNOWN_WORKERS: KNOWN_WORKERS,
      CUSTOM_WORKER: CUSTOM_WORKER,
      resolveReplaceWorker: domain.resolveReplaceWorker,
      roleWorkerPlan: domain.roleWorkerPlan,
      validateRoleRows: domain.validateRoleRows,
      sanitizeRoles: domain.sanitizeRoles,
      collectRoleRows: collectRoleRows,
      roleRowErrors: roleRowErrors,
      roleRowHtml: roleRowHtml,
      paintRolesVisibility: paintRolesVisibility,
      // —— 五期挂钩：自适应组队 / 角色指标 / 版本链返工 / 交付物 / 诊断 ——
      strategyDecisionOf: strategyDecisionOf,
      strategyBoxHtml: strategyBoxHtml,
      metricsFromPayload: metricsFromPayload,
      metricsCardsHtml: metricsCardsHtml,
      fmtMs: fmtMs,
      diffLines: diffLines,
      diffHtml: diffHtml,
      artifactTimelineHtml: artifactTimelineHtml,
      buildReworkBody: buildReworkBody,
      explainReworkError: explainReworkError,
      submitRework: submitRework,
      deliverablesFromPayload: deliverablesFromPayload,
      deliverablesBoxHtml: deliverablesBoxHtml,
      latestChangesReviewId: latestChangesReviewId,
      loadMetrics: loadMetrics,
      loadDeliverables: loadDeliverables,
      downloadDiagnostic: downloadDiagnostic,
      // —— 六期 ——
      workspaceFromTeam: workspaceFromTeam,
      workspaceBoxHtml: workspaceBoxHtml,
      workspaceTreeHtml: workspaceTreeHtml,
      gitStatusHtml: gitStatusHtml,
      templateBoxHtml: templateBoxHtml,
      failureCodeLabel: failureCodeLabel,
      failureBadgeHtml: failureBadgeHtml,
      failureSummaryHtml: failureSummaryHtml,
      loadWorkspace: loadWorkspace,
      loadWorkspaceView: loadWorkspaceView,
      paintWorkspace: paintWorkspace,
      // —— 七期挂钩：Worker 能力 / 写租约 / 文件变更 / 产物校验与下载交付 ——
      workerProfilesTable: workerProfilesTable,
      writeLeaseBox: writeLeaseBox,
      changesListHtml: changesListHtml,
      changeStateBadge: changeStateBadge,
      changesRemoteView: changesRemoteView,
      changeRecordsHtml: changeRecordsHtml,
      changesRuntimeHtml: changesRuntimeHtml,
      loadWorkspaceChanges: loadWorkspaceChanges,
      // 八期：ChangeSet 审批闭环
      normCsStatus: normCsStatus,
      changeSetsView: changeSetsView,
      changeSetBadge: changeSetBadge,
      changeSetsHtml: changeSetsHtml,
      loadChangeSets: loadChangeSets,
      startChangeSetAction: startChangeSetAction,
      // 九期：状态口径 / 批准门控 / 幂等键 / 跨面板刷新
      csStatusHint: csStatusHint,
      approvalBlockView: approvalBlockView,
      approvalBlockBanner: approvalBlockBanner,
      csIdemKey: csIdemKey,
      refreshInboxPanel: refreshInboxPanel,
      validationBadgeHtml: validationBadgeHtml,
      artifactFileName: artifactFileName,
      fmtAbsTime: fmtAbsTime,
      deliveryManifestText: deliveryManifestText,
      paintSevenRuntime: paintSevenRuntime,
      saveTextFile: saveTextFile,
      downloadArtifact: downloadArtifact,
      downloadDeliveryManifest: downloadDeliveryManifest,
      isGatedTeam: isGatedTeam,
      css: function () {
        return CSS;
      },
      state: state,
      // 测试注入传输层（H 在 mount 时可能被整体替换，故经访问器读写）
      getTransport: function () {
        return { get: H.get, post: H.post };
      },
      setTransport: function (t) {
        if (t && t.get) H.get = t.get;
        if (t && t.post) H.post = t.post;
      },
      setRoot: function (root) {
        rootEl = root;
      },
    };

    return {
      id: ID,
      title: "WorkSwarm（团队编排）",
      nav: nav,
      mount: mount,
      refresh: refresh,
      dispose: function () { stopLive("已离开团队页面"); rootEl = null; },
      open: openTeam, // 六期：Project Launcher 创建成功后直达团队详情
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效、零运行时差异。
if (typeof module !== "undefined" && module.exports) {
  var __wswWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __wswWin.OwoPanels.workswarm;
}
