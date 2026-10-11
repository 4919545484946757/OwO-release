/* Lane D 编排面板：Goal/Plan 编排 API + 云端 SSE 进度。
 * 纯脚本 IIFE，注册 window.OwoPanels.goal。
 * 防御性降级：helpers 缺省时自建（fetch + esc + friendlyError）。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.goal = (function () {
  "use strict";

  var id = "goal";

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      });
    }
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) {
          return r.json().then(function (j) {
            throw new Error((j && j.error) || "HTTP " + r.status);
          });
        }
        return r.json();
      });
    }
    function esc(s) {
      return String(s == null ? "" : s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function friendlyError(e) {
      return "操作失败：" + (e && e.message ? e.message : String(e));
    }
    function renderMarkdown(text) {
      return esc(text);
    }
    return { baseUrl: baseUrl, get: get, post: post, esc: esc, friendlyError: friendlyError, renderMarkdown: renderMarkdown };
  }

  var H = defaultHelpers();
  function notify(message, kind) {
    if (H.notify) H.notify(message, kind || "error");
    else if (window.OwoToast) window.OwoToast(message);
    else window.alert(message);
  }
  var panelGeneration = 0;
  var listGeneration = 0;
  var detailGeneration = 0;
  var statusGeneration = 0;
  var auditGeneration = 0;
  var creatingGoal = false;
  var activeCreateButton = null;
  var savingPlans = Object.create(null);
  var goalActionPending = Object.create(null);
  var panelMounted = false;
  var state = {
    goals: [],
    current: null,
    status: null,
    pollTimer: null,
    cloudLog: [],
  };

  function nav() {
    return (
      '<section data-panel="' + id + '">' +
      '<style>' +
      '.owo-goal-row{display:flex;gap:8px;align-items:center;padding:4px 0;border-bottom:1px solid var(--border)}' +
      '.owo-goal-steps{width:100%;min-height:120px;font-family:monospace;font-size:12px}' +
      '.owo-goal-table{width:100%;border-collapse:collapse;font-size:12px}' +
      '.owo-goal-table td,.owo-goal-table th{border:1px solid var(--border-strong);padding:3px 6px;text-align:left}' +
      '.owo-goal-badge{display:inline-block;padding:1px 6px;border-radius:8px;font-size:12px;color:var(--accent-ink)}' +
      '.owo-goal-badge.ok{background:var(--green)}.owo-goal-badge.bad{background:var(--red)}' +
      '.owo-goal-badge.warn{background:var(--yellow)}' +
      '.owo-goal-cloudlog{height:140px;overflow:auto;background:var(--surface-2);color:var(--green);font-family:monospace;font-size:12px;padding:6px}' +
      '.owo-goal-output{max-height:120px;overflow:auto;white-space:pre-wrap;font-family:monospace;font-size:12px;background:var(--surface-2);padding:4px;margin-top:2px}' +
      '</style>' +
      '<div class="stack">' +
      '<div class="sub">编排目标（Goal/Plan）</div>' +
      '<div class="owo-goal-row"><input id="owo-goal-objective" placeholder="目标描述（objective）" style="flex:1">' +
      '<button class="primary" id="owo-goal-create" data-core-action>创建目标</button></div>' +
      '<div id="owo-goal-list" class="list"></div>' +
      '<hr>' +
      '<div id="owo-goal-detail"></div>' +
      '<hr>' +
      '<div class="sub">云端进度（SSE 订阅）</div>' +
      '<div class="owo-goal-row"><input id="owo-goal-cloud-task" placeholder="云端任务编号（如 cloud-0001）" style="flex:1">' +
      '<button id="owo-goal-cloud-sub" data-core-action>订阅</button><button id="owo-goal-cloud-close">断开</button></div>' +
      '<div class="owo-goal-cloudlog" id="owo-goal-cloudlog">（输入云端任务编号以查看实时进度）</div>' +
      '</div>'
    );
  }

  function mount(root, helpers) {
    dispose();
    state.cloudLog = [];
    cloudReconnectNoticeShown = false;
    if (helpers) H = helpers;
    root.innerHTML = nav();
    panelMounted = true;
    var createButton = root.querySelector("#owo-goal-create");
    if (creatingGoal) {
      activeCreateButton = createButton;
      setCreateButtonBusy(createButton, true);
    }
    createButton.addEventListener("click", createGoal);
    root.querySelector("#owo-goal-cloud-sub").addEventListener("click", subscribeCloud);
    root.querySelector("#owo-goal-cloud-close").addEventListener("click", closeCloud);
    refresh();
  }

  function refresh() {
    var request = ++listGeneration;
    var owner = panelGeneration;
    var selectedAtRequest = state.current;
    return H.get("/goal")
      .then(function (data) {
        if (request !== listGeneration || owner !== panelGeneration) return;
        state.goals = (data && data.goals) || [];
        renderList();
        if (state.current && state.current === selectedAtRequest) loadGoal(state.current);
      })
      .catch(function (e) {
        if (request !== listGeneration || owner !== panelGeneration) return;
        var list = document.getElementById("owo-goal-list");
        if (list) list.innerHTML = '<div class="owo-goal-badge bad">' + H.esc(H.friendlyError(e)) + "</div>";
      });
  }

  function renderList() {
    var el = document.getElementById("owo-goal-list");
    if (!el) return;
    if (!state.goals.length) {
      el.innerHTML = '<div class="sub">暂无目标，先创建一个。</div>';
      return;
    }
    el.innerHTML = state.goals
      .map(function (g) {
        var badge = g.status === "Succeeded" ? "ok" : g.status === "Failed" || g.status === "Aborted" ? "bad" : "warn";
        var pendingAction = goalActionPending[g.id];
        var actionBusy = pendingAction ? ' disabled aria-busy="true"' : "";
        return (
          '<div class="owo-goal-row">' +
          '<span class="owo-goal-badge ' + badge + '">' + H.esc(g.status) + "</span>" +
          "<span>" + H.esc(g.objective) + "</span>" +
          '<button data-id="' + H.esc(g.id) + '" class="owo-goal-open">打开</button>' +
          '<button data-id="' + H.esc(g.id) + '" class="owo-goal-run" data-core-action' + actionBusy + '>' +
          (pendingAction === "run" ? "正在运行…" : pendingAction ? "处理中…" : "运行") + "</button>" +
          '<button data-id="' + H.esc(g.id) + '" class="owo-goal-abort" data-core-action' + actionBusy + '>' +
          (pendingAction === "abort" ? "正在中止…" : pendingAction ? "处理中…" : "中止") + "</button>" +
          "</div>"
        );
      })
      .join("");
    Array.prototype.forEach.call(el.querySelectorAll(".owo-goal-open"), function (b) {
      b.addEventListener("click", function () {
        state.current = b.getAttribute("data-id");
        loadGoal(state.current);
      });
    });
    Array.prototype.forEach.call(el.querySelectorAll(".owo-goal-run"), function (b) {
      b.addEventListener("click", function () {
        runGoal(b.getAttribute("data-id"), b, "运行");
      });
    });
    Array.prototype.forEach.call(el.querySelectorAll(".owo-goal-abort"), function (b) {
      b.addEventListener("click", function () {
        abortGoal(b.getAttribute("data-id"), b, "中止");
      });
    });
  }

  function setCreateButtonBusy(button, busy) {
    if (!button) return;
    button.disabled = busy;
    button.textContent = busy ? "正在创建…" : "创建目标";
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function createGoal() {
    if (creatingGoal) return Promise.resolve(null);
    var input = document.getElementById("owo-goal-objective");
    var objective = (input && input.value.trim()) || "新目标";
    var owner = panelGeneration;
    creatingGoal = true;
    activeCreateButton = document.getElementById("owo-goal-create");
    setCreateButtonBusy(activeCreateButton, true);
    return Promise.resolve()
      .then(function () {
        return H.post("/goal", { objective: objective });
      })
      .then(function (data) {
        if (owner !== panelGeneration) return null;
        if (input) input.value = "";
        state.current = data.goal.id;
        refresh();
        return data;
      })
      .catch(function (e) {
        if (owner === panelGeneration) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        creatingGoal = false;
        var currentButton = document.getElementById("owo-goal-create");
        setCreateButtonBusy(activeCreateButton, false);
        if (currentButton !== activeCreateButton) setCreateButtonBusy(currentButton, false);
        activeCreateButton = null;
      });
  }

  function loadGoal(goalId) {
    state.current = goalId;
    var request = ++detailGeneration;
    var owner = panelGeneration;
    function isCurrent() {
      return request === detailGeneration && owner === panelGeneration && state.current === goalId;
    }
    return H.get("/goal/" + encodeURIComponent(goalId))
      .then(function (goal) {
        if (!isCurrent()) return;
        return H.get("/goal/" + encodeURIComponent(goalId) + "/plan")
          .then(function (planData) {
            if (isCurrent()) renderDetail(goal, planData);
          })
          .catch(function () {
            if (isCurrent()) renderDetail(goal, null);
          });
      })
      .catch(function (e) {
        if (!isCurrent()) return;
        var detail = document.getElementById("owo-goal-detail");
        if (detail) detail.innerHTML = '<div class="owo-goal-badge bad">' + H.esc(H.friendlyError(e)) + "</div>";
      });
  }

  function renderWaves(waves) {
    if (!Array.isArray(waves)) return "";
    return waves.map(function (wave, index) {
      var steps = Array.isArray(wave) ? wave : [];
      return "wave" + (index + 1) + ": " +
        steps.map(function (step) { return H.esc(step == null ? "" : step); }).join(", ");
    }).join("<br>");
  }

  function renderDetail(goal, planData) {
    var el = document.getElementById("owo-goal-detail");
    if (!el) return;
    var plan = (planData && planData.plan) || null;
    var waves = (planData && planData.waves) || null;
    var stepsJson = plan
      ? JSON.stringify(
          (plan.steps || []).map(function (s) {
            return { id: s.id, worker: s.worker, deps: s.depends_on || [], verify: s.verify ? (s.verify.OutputContains || s.verify.OutputEquals || s.verify.OutputNonEmpty || "") : null, max_retries: s.retries || 0, input: s.input || {} };
          }),
          null,
          2
        )
      : '[{"id":"a","worker":"echo","input":{"text":"A"}},{"id":"b","worker":"sleep","input":{"ms":20},"deps":["a"]},{"id":"c","worker":"agent","input":{"prompt":"总结上一步","read_only":true},"deps":["b"]}]';
    var wavesHtml = waves ? renderWaves(waves) : "（暂无计划）";
    el.innerHTML =
      '<div class="sub">目标：' + H.esc(goal.objective) + "（" + H.esc(goal.status) + "）</div>" +
      '<div class="owo-goal-row"><span>步骤定义（JSON）</span>' +
      '<button id="owo-goal-save-plan" data-core-action' + (savingPlans[goal.id] ? ' disabled aria-busy="true"' : "") + '>' +
      (savingPlans[goal.id] ? "正在保存…" : "保存计划") + "</button></div>" +
      '<textarea class="owo-goal-steps" id="owo-goal-steps">' + H.esc(stepsJson) + "</textarea>" +
      '<div class="owo-goal-row"><span>waves 预览</span>' +
      '<button id="owo-goal-run-now" data-core-action' + (goalActionPending[goal.id] ? ' disabled aria-busy="true"' : "") + '>' +
      (goalActionPending[goal.id] === "run" ? "正在运行…" : goalActionPending[goal.id] ? "处理中…" : "运行（parallelism=2）") + "</button>" +
      '<button id="owo-goal-poll">刷新状态</button></div>' +
      '<div class="sub">' + wavesHtml + "</div>" +
      '<table class="owo-goal-table" id="owo-goal-status"><tr><th>步骤</th><th>状态</th><th>尝试</th><th>输出</th></tr></table>' +
      '<div class="owo-goal-row"><button id="owo-goal-audit">审计尾部</button></div>' +
      '<div id="owo-goal-audit-box"></div>';
    el.querySelector("#owo-goal-save-plan").addEventListener("click", function () {
      savePlan(goal.id);
    });
    el.querySelector("#owo-goal-run-now").addEventListener("click", function () {
      runGoal(goal.id, el.querySelector("#owo-goal-run-now"), "运行（parallelism=2）");
    });
    el.querySelector("#owo-goal-poll").addEventListener("click", function () {
      pollStatus(goal.id);
    });
    el.querySelector("#owo-goal-audit").addEventListener("click", function () {
      showAudit(goal.id);
    });
    pollStatus(goal.id);
  }

  function setPlanSaveButtonBusy(button, busy) {
    if (!button) return;
    button.disabled = busy;
    button.textContent = busy ? "正在保存…" : "保存计划";
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function savePlan(goalId) {
    if (savingPlans[goalId] || state.current !== goalId) return Promise.resolve(null);
    var textarea = document.getElementById("owo-goal-steps");
    var saveButton = document.getElementById("owo-goal-save-plan");
    if (!textarea) return Promise.resolve(null);
    var steps;
    try {
      steps = JSON.parse(textarea.value);
    } catch (e) {
      notify("steps JSON 非法：" + e.message, "error");
      return Promise.resolve(null);
    }
    if (!Array.isArray(steps)) {
      notify("steps 必须是 JSON 数组", "error");
      return Promise.resolve(null);
    }
    var normalized = steps.map(function (s) {
      return {
        id: s.id,
        worker: s.worker,
        deps: s.deps || [],
        verify: s.verify || null,
        max_retries: s.max_retries || 0,
        input: s.input || {},
        parallel: !!s.parallel,
      };
    });
    var owner = panelGeneration;
    savingPlans[goalId] = true;
    setPlanSaveButtonBusy(saveButton, true);
    return Promise.resolve()
      .then(function () {
        return H.post("/goal/" + encodeURIComponent(goalId) + "/plan", { steps: normalized });
      })
      .then(function (data) {
        if (owner !== panelGeneration || state.current !== goalId) return null;
        var waves = (data && data.waves) || [];
        var preview = renderWaves(waves);
        var box = document.querySelector("#owo-goal-detail .sub:nth-of-type(3)");
        if (box) box.innerHTML = preview;
        loadGoal(goalId);
        return data;
      })
      .catch(function (e) {
        if (owner === panelGeneration && state.current === goalId) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        delete savingPlans[goalId];
        setPlanSaveButtonBusy(saveButton, false);
        if (owner === panelGeneration && state.current === goalId) {
          setPlanSaveButtonBusy(document.getElementById("owo-goal-save-plan"), false);
        }
      });
  }

  function setGoalActionButtonBusy(button, busy, idleText, busyText) {
    if (!button) return;
    button.disabled = busy;
    button.textContent = busy ? busyText : idleText;
    if (button.setAttribute && busy) button.setAttribute("aria-busy", "true");
    else if (button.removeAttribute) button.removeAttribute("aria-busy");
  }

  function runGoal(goalId, button, idleText) {
    return startGoalAction(goalId, "run", button, idleText || "运行", "正在运行…");
  }

  function abortGoal(goalId, button, idleText) {
    return startGoalAction(goalId, "abort", button, idleText || "中止", "正在中止…");
  }

  function startGoalAction(goalId, action, button, idleText, busyText) {
    if (goalActionPending[goalId]) return Promise.resolve(null);
    var owner = panelGeneration;
    goalActionPending[goalId] = action;
    setGoalActionButtonBusy(button, true, idleText, busyText);
    var path = "/goal/" + encodeURIComponent(goalId) + (action === "run" ? "/run" : "/abort");
    var body = action === "run" ? { config: { parallelism: 2, allow_replan: true } } : {};
    return Promise.resolve()
      .then(function () {
        return H.post(path, body);
      })
      .then(function (result) {
        if (owner !== panelGeneration) return null;
        return result;
      })
      .catch(function (e) {
        if (owner === panelGeneration) notify(H.friendlyError(e), "error");
        return null;
      })
      .finally(function () {
        delete goalActionPending[goalId];
        setGoalActionButtonBusy(button, false, idleText, busyText);
        if (panelMounted) refresh();
      });
  }

  function pollStatus(goalId) {
    var request = ++statusGeneration;
    var owner = panelGeneration;
    return H.get("/goal/" + encodeURIComponent(goalId) + "/status")
      .then(function (status) {
        if (request !== statusGeneration || owner !== panelGeneration || state.current !== goalId) return;
        state.status = status;
        var table = document.getElementById("owo-goal-status");
        if (!table) return;
        // R5：优先用 steps（含 worker 名/截断输出/模型名），回退 records。
        var steps = (status && status.steps) || null;
        var rows = "";
        if (steps && steps.length) {
          rows = steps
            .map(function (s) {
              var badge = s.status === "Succeeded" ? "ok" : s.status === "Failed" || s.status === "Aborted" ? "bad" : "warn";
              var worker = H.esc(s.worker || "");
              var output = H.esc(s.output || "");
              var folded = output.length > 60
                ? '<details><summary>' + H.esc(output.slice(0, 60)) + "…</summary><div class=\"owo-goal-output\">" + output + "</div></details>"
                : output;
              var modelHint = (s.model && s.model !== "gpt-4.1-mini") ? " · " + H.esc(s.model) : "";
              return (
                "<tr><td>" + H.esc(s.step_id) + '</td><td><span class="owo-goal-badge ' + badge + '">' + H.esc(s.status) + "</span></td>" +
                "<td>" + worker + modelHint + "</td><td>" + folded + "</td></tr>"
              );
            })
            .join("");
        } else {
          var records = (status && status.records) || {};
          rows = Object.keys(records)
            .map(function (stepId) {
              var r = records[stepId];
              var badge = r.status === "Succeeded" ? "ok" : r.status === "Failed" || r.status === "Aborted" ? "bad" : "warn";
              return (
                "<tr><td>" + H.esc(stepId) + '</td><td><span class="owo-goal-badge ' + badge + '">' + H.esc(r.status) + "</span></td>" +
                "<td></td><td>" + H.esc((r.output || "").slice(0, 60)) + "</td></tr>"
              );
            })
            .join("");
        }
        var goalStatus = (status && status.goal_status) || "";
        table.innerHTML = "<tr><th>步骤</th><th>状态</th><th>worker/模型</th><th>输出</th></tr>" + rows +
          '<tr><td colspan="4">goal: ' + H.esc(goalStatus) + " · steps_taken: " + H.esc(status.steps_taken) + " · replan: " + H.esc(status.replan_count) + "</td></tr>";
      })
      .catch(function (error) {
        if (request !== statusGeneration || owner !== panelGeneration || state.current !== goalId) return;
        var table = document.getElementById("owo-goal-status");
        if (!table) return;
        table.innerHTML = "<tr><th>步骤</th><th>状态</th><th>尝试</th><th>输出</th></tr>" +
          '<tr><td colspan="4" class="owo-goal-badge bad">状态读取失败：' +
          H.esc(H.friendlyError(error)) + "（可点“刷新状态”重试）</td></tr>";
      });
  }

  function showAudit(goalId) {
    var request = ++auditGeneration;
    var owner = panelGeneration;
    function isCurrent() {
      return request === auditGeneration && owner === panelGeneration && state.current === goalId;
    }
    return H.get("/goal/" + encodeURIComponent(goalId) + "/audit")
      .then(function (data) {
        if (!isCurrent()) return;
        var box = document.getElementById("owo-goal-audit-box");
        if (!box) return;
        box.innerHTML = (data.audit || [])
          .slice(0, 20)
          .map(function (e) {
            return "<div>" + H.esc(e.event) + " — " + H.esc(e.detail) + "</div>";
          })
          .join("");
      })
      .catch(function (e) {
        if (!isCurrent()) return;
        var box = document.getElementById("owo-goal-audit-box");
        if (box) box.innerHTML = H.esc(H.friendlyError(e));
      });
  }

  var cloudSource = null;
  var cloudGeneration = 0;
  var cloudReconnectNoticeShown = false;

  function subscribeCloud() {
    var input = document.getElementById("owo-goal-cloud-task");
    var taskId = (input && input.value.trim()) || "";
    if (!taskId) return;
    closeCloud();
    var base = H.baseUrl || window.location.origin;
    var log = document.getElementById("owo-goal-cloudlog");
    if (log) log.textContent = "订阅 " + taskId + " ...";
    cloudReconnectNoticeShown = false;
    var generation = cloudGeneration;
    cloudSource = new EventSource(base + "/cloud/tasks/" + encodeURIComponent(taskId) + "/events");
    cloudSource.onopen = function () {
      if (generation !== cloudGeneration) return;
      cloudReconnectNoticeShown = false;
    };
    cloudSource.onmessage = function (event) {
      if (generation === cloudGeneration) onCloudProgress(event);
    };
    // The server names cloud progress frames `progress`; EventSource.onmessage only
    // receives unnamed `message` events, so subscribe to both wire forms.
    cloudSource.addEventListener("progress", function (event) {
      if (generation === cloudGeneration) onCloudProgress(event);
    });
    cloudSource.onerror = function () {
      if (generation !== cloudGeneration) return;
      // EventSource tracks Last-Event-ID and retries transient disconnects itself.
      // Only explicit user action or a terminal/not-found frame closes this source.
      if (!cloudReconnectNoticeShown) {
        appendCloudLog("（连接暂时中断，正在自动续传进度）");
        cloudReconnectNoticeShown = true;
      }
    };
  }

  function onCloudProgress(ev) {
    var raw = ev && ev.data != null ? String(ev.data) : "";
    try {
      var payload = JSON.parse(raw);
      if (payload && payload.kind === "stream_gap") {
        appendCloudLog(payload.message || "实时进度出现丢帧，正在续传保留的历史。");
        return;
      }
      if (payload && ["task_not_found", "stream_complete"].includes(payload.kind)) {
        appendCloudLog(payload.message || "任务已结束或没有找到该任务。");
        closeCloudSilently();
        return;
      }
      if (payload && ["succeeded", "failed", "canceled"].includes(payload.kind || payload.event)) {
        appendCloudLog(raw);
        closeCloudSilently();
        return;
      }
    } catch (_) {
      // Non-JSON compatibility messages remain visible as received.
    }
    appendCloudLog(raw);
  }

  function closeCloudSilently() {
    cloudGeneration += 1;
    var source = cloudSource;
    cloudSource = null;
    if (source) source.close();
  }

  function dispose() {
    panelMounted = false;
    panelGeneration += 1;
    listGeneration += 1;
    detailGeneration += 1;
    statusGeneration += 1;
    auditGeneration += 1;
    closeCloudSilently();
  }

  function closeCloud() {
    closeCloudSilently();
    appendCloudLog("（已断开）");
  }

  function appendCloudLog(line) {
    var log = document.getElementById("owo-goal-cloudlog");
    if (!log) return;
    state.cloudLog.push(String(line));
    if (state.cloudLog.length > 200) state.cloudLog.shift();
    log.textContent = state.cloudLog.join("\n");
    log.scrollTop = log.scrollHeight;
  }

  return {
    id: id,
    title: "编排（Goal/Plan + 云端进度）",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { loadGoal: loadGoal, pollStatus: pollStatus, showAudit: showAudit, createGoal: createGoal, savePlan: savePlan, runGoal: runGoal, abortGoal: abortGoal, renderWaves: renderWaves },
  };
})();
