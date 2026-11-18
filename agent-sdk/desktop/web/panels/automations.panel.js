/* 定时任务：左侧任务栏 + 右侧详情/模板页。
 * 数据与调度完全走核心 API（/automations、/automations/runs、toggle、delete），
 * 不伪造任务；新建/编辑为右侧页面式表单，编辑用「新建 + 删除旧任务」落地
 * （核心当前没有更新接口）。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.automations = (function () {
  "use strict";

  var id = "automations";

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || "";
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
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
    return {
      baseUrl: baseUrl,
      get: get,
      post: function (path, body) {
        return window.OwoApi.stream(path, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body || {}),
        }).then(function (r) {
          if (!r.ok) throw new Error("HTTP " + r.status);
          return r.status === 204 ? null : r.json();
        });
      },
      call: function (path, options) {
        return window.OwoApi.stream(path, options || {}).then(function (r) {
          if (!r.ok) throw new Error("HTTP " + r.status);
          return r.status === 204 ? null : r.json().catch(function () { return null; });
        });
      },
      del: function (path) {
        return window.OwoApi.stream(path, { method: "DELETE" }).then(function (r) {
          if (!r.ok) throw new Error("HTTP " + r.status);
          return null;
        });
      },
      confirm: function (options) {
        return Promise.resolve(window.confirm(options.message || ""));
      },
      notify: function (message) {
        window.alert(message);
      },
      esc: esc,
      friendlyError: friendlyError,
    };
  }

  var H = defaultHelpers();
  var panelGeneration = 0;
  var taskRefreshGeneration = 0;
  var reminderRefreshGeneration = 0;
  var creatingTask = false;
  var activeCreateButton = null;
  var clearingReminders = false;
  var taskToggleBusy = Object.create(null);
  var taskDeleteBusy = Object.create(null);
  var panelMounted = false;
  var rootEl = null;
  var hostScope = "";
  var cachedTasks = [];
  var selectedTaskId = "";
  var rowControls = Object.create(null);
  var tasksAvailable = false;
  var taskQuery = "";
  var taskFilter = "";
  var activeTab = "tasks";
  var editorOpen = false;
  var editingTaskId = "";
  var pausedOpen = false;
  var toolsOpen = false;
  var draft = {};
  var openRuns = Object.create(null);
  var fieldIds = ["name", "kind", "interval", "daily", "oneshot", "action", "content"];

  // helpers 由宿主注入，个别环境只给最小集合；统一兜底转义。
  function escText(value) {
    if (typeof H.esc === "function") return H.esc(value);
    return String(value == null ? "" : value)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function scopeOf(helpers) {
    return String(helpers.baseUrl || "") + "#" + String(helpers.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
  }

  function current(owner) { return panelMounted && owner === panelGeneration; }

  function captureDraft() {
    if (!panelMounted) return;
    fieldIds.forEach(function (key) {
      var node = el("owo-aut-" + key);
      if (node) draft[key] = node.value;
    });
  }

  function setEditorTitle(editing) {
    var title = el("owo-aut-editor-title");
    if (title) title.textContent = editing ? "编辑任务" : "新建任务";
    var submit = el("owo-aut-submit");
    if (submit) submit.textContent = editing ? "保存修改" : "创建任务";
    var note = el("owo-aut-edit-note");
    if (note) note.hidden = !editing;
  }

  function showEditor(open, task) {
    captureDraft();
    if (open) {
      editingTaskId = task ? String(task.id) : "";
      fillEditor(task || null);
    }
    editorOpen = !!open;
    var editor = el("owo-aut-editor");
    if (editor) editor.hidden = !editorOpen;
    var detail = el("owo-aut-detail");
    if (detail) detail.hidden = editorOpen;
    var trigger = el("owo-aut-new");
    if (trigger) trigger.setAttribute("aria-expanded", String(editorOpen));
    setEditorTitle(!!editingTaskId);
    if (open) {
      selectTab("tasks");
      var pane = el("owo-aut-pane-tasks");
      if (pane) pane.scrollTop = 0;
      var input = el("owo-aut-name");
      if (input) input.focus({ preventScroll: true });
    } else {
      editingTaskId = "";
    }
  }

  function fillEditor(task) {
    var name = el("owo-aut-name"), content = el("owo-aut-content");
    var action = el("owo-aut-action"), kind = el("owo-aut-kind");
    var interval = el("owo-aut-interval"), daily = el("owo-aut-daily"), oneshot = el("owo-aut-oneshot");
    var enabled = el("owo-aut-enabled");
    if (!task) {
      if (name) name.value = "";
      if (content) content.value = "";
      if (action) action.value = "run_prompt";
      if (kind) kind.value = "daily";
      if (interval) interval.value = "";
      if (daily && !daily.value) daily.value = "09:00";
      if (oneshot) oneshot.value = "";
      if (enabled) enabled.checked = true;
    } else {
      if (name) name.value = task.name || "";
      if (action) action.value = task.action && task.action.kind === "reminder" ? "reminder" : "run_prompt";
      if (content) content.value = (task.action && (task.action.prompt || task.action.text)) || "";
      var mapped = scheduleToEditor(task.schedule);
      if (kind) kind.value = mapped.kind;
      if (interval) interval.value = mapped.value || "";
      if (daily && mapped.value) daily.value = mapped.value;
      if (oneshot && mapped.value) oneshot.value = mapped.value;
      if (enabled) enabled.checked = task.enabled !== false;
    }
    syncScheduleFields();
    syncActionFields();
  }

  function pad(number) { return String(number).padStart(2, "0"); }

  function toLocalInput(value) {
    var date = new Date(value || "");
    if (!Number.isFinite(date.getTime())) return "";
    return date.getFullYear() + "-" + pad(date.getMonth() + 1) + "-" + pad(date.getDate()) +
      "T" + pad(date.getHours()) + ":" + pad(date.getMinutes());
  }

  function scheduleToEditor(schedule) {
    if (!schedule || typeof schedule !== "object") return { kind: "daily", value: "09:00" };
    if (schedule.kind === "interval") {
      var every = Number(schedule.every_secs);
      if (every === 604800) return { kind: "weekly", value: "" };
      if (every === 2592000) return { kind: "monthly", value: "" };
      return { kind: "interval", value: String(schedule.every_secs == null ? "" : schedule.every_secs) };
    }
    if (schedule.kind === "daily") return { kind: "daily", value: String(schedule.time || "09:00") };
    if (schedule.kind === "one_shot") return { kind: "oneshot", value: toLocalInput(schedule.at) };
    return { kind: "daily", value: "09:00" };
  }

  function selectTab(tab) {
    activeTab = tab;
    ["tasks", "reminders"].forEach(function (key) {
      var button = el("owo-aut-tab-" + key), pane = el("owo-aut-pane-" + key);
      if (button) {
        button.setAttribute("aria-selected", String(tab === key));
        button.setAttribute("tabindex", tab === key ? "0" : "-1");
      }
      if (pane) pane.hidden = tab !== key;
    });
  }

  function localTime(value) {
    var date = new Date(value || "");
    return Number.isFinite(date.getTime()) ? date.toLocaleString() : "时间未知";
  }

  function validateRows(rows, kind) {
    if (!Array.isArray(rows) || rows.some(function (row) {
      return kind === "reminders" ? typeof row !== "string" :
        !row || typeof row !== "object" || Array.isArray(row) || (kind === "tasks" && (typeof row.id !== "string" || typeof row.name !== "string" || typeof row.enabled !== "boolean"));
    })) throw new Error("返回的数据格式无效，请刷新重试");
    return rows;
  }

  function readRows(api, path, kind) {
    try { return Promise.resolve(api.get(path)).then(function (rows) { return validateRows(rows, kind); }); }
    catch (error) { return Promise.reject(error); }
  }

  function el(domId) {
    return panelMounted && rootEl ? rootEl.querySelector("#" + domId) : null;
  }

  function setStatus(text, isError) {
    var status = el("owo-aut-status");
    if (!status) return;
    status.textContent = text || "";
    status.style.color = isError ? "var(--red)" : "var(--text-3)";
  }

  function setButtonBusy(button, busy, idleText, busyText) {
    if (!button) return;
    button.disabled = !!busy;
    button.setAttribute("data-core-busy", String(!!busy));
    if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, busy);
    if (busy) {
      button.setAttribute("aria-busy", "true");
      button.textContent = busyText || idleText;
    } else {
      button.removeAttribute("aria-busy");
      button.textContent = idleText;
    }
  }

  function explainActionError(action, error) {
    var detail = H.friendlyError ? H.friendlyError(error) : (error && error.message) || String(error);
    var message = action + "失败：" + detail;
    setStatus(message, true);
    if (H.notify) H.notify(message, "error");
  }

  var ICONS = {
    search: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.4-3.4"/></svg>',
    filter: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M3 6h18M6 12h12M10 18h4"/></svg>',
    refresh: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 11a8 8 0 1 0-.6 4"/><path d="M20 5v6h-6"/></svg>',
    plus: '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>',
    chevron: '<svg class="owo-aut-chevron" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>',
    clock: '<svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.2 2"/></svg>',
    sun: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>',
    radar: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/><circle cx="12" cy="12" r="2.6"/></svg>',
    chart: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20V10M18 20V4M6 20v-4"/></svg>',
    broom: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 20 5-5M4 21h9"/><path d="M14 6l4 4-6.5 6.5-4-4L14 6Z"/><path d="m15 5 3-3 4 4-3 3"/></svg>',
    target: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1"/></svg>',
    bell: '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>',
  };

  function nav() {
    return '<section data-panel="automations" data-layout="custom" class="owo-aut" id="owo-aut-root">' +
      '<div class="owo-aut-shell">' +
      '<aside class="owo-aut-side" aria-label="定时任务列表">' +
      '<header class="owo-aut-side-head"><span class="owo-aut-side-title">全部任务</span>' +
      '<div class="owo-aut-side-tools">' +
      '<button type="button" id="owo-aut-search-toggle" class="owo-aut-icon" title="搜索" aria-label="搜索任务" aria-expanded="false">' + ICONS.search + '</button>' +
      '<button type="button" id="owo-aut-filter-toggle" class="owo-aut-icon" title="筛选" aria-label="筛选任务" aria-expanded="false">' + ICONS.filter + '</button>' +
      '<button type="button" id="owo-aut-refresh" class="owo-aut-icon" title="刷新" aria-label="刷新定时任务">' + ICONS.refresh + '</button>' +
      '</div></header>' +
      '<div class="owo-aut-tools" id="owo-aut-tools" hidden>' +
      '<input id="owo-aut-search" type="search" aria-label="搜索任务" placeholder="搜索任务">' +
      '<select id="owo-aut-filter" aria-label="任务状态"><option value="">全部状态</option><option value="enabled">已开启</option><option value="paused">已暂停</option></select></div>' +
      '<button type="button" class="owo-aut-new" id="owo-aut-new" aria-expanded="false" aria-controls="owo-aut-editor">' +
      ICONS.plus + '<span>新建任务</span></button>' +
      '<div id="owo-aut-status" role="status" aria-live="polite"></div>' +
      '<div class="owo-aut-scroll"><ul id="owo-aut-list" class="owo-aut-list"><li class="sub" role="status">加载中…</li></ul></div>' +
      '<footer class="owo-aut-side-foot" role="tablist" aria-label="定时任务分区">' +
      '<button type="button" role="tab" id="owo-aut-tab-tasks" aria-controls="owo-aut-pane-tasks" aria-selected="true">任务</button>' +
      '<button type="button" role="tab" id="owo-aut-tab-reminders" aria-controls="owo-aut-pane-reminders" aria-selected="false">提醒</button>' +
      '</footer></aside>' +
      '<div class="owo-aut-main">' +
      '<div id="owo-aut-pane-tasks" role="tabpanel" aria-labelledby="owo-aut-tab-tasks" class="owo-aut-pane">' +
      '<section id="owo-aut-editor" class="owo-aut-editor" aria-labelledby="owo-aut-editor-title" hidden>' +
      '<header class="owo-aut-editor-head"><div><h3 id="owo-aut-editor-title">新建任务</h3>' +
      '<p class="sub">到点后按设定执行；核心保持运行时才会触发。</p></div></header>' +
      '<form id="owo-aut-form" class="owo-aut-form">' +
      '<label class="owo-aut-field owo-aut-span"><span>任务名称</span>' +
      '<input id="owo-aut-name" placeholder="例如：每日工作简报" required maxlength="200"></label>' +
      '<label class="owo-aut-field"><span>执行方式</span><select id="owo-aut-action">' +
      '<option value="run_prompt">跑只读 Agent 任务</option><option value="reminder">工作台提醒</option></select></label>' +
      '<label class="owo-aut-field"><span>重复</span><select id="owo-aut-kind">' +
      '<option value="daily">每天</option><option value="weekly">每周</option><option value="monthly">每月</option>' +
      '<option value="interval">自定义间隔</option><option value="oneshot">只执行一次</option></select></label>' +
      '<label class="owo-aut-field owo-aut-span"><span id="owo-aut-content-label">任务指令</span>' +
      '<textarea id="owo-aut-content" rows="5" required placeholder="描述到点后要查询、巡检或总结的内容"></textarea>' +
      '<small id="owo-aut-action-hint" class="sub">定时任务无人值守，只读模式，不会修改文件或执行命令。</small></label>' +
      '<label class="owo-aut-field" data-schedule-group="daily" hidden><span>执行时间（本地）</span><input id="owo-aut-daily" type="time" step="60" disabled></label>' +
      '<label class="owo-aut-field" data-schedule-group="interval" hidden><span>间隔秒数</span><input id="owo-aut-interval" type="number" min="1" step="1" inputmode="numeric" disabled></label>' +
      '<label class="owo-aut-field" data-schedule-group="weekly" hidden><span>周期</span><input value="每 7 天执行一次" disabled></label>' +
      '<label class="owo-aut-field" data-schedule-group="monthly" hidden><span>周期</span><input value="每 30 天执行一次" disabled></label>' +
      '<label class="owo-aut-field" data-schedule-group="oneshot" hidden><span>执行时间（本地）</span><input id="owo-aut-oneshot" type="datetime-local" step="60" disabled></label>' +
      '<p class="sub owo-aut-time-hint">每周/每月按 7 天、30 天间隔循环，从创建时刻起算；单次任务按本地时间执行。</p>' +
      '<label class="owo-aut-switch owo-aut-span"><input id="owo-aut-enabled" type="checkbox" checked><span>创建后立即启用</span></label>' +
      '<p class="sub owo-aut-edit-note owo-aut-span" id="owo-aut-edit-note" hidden>保存后会以新配置重建任务，执行历史会重新开始。</p>' +
      '<div class="owo-aut-form-actions"><button type="button" id="owo-aut-cancel" class="owo-aut-ghost">取消</button>' +
      '<button type="submit" class="owo-aut-primary" id="owo-aut-submit" data-core-action>创建任务</button></div>' +
      '</form><p class="sub" data-core-action-hint>连接并授权本地核心后可创建自动化任务。</p></section>' +
      '<div id="owo-aut-detail" class="owo-aut-detail"></div>' +
      '</div>' +
      '<div id="owo-aut-pane-reminders" role="tabpanel" aria-labelledby="owo-aut-tab-reminders" hidden class="owo-aut-pane">' +
      '<header class="owo-aut-reminder-head"><div><h3>工作台提醒</h3>' +
      '<p class="sub">定时任务到点触发后，提醒会出现在这里。</p></div>' +
      '<button type="button" class="owo-aut-ghost danger" id="owo-aut-clear" data-core-action>清除提醒</button></header>' +
      '<p class="sub" data-core-action-hint>连接并授权本地核心后可管理工作台提醒。</p>' +
      '<ul id="owo-aut-reminders" class="owo-aut-reminders"><li class="sub" role="status">加载中…</li></ul></div>' +
      '</div></div></section>';
  }

  /// 右侧空闲时的引导：模板直接填好表单，点一下就能继续改。
  var TASK_TEMPLATES = [
    { icon: "sun", name: "每日晨间简报", action: "run_prompt",
      text: "汇总昨天完成的事情和今天需要处理的任务，各列三条，简短一点。" },
    { icon: "radar", name: "竞品动态巡检", action: "run_prompt",
      text: "定期检查关注的产品、竞品和行业动态，列出值得跟进的变化并附上来源。" },
    { icon: "chart", name: "每周工作总结", action: "run_prompt",
      text: "整理本周的任务、产出和未完成事项，生成一份周报草稿。" },
    { icon: "broom", name: "工作区清理建议", action: "run_prompt",
      text: "检查临时文件和工作区，列出可以安全清理的内容和清理建议。" },
    { icon: "target", name: "目标进度复盘", action: "run_prompt",
      text: "回顾当前目标的进展，找出阻塞问题，给出下一步建议。" },
    { icon: "bell", name: "固定时间提醒", action: "reminder",
      text: "到点提醒我处理手头的重要事项。" },
  ];

  function renderDetailIdle() {
    var detail = el("owo-aut-detail");
    if (!detail) return;
    var pane = el("owo-aut-pane-tasks");
    if (pane) pane.scrollTop = 0;
    var cards = TASK_TEMPLATES.map(function (item, index) {
      return '<button type="button" class="owo-aut-template" data-aut-template="' + index + '">' +
        '<span class="owo-aut-template-icon" aria-hidden="true">' + ICONS[item.icon] + '</span>' +
        '<span class="owo-aut-template-text"><strong>' + escText(item.name) + '</strong>' +
        '<small>' + escText(item.text) + '</small></span></button>';
    }).join("");
    detail.innerHTML =
      '<div class="owo-aut-idle">' +
      '<div class="owo-aut-idle-icon" aria-hidden="true">' + ICONS.clock + '</div>' +
      '<h3>安排任务</h3>' +
      '<p>让 OwO 按计划替你做事，定时提醒、自动巡检和周期性任务都可以轻松安排。</p>' +
      '<div class="owo-aut-templates">' + cards + '</div></div>';
    if (typeof detail.querySelectorAll === "function") {
      detail.querySelectorAll("[data-aut-template]").forEach(function (button) {
        button.addEventListener("click", function () {
          var item = TASK_TEMPLATES[Number(button.getAttribute("data-aut-template"))];
          if (!item) return;
          showEditor(true, null);
          var nameInput = el("owo-aut-name"), contentInput = el("owo-aut-content");
          var actionSelect = el("owo-aut-action"), kindSelect = el("owo-aut-kind");
          if (nameInput) nameInput.value = item.name;
          if (contentInput) contentInput.value = item.text;
          if (actionSelect) actionSelect.value = item.action;
          if (kindSelect) kindSelect.value = "daily";
          syncActionFields();
          syncScheduleFields();
        });
      });
    }
  }

  function runState(run) {
    if (run.status === "ok") return { state: "succeeded", label: "已完成" };
    if (run.status === "failed") return { state: "failed", label: "执行失败" };
    if (run.status === "skipped") return { state: "skipped", label: "已跳过" };
    return { state: "unknown", label: "状态未知" };
  }

  function renderRuns(runs) {
    return runs.map(function (run) {
      var verdict = runState(run);
      var output = run.output == null ? "" : String(run.output).trim();
      var preview = output.length > 240 ? output.slice(0, 240) + "…" : output;
      return '<li class="owo-aut-run" data-state="' + verdict.state + '">' +
        '<span class="owo-aut-run-dot" aria-hidden="true"></span>' +
        '<div class="owo-aut-run-body"><div class="owo-aut-run-meta"><strong>' + verdict.label + '</strong>' +
        '<time class="panel-num">' + escText(localTime(run.at)) + '</time></div>' +
        (preview ? '<p>' + escText(preview) + '</p>' : "") + '</div></li>';
    }).join("");
  }

  function loadDetailRuns(task) {
    var box = el("owo-aut-detail-runs");
    if (!box) return;
    var taskId = String(task.id);
    box.innerHTML = '<li class="sub" role="status">正在加载执行记录…</li>';
    readRows(H, "/automations/runs?task_id=" + encodeURIComponent(taskId) + "&limit=20", "runs")
      .then(function (runs) {
        if (!panelMounted || selectedTaskId !== taskId || !el("owo-aut-detail-runs")) return;
        box.innerHTML = runs.length ? renderRuns(runs) :
          '<li class="sub">还没有执行记录，到点触发后会显示在这里。</li>';
      })
      .catch(function (error) {
        if (!panelMounted || selectedTaskId !== taskId) return;
        box.innerHTML = '<li class="sub">执行记录加载失败：' +
          escText(H.friendlyError ? H.friendlyError(error) : String(error)) + '</li>';
      });
  }

  function renderDetail(task) {
    var detail = el("owo-aut-detail");
    if (!detail) return;
    if (!task) { renderDetailIdle(); return; }
    var content = escText(task.action && (task.action.text || task.action.prompt) || "服务端未提供内容");
    var desc = task.action && task.action.kind === "reminder" ? "到点向工作台推送一条提醒。" : "到点让 Agent 执行下面的只读任务。";
    detail.innerHTML =
      '<article class="owo-aut-card">' +
      '<header class="owo-aut-card-head"><div class="owo-aut-card-title"><h3>' + escText(task.name) + '</h3>' +
      '<p class="sub">' + escText(desc) + '</p></div>' +
      '<span class="owo-aut-state" data-state="' + (task.enabled ? "enabled" : "paused") + '">' +
      (task.enabled ? "已开启" : "已暂停") + '</span></header>' +
      '<dl class="owo-aut-facts">' +
      '<div><dt>执行计划</dt><dd title="' + escText(scheduleTitle(task.schedule)) + '">' + escText(describeSchedule(task.schedule)) + '</dd></div>' +
      '<div><dt>下次执行</dt><dd class="panel-num">' + escText(nextRunText(task)) + '</dd></div>' +
      '<div><dt>上次执行</dt><dd>' + (task.last_run_at ? escText(localTime(task.last_run_at)) : "尚未执行") + '</dd></div>' +
      '</dl>' +
      '<section class="owo-aut-block"><h4>' + (task.action && task.action.kind === "reminder" ? "提醒内容" : "任务指令") + '</h4>' +
      '<p class="owo-aut-instruction">' + content + '</p></section>' +
      '<section class="owo-aut-block"><div class="owo-aut-block-head"><h4>执行历史</h4>' +
      '<span class="sub">最近 20 次</span></div>' +
      '<ul class="owo-aut-runs" id="owo-aut-detail-runs"><li class="sub">正在加载…</li></ul></section>' +
      '<footer class="owo-aut-card-actions">' +
      '<button type="button" class="owo-aut-primary" data-aut-detail-edit>编辑</button>' +
      '<button type="button" class="owo-aut-ghost" data-aut-detail-toggle data-core-action>' + (task.enabled ? "暂停" : "启用") + '</button>' +
      '<button type="button" class="owo-aut-ghost danger" data-aut-detail-delete data-core-action>删除</button>' +
      '</footer></article>';
    var controls = rowControls[String(task.id)];
    var toggle = detail.querySelector("[data-aut-detail-toggle]");
    var remove = detail.querySelector("[data-aut-detail-delete]");
    var edit = detail.querySelector("[data-aut-detail-edit]");
    if (toggle && controls) toggle.addEventListener("click", function () { controls.toggle.click(); });
    if (remove && controls) remove.addEventListener("click", function () { controls.remove.click(); });
    if (edit) edit.addEventListener("click", function () { showEditor(true, task); });
    loadDetailRuns(task);
  }

  function syncScheduleFields() {
    var kindControl = el("owo-aut-kind");
    var selected = kindControl ? kindControl.value : "daily";
    ["daily", "interval", "weekly", "monthly", "oneshot"].forEach(function (kind) {
      var group = rootEl.querySelector('[data-schedule-group="' + kind + '"]');
      var input = el(kind === "daily" ? "owo-aut-daily" : kind === "interval" ? "owo-aut-interval" : kind === "oneshot" ? "owo-aut-oneshot" : null);
      var active = kind === selected;
      if (group) group.hidden = !active;
      if (input) {
        input.disabled = !active;
        input.required = active;
      }
    });
  }

  function formatOffset(offsetMinutes) {
    if (!Number.isInteger(offsetMinutes) || Math.abs(offsetMinutes) > 14 * 60) {
      throw new Error("本地时区偏移无效");
    }
    var sign = offsetMinutes <= 0 ? "+" : "-";
    var absolute = Math.abs(offsetMinutes);
    return sign + String(Math.floor(absolute / 60)).padStart(2, "0") + ":" +
      String(absolute % 60).padStart(2, "0");
  }

  function buildSchedule(kind, value, offsetMinutes) {
    var rawValue = String(value == null ? "" : value).trim();
    if (kind === "interval") {
      if (!/^\d+$/.test(rawValue)) throw new Error("间隔需为正整数（秒）");
      var seconds = Number(rawValue);
      if (!Number.isSafeInteger(seconds) || seconds <= 0) throw new Error("间隔需为正整数（秒）");
      return { kind: "interval", every_secs: seconds };
    }
    // 每周/每月用固定间隔落地：核心只支持 interval/daily/one_shot。
    if (kind === "weekly") return { kind: "interval", every_secs: 7 * 86400 };
    if (kind === "monthly") return { kind: "interval", every_secs: 30 * 86400 };
    if (kind === "daily") {
      if (!/^(?:[01]\d|2[0-3]):[0-5]\d$/.test(rawValue)) throw new Error("每天时间需使用 HH:MM 格式");
      return { kind: "daily", time: rawValue, timezone: "local" };
    }
    if (kind === "oneshot") {
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(rawValue)) {
        throw new Error("请选择有效的本地日期和时间");
      }
      var localDate = new Date(rawValue);
      if (!Number.isFinite(localDate.getTime()) ||
          localDate.getFullYear() !== Number(rawValue.slice(0, 4)) ||
          localDate.getMonth() + 1 !== Number(rawValue.slice(5, 7)) ||
          localDate.getDate() !== Number(rawValue.slice(8, 10)) ||
          localDate.getHours() !== Number(rawValue.slice(11, 13)) ||
          localDate.getMinutes() !== Number(rawValue.slice(14, 16))) {
        throw new Error("请选择有效的本地日期和时间");
      }
      var offset = offsetMinutes == null ? localDate.getTimezoneOffset() : offsetMinutes;
      return { kind: "one_shot", at: rawValue + ":00" + formatOffset(offset) };
    }
    throw new Error("未知的触发方式");
  }

  function formatInterval(seconds) {
    var every = Number(seconds);
    if (!Number.isFinite(every) || every <= 0) return "间隔未知";
    if (every % 86400 === 0) return "每 " + every / 86400 + " 天";
    if (every % 3600 === 0) return "每 " + every / 3600 + " 小时";
    if (every % 60 === 0) return "每 " + every / 60 + " 分钟";
    return "每 " + every + " 秒";
  }

  /// 触发方式可读化。
  function describeSchedule(schedule) {
    if (!schedule || typeof schedule !== "object") return "";
    if (schedule.kind === "interval") return formatInterval(schedule.every_secs);
    if (schedule.kind === "daily") {
      var zone = schedule.timezone === "local" ? "本地时间" : "UTC";
      return "每天 " + (schedule.time || "") + " " + zone;
    }
    if (schedule.kind === "one_shot") {
      return "单次 " + localTime(schedule.at) + "（本地时间）";
    }
    return JSON.stringify(schedule);
  }

  /// 动作类型可读化（提醒 / 跑只读任务）。
  function describeAction(action) {
    if (!action || typeof action !== "object") return "动作未知";
    if (action.kind === "run_prompt") return "只读 Agent";
    if (action.kind === "reminder") return "工作台提醒";
    return String(action.kind || "动作未知");
  }

  /// 下次触发：按调度规则从最近触发时间向前推算，不伪造服务端数据。
  function nextRunText(task, now) {
    if (!task.enabled) return "已暂停";
    var schedule = task.schedule;
    if (!schedule || typeof schedule !== "object") return "时间未知";
    var reference = Number.isFinite(now) ? now : Date.now();
    if (schedule.kind === "interval") {
      var every = Number(schedule.every_secs) * 1000;
      if (!Number.isFinite(every) || every <= 0) return "时间未知";
      var last = Date.parse(task.last_run_at || "");
      if (!Number.isFinite(last)) return "启用后开始";
      var next = last + every;
      if (next <= reference) next += Math.ceil((reference - next) / every) * every;
      return localTime(new Date(next).toISOString());
    }
    if (schedule.kind === "daily") {
      var parts = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(String(schedule.time || ""));
      if (!parts) return "时间未知";
      var candidate;
      if (schedule.timezone === "local") {
        var base = new Date(reference);
        candidate = new Date(base.getFullYear(), base.getMonth(), base.getDate(), Number(parts[1]), Number(parts[2])).getTime();
        if (candidate <= reference) {
          var tomorrow = new Date(base.getFullYear(), base.getMonth(), base.getDate() + 1, Number(parts[1]), Number(parts[2]));
          candidate = tomorrow.getTime();
        }
        return localTime(new Date(candidate).toISOString());
      }
      var utcBase = new Date(reference);
      candidate = Date.UTC(utcBase.getUTCFullYear(), utcBase.getUTCMonth(), utcBase.getUTCDate(), Number(parts[1]), Number(parts[2]));
      if (candidate <= reference) candidate += 86400000;
      return localTime(new Date(candidate).toISOString());
    }
    if (schedule.kind === "one_shot") {
      var at = Date.parse(schedule.at || "");
      if (!Number.isFinite(at)) return "时间未知";
      return at <= reference ? "已过期" : localTime(schedule.at);
    }
    return "时间未知";
  }

  function scheduleTitle(schedule) {
    if (!schedule || typeof schedule !== "object") return "服务端未提供调度规则";
    try { return "原始调度规则：" + JSON.stringify(schedule); }
    catch (error) { return "原始调度规则不可序列化"; }
  }

  function skeletonRows(count) {
    var rows = "";
    for (var i = 0; i < count; i += 1) rows += '<div class="panel-skeleton owo-aut-skeleton-row"></div>';
    return rows;
  }

  function buildAction(kind, value) {
    var content = String(value == null ? "" : value).trim();
    if (!content) throw new Error("内容不能为空");
    if (kind === "reminder") return { kind: "reminder", text: content };
    if (kind === "run_prompt") return { kind: "run_prompt", prompt: content };
    throw new Error("未知的自动化动作");
  }

  function syncActionFields() {
    var action = el("owo-aut-action");
    var label = el("owo-aut-content-label");
    var content = el("owo-aut-content");
    var hint = el("owo-aut-action-hint");
    if (!action || !content) return;
    var runPrompt = action.value === "run_prompt";
    if (label) label.textContent = runPrompt ? "任务指令" : "提醒内容";
    content.placeholder = runPrompt
      ? "描述到点后要查询、巡检或总结的内容"
      : "到点显示的提醒文案";
    if (hint) hint.textContent = runPrompt
      ? "定时任务无人值守，只读模式，不会修改文件或执行命令。"
      : "提醒只会推送到工作台，不会调用模型。";
  }

  function renderListLoadError(list, error, retryKind) {
    if (!list) return;
    var detail = H.friendlyError ? H.friendlyError(error) : (error && error.message) || String(error);
    var message = retryKind === "tasks" ? "自动化任务加载失败：" : "提醒加载失败：";
    list.innerHTML =
      '<li class="owo-aut-load-error" role="alert"><span>' +
      escText(message + detail) +
      '</span><button type="button" class="owo-aut-retry" data-aut-retry="' +
      retryKind +
      '">重试</button></li>';
  }

  function refreshTasks() {
    var request = ++taskRefreshGeneration;
    var owner = panelGeneration;
    var list = el("owo-aut-list");
    tasksAvailable = false;
    if (list) list.innerHTML = '<li class="sub" role="status">正在加载自动化任务…</li>';
    var api = H;
    return readRows(api, "/automations", "tasks")
      .then(function (tasks) {
        if (request !== taskRefreshGeneration || owner !== panelGeneration) return;
        cachedTasks = tasks;
        tasksAvailable = true;
        renderTasks(cachedTasks);
        return true;
      })
      .catch(function (error) {
        if (request !== taskRefreshGeneration || owner !== panelGeneration) return;
        renderListLoadError(el("owo-aut-list"), error, "tasks");
        return false;
      });
  }

  function refreshReminders() {
    var request = ++reminderRefreshGeneration;
    var owner = panelGeneration;
    var list = el("owo-aut-reminders");
    if (list) list.innerHTML = '<li class="owo-aut-loading" role="status" aria-label="正在加载提醒">' + skeletonRows(2) + '</li>';
    var api = H;
    return readRows(api, "/automations/reminders", "reminders")
      .then(function (reminders) {
        if (request !== reminderRefreshGeneration || owner !== panelGeneration) return;
        renderReminders(reminders);
      })
      .catch(function (error) {
        if (request !== reminderRefreshGeneration || owner !== panelGeneration) return;
        renderListLoadError(el("owo-aut-reminders"), error, "reminders");
      });
  }

  function refresh() {
    if (!panelMounted) return Promise.resolve();
    return Promise.all([refreshTasks(), refreshReminders()]);
  }

  function bindListRetry(listId, retryKind) {
    var list = el(listId);
    if (!list) return;
    list.addEventListener("click", function (event) {
      var target = event.target && event.target.closest ? event.target : null;
      if (!target) return;
      var newButton = target.closest("[data-aut-new]");
      if (newButton) { event.preventDefault(); showEditor(true, null); return; }
      var button = target.closest('[data-aut-retry="' + retryKind + '"]');
      if (!button) return;
      event.preventDefault();
      if (retryKind === "tasks") refreshTasks();
      else refreshReminders();
    });
  }

  function filterTasks(tasks) {
    return tasks.filter(function (task) {
      return (!taskFilter || (taskFilter === "enabled" ? task.enabled === true : task.enabled === false)) &&
        (!taskQuery || [task.name, describeSchedule(task.schedule), describeAction(task.action), task.action && (task.action.text || task.action.prompt)].join(" ").toLocaleLowerCase().indexOf(taskQuery.toLocaleLowerCase()) !== -1);
    });
  }

  function appendGroupHeader(title, count, collapsed) {
    var list = el("owo-aut-list");
    var head = document.createElement("li");
    head.className = "owo-aut-group";
    head.innerHTML = '<button type="button" class="owo-aut-group-head" aria-expanded="' + String(!collapsed) + '">' +
      ICONS.chevron + '<span>' + escText(title) + '</span>' +
      '<span class="owo-aut-count panel-num">' + count + '</span></button>';
    head.addEventListener("click", function () {
      pausedOpen = !pausedOpen;
      renderTasks(cachedTasks);
    });
    list.appendChild(head);
  }

  function renderTasks(tasks) {
    var list = el("owo-aut-list");
    if (!list) return;
    rowControls = Object.create(null);
    list.innerHTML = "";
    if (!tasks.length) {
      list.innerHTML = '<li class="owo-aut-empty-list">还没有定时任务<br><span class="sub">右侧挑一个模板，或者点上方新建。</span></li>';
      selectedTaskId = "";
      renderDetail(null);
      return;
    }
    var visible = filterTasks(tasks);
    if (!visible.length) { list.innerHTML = '<li class="sub owo-aut-empty-list">没有符合条件的任务</li>'; return; }
    var enabled = visible.filter(function (task) { return task.enabled; });
    var paused = visible.filter(function (task) { return !task.enabled; });
    var collapsed = !pausedOpen && !taskQuery && enabled.length > 0;
    if (enabled.length) {
      appendGroupHeader("已开启", enabled.length, false);
      enabled.forEach(renderTaskRow);
    }
    if (paused.length) {
      appendGroupHeader("已暂停", paused.length, collapsed);
      if (!collapsed) paused.forEach(renderTaskRow);
    }
    var selected = cachedTasks.filter(function (item) { return String(item.id) === selectedTaskId; })[0] || null;
    renderDetail(selected);
  }

  function renderTaskRow(task) {
    var list = el("owo-aut-list");
    if (!list) return;
    var li = document.createElement("li");
    li.className = "owo-aut-task";
    li.setAttribute("data-task-id", String(task.id));
    li.innerHTML =
      '<button type="button" class="owo-aut-row" data-aut-open="' + escText(String(task.id)) + '" aria-pressed="' + (String(task.id) === selectedTaskId) + '">' +
      '<span class="owo-aut-dot" data-state="' + (task.enabled ? "enabled" : "paused") + '" aria-hidden="true"></span>' +
      '<span class="owo-aut-row-text"><strong>' + escText(task.name) + '</strong>' +
      '<span class="sub">' + escText(describeSchedule(task.schedule)) + ' · 下次 ' + escText(nextRunText(task)) + '</span></span></button>';
    if (String(task.id) === selectedTaskId) li.classList.add("is-active");
    li.querySelector("[data-aut-open]").addEventListener("click", function () { selectTask(task.id); });

    var toggleBtn = document.createElement("button");
    toggleBtn.setAttribute("data-core-action", "true");
    var toggleKey = String(task.id);
    var idleText = task.enabled ? "暂停" : "启用";
    setButtonBusy(toggleBtn, !!(taskToggleBusy[toggleKey] || taskDeleteBusy[toggleKey]), idleText, "处理中…");
    toggleBtn.addEventListener("click", async function (event) {
      event.stopPropagation();
      if (taskToggleBusy[toggleKey] || taskDeleteBusy[toggleKey]) return;
      taskToggleBusy[toggleKey] = true;
      var owner = panelGeneration, scope = hostScope, api = H, locks = taskToggleBusy;
      setButtonBusy(deleteBtn, true, "删除", "处理中…");
      setButtonBusy(toggleBtn, true, idleText, "处理中…");
      try {
        var result = await api.call("/automations/" + encodeURIComponent(task.id) + "/toggle", { method: "POST" });
        if (current(owner)) setStatus((result && typeof result.enabled === "boolean" ? (result.enabled ? "已启用：" : "已暂停：") : "状态已更新：") + task.name, false);
      } catch (error) {
        if (owner === panelGeneration) explainActionError("自动化启停", error);
      } finally {
        // Hold both actions until the authoritative state has been reread.
        if (panelMounted && scope === hostScope) await refreshTasks();
        delete locks[toggleKey];
        setButtonBusy(toggleBtn, false, idleText);
        setButtonBusy(deleteBtn, false, "删除");
        if (panelMounted && scope === hostScope && tasksAvailable) renderTasks(cachedTasks);
      }
    });

    // 行内执行记录（详情页也会展示完整历史）。
    var runsBtn = document.createElement("button");
    runsBtn.textContent = "记录";
    var runsGeneration = 0;
    var activeRunsBox = null;
    async function loadRuns(box) {
      var request = ++runsGeneration;
      var owner = panelGeneration;
      activeRunsBox = box;
      box.innerHTML = '<li class="owo-aut-loading" role="status" aria-label="正在加载执行记录">' + skeletonRows(2) + '</li>';
      try {
        var runs = await readRows(H, "/automations/runs?task_id=" + encodeURIComponent(task.id) + "&limit=10", "runs");
        if (request !== runsGeneration || activeRunsBox !== box || owner !== panelGeneration) return;
        box.innerHTML = '<li class="owo-aut-runs-caption">最近 10 次执行</li>';
        if (!runs.length) {
          box.innerHTML = '<li class="sub">尚无执行记录（到点触发后可见）</li>';
          return;
        }
        runs.forEach(function (run) {
          var item = document.createElement("li");
          var verdict = runState(run);
          item.innerHTML =
            '<span class="owo-aut-run-head"><span class="owo-wf-state" data-state="' + verdict.state + '">' + verdict.label + '</span><time class="panel-num">' + escText(localTime(run.at)) + '（本地时间）</time></span><pre>' +
            escText(run.output == null ? "（无输出）" : String(run.output)) + "</pre>";
          box.appendChild(item);
        });
      } catch (error) {
        if (request !== runsGeneration || activeRunsBox !== box || owner !== panelGeneration) return;
        box.innerHTML = "";
        var errorItem = document.createElement("li");
        errorItem.className = "owo-aut-runs-error";
        errorItem.setAttribute("role", "alert");
        var errorText = document.createElement("span");
        errorText.textContent = "读取执行记录失败：" + (H.friendlyError ? H.friendlyError(error) : error.message || String(error));
        var retry = document.createElement("button");
        retry.type = "button";
        retry.textContent = "重试";
        retry.setAttribute("aria-label", "重新加载执行记录");
        retry.addEventListener("click", function () { loadRuns(box); });
        errorItem.appendChild(errorText);
        errorItem.appendChild(retry);
        box.appendChild(errorItem);
      }
    }
    runsBtn.addEventListener("click", function (event) {
      event.stopPropagation();
      var existing = li.querySelector(".automation-runs");
      if (existing) {
        runsGeneration++;
        delete openRuns[String(task.id)];
        runsBtn.setAttribute("aria-expanded", "false");
        activeRunsBox = null;
        existing.remove();
        runsBtn.textContent = "记录";
        return;
      }
      openRuns[String(task.id)] = true;
      runsBtn.setAttribute("aria-expanded", "true");
      var box = document.createElement("ul");
      box.className = "list automation-runs";
      box.setAttribute("aria-label", "最近 10 次执行");
      li.appendChild(box);
      runsBtn.textContent = "收起";
      loadRuns(box);
    });

    var deleteBtn = document.createElement("button");
    deleteBtn.className = "danger";
    deleteBtn.setAttribute("data-core-action", "true");
    var deleteKey = String(task.id);
    setButtonBusy(deleteBtn, !!(taskDeleteBusy[deleteKey] || taskToggleBusy[deleteKey]), "删除", "处理中…");
    deleteBtn.addEventListener("click", async function (event) {
      event.stopPropagation();
      if (taskDeleteBusy[deleteKey] || taskToggleBusy[deleteKey]) return;
      taskDeleteBusy[deleteKey] = true;
      var owner = panelGeneration, scope = hostScope, api = H, locks = taskDeleteBusy;
      setButtonBusy(toggleBtn, true, idleText, "处理中…");
      setButtonBusy(deleteBtn, true, "删除", "等待确认…");
      try {
        var confirmed = await api.confirm({
          title: "删除自动化任务",
          message: "确定删除“" + task.name + "”？此操作无法撤销。",
          confirmText: "删除任务",
          kind: "danger",
        });
        if (!confirmed || !current(owner) || scope !== hostScope) return;
        setButtonBusy(deleteBtn, true, "删除", "删除中…");
        await api.del("/automations/" + encodeURIComponent(task.id));
        if (owner === panelGeneration) setStatus("已删除：" + task.name, false);
      } catch (error) {
        if (owner === panelGeneration) explainActionError("删除自动化", error);
      } finally {
        if (panelMounted && scope === hostScope) await refreshTasks();
        delete locks[deleteKey];
        setButtonBusy(deleteBtn, false, "删除");
        setButtonBusy(toggleBtn, false, idleText);
        if (panelMounted && scope === hostScope && tasksAvailable) renderTasks(cachedTasks);
      }
    });

    var editBtn = document.createElement("button");
    editBtn.textContent = "编辑";
    editBtn.addEventListener("click", function (event) {
      event.stopPropagation();
      showEditor(true, task);
    });

    var actions = document.createElement("div");
    actions.className = "owo-aut-task-actions";
    actions.appendChild(toggleBtn);
    actions.appendChild(runsBtn);
    actions.appendChild(deleteBtn);
    actions.appendChild(editBtn);
    li.appendChild(actions);
    list.appendChild(li);
    runsBtn.setAttribute("aria-expanded", "false");
    rowControls[String(task.id)] = { toggle: toggleBtn, runs: runsBtn, remove: deleteBtn, edit: editBtn };
    if (openRuns[String(task.id)]) runsBtn.click();
  }

  /// 左侧选中一条任务：右侧换成详情卡；新建表单打开时先收起。
  function selectTask(taskId) {
    selectedTaskId = String(taskId || "");
    showEditor(false);
    var list = el("owo-aut-list");
    if (list && typeof list.querySelectorAll === "function") {
      Array.prototype.forEach.call(list.querySelectorAll(".owo-aut-task"), function (item) {
        var active = item.getAttribute("data-task-id") === selectedTaskId;
        item.classList.toggle("is-active", active);
        var trigger = item.querySelector("[data-aut-open]");
        if (trigger) trigger.setAttribute("aria-pressed", String(active));
      });
    }
    var task = cachedTasks.filter(function (item) { return String(item.id) === selectedTaskId; })[0] || null;
    renderDetail(task);
  }

  function renderReminders(reminders) {
    var list = el("owo-aut-reminders");
    if (!list) return;
    list.innerHTML = "";
    if (!reminders.length) {
      list.innerHTML = '<li class="owo-aut-reminders-empty"><span class="sub">暂无提醒</span><strong>定时任务到点触发后，提醒会出现在这里。</strong></li>';
      return;
    }
    reminders.forEach(function (text) {
      var li = document.createElement("li");
      li.textContent = text;
      list.appendChild(li);
    });
  }

  async function createTask(event) {
    event.preventDefault();
    if (creatingTask) return;
    creatingTask = true;
    var owner = panelGeneration, scope = hostScope, api = H;
    var form = el("owo-aut-form");
    var submitButton = event.submitter || (form && form.querySelector('button[type="submit"]'));
    activeCreateButton = submitButton;
    var editingId = editingTaskId;
    setButtonBusy(submitButton, true, editingId ? "保存修改" : "创建任务", "正在保存…");
    try {
      var name = (el("owo-aut-name").value || "").trim();
      var kind = el("owo-aut-kind").value;
      var valueId = kind === "interval" ? "owo-aut-interval" : kind === "daily" ? "owo-aut-daily" : kind === "oneshot" ? "owo-aut-oneshot" : null;
      var value = valueId ? (el(valueId).value || "").trim() : "";
      var actionKind = el("owo-aut-action").value;
      var content = (el("owo-aut-content").value || "").trim();
      var enabledNode = el("owo-aut-enabled");
      var wantEnabled = !enabledNode || enabledNode.checked !== false;
      if (!name || !content) { setStatus("请填写任务名和内容", true); return; }
      var schedule;
      try {
        schedule = buildSchedule(kind, value);
      } catch (error) {
        setStatus(error.message, true);
        if (valueId) el(valueId).focus();
        return;
      }
      var action;
      try {
        action = buildAction(actionKind, content);
      } catch (error) {
        setStatus(error.message, true);
        el("owo-aut-content").focus();
        return;
      }
      try {
        captureDraft();
        var submittedDraft = JSON.stringify(draft);
        if (!current(owner) || scope !== hostScope) return;
        if (schedule.kind === "one_shot" && Date.parse(schedule.at) <= Date.now()) {
          setStatus("单次任务请选择未来的时间", true);
          if (valueId) el(valueId).focus();
          return;
        }
        var created = await api.post("/automations", { name: name, schedule: schedule, action: action });
        if (owner !== panelGeneration) return;
        if (!wantEnabled && created && created.id) {
          await api.call("/automations/" + encodeURIComponent(created.id) + "/toggle", { method: "POST" });
        }
        if (editingId && created && created.id) {
          try {
            await api.del("/automations/" + encodeURIComponent(editingId));
          } catch (error) {
            setStatus("已保存新配置，但旧任务删除失败：" + (H.friendlyError ? H.friendlyError(error) : error.message || error), true);
          }
        }
        if (owner !== panelGeneration) return;
        captureDraft();
        var unchanged = submittedDraft === JSON.stringify(draft);
        if (unchanged) {
          el("owo-aut-name").value = "";
          if (valueId) el(valueId).value = "";
          el("owo-aut-content").value = "";
          draft = {};
          captureDraft();
          showEditor(false);
        }
        if (editingId) selectedTaskId = created && created.id ? String(created.id) : "";
        setStatus((editingId ? "已更新：" : "已创建：") + name + (unchanged ? "" : "；新输入的草稿已保留"), false);
        await refresh();
      } catch (error) {
        if (owner === panelGeneration) setStatus("保存自动化失败：" + (error && error.message ? error.message : error), true);
      }
    } finally {
      if (scope === hostScope) {
        creatingTask = false;
        var buttonToRestore = activeCreateButton || submitButton;
        activeCreateButton = null;
        setButtonBusy(buttonToRestore, false, editingTaskId ? "保存修改" : "创建任务");
        if (panelMounted && owner !== panelGeneration) await refresh();
      }
    }
  }

  function dispose() {
    captureDraft();
    panelMounted = false;
    rootEl = null;
    panelGeneration += 1;
    taskRefreshGeneration += 1;
    reminderRefreshGeneration += 1;
    // Same-host mutations keep their locks until their original request settles.
  }

  function mount(root, helpers) {
    dispose();
    if (helpers) H = helpers;
    var scope = scopeOf(H);
    if (scope !== hostScope) {
      draft = {}; cachedTasks = []; tasksAvailable = false; taskQuery = ""; taskFilter = ""; activeTab = "tasks"; editorOpen = false;
      selectedTaskId = ""; rowControls = Object.create(null); editingTaskId = ""; pausedOpen = false; toolsOpen = false;
      creatingTask = false; clearingReminders = false; activeCreateButton = null;
      taskToggleBusy = Object.create(null); taskDeleteBusy = Object.create(null); openRuns = Object.create(null);
    }
    hostScope = scope;
    panelMounted = true;
    rootEl = root;
    root.innerHTML = nav();
    fieldIds.forEach(function (key) {
      var node = el("owo-aut-" + key);
      if (node && Object.prototype.hasOwnProperty.call(draft, key)) node.value = draft[key];
    });
    var form = el("owo-aut-form"), submitButton = form && form.querySelector('button[type="submit"]');
    if (creatingTask) {
      activeCreateButton = submitButton;
      setButtonBusy(submitButton, true, "创建任务", "正在创建…");
    }
    form.addEventListener("submit", createTask);
    form.addEventListener("input", captureDraft);
    form.addEventListener("change", captureDraft);
    renderDetail(null);
    bindListRetry("owo-aut-list", "tasks");
    bindListRetry("owo-aut-reminders", "reminders");
    el("owo-aut-kind").addEventListener("change", syncScheduleFields);
    el("owo-aut-action").addEventListener("change", syncActionFields);
    el("owo-aut-new").addEventListener("click", function () { showEditor(true, null); });
    var collapseButton = el("owo-aut-collapse");
    if (collapseButton) collapseButton.addEventListener("click", function () { showEditor(false); });
    el("owo-aut-cancel").addEventListener("click", function () { showEditor(false); });
    el("owo-aut-tab-tasks").addEventListener("click", function () { selectTab("tasks"); });
    el("owo-aut-tab-reminders").addEventListener("click", function () { selectTab("reminders"); });
    ["tasks", "reminders"].forEach(function (key) {
      el("owo-aut-tab-" + key).addEventListener("keydown", function (event) {
        if (["ArrowLeft", "ArrowRight", "Home", "End"].indexOf(event.key) === -1) return;
        event.preventDefault();
        var next = event.key === "Home" ? "tasks" : event.key === "End" ? "reminders" : key === "tasks" ? "reminders" : "tasks";
        selectTab(next); el("owo-aut-tab-" + next).focus();
      });
    });
    el("owo-aut-refresh").addEventListener("click", refresh);
    var searchToggle = el("owo-aut-search-toggle"), filterToggle = el("owo-aut-filter-toggle");
    function setToolsOpen(open, focusInput) {
      toolsOpen = !!open;
      var tools = el("owo-aut-tools");
      if (tools) tools.hidden = !toolsOpen;
      if (searchToggle) searchToggle.setAttribute("aria-expanded", String(toolsOpen));
      if (filterToggle) filterToggle.setAttribute("aria-expanded", String(toolsOpen));
      if (toolsOpen && focusInput) {
        var input = el("owo-aut-search");
        if (input) input.focus();
      }
    }
    if (searchToggle) searchToggle.addEventListener("click", function () { setToolsOpen(!toolsOpen, true); });
    if (filterToggle) filterToggle.addEventListener("click", function () { setToolsOpen(!toolsOpen, false); });
    var search = el("owo-aut-search"), filter = el("owo-aut-filter");
    search.value = taskQuery; filter.value = taskFilter;
    search.addEventListener("input", function () { taskQuery = search.value; if (tasksAvailable) renderTasks(cachedTasks); });
    filter.addEventListener("change", function () { taskFilter = filter.value; if (tasksAvailable) renderTasks(cachedTasks); });
    var editor = el("owo-aut-editor");
    if (editor) editor.hidden = !editorOpen;
    el("owo-aut-new").setAttribute("aria-expanded", String(editorOpen));
    selectTab(activeTab);
    syncScheduleFields();
    syncActionFields();
    setEditorTitle(false);
    var clear = el("owo-aut-clear");
    setButtonBusy(clear, clearingReminders, "清除提醒", "正在清除…");
    clear.addEventListener("click", async function () {
      if (clearingReminders) return;
      clearingReminders = true;
      var owner = panelGeneration, scope = hostScope, api = H;
      setButtonBusy(clear, true, "清除提醒", "等待确认…");
      try {
        var confirmed = await api.confirm({
          title: "清除工作台提醒",
          message: "确定清除全部工作台提醒？此操作无法撤销。",
          confirmText: "清除提醒",
          kind: "danger",
        });
        if (!confirmed || !current(owner) || scope !== hostScope) return;
        setButtonBusy(clear, true, "清除提醒", "正在清除…");
        await api.call("/automations/reminders/clear", { method: "POST" });
        if (current(owner)) setStatus("已清除提醒", false);
      } catch (error) {
        if (current(owner)) explainActionError("清除提醒", error);
      } finally {
        if (scope === hostScope) {
          clearingReminders = false;
          setButtonBusy(el("owo-aut-clear") || clear, false, "清除提醒");
          if (panelMounted) await refreshReminders();
        }
      }
    });
    refresh();
  }

  return {
    id: id,
    title: "自动化",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { buildSchedule: buildSchedule, buildAction: buildAction, createTask: createTask, describeSchedule: describeSchedule, validateRows: validateRows, scheduleToEditor: scheduleToEditor },
  };
})();
