/* 工作流面板（Lane C）：.owflow 列表 / 定义预览 / validate / 运行 / runs / 步骤时间线 / abort / audit。
 * 纯脚本 IIFE，依赖 window.OwoPanels.<lane> 契约与 helpers（防御性降级）。 */
(function () {
  "use strict";

  window.OwoPanels = window.OwoPanels || {};

  var BASE = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
  var self = null; // 面板实例（模块级单例）
  var workflowGeneration = 0;
  var workflowPollTimers = new Set();
  var flowRequestGeneration = 0;
  var runsRequestGeneration = 0;
  var auditRequestGeneration = 0;
  var validationRequestGeneration = 0;
  var validationPending = false;
  var workflowStartPending = new Set();
  var workflowRunActionPending = Object.create(null);
  var workflowRunSnapshots = Object.create(null);

  function stopWorkflowRuntime() {
    workflowGeneration += 1;
    flowRequestGeneration += 1;
    runsRequestGeneration += 1;
    auditRequestGeneration += 1;
    validationRequestGeneration += 1;
    validationPending = false;
    workflowStartPending = new Set();
    workflowRunActionPending = Object.create(null);
    workflowRunSnapshots = Object.create(null);
    workflowPollTimers.forEach(function (timer) { clearTimeout(timer); });
    workflowPollTimers.clear();
    if (window.OwoWorkflowEventSource) {
      window.OwoWorkflowEventSource.close();
      window.OwoWorkflowEventSource = null;
    }
  }

  function schedulePoll(runId, delayMs, generation, failures) {
    var timer = setTimeout(function () {
      workflowPollTimers.delete(timer);
      return pollRun(runId, 0, generation, failures);
    }, delayMs);
    workflowPollTimers.add(timer);
  }

  function getHelpers() {
    return (self && self.helpers) || {};
  }

  function esc(s) {
    var h = getHelpers().esc;
    if (h) { return h(s); }
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  }

  function friendlyError(e) {
    var h = getHelpers().friendlyError;
    if (h) { return h(e); }
    if (e && e.error) { return String(e.error); }
    return String((e && e.message) || e || "请求失败");
  }

  function get(path) {
    var h = getHelpers();
    if (h && h.get) { return h.get(path); }
    return window.OwoApi.stream(path).then(function (r) {
      if (!r.ok) { return r.json().then(function (j) { throw j; }); }
      return r.json();
    });
  }

  function post(path, body) {
    var h = getHelpers();
    if (h && h.post) { return h.post(path, body); }
    return window.OwoApi.stream(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body == null ? {} : body),
    }).then(function (r) {
      if (!r.ok) { return r.json().then(function (j) { throw j; }); }
      return r.json();
    });
  }

  function toast(text, kind) {
    var t = window.showToast;
    if (t) { t(text, kind || ""); }
  }

  function style() {
    return (
      "<style>" +
      ".owo-workflow-section{margin-bottom:14px}" +
      ".owo-workflow-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px}" +
      ".owo-workflow-card{border:1px solid var(--border-strong);border-radius:8px;padding:10px;margin-bottom:8px}" +
      ".owo-workflow-card h4{margin:0 0 6px 0}" +
      ".owo-workflow-name{font-weight:600}" +
      ".owo-workflow-badge{display:inline-block;padding:1px 8px;border-radius:10px;font-size:12px}" +
      ".owo-workflow-badge-ok{background:var(--green-soft);color:var(--green)}" +
      ".owo-workflow-badge-bad{background:var(--red-soft);color:var(--red)}" +
      ".owo-workflow-badge-run{background:var(--accent-soft);color:var(--accent)}" +
      ".owo-workflow-btn{margin-right:6px}" +
      ".owo-workflow-input{width:100%;box-sizing:border-box;margin:4px 0;padding:6px;border:1px solid var(--border-strong);border-radius:6px}" +
      ".owo-workflow-audit{max-height:180px;overflow:auto;font-size:12px}" +
      /* 定义概览卡片 */
      ".owo-wf-def{border:1px solid var(--border-strong);border-radius:10px;padding:12px;background:var(--surface)}" +
      ".owo-wf-def-head{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;margin-bottom:8px}" +
      ".owo-wf-def-head strong{font-size:13.5px}" +
      ".owo-wf-chips{display:flex;flex-wrap:wrap;gap:6px;margin:8px 0}" +
      ".owo-wf-chip{display:inline-block;padding:2px 9px;border-radius:999px;background:var(--surface-2);border:1px solid var(--border);color:var(--text-2);font-size:11.5px}" +
      ".owo-wf-step{display:flex;gap:9px;align-items:baseline;padding:6px 8px;border:1px solid var(--border);border-radius:8px;background:var(--surface);margin-bottom:5px}" +
      ".owo-wf-step-idx{flex:none;width:20px;height:20px;display:grid;place-items:center;border-radius:99px;background:var(--accent-soft);color:var(--accent);font-size:12px;font-weight:700}" +
      ".owo-wf-step-kind{flex:none;font-weight:600;font-size:12px;color:var(--accent)}" +
      ".owo-wf-step-body{flex:1;min-width:0;font-size:12px;color:var(--text-2);word-break:break-word}" +
      ".owo-wf-step-body code{font-size:11.5px;background:var(--surface-2);border-radius:4px;padding:0 4px}" +
      /* 事件时间线 */
      ".owo-wf-events{max-height:220px;overflow:auto;font-size:12px}" +
      ".owo-wf-event{display:flex;gap:8px;align-items:baseline;padding:3px 0;border-bottom:1px dashed var(--border)}" +
      ".owo-wf-event-time{flex:none;color:var(--text-3);font-size:12px;font-variant-numeric:tabular-nums}" +
      ".owo-wf-event-kind{flex:none;font-weight:600;color:var(--accent)}" +
      ".owo-wf-event-detail{flex:1;min-width:0;color:var(--text-2);word-break:break-word}" +
      "</style>"
    );
  }

  function nav() {
    return (
      '<section data-panel="workflow" class="owo-workflow-root">' +
      '<h3>工作流（.owflow）</h3>' +
      '<div class="owo-workflow-grid">' +
      '<div class="owo-workflow-section">' +
      '<h4>流程列表</h4>' +
      '<div id="owo-workflow-list" class="owo-workflow-list">加载中…</div>' +
      '<button id="owo-workflow-refresh" class="primary owo-workflow-btn">刷新</button>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>运行器</h4>' +
      '<div id="owo-workflow-runner">选择左侧流程查看定义并运行</div>' +
      '</div>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>校验（内联 DSL）</h4>' +
      '<textarea id="owo-workflow-validate-dsl" class="owo-workflow-input" rows="6" placeholder="粘贴 .owflow JSON 定义…"></textarea>' +
      '<button id="owo-workflow-validate-btn" class="owo-workflow-btn" data-core-action>校验</button>' +
      '<span id="owo-workflow-validate-result" role="status" aria-live="polite"></span>' +
      '<p class="sub" data-core-action-hint>JSON 语法会在本地检查；流程结构校验需要连接并授权本地核心。</p>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>Runs</h4>' +
      '<div id="owo-workflow-runs" class="owo-empty">暂无运行记录：选中流程后点「运行」，这里会显示分步时间线</div>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>实时事件（SSE）</h4>' +
      '<div id="owo-workflow-events" class="owo-wf-events">（运行后自动订阅 /events）</div>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>审批卡</h4>' +
      '<div id="owo-workflow-approval" class="sub">等待审批的请求会显示在这里。</div>' +
      '</div>' +
      '<div class="owo-workflow-section">' +
      '<h4>审计尾部</h4>' +
      '<div id="owo-workflow-audit" class="owo-workflow-audit owo-empty">暂无审计记录</div>' +
      '</div>' +
      '</section>'
    );
  }

  function renderList(flows) {
    var el = document.getElementById("owo-workflow-list");
    if (!el) { return; }
    if (!flows || !flows.length) {
      el.innerHTML = '<div class="owo-empty">当前项目没有工作流。将 .owflow 文件放入项目目录后刷新；运行前会校验流程与权限。</div>';
      return;
    }
    el.innerHTML = flows
      .map(function (f) {
        return (
          '<div class="owo-workflow-card">' +
          '<span class="owo-workflow-name">' + esc(f.name) + '</span>' +
          '<span class="sub"> — ' + esc(f.path) + '</span><br/>' +
          '<button class="owo-workflow-btn owo-workflow-load" data-name="' + esc(f.name) + '">加载</button>' +
          '<button class="owo-workflow-btn owo-workflow-run" data-core-action data-name="' + esc(f.name) + '">运行</button>' +
          '</div>'
        );
      })
      .join("");
    el.querySelectorAll(".owo-workflow-load").forEach(function (btn) {
      btn.addEventListener("click", function () { loadFlow(btn.getAttribute("data-name")); });
    });
    el.querySelectorAll(".owo-workflow-run").forEach(function (btn) {
      btn.addEventListener("click", function () { runFlow(btn.getAttribute("data-name")); });
    });
  }

  /* 单步摘要：serde tag enum（{ "Act": { id, spec } } 形式），防御未知结构 */
  function stepSummary(kind, s) {
    s = s || {};
    var spec = s.spec || {};
    switch (kind) {
      case "Sense":
      case "Locate":
        return esc(spec.pattern || spec.target || spec.query || spec.name || brief(spec));
      case "Act":
        return (s.scope ? "[" + esc(s.scope) + "] " : "") + esc(spec.action || spec.name || brief(spec));
      case "Assert":
        return "条件 <code>" + esc(s.expr || "") + "</code>" + (s.timeout_ms ? " · 超时 " + esc(String(s.timeout_ms)) + "ms" : "");
      case "InvokeSkill":
        return "技能 <code>" + esc(s.skill || "") + "</code>" + (s.args && Object.keys(s.args).length ? " · " + esc(Object.keys(s.args).join(", ")) : "");
      case "InvokeMcp":
        return "MCP <code>" + esc((s.server || "?") + "." + (s.tool || "?")) + "</code>";
      case "HumanApprove":
        return "等待人工确认：" + esc(s.prompt || "");
      case "Notify":
        return esc(s.message || "");
      case "Subflow":
        return "子流程 <code>" + esc(s.flow || "") + "</code>";
      case "Loop":
        return "循环 " + esc(String((s.body || []).length)) + " 步" + (s.cond ? " · 条件 <code>" + esc(s.cond) + "</code>" : "");
      case "Cond":
        return "分支 <code>" + esc(s.expr || s.cond || "") + "</code>";
      case "RollbackPoint":
        return "检查点 " + esc(s.name || s.id || "");
      default:
        return esc(brief(s));
    }
  }

  function brief(v) {
    try {
      var t = JSON.stringify(v);
      return t && t !== "{}" ? t.slice(0, 60) : "";
    } catch (e) { return ""; }
  }

  /* 定义概览：名称/版本/元信息 chips + 步骤时间线（替代裸 JSON） */
  function renderDefinition(name, def, issues) {
    var d = def || {};
    var steps = Array.isArray(d.steps) ? d.steps : [];
    var chips = [];
    if (d.id) { chips.push('<span class="owo-wf-chip">id ' + esc(d.id) + "</span>"); }
    chips.push('<span class="owo-wf-chip">v' + esc(String(d.version == null ? 1 : d.version)) + "</span>");
    chips.push('<span class="owo-wf-chip">' + esc(String(steps.length)) + " 步</span>");
    chips.push('<span class="owo-wf-chip">步数上限 ' + esc(String(d.max_steps == null ? 50 : d.max_steps)) + "</span>");
    if (d.subflow_depth_limit != null) { chips.push('<span class="owo-wf-chip">子流程深度 ' + esc(String(d.subflow_depth_limit)) + "</span>"); }
    if ((d.triggers || []).length) { chips.push('<span class="owo-wf-chip">触发器 ' + esc(String(d.triggers.length)) + "</span>"); }
    if ((d.permissions || []).length) { chips.push('<span class="owo-wf-chip">权限声明 ' + esc(String(d.permissions.length)) + "</span>"); }
    if ((d.preconditions || []).length) { chips.push('<span class="owo-wf-chip">前置条件 ' + esc(String(d.preconditions.length)) + "</span>"); }
    if ((d.rollback_points || []).length) { chips.push('<span class="owo-wf-chip">回滚点 ' + esc(String(d.rollback_points.length)) + "</span>"); }
    var rows = steps
      .map(function (step, i) {
        var kind = "";
        var body = step;
        if (step && typeof step === "object" && !Array.isArray(step)) {
          var keys = Object.keys(step);
          if (keys.length === 1) { kind = keys[0]; body = step[kind]; }
        }
        if (!kind) { kind = "Step"; }
        return (
          '<div class="owo-wf-step">' +
          '<span class="owo-wf-step-idx">' + (i + 1) + "</span>" +
          '<span class="owo-wf-step-kind">' + esc(kind) + "</span>" +
          '<span class="owo-wf-step-body">' + stepSummary(kind, body) + "</span>" +
          "</div>"
        );
      })
      .join("") || '<div class="sub">（无步骤）</div>';
    var issueRows = (issues || [])
      .map(function (i) { return '<div class="owo-workflow-step owo-workflow-step-fail">' + esc(i) + "</div>"; })
      .join("");
    return (
      '<div class="owo-wf-def">' +
      '<div class="owo-wf-def-head"><strong>' + esc(name) + "</strong></div>" +
      '<div class="owo-wf-chips">' + chips.join("") + "</div>" +
      rows +
      issueRows +
      "</div>"
    );
  }

  function loadFlow(name) {
    var request = ++flowRequestGeneration;
    var owner = workflowGeneration;
    var runner = document.getElementById("owo-workflow-runner");
    if (!runner) { return Promise.resolve(); }
    runner.innerHTML = "加载 " + esc(name) + "…";
    return get("/workflow/" + encodeURIComponent(name))
      .then(function (data) {
        if (request !== flowRequestGeneration || owner !== workflowGeneration) return;
        var currentRunner = document.getElementById("owo-workflow-runner");
        if (!currentRunner) return;
        var badge = data.valid
          ? '<span class="owo-workflow-badge owo-workflow-badge-ok">valid</span>'
          : '<span class="owo-workflow-badge owo-workflow-badge-bad">invalid</span>';
        currentRunner.innerHTML =
          "<h4>" + esc(name) + " " + badge + "</h4>" +
          renderDefinition(name, data.definition, data.issues) +
          '<label>ctx（JSON 对象，可选）</label>' +
          '<input id="owo-workflow-ctx" class="owo-workflow-input" placeholder=\'{"key": "value"}\' />' +
          '<label>执行后端：</label>' +
          '<select id="owo-workflow-backend" class="owo-workflow-input">' +
          '<option value="mock" selected title="直接以 mock 沙箱运行，不执行真实桌面动作">运行（沙箱）/ mock（默认）</option>' +
          '<option value="real">real（真实后端，桌面动作需门禁）</option>' +
          "</select>" +
          '<button id="owo-workflow-run-this" class="primary" data-core-action>运行</button>';
        document.getElementById("owo-workflow-run-this").addEventListener("click", function () {
          runFlow(name, document.getElementById("owo-workflow-ctx").value);
        });
      })
      .catch(function (e) {
        if (request !== flowRequestGeneration || owner !== workflowGeneration) return;
        var currentRunner = document.getElementById("owo-workflow-runner");
        if (currentRunner) currentRunner.innerHTML = '<div class="owo-workflow-step owo-workflow-step-fail">' + esc(friendlyError(e)) + "</div>";
      });
  }
  function runFlow(name, ctxText) {
    var key = String(name || "");
    if (workflowStartPending.has(key)) {
      toast("该工作流正在启动，已避免重复提交", "info");
      return Promise.resolve(null);
    }
    var ctx = {};
    if (ctxText && ctxText.trim()) {
      try { ctx = JSON.parse(ctxText); } catch (e) { toast("ctx 不是合法 JSON：" + e.message, "error"); return Promise.resolve(null); }
      if (!ctx || typeof ctx !== "object" || Array.isArray(ctx)) {
        toast("ctx 必须是 JSON 对象", "error");
        return Promise.resolve(null);
      }
    }
    var backendEl = document.getElementById("owo-workflow-backend");
    var backend = (backendEl && backendEl.value) || "mock";
    var owner = workflowGeneration;
    workflowStartPending.add(key);
    var runButton = document.getElementById("owo-workflow-run-this");
    if (runButton) {
      runButton.disabled = true;
      runButton.setAttribute("aria-busy", "true");
      runButton.textContent = "正在启动…";
    }
    return post("/workflow/" + encodeURIComponent(key) + "/run", { ctx: ctx, backend: backend })
      .then(function (data) {
        if (owner !== workflowGeneration) return null;
        var runId = data && data.run_id;
        if (!runId) throw new Error("工作流服务未返回 run_id");
        var result = document.getElementById("owo-workflow-runner");
        if (result) {
          result.innerHTML += '<div class="sub">已启动 run：' + esc(runId) + "（backend=" + esc(backend) + "）</div>";
        }
        connectEvents(runId);
        pollRun(runId, 0, owner);
        refreshRuns(key);
        return data;
      })
      .catch(function (e) {
        if (owner === workflowGeneration) toast(friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        workflowStartPending.delete(key);
        if (owner !== workflowGeneration) return;
        var currentButton = document.getElementById("owo-workflow-run-this");
        if (currentButton) {
          currentButton.disabled = false;
          currentButton.removeAttribute("aria-busy");
          currentButton.textContent = "运行";
        }
      });
  }

  function connectEvents(runId) {
    var es = window.OwoWorkflowEventSource;
    if (es) { es.close(); }
    var el = document.getElementById("owo-workflow-events");
    if (!el) { return; }
    var h = getHelpers();
    var base = (h && h.baseUrl) || BASE;
    es = new EventSource(base + "/workflow/run/" + encodeURIComponent(runId) + "/events");
    window.OwoWorkflowEventSource = es;
    var generation = workflowGeneration;
    es.onmessage = function (ev) {
      if (generation === workflowGeneration) appendEvent(ev.data);
    };
    es.onerror = function () {
      if (generation === workflowGeneration) appendEvent("[events 连接中断]");
    };
  }

  function appendEvent(frame) {
    var el = document.getElementById("owo-workflow-events");
    if (!el) { return; }
    var time = "";
    var kind = "";
    var detail = "";
    try {
      var d = JSON.parse(frame);
      time = d.ts || d.at || d.time || "";
      kind = d.event || d.kind || d.type || "";
      detail = d.detail || d.step_id || d.message || d.run_id || "";
      if (!detail && typeof d === "object") { detail = brief(d); }
    } catch (e) {
      detail = String(frame);
    }
    var row = document.createElement("div");
    row.className = "owo-wf-event";
    row.innerHTML =
      '<span class="owo-wf-event-time">' + esc(time) + "</span>" +
      '<span class="owo-wf-event-kind">' + esc(kind || "event") + "</span>" +
      '<span class="owo-wf-event-detail">' + esc(detail) + "</span>";
    el.appendChild(row);
    el.scrollTop = el.scrollHeight;
  }

  function pollRun(runId, attempt, generation, failures) {
    var owner = generation == null ? workflowGeneration : generation;
    var failureCount = failures || 0;
    if (owner !== workflowGeneration) return Promise.resolve();
    return get("/workflow/run/" + encodeURIComponent(runId))
      .then(function (snap) {
        if (owner !== workflowGeneration) return;
        if (snap && snap.run_id) workflowRunSnapshots[snap.run_id] = snap;
        renderSnapshot(snap);
        if (snap.state === "running" || snap.state === "waiting_approval") {
          schedulePoll(runId, 1000, owner, 0);
        }
      })
      .catch(function (error) {
        if (owner !== workflowGeneration) return;
        var runner = document.getElementById("owo-workflow-runner");
        if (runner) {
          var note = runner.querySelector("[data-workflow-poll-status]");
          if (!note) {
            note = document.createElement("div");
            note.setAttribute("data-workflow-poll-status", "1");
            note.setAttribute("role", "status");
            note.className = "sub";
            runner.appendChild(note);
          }
          var retryMs = Math.min(1000 * Math.pow(2, Math.min(failureCount, 4)), 15000);
          note.textContent = "运行状态同步暂时失败，" + Math.ceil(retryMs / 1000) +
            " 秒后重试：" + friendlyError(error);
          schedulePoll(runId, retryMs, owner, failureCount + 1);
        }
      });
  }

  function setRunActionUi(runId, action) {
    ["owo-workflow-approve", "owo-workflow-reject", "owo-workflow-abort"].forEach(function (id) {
      var button = document.getElementById(id);
      if (!button) return;
      var buttonRunId = button.getAttribute
        ? button.getAttribute("data-run-id")
        : (button.attributes && button.attributes["data-run-id"]);
      if (buttonRunId && buttonRunId !== String(runId)) return;
      button.disabled = Boolean(action);
      if (action) button.setAttribute("aria-busy", "true");
      else button.removeAttribute("aria-busy");
      if (id === "owo-workflow-abort") {
        button.textContent = action === "abort" ? "正在中止…" : "abort";
      } else if (id === "owo-workflow-approve") {
        button.textContent = action === "approve" ? "正在提交…" : "批准";
      } else {
        button.textContent = action === "reject" ? "正在提交…" : "拒绝";
      }
    });
  }

  function renderApprovalCard(snap) {
    var el = document.getElementById("owo-workflow-approval");
    if (!el) { return; }
    var pending = snap.pending_approval;
    if (!pending || !pending.id) {
      el.innerHTML = '<span class="sub">等待审批的请求会显示在这里。</span>';
      return;
    }
    var busy = Boolean(workflowRunActionPending[snap.run_id]);
    el.innerHTML =
      '<div class="owo-workflow-card">' +
      '<h4>等待审批：' + esc(snap.run_id) + "</h4>" +
      '<p style="margin:4px 0;white-space:pre-wrap">' + esc(pending.prompt || "") + "</p>" +
      '<div class="sub">' + esc(pending.created_at || "") + "</div>" +
      '<button id="owo-workflow-approve" data-core-action data-run-id="' + esc(snap.run_id) + '" class="primary"' + (busy ? ' disabled aria-busy="true"' : "") + ">" + (busy ? "操作处理中…" : "批准") + "</button>" +
      '<button id="owo-workflow-reject" data-core-action data-run-id="' + esc(snap.run_id) + '" class="owo-workflow-btn"' + (busy ? ' disabled aria-busy="true"' : "") + ">拒绝</button>" +
      "</div>";
    document.getElementById("owo-workflow-approve").addEventListener("click", function () {
      decideApproval(snap.run_id, "approve");
    });
    document.getElementById("owo-workflow-reject").addEventListener("click", function () {
      decideApproval(snap.run_id, "reject");
    });
  }

  function startRunAction(runId, action, body) {
    var key = String(runId || "");
    if (!key || workflowRunActionPending[key]) return Promise.resolve(null);
    if (action !== "approve" && action !== "reject" && action !== "abort") return Promise.resolve(null);
    var snapshot = workflowRunSnapshots[key];
    var canAbort = snapshot && (snapshot.state === "running" || snapshot.state === "waiting_approval");
    var hasPendingApproval = snapshot && snapshot.state === "waiting_approval" &&
      snapshot.pending_approval && snapshot.pending_approval.id;
    if (action === "abort" && !canAbort) return Promise.resolve(null);
    if ((action === "approve" || action === "reject") && !hasPendingApproval) return Promise.resolve(null);
    var owner = workflowGeneration;
    workflowRunActionPending[key] = action;
    setRunActionUi(key, action);
    var path = action === "abort"
      ? "/workflow/run/" + encodeURIComponent(key) + "/abort"
      : "/workflow/run/" + encodeURIComponent(key) + "/approval";
    return post(path, body || {})
      .then(function (result) {
        if (owner !== workflowGeneration) return null;
        return pollRun(key, 0, owner).then(function () { return result; });
      })
      .catch(function (error) {
        if (owner === workflowGeneration) toast(friendlyError(error), "error");
        return null;
      })
      .finally(function () {
        if (workflowRunActionPending[key] === action) delete workflowRunActionPending[key];
        if (owner !== workflowGeneration) return;
        var current = workflowRunSnapshots[key];
        if (current) renderSnapshot(current);
        else setRunActionUi(key, null);
      });
  }

  function decideApproval(runId, decision) {
    return startRunAction(runId, decision, { decision: decision });
  }

  function renderSnapshot(snap) {
    if (!snap || !snap.run_id) return;
    workflowRunSnapshots[snap.run_id] = snap;
    var el = document.getElementById("owo-workflow-runner");
    if (!el) { return; }
    renderApprovalCard(snap);
    setRunActionUi(snap.run_id, workflowRunActionPending[snap.run_id] || null);
    var badge = "owo-workflow-badge-run";
    if (snap.state === "succeeded") { badge = "owo-workflow-badge-ok"; }
    if (snap.state === "failed" || snap.state === "aborted") { badge = "owo-workflow-badge-bad"; }
    if (snap.state === "waiting_approval") { badge = "owo-workflow-badge-run"; }
    var steps = (snap.steps || [])
      .map(function (s) {
        var cls = s.ok ? "owo-workflow-step-ok" : "owo-workflow-step-fail";
        return (
          '<div class="owo-workflow-step"><span class="' + cls + '">' +
          (s.ok ? "✓" : "✗") + " " + esc(s.kind) + "</span>" +
          '<span class="sub">' + esc(s.id) + " — " + esc(s.detail) + "</span></div>"
        );
      })
      .join("") || '<div class="sub">（无步骤）</div>';
    var active = snap.state === "running" || snap.state === "waiting_approval";
    var pendingAction = workflowRunActionPending[snap.run_id];
    var abortLabel = pendingAction === "abort" ? "正在中止…" : (active ? "abort" : "运行已结束");
    el.innerHTML =
      "<h4>run " + esc(snap.run_id) + ' <span class="owo-workflow-badge ' + badge + '">' + esc(snap.state) + "</span></h4>" +
      (snap.rollback_to ? '<div class="owo-workflow-step owo-workflow-step-fail">已回滚到检查点：' + esc(snap.rollback_to) + "</div>" : "") +
      steps +
      '<div><button id="owo-workflow-abort" data-core-action data-run-id="' + esc(snap.run_id) + '" class="owo-workflow-btn"' + ((!active || pendingAction) ? ' disabled' : "") + (pendingAction === "abort" ? ' aria-busy="true"' : "") + ">" + abortLabel + "</button>" +
      '<button id="owo-workflow-audit-btn" class="owo-workflow-btn">审计尾部</button></div>';
    document.getElementById("owo-workflow-abort").addEventListener("click", function () {
      startRunAction(snap.run_id, "abort", {});
    });
    document.getElementById("owo-workflow-audit-btn").addEventListener("click", function () {
      loadAudit(snap.run_id);
    });
  }

  function loadAudit(runId) {
    var request = ++auditRequestGeneration;
    var owner = workflowGeneration;
    return get("/workflow/run/" + encodeURIComponent(runId) + "/audit")
      .then(function (data) {
        if (request !== auditRequestGeneration || owner !== workflowGeneration) return;
        var auditBox = document.getElementById("owo-workflow-audit");
        if (!auditBox) return;
        auditBox.innerHTML = (data.audit || [])
          .map(function (a) {
            return '<div>' + esc(a.ts || "") + " [" + esc(a.event || "") + "] " + esc(a.detail || "") + "</div>";
          })
          .join("") || '<div class="owo-empty">暂无审计记录</div>';
      })
      .catch(function (e) {
        if (request !== auditRequestGeneration || owner !== workflowGeneration) return;
        var auditBox = document.getElementById("owo-workflow-audit");
        if (auditBox) auditBox.innerHTML = '<span class="owo-workflow-step owo-workflow-step-fail">' + esc(friendlyError(e)) + "</span>";
        toast(friendlyError(e), "error");
      });
  }
  function refreshRuns(name) {
    var el = document.getElementById("owo-workflow-runs");
    if (!el) { return; }
    var request = ++runsRequestGeneration;
    var owner = workflowGeneration;
    get("/workflow/" + encodeURIComponent(name) + "/runs")
      .then(function (data) {
        if (request !== runsRequestGeneration || owner !== workflowGeneration) return;
        var currentList = document.getElementById("owo-workflow-runs");
        if (!currentList) return;
        var runs = data.runs || [];
        if (!runs.length) { currentList.innerHTML = '<div class="owo-empty">暂无运行记录</div>'; return; }
        currentList.innerHTML = runs
          .map(function (r) {
            return (
              '<div class="owo-workflow-step">' +
              '<span class="owo-workflow-name">' + esc(r.run_id) + "</span>" +
              '<span class="sub">' + esc(r.state) + " · " + esc(r.created_at) + "</span>" +
              '<button class="owo-workflow-btn owo-workflow-snap" data-run="' + esc(r.run_id) + '">快照</button>' +
              "</div>"
            );
          })
          .join("");
        currentList.querySelectorAll(".owo-workflow-snap").forEach(function (btn) {
          btn.addEventListener("click", function () { pollRun(btn.getAttribute("data-run"), 0); });
        });
      })
      .catch(function (e) {
        if (request !== runsRequestGeneration || owner !== workflowGeneration) return;
        var currentList = document.getElementById("owo-workflow-runs");
        if (currentList) currentList.innerHTML = '<span class="owo-workflow-step owo-workflow-step-fail">' + esc(friendlyError(e)) + "</span>";
      });
  }
  function bindValidate(root) {
    var scope = root || document;
    function find(id) {
      var current = document.getElementById(id);
      return current || (scope.querySelector && scope.querySelector("#" + id));
    }
    var btn = find("owo-workflow-validate-btn");
    var input = find("owo-workflow-validate-dsl");
    var result = find("owo-workflow-validate-result");
    if (!btn || !input || !result) { return; }
    function parseAndRenderLocalSyntax() {
      var dsl = String(input.value || "").trim();
      if (!dsl) {
        result.innerHTML = '<span class="sub">粘贴 .owflow JSON 后会在本地检查语法。</span>';
        return { valid: false };
      }
      try {
        return {
          valid: true,
          value: JSON.parse(dsl),
        };
      } catch (e) {
        result.innerHTML = '<span class="owo-workflow-step-fail">JSON 解析失败：' + esc(e.message) + "</span>";
        return { valid: false };
      }
    }
    input.addEventListener("input", function () {
      var parsed = parseAndRenderLocalSyntax();
      if (parsed.valid) {
        result.innerHTML = '<span class="owo-workflow-badge owo-workflow-badge-ok">JSON 语法有效；连接核心后可继续校验流程结构</span>';
      }
    });
    result.setAttribute("role", "status");
    result.setAttribute("aria-live", "polite");
    btn.addEventListener("click", function () {
      if (validationPending) return Promise.resolve(null);
      var syntax = parseAndRenderLocalSyntax();
      if (!syntax.valid) return Promise.resolve(null);
      var parsed = syntax.value;
      var owner = workflowGeneration;
      var request = ++validationRequestGeneration;
      validationPending = true;
      btn.disabled = true;
      btn.setAttribute("aria-busy", "true");
      return post("/workflow/validate", parsed)
        .then(function (data) {
          if (request !== validationRequestGeneration || owner !== workflowGeneration) return null;
          if (data.valid) {
            result.innerHTML = '<span class="owo-workflow-badge owo-workflow-badge-ok">valid</span>';
          } else {
            result.innerHTML = '<span class="owo-workflow-badge owo-workflow-badge-bad">invalid</span>' +
              (data.issues || []).map(function (i) { return "<div class='owo-workflow-step owo-workflow-step-fail'>" + esc(i) + "</div>"; }).join("");
          }
          return data;
        })
        .catch(function (e) {
          if (request === validationRequestGeneration && owner === workflowGeneration) {
            result.innerHTML = '<span class="owo-workflow-step-fail">' + esc(friendlyError(e)) + "</span>";
          }
          return null;
        })
        .finally(function () {
          if (request !== validationRequestGeneration || owner !== workflowGeneration) return;
          validationPending = false;
          btn.disabled = false;
          btn.removeAttribute("aria-busy");
        });
    });
  }

  function bindRefresh() {
    var btn = document.getElementById("owo-workflow-refresh");
    if (!btn) { return; }
    btn.addEventListener("click", function () { self.refresh(); });
  }

  window.OwoPanels.workflow = {
    id: "workflow",
    title: "工作流",
    nav: nav,
    mount: function (root, helpers) {
      stopWorkflowRuntime();
      self = this;
      this.helpers = helpers || {};
      root.innerHTML = style() + this.nav();
      this.refresh();
      bindValidate(root);
      bindRefresh();
    },
    dispose: stopWorkflowRuntime,
    refresh: function () {
      var generation = workflowGeneration;
      get("/workflow")
        .then(function (data) {
          if (generation === workflowGeneration) renderList(data.flows);
        })
        .catch(function (e) {
          if (generation !== workflowGeneration) return;
          var el = document.getElementById("owo-workflow-list");
          if (el) { el.innerHTML = '<span class="owo-workflow-step owo-workflow-step-fail">' + esc(friendlyError(e)) + "</span>"; }
        });
    },
    _test: {
      pollRun: pollRun,
      loadFlow: loadFlow,
      loadAudit: loadAudit,
      runFlow: runFlow,
      renderSnapshot: renderSnapshot,
      decideApproval: decideApproval,
      startRunAction: startRunAction,
      stopWorkflowRuntime: stopWorkflowRuntime,
    },
  };
})();
