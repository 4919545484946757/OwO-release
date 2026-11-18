// 统一任务运行面板（Agent V2 Phase 2/3）：把 /agent-tasks 的状态机、检查点、
// 产物、审批与事件暴露到桌面端。所有状态变化都由服务端状态机校验，前端只发动作、
// 不推导终态；未注册后端时明确显示「未分发」，不把 running 当成执行成功。
//
// 本面板是增量新增：不改首页布局，也不改既有页面的视觉与样式。
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels["agent-tasks"] = (function () {
    var ID = "agent-tasks";
    var H = {};
    var rootEl = null;
    var lifecycleGeneration = 0;
    var refreshGeneration = 0;
    var tasks = [];
    var selectedId = null;
    var hasSnapshot = false;
    var apiClient = null;
    var workspaceClient = null;
    var eventStore = null;
    var selectedState = "";
    var liveReloadTaskId = "";
    var liveStateCursor = 0;
    // 「拒绝改动」会回滚并删除 worktree，两步确认避免误点。
    var wsRejectArmed = "";

    var STATE_META = {
      draft: ["草稿", "idle"],
      queued: ["排队中", "run"],
      clarifying: ["澄清中", "wait"],
      planning: ["规划中", "run"],
      running: ["执行中", "run"],
      awaiting_approval: ["待审批", "wait"],
      verifying: ["验证中", "run"],
      repairing: ["修复中", "wait"],
      succeeded: ["已完成", "ok"],
      paused: ["已暂停", "wait"],
      cancel_requested: ["取消中", "wait"],
      cancelled: ["已取消", "bad"],
      failed: ["失败", "bad"],
      interrupted: ["已中断", "wait"],
      needs_user_input: ["需处理", "wait"],
    };
    var STRATEGY_LABEL = {
      single_agent: "单人执行",
      work_swarm: "团队协作",
      computer_use: "桌面操作",
      workflow: "工作流",
      cloud_remote: "云端执行",
    };
    var STATE_ORDER = [
      "queued", "planning", "running", "awaiting_approval", "verifying", "repairing",
      "paused", "needs_user_input", "interrupted", "succeeded", "failed", "cancelled", "draft",
    ];

    function defaultEsc(s) {
      return String(s == null ? "" : s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function esc(s) {
      return H.esc ? H.esc(s) : defaultEsc(s);
    }
    function defaultGet(path) {
      return win.OwoApi.get(path);
    }
    function defaultFriendlyError(error) {
      return String((error && error.message) || error || "未知错误");
    }
    function friendly(error) {
      var text = H.friendlyError ? H.friendlyError(error) : defaultFriendlyError(error);
      var code = error && error.code ? String(error.code) : "";
      return code ? text + "（code: " + code + "）" : text;
    }

    function stateInfo(state) {
      var hit = STATE_META[String(state || "")] || [String(state || "—"), "idle"];
      return { label: hit[0], tone: hit[1] };
    }
    function stateBadge(state) {
      var info = stateInfo(state);
      return '<span class="owo-at-badge owo-at-tone-' + info.tone + '">' + esc(info.label) + "</span>";
    }
    function strategyLabel(strategy) {
      return STRATEGY_LABEL[String(strategy || "")] || String(strategy || "—");
    }
    function formatTime(value) {
      var text = String(value || "");
      if (!text) return "—";
      return text.replace("T", " ").replace("Z", "").split(".")[0].slice(0, 19);
    }
    function firstLine(text) {
      return String(text || "").split(/\r?\n/)[0].trim().slice(0, 60);
    }
    function post(path, body) {
      if (typeof H.post === "function") return H.post(path, body);
      if (win.OwoApi && typeof win.OwoApi.post === "function") return win.OwoApi.post(path, body);
      return Promise.reject(new Error("没有可用的 POST 通道"));
    }
    function del(path) {
      if (typeof H.del === "function") return H.del(path);
      if (win.OwoApi && typeof win.OwoApi.delete === "function") return win.OwoApi.delete(path);
      return Promise.reject(new Error("没有可用的 DELETE 通道"));
    }
    function isTerminal(state) {
      return ["succeeded", "failed", "cancelled"].indexOf(String(state || "")) >= 0;
    }
    function needsAttention(task) {
      return ["awaiting_approval", "needs_user_input", "paused", "interrupted"].indexOf(String(task && task.state || "")) >= 0;
    }

    /// 暂停后的恢复语义提示：不同后端的「继续执行」含义不同，不能只显示按钮。
    /// 云端没有冻结语义（暂停=投递取消，恢复=重新提交），工作流引擎没有断点续跑
    /// （恢复=新建一次运行），两者都会让命令/步骤再执行一遍。
    function resumeHint(task) {
      if (String(task && task.state || "") !== "paused") return "";
      var strategy = String(task && task.execution_strategy || "");
      if (strategy === "cloud_remote") {
        return "云端执行没有冻结语义：暂停已经向云端任务投递取消，继续执行会重新提交一次云端任务，命令会再执行一遍。";
      }
      if (strategy === "workflow") {
        return "工作流引擎没有断点续跑：继续执行会新建一次运行，已完成的步骤和追加型写入会再执行一遍。";
      }
      if (strategy === "work_swarm") {
        return "团队协作支持续跑：继续执行沿用同一次团队运行，已完成的步骤不会重跑。";
      }
      return "";
    }

    /// 服务端状态机允许的动作；前端只按状态展示入口，真正的校验仍在服务端。
    function actionsFor(state) {
      var s = String(state || "");
      var actions = [];
      if (["draft", "queued", "needs_user_input", "interrupted"].indexOf(s) >= 0) actions.push({ id: "start", label: "开始执行", primary: true });
      if (s === "paused") actions.push({ id: "resume", label: "继续执行" });
      if (["running", "awaiting_approval", "verifying", "repairing", "planning", "clarifying"].indexOf(s) >= 0) actions.push({ id: "pause", label: "暂停" });
      if (["running", "verifying", "repairing", "paused"].indexOf(s) >= 0) actions.push({ id: "verify-open", label: "验证" });
      if (!isTerminal(s)) actions.push({ id: "cancel", label: "取消", danger: true });
      return actions;
    }

    // ---------------------------------------------------------------- 渲染

    function shell() {
      var options = ['<option value="">全部状态</option>'].concat(STATE_ORDER.map(function (state) {
        return '<option value="' + esc(state) + '">' + esc(stateInfo(state).label) + "</option>";
      })).join("");
      var strategies = Object.keys(STRATEGY_LABEL).map(function (key) {
        return '<option value="' + esc(key) + '">' + esc(STRATEGY_LABEL[key]) + "</option>";
      }).join("");
      return '<section data-panel="agent-tasks" data-layout="custom" class="owo-at-panel">' +
        '<div class="owo-at-toolbar"><div class="owo-at-filters">' +
        '<select class="owo-at-state" aria-label="按任务状态筛选">' + options + "</select>" +
        '<button type="button" class="secondary owo-at-refresh" data-at="refresh">刷新</button></div>' +
        '<button type="button" class="primary" data-at="toggle-create">新建任务</button></div>' +
        '<form class="owo-at-create" hidden>' +
        '<label class="owo-at-field"><span>目标</span><textarea class="owo-at-goal" rows="3" placeholder="描述要完成的工作，例如：修复登录按钮并验证（目标不能为空）"></textarea></label>' +
        '<div class="owo-at-create-row"><label class="owo-at-field"><span>标题</span><input class="owo-at-title" placeholder="可选，默认取目标首行"></label>' +
        '<label class="owo-at-field"><span>执行方式</span><select class="owo-at-strategy">' + strategies + "</select></label>" +
        '<button type="submit" class="primary">创建任务</button></div></form>' +
        '<div class="owo-at-notice" role="status" aria-live="polite" hidden></div>' +
        '<div class="owo-at-meta sub" role="status" aria-live="polite">加载中…</div>' +
        '<div class="owo-at-list"></div>' +
        '<div class="owo-at-detail" hidden></div></section>';
    }

    function taskCard(task) {
      var title = task.title || firstLine(task.user_goal) || "未命名任务";
      var attention = needsAttention(task) ? " is-attention" : "";
      return '<button type="button" class="owo-at-row' + attention + '" data-at="open" data-task="' + esc(task.task_id) + '">' +
        '<span class="owo-at-row-main"><strong>' + esc(title) + "</strong>" +
        '<span class="sub">' + esc(strategyLabel(task.execution_strategy)) +
        " · 第 " + esc(task.current_attempt || 0) + " 次尝试 · " + esc(formatTime(task.updated_at)) + "</span></span>" +
        stateBadge(task.state) + "</button>";
    }

    function listHtml(data, filter) {
      if (!data || !Array.isArray(data.tasks)) throw new Error("任务列表响应不完整");
      if (data.tasks.some(function (task) { return !task || typeof task !== "object"; })) throw new Error("任务列表包含无效条目");
      tasks = data.tasks;
      if (!tasks.length) {
        return '<div class="owo-at-empty">' + (filter
          ? "没有处于「" + esc(stateInfo(filter).label) + "」的任务"
          : "还没有任务。新建任务后，它会进入统一状态机并留下事件记录。") + "</div>";
      }
      return tasks.map(taskCard).join("");
    }

    function metaHtml(data) {
      var count = data && Array.isArray(data.tasks) ? data.tasks.length : tasks.length;
      var attention = (data && Array.isArray(data.tasks) ? data.tasks : tasks).filter(needsAttention).length;
      return "共 " + count + " 个任务" + (attention ? " · " + attention + " 个需要处理" : "") +
        " · 状态由服务端状态机校验";
    }

    function actionBar(state, busy) {
      var actions = actionsFor(state);
      if (!actions.length) return '<p class="sub">任务已处于终态，不能再执行动作。</p>';
      return actions.map(function (action) {
        var cls = action.danger ? "danger" : action.primary ? "primary" : "secondary";
        return '<button type="button" class="' + cls + '" data-at="' + esc(action.id) + '"' + (busy ? " disabled" : "") + ">" + esc(action.label) + "</button>";
      }).join("");
    }

    function approvalHtml(approvals) {
      if (!Array.isArray(approvals) || !approvals.length) return '<p class="sub">没有审批记录。</p>';
      return approvals.map(function (approval) {
        var pending = !approval.decision;
        var decision = approval.decision === "approved" ? "已批准" : approval.decision === "rejected" ? "已拒绝" : "待决定";
        return '<div class="owo-at-item"><div><strong>' + esc(approval.action || "操作") + "</strong> " +
          '<span class="sub">' + esc(decision) + " · 请求于 " + esc(formatTime(approval.requested_at)) + "</span></div>" +
          (pending
            ? '<div class="owo-at-item-actions"><button type="button" class="allow" data-at="approve" data-approval="' + esc(approval.approval_id) + '">批准</button>' +
              '<button type="button" class="deny" data-at="reject" data-approval="' + esc(approval.approval_id) + '">拒绝</button></div>'
            : "") + "</div>";
      }).join("");
    }

    function checkpointHtml(checkpoint) {
      if (!checkpoint) return '<p class="sub">还没有检查点。检查点用于进程重启后的恢复。</p>';
      return '<div class="owo-at-item"><div><strong>' + esc(checkpoint.phase || "检查点") + "</strong>" +
        '<span class="sub"> · 第 ' + esc(checkpoint.attempt || 0) + " 次尝试 · " + esc(formatTime(checkpoint.created_at)) + "</span></div>" +
        '<code class="hint">' + esc(checkpoint.state_snapshot_ref || "") + "</code></div>";
    }

    function artifactBadge(status) {
      var map = {
        pending: ["待验证", "wait"],
        verified: ["已验证", "ok"],
        failed: ["验证失败", "bad"],
        not_applicable: ["不适用", "idle"],
      };
      var hit = map[String(status || "")] || [String(status || "—"), "idle"];
      return '<span class="owo-at-badge owo-at-tone-' + hit[1] + '">' + esc(hit[0]) + "</span>";
    }

    function artifactsHtml(artifacts) {
      if (!Array.isArray(artifacts) || !artifacts.length) return '<p class="sub">还没有产物。产物只登记引用与校验值。</p>';
      return artifacts.map(function (artifact) {
        return '<div class="owo-at-item"><div><strong>' + esc(artifact.kind || "产物") + "</strong> " +
          artifactBadge(artifact.verification_status) +
          '<span class="sub"> · ' + esc(formatTime(artifact.created_at)) + "</span></div>" +
          '<code class="hint">' + esc(artifact.uri_or_path || "") + "</code></div>";
      }).join("");
    }

    function eventSummary(payload) {
      var text;
      try {
        text = JSON.stringify(payload == null ? {} : payload);
      } catch (_) {
        text = "[无法序列化的事件载荷]";
      }
      text = String(text || "");
      return text.length > 180 ? text.slice(0, 180) + "…" : text;
    }

    function eventsHtml(data) {
      var events = data && Array.isArray(data.events) ? data.events : [];
      if (!events.length) return '<p class="sub">还没有事件。</p>';
      return events.map(function (event) {
        return '<div class="owo-at-event"><span class="owo-at-seq">#' + esc(event.sequence) + "</span>" +
          '<span class="owo-at-event-type">' + esc(event.event_type) + "</span>" +
          '<span class="sub">' + esc(formatTime(event.occurred_at)) + "</span>" +
          '<code class="hint">' + esc(eventSummary(event.payload)) + "</code></div>";
      }).join("");
    }

    /// 执行环境的诚实标注：CloudRemote 只是「远端命令执行」，模拟传输必须写「模拟执行」，
    /// 不能让界面看起来像一台完整的远端 Agent。
    function transportOf(events) {
      var list = events && Array.isArray(events.events) ? events.events : [];
      for (var i = list.length - 1; i >= 0; i -= 1) {
        var payload = list[i] && list[i].payload;
        if (payload && typeof payload === "object" && payload.transport) return String(payload.transport);
      }
      return "";
    }
    function environmentBadge(task, events) {
      if (String(task && task.execution_strategy || "") !== "cloud_remote") return "";
      var transport = transportOf(events);
      if (transport === "mock") {
        return '<span class="owo-at-badge owo-at-tone-wait">模拟执行（本地隔离副本，不联网）</span>';
      }
      return '<span class="owo-at-badge owo-at-tone-idle">远端命令执行 · 不是完整远端 Agent</span>';
    }

    var WS_STATE_META = {
      prepared: ["就绪", "run"],
      applied: ["已应用", "ok"],
      rejected: ["已拒绝", "bad"],
      released: ["已释放", "idle"],
    };
    function wsStateBadge(state) {
      var hit = WS_STATE_META[String(state || "")] || [String(state || "—"), "idle"];
      return '<span class="owo-at-badge owo-at-tone-' + hit[1] + '">' + esc(hit[0]) + "</span>";
    }
    function workspaceFilesHtml(files) {
      if (!Array.isArray(files) || !files.length) return "";
      return '<div class="owo-at-ws-files">' + files.map(function (file) {
        var counts = "";
        if (typeof file.insertions === "number" || typeof file.deletions === "number") {
          counts = " +" + (file.insertions || 0) + " / -" + (file.deletions || 0);
        }
        return '<div class="owo-at-item"><div><strong>' + esc(file.path || "") + "</strong>" +
          '<span class="sub">' + esc(file.status || "") + counts + "</span></div></div>";
      }).join("") + "</div>";
    }
    function truncatePatch(patch) {
      var text = String(patch || "");
      return text.length > 120000 ? text.slice(0, 120000) + "\n…（补丁过长，已截断显示；应用时仍使用完整补丁）" : text;
    }
    function workspaceHtml(lease, diff, error) {
      if (error) {
        return '<p class="sub">工作区读取失败：' + esc(error.message || error) + "。</p>";
      }
      if (!lease) {
        return '<p class="sub">没有找到该任务的工作区租约（可能已释放，或任务没有在隔离目录里执行）。</p>';
      }
      var actions = [];
      var empty = !diff || diff.empty;
      if (lease.kind === "current_workspace") {
        actions.push('<span class="sub">当前工作区只读审阅：不能在这里应用、拒绝或回滚。</span>');
      } else if (lease.state === "prepared") {
        if (!empty) {
          actions.push('<button type="button" class="primary" data-at="ws-apply" data-workspace="' + esc(lease.workspace_id) + '">应用改动</button>');
          actions.push('<button type="button" class="danger" data-at="ws-reject" data-workspace="' + esc(lease.workspace_id) + '">' +
            (wsRejectArmed === lease.workspace_id ? "确认拒绝并删除工作区" : "拒绝改动") + "</button>");
        }
        actions.push('<button type="button" class="secondary" data-at="ws-release" data-workspace="' + esc(lease.workspace_id) + '">释放工作区</button>');
      } else if (lease.state === "applied") {
        actions.push('<button type="button" class="danger" data-at="ws-rollback" data-workspace="' + esc(lease.workspace_id) + '">回滚改动</button>');
        actions.push('<button type="button" class="secondary" data-at="ws-release" data-workspace="' + esc(lease.workspace_id) + '">释放工作区</button>');
      }
      return '<div class="owo-at-ws-head">' + wsStateBadge(lease.state) +
        '<span class="sub">基线 ' + esc(String(lease.base_commit || "").slice(0, 10) || "—") + " · " + esc(lease.path) + "</span></div>" +
        (actions.length ? '<div class="owo-at-item-actions">' + actions.join("") + "</div>" : "") +
        (empty
          ? '<p class="sub">工作区还没有改动。</p>'
          : workspaceFilesHtml(diff.diff.files) + '<pre class="owo-at-diff">' + esc(truncatePatch(diff.diff.patch)) + "</pre>");
    }

    function detailHtml(detail, events, busy) {
      if (!detail || !detail.task || typeof detail.task !== "object") throw new Error("任务详情响应不完整");
      var task = detail.task;
      selectedState = String(task.state || "");
      var hint = resumeHint(task);
      var meta = "";
      if (task.workspace_ref) meta += "<span>工作区：" + esc(task.workspace_ref) + "</span>";
      meta += "<span>策略：" + esc(strategyLabel(task.execution_strategy)) + "</span>" + environmentBadge(task, events);
      meta += "<span>尝试：" + esc(task.current_attempt || 0) + " / " + esc(task.budget && task.budget.max_attempts || 0) + "</span>";
      meta += "<span>创建：" + esc(formatTime(task.created_at)) + "</span>";
      meta += "<span>更新：" + esc(formatTime(task.updated_at)) + "</span>";
      return '<div class="owo-at-detail-head"><button type="button" class="secondary" data-at="back">返回列表</button>' +
        "<h2>" + esc(task.title || "任务详情") + "</h2>" + stateBadge(task.state) + "</div>" +
        '<p class="owo-at-goal-text">' + esc(task.user_goal || "") + "</p>" +
        (task.terminal_summary ? '<p class="owo-at-summary">' + esc(task.terminal_summary) + "</p>" : "") +
        '<div class="owo-at-meta-line sub">' + meta + "</div>" +
        '<div class="owo-at-actions">' + actionBar(task.state, busy) + "</div>" +
        (hint ? '<p class="owo-at-hint">' + esc(hint) + "</p>" : "") +
        '<div class="owo-at-verify" hidden><label class="owo-at-field"><span>验证说明</span>' +
        '<input class="owo-at-verify-detail" placeholder="例如：登录按钮已出现且可点击"></label>' +
        '<div class="owo-at-item-actions"><button type="button" class="allow" data-at="verify-pass">标记验证通过</button>' +
        '<button type="button" class="deny" data-at="verify-fail">标记验证失败</button></div>' +
        '<p class="sub">只有目标验证通过才会进入「已完成」；失败会进入有界修复或等待用户处理。</p></div>' +
        '<details class="owo-at-section" open><summary>审批</summary>' + approvalHtml(detail.approvals) + "</details>" +
        '<details class="owo-at-section"><summary>检查点</summary>' + checkpointHtml(detail.latest_checkpoint) + "</details>" +
        '<details class="owo-at-section"><summary>产物</summary>' + artifactsHtml(detail.artifacts) + "</details>" +
        (task.workspace_ref
          ? '<details class="owo-at-section" open><summary>工作区改动 <span class="sub owo-at-ws-state">读取中…</span></summary>' +
            '<div class="owo-at-workspace"><p class="sub">正在读取工作区租约…</p></div></details>'
          : "") +
        '<details class="owo-at-section"><summary>事件 <span class="sub owo-at-live' +
        (isTerminal(task.state) ? " is-closed" : "") + '">' +
        (isTerminal(task.state) ? "已结束" : "实时") + "</span></summary>" +
        '<div class="owo-at-event-log">' + eventsHtml(events) + "</div></details>";
    }

    // ------------------------------------------------------------ 数据加载

    function isCurrent(root, lifecycle, request) {
      return root === rootEl && lifecycle === lifecycleGeneration &&
        request === refreshGeneration &&
        (typeof root.isConnected !== "boolean" || root.isConnected);
    }

    function showNotice(root, tone, text) {
      var box = root.querySelector(".owo-at-notice");
      if (!box) return;
      box.hidden = !text;
      box.className = "owo-at-notice" + (tone ? " is-" + tone : "");
      box.textContent = text || "";
    }

    function setBusy(root, busy) {
      rootEl.dataset && (rootEl.dataset.atBusy = busy ? "1" : "");
      var actions = root.querySelector(".owo-at-actions");
      if (!actions) return;
      Array.prototype.forEach.call(root.querySelectorAll ? root.querySelectorAll(".owo-at-actions button") : [], function (button) {
        button.disabled = Boolean(busy);
      });
    }

    function paintList(root, data) {
      var list = root.querySelector(".owo-at-list");
      var meta = root.querySelector(".owo-at-meta");
      var html = listHtml(data, filterValue(root));
      if (list) list.innerHTML = html;
      if (meta) meta.textContent = metaHtml(data);
      var detail = root.querySelector(".owo-at-detail");
      if (detail) detail.hidden = true;
      var listBox = root.querySelector(".owo-at-list");
      if (listBox) listBox.hidden = false;
    }

    function filterValue(root) {
      var select = root.querySelector(".owo-at-state");
      return String(select && select.value || "");
    }

    function refresh(root) {
      root = root || rootEl;
      if (!root || !apiClient || (rootEl && root !== rootEl)) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      var request = ++refreshGeneration;
      var meta = root.querySelector(".owo-at-meta");
      var filter = filterValue(root);
      if (meta) meta.textContent = hasSnapshot ? "正在刷新任务…（保留当前结果）" : "加载中…";
      return apiClient.list({ state: filter || undefined })
        .then(function (data) {
          if (!isCurrent(root, lifecycle, request)) return;
          paintList(root, data);
          hasSnapshot = true;
        })
        .catch(function (error) {
          if (!isCurrent(root, lifecycle, request)) return;
          if (meta) meta.textContent = hasSnapshot ? "刷新失败，已保留上次读取结果" : "任务列表加载失败，可重试";
          showNotice(root, "error", "任务列表读取失败：" + friendly(error));
        });
    }

    function openTask(root, id, preserveDetail) {
      root = root || rootEl;
      if (!root || !id || !apiClient) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      var request = ++refreshGeneration;
      if (selectedId !== id) liveStateCursor = 0;
      selectedId = id;
      showNotice(root, "", "");
      var detail = root.querySelector(".owo-at-detail");
      var list = root.querySelector(".owo-at-list");
      if (detail) {
        detail.hidden = false;
        if (!preserveDetail) detail.innerHTML = '<p class="sub">正在读取任务详情…</p>';
      }
      if (list) list.hidden = true;
      if (!preserveDetail) {
        stopLive();
        liveReloadTaskId = "";
      }
      wsRejectArmed = "";
      return Promise.all([
        apiClient.get(id),
        apiClient.events(id, { limit: 100 }),
      ]).then(function (results) {
        if (!isCurrent(root, lifecycle, request)) return;
        if (detail) detail.innerHTML = detailHtml(results[0], results[1], false);
        // 工作区租约与 Diff 只在任务绑定 workspace_ref 时读取；失败只影响这一块。
        if (results[0].task.workspace_ref) loadWorkspace(root, results[0].task);
        // 终态任务没有后续事件：不再订阅，避免每次打开都重建一条立刻结束的流。
        if (isTerminal(selectedState)) stopLive();
        else startLive(root, id);
      }).catch(function (error) {
        if (!isCurrent(root, lifecycle, request)) return;
        if (preserveDetail) {
          liveStateCursor = 0;
          // A detail request can fail while the event stream stays open.
          // Retry even if no further stream notification arrives.
          setTimeout(function () {
            if (isCurrent(root, lifecycle, request) && selectedId === id) {
              openTask(root, id, true);
            }
          }, 3000);
          return;
        }
        if (detail) detail.innerHTML = '<p class="sub">任务详情读取失败。</p>';
        showNotice(root, "error", "任务详情读取失败：" + friendly(error));
      });
    }

    function reloadDetail(root) {
      return selectedId ? openTask(root, selectedId) : refresh(root);
    }

    /// 事件流的展示状态：连接中 / 实时 / 重连中 / 已结束，不让用户以为断线后界面仍是最新。
    function liveLabel(state) {
      var map = { connecting: "连接中", live: "实时", degraded: "重连中", closed: "已结束" };
      return map[String(state || "")] || "实时";
    }

    function paintLiveEvents(root, snapshot) {
      if (!root || root !== rootEl || !snapshot || snapshot.taskId !== selectedId) return;
      var log = root.querySelector(".owo-at-event-log");
      if (log) log.innerHTML = eventsHtml({ events: snapshot.events });
      var badge = root.querySelector(".owo-at-live");
      if (badge) {
        badge.textContent = liveLabel(snapshot.state);
        badge.className = "sub owo-at-live is-" + String(snapshot.state || "idle");
      }
    }

    function onLiveChange(root, snapshot) {
      paintLiveEvents(root, snapshot);
      if (!root || root !== rootEl || !snapshot || snapshot.taskId !== selectedId) return;
      if (snapshot.state !== "live" && snapshot.state !== "closed") return;
      // State transition events carry both states. Fetch the authoritative
      // detail once for each new transition, including non-terminal states.
      var transition = snapshot.events.findLast(function (event) {
        var payload = event.payload;
        return payload && Object.hasOwn(STATE_META, payload.from) && Object.hasOwn(STATE_META, payload.to);
      });
      if (transition && transition.sequence > liveStateCursor) {
        liveStateCursor = transition.sequence;
        if (transition.payload.to !== selectedState) {
          openTask(root, selectedId, true);
          return;
        }
      }
      // 终态帧到达：事件流不会再增长，重读一次详情把终态、摘要与产物补全。
      // 只在任务尚未显示终态时读一次，避免「读完 → 又结束」的循环。
      if (snapshot.state === "closed" && liveReloadTaskId !== snapshot.taskId && !isTerminal(selectedState)) {
        liveReloadTaskId = snapshot.taskId;
        openTask(root, selectedId, true);
      }
    }

    function startLive(root, id) {
      if (!eventStore) return;
      stopLive();
      eventStore.start(id);
    }

    function stopLive() {
      if (eventStore) eventStore.stop();
    }

    /// 租约按目录或任务关联；两者都匹配不上时明确显示"没有租约"，不猜。
    function findLease(leases, task) {
      var path = String(task && task.workspace_ref || "");
      var taskId = String(task && task.task_id || "");
      var list = Array.isArray(leases) ? leases : [];
      for (var i = 0; i < list.length; i += 1) {
        if (path && String(list[i].path || "") === path) return list[i];
      }
      for (var j = 0; j < list.length; j += 1) {
        if (taskId && String(list[j].task_id || "") === taskId) return list[j];
      }
      return null;
    }

    function paintWorkspaceState(root, lease) {
      var badge = root.querySelector(".owo-at-ws-state");
      if (!badge) return;
      var hit = WS_STATE_META[String(lease && lease.state || "")];
      badge.textContent = hit ? hit[0] : "无租约";
    }

    function loadWorkspace(root, task) {
      var box = root.querySelector(".owo-at-workspace");
      if (!box || !workspaceClient || !task || !task.workspace_ref) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      return workspaceClient.list().then(function (data) {
        var lease = findLease(data.workspaces, task);
        if (!lease) return { lease: null, diff: null };
        return workspaceClient.diff(lease.workspace_id).then(function (diff) {
          return { lease: lease, diff: diff };
        });
      }).then(function (result) {
        if (root !== rootEl || lifecycle !== lifecycleGeneration) return;
        var target = root.querySelector(".owo-at-workspace");
        if (!target) return;
        target.innerHTML = workspaceHtml(result.lease, result.diff, null);
        paintWorkspaceState(root, result.lease);
      }).catch(function (error) {
        if (root !== rootEl || lifecycle !== lifecycleGeneration) return;
        var target = root.querySelector(".owo-at-workspace");
        if (target) target.innerHTML = workspaceHtml(null, null, error);
      });
    }

    function runWorkspaceAction(root, workspaceId, action) {
      if (!workspaceClient || !workspaceId) return Promise.resolve();
      var labels = { apply: "已应用工作区改动", reject: "已拒绝并删除工作区", rollback: "已回滚工作区改动", release: "已释放工作区" };
      var run;
      if (action === "apply") run = workspaceClient.apply(workspaceId);
      else if (action === "reject") run = workspaceClient.reject(workspaceId);
      else if (action === "rollback") run = workspaceClient.rollback(workspaceId);
      else run = workspaceClient.release(workspaceId);
      setBusy(root, true);
      return run.then(function () {
        if (root !== rootEl) return;
        wsRejectArmed = "";
        setBusy(root, false);
        return reloadDetail(root).then(function () {
          showNotice(root, "ok", labels[action] || "工作区操作已完成");
        });
      }).catch(function (error) {
        if (root !== rootEl) return;
        setBusy(root, false);
        showNotice(root, "error", "工作区操作未执行：" + friendly(error));
      });
    }

    function runAction(root, action) {
      if (!selectedId) return Promise.resolve();
      var run;
      if (action === "start") run = apiClient.start(selectedId);
      else if (action === "pause") run = apiClient.pause(selectedId);
      else if (action === "resume") run = apiClient.resume(selectedId);
      else run = apiClient.cancel(selectedId);
      setBusy(root, true);
      return run.then(function (result) {
        if (root !== rootEl) return;
        setBusy(root, false);
        var noticeTone = "";
        var noticeText = "";
        if (result && result.dispatched === false) {
          noticeTone = "warn";
          noticeText = (result.message || "执行后端尚未注册") + "（code: " + (result.code || "UNSUPPORTED") + "）";
        }
        return reloadDetail(root).then(function () {
          if (noticeText) showNotice(root, noticeTone, noticeText);
        });
      }).catch(function (error) {
        if (root !== rootEl) return;
        setBusy(root, false);
        showNotice(root, "error", "动作未执行：" + friendly(error));
      });
    }

    function submitVerify(root, passed) {
      if (!selectedId) return Promise.resolve();
      var input = root.querySelector(".owo-at-verify-detail");
      var detail = String(input && input.value || "").trim() ||
        (passed ? "人工确认：目标已达成" : "人工确认：目标未达成");
      setBusy(root, true);
      return apiClient.verify(selectedId, {
        goal: { passed: Boolean(passed), detail: detail },
      }).then(function (result) {
        if (root !== rootEl) return;
        setBusy(root, false);
        var report = result && result.report;
        var summary = report && report.summary ? report.summary : "验证已提交";
        return reloadDetail(root).then(function () {
          showNotice(root, report && report.verified ? "ok" : "warn", summary);
        });
      }).catch(function (error) {
        if (root !== rootEl) return;
        setBusy(root, false);
        showNotice(root, "error", "验证未提交：" + friendly(error));
      });
    }

    function decideApproval(root, approvalId, decision) {
      if (!selectedId || !approvalId) return Promise.resolve();
      setBusy(root, true);
      return apiClient.approve(selectedId, approvalId, decision).then(function () {
        if (root !== rootEl) return;
        setBusy(root, false);
        return reloadDetail(root);
      }).catch(function (error) {
        if (root !== rootEl) return;
        setBusy(root, false);
        showNotice(root, "error", "审批未提交：" + friendly(error));
      });
    }

    function createTask(root) {
      var goalEl = root.querySelector(".owo-at-goal");
      var titleEl = root.querySelector(".owo-at-title");
      var strategyEl = root.querySelector(".owo-at-strategy");
      var goal = String(goalEl && goalEl.value || "").trim();
      if (!goal) {
        showNotice(root, "error", "请先填写目标：空目标会被服务端拒绝。");
        return Promise.resolve();
      }
      var body = { user_goal: goal };
      var title = String(titleEl && titleEl.value || "").trim();
      if (title) body.title = title;
      var strategy = String(strategyEl && strategyEl.value || "");
      if (strategy) body.execution_strategy = strategy;
      setBusy(root, true);
      return apiClient.create(body).then(function () {
        if (root !== rootEl) return;
        setBusy(root, false);
        if (goalEl) goalEl.value = "";
        if (titleEl) titleEl.value = "";
        var form = root.querySelector(".owo-at-create");
        if (form) form.hidden = true;
        selectedId = null;
        return refresh(root).then(function () {
          showNotice(root, "ok", "任务已创建并进入排队状态。");
        });
      }).catch(function (error) {
        if (root !== rootEl) return;
        setBusy(root, false);
        showNotice(root, "error", "任务未创建：" + friendly(error));
      });
    }

    // --------------------------------------------------------------- 事件

    function handleClick(root, event) {
      var target = event && event.target && event.target.closest ? event.target.closest("[data-at]") : null;
      if (!target) return;
      var action = target.getAttribute ? target.getAttribute("data-at") : target.dataset.at;
      if (!action) return;
      if (root !== rootEl) return;
      var taskId = target.getAttribute ? target.getAttribute("data-task") : target.dataset.task;
      var approvalId = target.getAttribute ? target.getAttribute("data-approval") : target.dataset.approval;
      var workspaceId = target.getAttribute ? target.getAttribute("data-workspace") : target.dataset.workspace;
      switch (action) {
        case "refresh":
          reloadDetail(root);
          return;
        case "toggle-create": {
          var form = root.querySelector(".owo-at-create");
          if (form) form.hidden = !form.hidden;
          return;
        }
        case "open":
          openTask(root, taskId);
          return;
        case "back":
          selectedId = null;
          liveReloadTaskId = "";
          stopLive();
          var detail = root.querySelector(".owo-at-detail");
          if (detail) detail.hidden = true;
          var list = root.querySelector(".owo-at-list");
          if (list) list.hidden = false;
          refresh(root);
          return;
        case "verify-open": {
          var verify = root.querySelector(".owo-at-verify");
          if (verify) verify.hidden = !verify.hidden;
          return;
        }
        case "verify-pass":
          submitVerify(root, true);
          return;
        case "verify-fail":
          submitVerify(root, false);
          return;
        case "approve":
          decideApproval(root, approvalId, "approved");
          return;
        case "reject":
          decideApproval(root, approvalId, "rejected");
          return;
        case "start":
        case "pause":
        case "resume":
        case "cancel":
          runAction(root, action);
          return;
        case "ws-apply":
          wsRejectArmed = "";
          runWorkspaceAction(root, workspaceId, "apply");
          return;
        case "ws-rollback":
          runWorkspaceAction(root, workspaceId, "rollback");
          return;
        case "ws-release":
          runWorkspaceAction(root, workspaceId, "release");
          return;
        case "ws-reject":
          // 第一次点击只进入确认态；第二次才真正回滚并删除 worktree。
          if (wsRejectArmed !== workspaceId) {
            wsRejectArmed = workspaceId;
            if (selectedId) reloadDetail(root);
            return;
          }
          runWorkspaceAction(root, workspaceId, "reject");
          return;
        default:
          return;
      }
    }

    function handleSubmit(root, event) {
      if (!event) return;
      var target = event.target;
      if (!target || !target.classList || !target.classList.contains("owo-at-create")) return;
      if (typeof event.preventDefault === "function") event.preventDefault();
      if (root !== rootEl) return;
      createTask(root);
    }

    function handleChange(root, event) {
      var target = event && event.target;
      if (!target || !target.classList) return;
      if (target.classList.contains("owo-at-state")) refresh(root);
    }

    function mount(root, helpers) {
      dispose();
      H = {
        get: (helpers && helpers.get) || defaultGet,
        post: helpers && helpers.post,
        del: helpers && (helpers.del || helpers.delete),
        esc: (helpers && helpers.esc) || defaultEsc,
        friendlyError: (helpers && helpers.friendlyError) || defaultFriendlyError,
        openStream: helpers && helpers.openStream,
      };
      if (!root) return;
      rootEl = root;
      tasks = [];
      selectedId = null;
      hasSnapshot = false;
      liveReloadTaskId = "";
      // 所有任务读写统一走 TaskApiClient；面板不再自己拼 /agent-tasks 路径。
      apiClient = win.OwoTaskApi && typeof win.OwoTaskApi.createTaskApiClient === "function"
        ? win.OwoTaskApi.createTaskApiClient({
            get: function (path) { return H.get(path); },
            post: function (path, body) { return post(path, body); },
          })
        : null;
      workspaceClient = win.OwoWorkspaceApi && typeof win.OwoWorkspaceApi.createWorkspaceApiClient === "function"
        ? win.OwoWorkspaceApi.createWorkspaceApiClient({
            get: function (path) { return H.get(path); },
            post: function (path, body) { return post(path, body); },
            del: function (path) { return del(path); },
          })
        : null;
      var openStream = H.openStream ||
        (win.OwoApi && typeof win.OwoApi.openEventStream === "function"
          ? function (path, options) { return win.OwoApi.openEventStream(path, options); }
          : null);
      eventStore = apiClient && openStream && win.OwoTaskEvents &&
        typeof win.OwoTaskEvents.createTaskEventStore === "function"
        ? win.OwoTaskEvents.createTaskEventStore({
            client: apiClient,
            openStream: openStream,
            onChange: function (snapshot) { onLiveChange(root, snapshot); },
          })
        : null;
      root.innerHTML = shell();
      if (!apiClient) {
        var missing = root.querySelector(".owo-at-meta");
        if (missing) missing.textContent = "任务客户端未加载";
        showNotice(root, "error", "任务客户端未加载：需要先加载 core/task-api-client.js。");
        return;
      }
      var lifecycle = lifecycleGeneration;
      root.addEventListener("click", function (event) {
        if (lifecycle !== lifecycleGeneration) return;
        handleClick(root, event);
      });
      root.addEventListener("submit", function (event) {
        if (lifecycle !== lifecycleGeneration) return;
        handleSubmit(root, event);
      });
      root.addEventListener("change", function (event) {
        if (lifecycle !== lifecycleGeneration) return;
        handleChange(root, event);
      });
      refresh(root);
    }

    function dispose() {
      lifecycleGeneration += 1;
      refreshGeneration += 1;
      stopLive();
      eventStore = null;
      apiClient = null;
      workspaceClient = null;
      liveReloadTaskId = "";
      wsRejectArmed = "";
      rootEl = null;
      tasks = [];
      selectedId = null;
      hasSnapshot = false;
    }

    var TEST_API = {
      stateInfo: stateInfo,
      stateBadge: stateBadge,
      strategyLabel: strategyLabel,
      actionsFor: actionsFor,
      resumeHint: resumeHint,
      taskCard: taskCard,
      listHtml: listHtml,
      detailHtml: detailHtml,
      eventsHtml: eventsHtml,
      workspaceHtml: workspaceHtml,
      environmentBadge: environmentBadge,
      findLease: findLease,
      formatTime: formatTime,
    };

    return {
      id: ID,
      title: "任务运行",
      nav: shell,
      mount: mount,
      open: function (id) {
        if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id || "")) {
          return Promise.reject(new Error("任务编号无效"));
        }
        if (!rootEl || !apiClient) return Promise.reject(new Error("任务面板尚未就绪"));
        return openTask(rootEl, id);
      },
      refresh: refresh,
      dispose: dispose,
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效。
if (typeof module !== "undefined" && module.exports) {
  var __agentTasksWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __agentTasksWin.OwoPanels["agent-tasks"];
}
