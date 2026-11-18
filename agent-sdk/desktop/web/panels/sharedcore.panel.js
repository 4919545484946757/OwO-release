// SharedCore 面板：项目共享上下文、任务 DAG、并行批次、契约与最近事件。
// 只读 GET /sharedcore/state；状态由 Agent 的 SharedCore MCP 工具维护，本面板不写入。
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels.sharedcore = (function () {
    var ID = "sharedcore";
    var H = {};
    var rootEl = null;
    var generation = 0;

    function defaultEsc(text) {
      var div = document.createElement("div");
      div.textContent = text == null ? "" : String(text);
      return div.innerHTML;
    }
    function esc(text) {
      return H.esc ? H.esc(text) : defaultEsc(text);
    }
    function friendly(error) {
      if (H.friendlyError) return H.friendlyError(error);
      return String((error && error.message) || error || "未知错误");
    }
    function get(path) {
      if (H.get) return H.get(path);
      return window.OwoApi.stream(path).then(function (response) {
        if (!response.ok) {
          return response.text().then(function (body) {
            throw new Error(response.status + ": " + body);
          });
        }
        return response.json();
      });
    }

    function shell() {
      return (
        '<div class="owo-sc-head">' +
        '<div><strong>SharedCore</strong>' +
        '<div class="sub">多 Agent 共享核心上下文 · 任务 DAG · 并行批次</div></div>' +
        '<div class="owo-sc-actions">' +
        '<button type="button" class="owo-sc-refresh" data-sc-refresh="1">刷新</button></div>' +
        "</div>" +
        '<div class="owo-sc-meta sub" role="status" aria-live="polite">加载中…</div>' +
        '<div class="owo-sc-body"></div>'
      );
    }

    function chip(text, tone) {
      return '<span class="owo-sc-chip' + (tone ? " owo-sc-" + tone : "") + '">' + esc(text) + "</span>";
    }

    var STATUS_LABELS = {
      ready: "就绪",
      running: "进行中",
      done: "完成",
      failed: "失败",
      blocked: "阻塞",
      skipped: "跳过",
      cancelled: "取消",
      proposed: "提案中",
      accepted: "已接受",
    };

    function statusLabel(status) {
      return STATUS_LABELS[status] || status || "未知";
    }

    function renderProject(project) {
      var rules = (project.rules || [])
        .map(function (rule) {
          return '<li>' + esc(rule) + "</li>";
        })
        .join("");
      var architecture = project.architecture || {};
      var archRows = Object.keys(architecture)
        .map(function (key) {
          return "<li><code>" + esc(key) + "</code> " + esc(architecture[key]) + "</li>";
        })
        .join("");
      return (
        '<section class="owo-sc-card">' +
        '<div class="owo-sc-card-head"><strong>' + esc(project.name || "(未命名)") + "</strong>" +
        "</div>" +
        (project.goal ? '<p class="sub">' + esc(project.goal) + "</p>" : "") +
        '<div class="owo-sc-chips">' +
        chip("core v" + (project.core_version == null ? "-" : project.core_version)) +
        chip("revision " + (project.context_revision == null ? "-" : project.context_revision)) +
        chip("基线 " + (project.baseline_commit || "未指定")) +
        "</div>" +
        (rules ? '<div class="owo-sc-section"><span class="sub">项目规则</span><ul class="owo-sc-list">' + rules + "</ul></div>" : "") +
        (archRows ? '<div class="owo-sc-section"><span class="sub">架构</span><ul class="owo-sc-list">' + archRows + "</ul></div>" : "") +
        "</section>"
      );
    }

    function renderPlan(plan) {
      var batches = plan.batches || [];
      var batchHtml = batches
        .map(function (batch, index) {
          return (
            '<div class="owo-sc-batch"><span class="sub">批次 ' + (index + 1) + "</span>" +
            batch
              .map(function (id) {
                return chip(id);
              })
              .join("") +
            "</div>"
          );
        })
        .join("");
      var blocked = plan.blocked || {};
      var blockedHtml = Object.keys(blocked)
        .map(function (id) {
          return (
            '<div class="owo-sc-blocked"><code>' + esc(id) + "</code> 等 " +
            esc((blocked[id] || []).join("、")) + "</div>"
          );
        })
        .join("");
      var conflicts = plan.conflicts || [];
      var conflictHtml = conflicts
        .map(function (item) {
          return (
            '<div class="owo-sc-conflict"><code>' + esc(item.task) + "</code> 与 " +
            esc((item.overlaps || []).join("、")) + " 写范围重叠</div>"
          );
        })
        .join("");
      return (
        '<section class="owo-sc-card">' +
        '<div class="owo-sc-card-head"><strong>并行计划</strong>' +
        '<span class="sub">' + batches.length + " 批 · 就绪 " + (plan.ready || []).length + " · 阻塞 " + Object.keys(blocked).length + "</span></div>" +
        (batchHtml || '<div class="hint">没有可运行任务（全部完成或阻塞）</div>') +
        (blockedHtml ? '<div class="owo-sc-section"><span class="sub">阻塞</span>' + blockedHtml + "</div>" : "") +
        (conflictHtml ? '<div class="owo-sc-section"><span class="sub">写范围冲突</span>' + conflictHtml + "</div>" : "") +
        "</section>"
      );
    }

    function renderTasks(tasks) {
      var ids = Object.keys(tasks || {}).sort(function (left, right) {
        return String(tasks[left].id || left).localeCompare(String(tasks[right].id || right));
      });
      if (!ids.length) return "";
      var rows = ids
        .map(function (key) {
          var task = tasks[key] || {};
          var status = task.status || "unknown";
          var kind = status === "done" ? "ok" : status === "failed" ? "bad" : status === "blocked" ? "warn" : "";
          return (
            '<div class="owo-sc-task">' +
            chip(task.id || key) +
            chip(statusLabel(status), kind) +
            '<span class="owo-sc-task-title">' + esc(task.title || "") + "</span>" +
            (task.owner ? '<span class="sub">' + esc(task.owner) + "</span>" : "") +
            '<span class="sub">' + esc((task.write_scope || []).join("、") || "未限定范围") + "</span>" +
            (task.depends_on && task.depends_on.length
              ? '<span class="sub">依赖 ' + esc(task.depends_on.join("、")) + "</span>"
              : "") +
            "</div>"
          );
        })
        .join("");
      return (
        '<section class="owo-sc-card"><div class="owo-sc-card-head"><strong>任务</strong>' +
        '<span class="sub">' + ids.length + " 个</span></div>" + rows + "</section>"
      );
    }

    function renderContracts(contracts) {
      var ids = Object.keys(contracts || {});
      if (!ids.length) return "";
      var rows = ids
        .map(function (id) {
          var contract = contracts[id] || {};
          return (
            '<div class="owo-sc-row"><code>' + esc(id) + "</code>" +
            chip("v" + (contract.version == null ? "-" : contract.version)) +
            chip(statusLabel(contract.status || "unknown"), contract.status === "accepted" ? "ok" : "warn") +
            "</div>"
          );
        })
        .join("");
      return (
        '<section class="owo-sc-card"><div class="owo-sc-card-head"><strong>契约</strong>' +
        '<span class="sub">' + ids.length + " 个</span></div>" + rows + "</section>"
      );
    }

    function renderEvents(events) {
      if (!events || !events.length) return "";
      var shown = events.slice(-10);
      var rows = shown
        .reverse()
        .map(function (event) {
          return (
            '<div class="owo-sc-row"><code>' + esc(event.type || "?") + "</code>" +
            '<span class="sub">' + esc(event.resource || "") + "</span>" +
            '<span class="sub">' + esc(event.ts || "") + "</span></div>"
          );
        })
        .join("");
      return (
        '<section class="owo-sc-card"><div class="owo-sc-card-head"><strong>最近事件</strong>' +
        '<span class="sub">' + shown.length + " 条</span></div>" + rows + "</section>"
      );
    }

    function paint(root, data) {
      var meta = root.querySelector(".owo-sc-meta");
      var body = root.querySelector(".owo-sc-body");
      if (!body) return;
      if (!data || data.exists === false) {
        if (meta) meta.textContent = "尚未初始化";
        body.innerHTML =
          '<div class="owo-sc-empty">' +
          "<strong>这个工作区还没有 SharedCore 项目上下文</strong>" +
          '<p class="sub">启用 <code>owo-sharedcore</code> 插件后，在对话里说「用 SharedCore 并行开发…」，' +
          "主 Agent 会调用 <code>sharedcore_init</code> 建立项目、拆任务并计算并行批次。</p>" +
          '<p class="sub">状态目录：<code>' + esc((data && data.dir) || ".sharedcore") + "</code></p>" +
          "</div>";
        return;
      }
      var project = data.project || {};
      if (meta) {
        meta.textContent =
          "并行批次 " + ((data.plan && data.plan.batches) || []).length +
          " · 任务 " + Object.keys(data.tasks || {}).length +
          " · 事件 " + ((data.events || []).length);
      }
      body.innerHTML =
        renderProject(project) +
        renderPlan(data.plan || {}) +
        renderTasks(data.tasks || {}) +
        renderContracts(data.contracts || {}) +
        renderEvents(data.events || []);
    }

    function refresh(root) {
      var target = root || rootEl;
      if (!target) return Promise.resolve();
      var current = ++generation;
      var meta = target.querySelector(".owo-sc-meta");
      if (meta) meta.textContent = "加载中…";
      return get("/sharedcore/state")
        .then(function (data) {
          if (current !== generation || !rootEl) return;
          paint(target, data);
        })
        .catch(function (error) {
          if (current !== generation || !rootEl) return;
          var body = target.querySelector(".owo-sc-body");
          if (meta) meta.textContent = "读取失败";
          if (body) {
            body.innerHTML =
              '<div class="hint">读取 SharedCore 状态失败：' + esc(friendly(error)) + "</div>";
          }
        });
    }

    function mount(root, helpers) {
      H = helpers || {};
      rootEl = root;
      generation = 0;
      root.innerHTML = shell();
      root.addEventListener("click", function (event) {
        if (event.target.closest("[data-sc-refresh]")) refresh(root);
      });
      return refresh(root);
    }

    function unmount() {
      rootEl = null;
      generation += 1;
    }

    return { ID: ID, mount: mount, unmount: unmount, _test: { paint: paint, renderPlan: renderPlan } };
  })();
})();
