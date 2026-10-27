/* 定时任务：任务/提醒分页、搜索筛选、折叠草稿表单、启停/执行记录/删除。
 * 由工具视图的静态卡片迁移而来（此前 index.html 手写表单 + app.js 零散函数，
 * 与扩展面板体系割裂）；现在使用共享工作台布局与真实 API 状态。
 * 纯脚本 IIFE，注册 window.OwoPanels.automations；helpers 缺失时复用统一 ApiClient。
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
  var tasksAvailable = false;
  var taskQuery = "";
  var taskFilter = "";
  var activeTab = "tasks";
  var editorOpen = false;
  var draft = {};
  var openRuns = Object.create(null);
  var fieldIds = ["name", "kind", "interval", "daily", "oneshot", "action", "content"];

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

  function showEditor(open) {
    captureDraft();
    editorOpen = !!open;
    var editor = el("owo-aut-editor");
    if (editor) editor.hidden = !editorOpen;
    var trigger = el("owo-aut-new");
    if (trigger) trigger.setAttribute("aria-expanded", String(editorOpen));
    if (open) {
      selectTab("tasks");
      var input = el("owo-aut-name");
      if (input) input.focus({ preventScroll: true });
    }
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

  function nav() {
    return '<section data-panel="automations" data-layout="custom" class="owo-aut">' +
      '<p class="owo-aut-intro">核心保持运行时，定时任务才会触发。</p>' +
      '<div class="owo-aut-toolbar"><div role="tablist" aria-label="定时任务分区">' +
      '<button type="button" role="tab" id="owo-aut-tab-tasks" aria-controls="owo-aut-pane-tasks" aria-selected="true">任务</button>' +
      '<button type="button" role="tab" id="owo-aut-tab-reminders" aria-controls="owo-aut-pane-reminders" aria-selected="false">提醒</button></div>' +
      '<div class="owo-aut-toolbar-actions"><button type="button" id="owo-aut-refresh">刷新</button>' +
      '<button type="button" class="primary" id="owo-aut-new" aria-expanded="false" aria-controls="owo-aut-editor">新建定时任务</button></div></div>' +
      '<div id="owo-aut-status" role="status" aria-live="polite"></div>' +
      '<div id="owo-aut-pane-tasks" role="tabpanel" aria-labelledby="owo-aut-tab-tasks">' +
      '<section id="owo-aut-editor" class="owo-aut-editor" aria-labelledby="owo-aut-editor-title" hidden>' +
      '<div class="owo-aut-editor-heading"><h3 id="owo-aut-editor-title">新建定时任务</h3><button type="button" id="owo-aut-collapse">收起</button></div>' +
      '<form id="owo-aut-form" class="owo-aut-form">' +
      '<div class="tool-field"><label for="owo-aut-name">任务名</label><input id="owo-aut-name" placeholder="如 每日汇总" required maxlength="200"></div>' +
      '<div class="tool-field"><label for="owo-aut-action">动作</label><select id="owo-aut-action">' +
      '<option value="reminder">工作台提醒</option><option value="run_prompt">跑只读 Agent 任务</option></select></div>' +
      '<div class="tool-field"><label id="owo-aut-content-label" for="owo-aut-content">提醒内容</label><div>' +
      '<textarea id="owo-aut-content" placeholder="到点显示的提醒文案" rows="4" required></textarea>' +
      '<p id="owo-aut-action-hint" class="sub">提醒只会推送到工作台，不会调用模型。</p></div></div>' +
      '<div class="tool-field"><label for="owo-aut-kind">触发方式</label><select id="owo-aut-kind">' +
      '<option value="interval">按间隔</option><option value="daily">每天（UTC）</option><option value="oneshot">单次（本地时间）</option></select></div>' +
      '<div class="tool-field" data-schedule-group="interval"><label for="owo-aut-interval">间隔秒数</label><input id="owo-aut-interval" type="number" min="1" step="1" inputmode="numeric" required></div>' +
      '<div class="tool-field" data-schedule-group="daily" hidden><label for="owo-aut-daily">每天时间（UTC）</label><input id="owo-aut-daily" type="time" step="60" disabled></div>' +
      '<div class="tool-field" data-schedule-group="oneshot" hidden><label for="owo-aut-oneshot">执行时间（本地）</label><input id="owo-aut-oneshot" type="datetime-local" step="60" disabled></div>' +
      '<p class="sub owo-aut-time-hint">每天任务按 UTC 调度；单次任务按本地时间输入。</p>' +
      '<div class="owo-aut-form-actions"><button type="button" id="owo-aut-cancel">收起</button><button type="submit" class="primary" data-core-action>创建任务</button></div>' +
      '</form><p class="sub" data-core-action-hint>连接并授权本地核心后可创建自动化任务。</p></section>' +
      '<div class="owo-aut-filters"><input id="owo-aut-search" type="search" aria-label="搜索任务" placeholder="搜索任务">' +
      '<select id="owo-aut-filter" aria-label="任务状态"><option value="">全部状态</option><option value="enabled">已启用</option><option value="paused">已暂停</option></select></div>' +
      '<ul id="owo-aut-list" class="owo-aut-list"><li class="sub" role="status">加载中…</li></ul></div>' +
      '<div id="owo-aut-pane-reminders" role="tabpanel" aria-labelledby="owo-aut-tab-reminders" hidden>' +
      '<div class="owo-aut-reminder-heading"><h3>工作台提醒</h3><button type="button" id="owo-aut-clear" data-core-action>清除提醒</button></div>' +
      '<p class="sub" data-core-action-hint>连接并授权本地核心后可管理工作台提醒。</p>' +
      '<ul id="owo-aut-reminders" class="owo-aut-reminders"><li class="sub" role="status">加载中…</li></ul></div></section>';
  }

  function syncScheduleFields() {
    var kindControl = el("owo-aut-kind");
    var selected = kindControl ? kindControl.value : "interval";
    ["interval", "daily", "oneshot"].forEach(function (kind) {
      var group = rootEl.querySelector('[data-schedule-group="' + kind + '"]');
      var input = el(kind === "interval" ? "owo-aut-interval" : kind === "daily" ? "owo-aut-daily" : "owo-aut-oneshot");
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
    if (kind === "daily") {
      if (!/^(?:[01]\d|2[0-3]):[0-5]\d$/.test(rawValue)) throw new Error("每天时间需使用 HH:MM 格式");
      return { kind: "daily", time: rawValue };
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

  /// 触发方式可读化。
  function describeSchedule(schedule) {
    if (!schedule || typeof schedule !== "object") return "";
    if (schedule.kind === "interval") return "每 " + schedule.every_secs + " 秒";
    if (schedule.kind === "daily") return "每天 " + (schedule.time || "") + " UTC";
    if (schedule.kind === "one_shot") {
      return "单次 " + localTime(schedule.at) + "（本地时间）";
    }
    return JSON.stringify(schedule);
  }

  /// 动作类型可读化（提醒 / 跑只读任务）。
  function describeAction(action) {
    if (!action || typeof action !== "object") return "动作未知";
    if (action.kind === "run_prompt") return "跑只读 Agent 任务";
    if (action.kind === "reminder") return "工作台提醒";
    return String(action.kind || "动作未知");
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
    if (label) label.textContent = runPrompt ? "提示词（只读）" : "提醒内容";
    content.placeholder = runPrompt
      ? "描述要查询、巡检或总结的内容"
      : "到点显示的提醒文案";
    if (hint) hint.textContent = runPrompt
      ? "到点后调用 Agent；定时任务无人值守，只读模式，不会修改文件或执行命令。"
      : "提醒只会推送到工作台，不会调用模型。";
  }

  function renderListLoadError(list, error, retryKind) {
    if (!list) return;
    var detail = H.friendlyError ? H.friendlyError(error) : (error && error.message) || String(error);
    var message = retryKind === "tasks" ? "自动化任务加载失败：" : "提醒加载失败：";
    list.innerHTML =
      '<li class="owo-aut-load-error" role="alert"><span>' +
      H.esc(message + detail) +
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
    if (list) list.innerHTML = '<li class="sub" role="status">正在加载提醒…</li>';
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
      var button = event.target && event.target.closest
        ? event.target.closest('[data-aut-retry="' + retryKind + '"]')
        : null;
      if (!button) return;
      event.preventDefault();
      if (retryKind === "tasks") refreshTasks();
      else refreshReminders();
    });
  }

  function renderTasks(tasks) {
    var list = el("owo-aut-list");
    if (!list) return;
    list.innerHTML = "";
    if (!tasks.length) {
      list.innerHTML = '<li class="sub">暂无自动化任务</li>';
      return;
    }
    var visible = tasks.filter(function (task) {
      return (!taskFilter || (taskFilter === "enabled" ? task.enabled === true : task.enabled === false)) &&
        (!taskQuery || [task.name, describeSchedule(task.schedule), describeAction(task.action), task.action && (task.action.text || task.action.prompt)].join(" ").toLocaleLowerCase().indexOf(taskQuery.toLocaleLowerCase()) !== -1);
    });
    if (!visible.length) { list.innerHTML = '<li class="sub">没有符合条件的任务</li>'; return; }
    visible.slice().sort(function (a, b) { return (Date.parse(b.created_at || "") || 0) - (Date.parse(a.created_at || "") || 0); }).forEach(function (task) {
      var li = document.createElement("li");
      li.className = "owo-aut-task";
      li.innerHTML =
        '<div class="owo-aut-task-summary"><div class="owo-aut-title"><strong>' + H.esc(task.name) +
        '</strong><span class="owo-aut-state">' + (task.enabled ? "已启用" : "已暂停") + '</span></div><p class="sub">' +
        H.esc(describeSchedule(task.schedule)) + ' · ' + H.esc(describeAction(task.action)) +
        '</p><p class="sub">最近触发：' + (task.last_run_at ? H.esc(localTime(task.last_run_at)) + "（本地时间）" : "尚未触发") + '</p>' +
        '<details class="owo-aut-content"><summary>查看内容</summary><pre>' + H.esc(task.action && (task.action.text || task.action.prompt) || "服务端未提供内容") + '</pre></details></div>';

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

      // A8-1：展开执行记录（次日可查「跑没跑、结果如何」）。
      var runsBtn = document.createElement("button");
      runsBtn.textContent = "记录";
      var runsGeneration = 0;
      var activeRunsBox = null;
      async function loadRuns(box) {
        var request = ++runsGeneration;
        var owner = panelGeneration;
        activeRunsBox = box;
        box.innerHTML = '<li class="sub" role="status">正在加载执行记录…</li>';
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
            var when = localTime(run.at) + "（本地时间）";
            var verdict = run.status === "ok" ? "已完成" : run.status === "skipped" ? "已跳过" : run.status === "failed" ? "执行失败" : "状态未知";
            item.innerHTML =
              "<strong>" + H.esc(verdict) + " · " + H.esc(when) + '</strong><pre>' +
              H.esc(run.output == null ? "（无输出）" : String(run.output)) + "</pre>";
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

      var actions = document.createElement("div");
      actions.className = "owo-aut-task-actions";
      actions.appendChild(toggleBtn);
      actions.appendChild(runsBtn);
      actions.appendChild(deleteBtn);
      li.appendChild(actions);
      list.appendChild(li);
      runsBtn.setAttribute("aria-expanded", "false");
      if (openRuns[String(task.id)]) runsBtn.click();
    });
  }

  function renderReminders(reminders) {
    var list = el("owo-aut-reminders");
    if (!list) return;
    list.innerHTML = "";
    if (!reminders.length) {
      list.innerHTML = '<li class="sub">暂无提醒</li>';
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
    setButtonBusy(submitButton, true, "创建任务", "正在创建…");
    try {
      var name = (el("owo-aut-name").value || "").trim();
      var kind = el("owo-aut-kind").value;
      var valueId = kind === "interval" ? "owo-aut-interval" : kind === "daily" ? "owo-aut-daily" : "owo-aut-oneshot";
      var value = (el(valueId).value || "").trim();
      var actionKind = el("owo-aut-action").value;
      var content = (el("owo-aut-content").value || "").trim();
      if (!name || !content) { setStatus("请填写任务名和内容", true); return; }
      var schedule;
      try {
        schedule = buildSchedule(kind, value);
      } catch (error) {
        setStatus(error.message, true);
        el(valueId).focus();
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
          el(valueId).focus();
          return;
        }
        await api.post("/automations", { name: name, schedule: schedule, action: action });
        if (owner !== panelGeneration) return;
        captureDraft();
        var unchanged = submittedDraft === JSON.stringify(draft);
        if (unchanged) {
          el("owo-aut-name").value = "";
          el(valueId).value = "";
          el("owo-aut-content").value = "";
          draft = {};
          captureDraft();
          showEditor(false);
        }
        setStatus("已创建：" + name + (unchanged ? "" : "；新输入的草稿已保留"), false);
        await refresh();
      } catch (error) {
        if (owner === panelGeneration) setStatus("创建自动化失败：" + (error && error.message ? error.message : error), true);
      }
    } finally {
      if (scope === hostScope) {
        creatingTask = false;
        var buttonToRestore = activeCreateButton || submitButton;
        activeCreateButton = null;
        setButtonBusy(buttonToRestore, false, "创建任务");
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
    bindListRetry("owo-aut-list", "tasks");
    bindListRetry("owo-aut-reminders", "reminders");
    el("owo-aut-kind").addEventListener("change", syncScheduleFields);
    el("owo-aut-action").addEventListener("change", syncActionFields);
    el("owo-aut-new").addEventListener("click", function () { showEditor(true); });
    el("owo-aut-collapse").addEventListener("click", function () { showEditor(false); });
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
    var clear = el("owo-aut-clear");
    setButtonBusy(clear, clearingReminders, "清除提醒", "正在清除…");
    clear.addEventListener("click", async function () {
      if (clearingReminders) return;
      clearingReminders = true;
      var owner = panelGeneration, scope = hostScope, api = H;
      setButtonBusy(clear, true, "清除提醒", "正在清除…");
      try {
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
    _test: { buildSchedule: buildSchedule, buildAction: buildAction, createTask: createTask, describeSchedule: describeSchedule, validateRows: validateRows },
  };
})();
