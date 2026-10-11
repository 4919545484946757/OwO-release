// 控制面（P2 双节点网格）面板：节点注册/列表、任务提交/查询/取消/事件、审批响应。
// 纯脚本 IIFE 注册 window.OwoPanels.fleet；helpers 缺省时自建（fetch + esc + friendlyError）。
// 结果区一律结构化卡片/网格/时间线渲染（无裸 JSON）；异步渲染带"面板已卸载则跳过"防御。
(function () {
  "use strict";

  window.OwoPanels = window.OwoPanels || {};

  window.OwoPanels.fleet = (function () {
    var H = {};
    var sectionEl = null;
    var panelGeneration = 0;
    var nodesRequestGeneration = 0;
    var taskViewRequestGeneration = 0;
    var actionBusy = { register: false, submit: false, approval: false };

    function defaultGet(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) return r.json().catch(function () { return {}; }).then(function (body) {
          throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
        });
        return r.status === 204 ? null : r.json();
      });
    }
    function defaultPost(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) return r.json().catch(function () { return {}; }).then(function (body) {
          throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
        });
        return r.status === 204 ? null : r.json();
      });
    }
    function safePost(path, body) {
      try {
        return Promise.resolve(H.post(path, body));
      } catch (error) {
        return Promise.reject(error);
      }
    }
    function defaultEsc(s) {
      return String(s == null ? "" : s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function defaultFriendlyError(e) {
      return String((e && e.message) || e || "未知错误");
    }

    function nav() {
      return (
        '<section data-panel="fleet" class="owo-fleet-panel">' +
        '<div class="owo-fleet-tools">' +
        "<h3>节点注册</h3>" +
        '<div class="inline">' +
        '<input class="owo-fleet-node-id" placeholder="node_id（如 node-a）">' +
        '<input class="owo-fleet-node-worker" placeholder="worker（如 shell）">' +
        '<button class="owo-fleet-node-register" data-core-action>注册</button>' +
        "</div>" +
        '<div class="owo-fleet-node-result sub"></div>' +
        "</div>" +
        '<div class="owo-fleet-tools">' +
        "<h3>节点列表</h3>" +
        '<div class="inline"><button class="owo-fleet-nodes-refresh">刷新</button></div>' +
        '<div class="owo-fleet-nodes sub"></div>' +
        "</div>" +
        '<div class="owo-fleet-tools">' +
        "<h3>任务提交</h3>" +
        '<div class="inline">' +
        '<input class="owo-fleet-task-id" placeholder="task_id（如 t-1）">' +
        '<input class="owo-fleet-task-worker" placeholder="worker（如 node-a）">' +
        "</div>" +
        '<textarea class="owo-fleet-task-input" rows="3" spellcheck="false" placeholder=\'input JSON，如 {"q":1}\'></textarea>' +
        '<label class="inline"><input type="checkbox" class="owo-fleet-task-approval"> 需审批（approval_required）</label>' +
        '<div class="inline"><button class="owo-fleet-task-submit primary" data-core-action>提交</button></div>' +
        '<div class="owo-fleet-task-submit-result sub"></div>' +
        "</div>" +
        '<div class="owo-fleet-tools">' +
        "<h3>任务查询 / 取消 / 事件</h3>" +
        '<div class="inline">' +
        '<input class="owo-fleet-task-get-id" placeholder="task_id">' +
        '<button class="owo-fleet-task-get">查询</button>' +
        '<button class="owo-fleet-task-cancel" data-core-action>取消</button>' +
        '<button class="owo-fleet-task-events">事件</button>' +
        "</div>" +
        '<div class="owo-fleet-task-view sub"></div>' +
        "</div>" +
        '<div class="owo-fleet-tools">' +
        "<h3>审批响应</h3>" +
        '<div class="inline">' +
        '<input class="owo-fleet-approval-id" placeholder="task_id（审批任务）">' +
        '<select class="owo-fleet-approval-decision"><option value="approve">approve</option><option value="reject">reject</option></select>' +
        '<input class="owo-fleet-approval-by" placeholder="approved_by（如 owner）">' +
        '<button class="owo-fleet-approval-respond primary" data-core-action>裁决</button>' +
        "</div>" +
        '<div class="owo-fleet-approval-result sub"></div>' +
        "</div>" +
        "<style>" +
        ".owo-fleet-panel { display: flex; flex-direction: column; gap: 10px; }" +
        ".owo-fleet-tools { border: 1px solid var(--border); border-radius: 8px; padding: 10px; }" +
        ".owo-fleet-tools h3 { margin: 0 0 8px; font-size: 13px; color: var(--text-2); }" +
        ".owo-fleet-panel textarea { width: 100%; box-sizing: border-box; }" +
        ".owo-fleet-panel input { width: auto; }" +
        // 结果卡片（原 pre 直出 JSON → 结构化卡片）
        ".owo-fleet-node-result, .owo-fleet-nodes, .owo-fleet-task-view, .owo-fleet-task-submit-result, .owo-fleet-approval-result { margin-top: 8px; display: flex; flex-direction: column; gap: 8px; }" +
        ".owo-fleet-card { border: 1px solid var(--border); border-radius: 8px; padding: 8px 10px; background: var(--surface-2); display: flex; flex-direction: column; gap: 4px; }" +
        ".owo-fleet-card.owo-fleet-err { border-color: var(--red); background: var(--red-soft); color: var(--red); font-size: 12px; }" +
        ".owo-fleet-card-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }" +
        ".owo-fleet-meta { color: var(--text-3); font-size: 12px; }" +
        ".owo-fleet-badge { display: inline-block; padding: 1px 8px; border-radius: 999px; font-size:12px; line-height: 18px; background: var(--surface-3); color: var(--text-2); }" +
        ".owo-fleet-badge.ok { background: var(--green-soft); color: var(--green); }" +
        ".owo-fleet-badge.bad { background: var(--red-soft); color: var(--red); }" +
        ".owo-fleet-badge.warn { background: var(--yellow-soft); color: var(--yellow); }" +
        ".owo-fleet-badge.run { background: var(--accent-soft); color: var(--accent); }" +
        ".owo-fleet-chips { display: flex; flex-wrap: wrap; gap: 6px; }" +
        ".owo-fleet-chip { font-size:12px; padding: 1px 8px; border-radius: 999px; background: var(--surface-3); color: var(--text-2); }" +
        ".owo-fleet-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 8px; }" +
        ".owo-fleet-events { display: flex; flex-direction: column; gap: 6px; margin-top: 2px; }" +
        ".owo-fleet-event { display: flex; gap: 8px; align-items: baseline; border-left: 2px solid var(--border-strong); padding: 2px 0 2px 8px; }" +
        ".owo-fleet-event-kind { flex: none; font-size:12px; color: var(--accent); }" +
        ".owo-fleet-event-text { font-size: 12px; color: var(--text-2); word-break: break-all; }" +
        ".owo-fleet-approval { border: 1px dashed var(--border-strong); border-radius: 8px; padding: 8px 10px; display: flex; flex-direction: column; gap: 6px; }" +
        ".owo-fleet-kv { font-size: 12px; color: var(--text-2); }" +
        ".owo-fleet-kv strong { color: var(--text); margin-right: 4px; }" +
        "</style>" +
        "</section>"
      );
    }

    function mount(root, helpers) {
      dispose();
      H = helpers || {};
      H.baseUrl = H.baseUrl || (window.OwoPanels && window.OwoPanels.baseUrl) || "";
      H.get = H.get || defaultGet;
      H.post = H.post || defaultPost;
      H.esc = H.esc || defaultEsc;
      H.friendlyError = H.friendlyError || defaultFriendlyError;

      root.innerHTML = nav();
      sectionEl = root.querySelector(".owo-fleet-panel");
      var $ = function (sel) {
        return root.querySelector(sel);
      };

      $(".owo-fleet-node-register").addEventListener("click", function () {
        doRegister($(".owo-fleet-node-id").value, $(".owo-fleet-node-worker").value);
      });
      $(".owo-fleet-nodes-refresh").addEventListener("click", function () {
        listNodes();
      });
      $(".owo-fleet-task-submit").addEventListener("click", function () {
        doSubmit(
          $(".owo-fleet-task-id").value,
          $(".owo-fleet-task-worker").value,
          $(".owo-fleet-task-input").value,
          $(".owo-fleet-task-approval").checked
        );
      });
      $(".owo-fleet-task-get").addEventListener("click", function () {
        getTask($(".owo-fleet-task-get-id").value);
      });
      $(".owo-fleet-task-cancel").addEventListener("click", function () {
        cancelTask($(".owo-fleet-task-get-id").value);
      });
      $(".owo-fleet-task-events").addEventListener("click", function () {
        taskEvents($(".owo-fleet-task-get-id").value);
      });
      $(".owo-fleet-approval-respond").addEventListener("click", function () {
        respondApproval(
          $(".owo-fleet-approval-id").value,
          $(".owo-fleet-approval-decision").value,
          $(".owo-fleet-approval-by").value
        );
      });

      syncWriteButtons();
      listNodes();
    }

    function refresh() {
      listNodes();
    }

    // ---------- 渲染基础设施 ----------

    // 面板可能已被切换/卸载（异步返回时），此时静默跳过渲染
    function alive() {
      return sectionEl && document.contains(sectionEl);
    }

    // 向指定结果区写结构化 HTML（限面板内查找，避免误匹配）
    function syncWriteButtons() {
      if (!sectionEl) return;
      var buttons = [
        [".owo-fleet-node-register", actionBusy.register, "注册中…", "注册"],
        [".owo-fleet-task-submit", actionBusy.submit, "提交中…", "提交"],
        [".owo-fleet-approval-respond", actionBusy.approval, "处理中…", "裁决"],
      ];
      for (var i = 0; i < buttons.length; i++) {
        var button = sectionEl.querySelector(buttons[i][0]);
        if (!button) continue;
        button.disabled = buttons[i][1];
        button.textContent = buttons[i][1] ? buttons[i][2] : buttons[i][3];
      }
    }

    function render(sel, html) {
      if (!alive()) return;
      var el = sectionEl.querySelector(sel);
      if (el) el.innerHTML = html;
    }

    function errCard(text) {
      return '<div class="owo-fleet-card owo-fleet-err">' + H.esc(text) + "</div>";
    }

    // TransportStatus（snake_case）→ 徽章样式 + 中文标签
    function statusBadge(status) {
      var s = String(status || "");
      if (s === "succeeded") return '<span class="owo-fleet-badge ok">成功</span>';
      if (s === "failed") return '<span class="owo-fleet-badge bad">失败</span>';
      if (s === "cancelled") return '<span class="owo-fleet-badge">已取消</span>';
      if (s === "running") return '<span class="owo-fleet-badge run">运行中</span>';
      if (s === "awaiting_approval") return '<span class="owo-fleet-badge warn">待审批</span>';
      if (s === "pending") return '<span class="owo-fleet-badge warn">等待中</span>';
      return '<span class="owo-fleet-badge">' + H.esc(s || "未知") + "</span>";
    }

    // TransportEventKind → 中文标签
    function eventKindLabel(kind) {
      var k = String(kind || "");
      if (k === "progress") return "进展";
      if (k === "result") return "结果";
      if (k === "approval_requested") return "审批请求";
      if (k === "approval_granted") return "审批通过";
      if (k === "cancelled") return "已取消";
      return k || "事件";
    }

    // 任意 payload → 单行摘要（优先常见文本字段，否则截断 JSON）
    function brief(value) {
      if (value == null) return "—";
      if (typeof value === "string") return value;
      if (typeof value === "number" || typeof value === "boolean") return String(value);
      if (typeof value === "object") {
        var keys = ["message", "summary", "detail", "text", "note", "reason", "output"];
        for (var i = 0; i < keys.length; i++) {
          var v = value[keys[i]];
          if (typeof v === "string" && v) return v;
        }
        var s = JSON.stringify(value);
        return s.length > 160 ? s.slice(0, 160) + "…" : s;
      }
      return String(value);
    }

    function chip(text) {
      return '<span class="owo-fleet-chip">' + H.esc(text) + "</span>";
    }

    // 浏览器环境探测（CapabilityCard 必填 os/arch；枚举值须与后端 snake_case 对齐）
    function detectOs() {
      var p = "";
      if (navigator.userAgentData && navigator.userAgentData.platform) {
        p = navigator.userAgentData.platform;
      } else {
        p = navigator.platform || "";
      }
      if (/win/i.test(p)) return "windows";
      if (/mac/i.test(p)) return "mac_os";
      if (/linux/i.test(p)) return "linux";
      return "other";
    }
    function detectArch() {
      var ua = navigator.userAgent || "";
      if (/arm|aarch64/i.test(ua)) return "aarch64";
      return "x86_64";
    }

    // ---------- 节点注册 ----------

    function doRegister(nodeId, worker) {
      var owner = panelGeneration;
      if (!nodeId) return render(".owo-fleet-node-result", errCard("请填写 node_id"));
      if (actionBusy.register) return;
      actionBusy.register = true;
      syncWriteButtons();
      safePost("/fleet/nodes/register", {
        node_id: nodeId,
        card: {
          worker: worker || nodeId,
          os: detectOs(),
          arch: detectArch(),
          actions: ["shell"],
        },
      })
        .then(function (d) {
          if (owner !== panelGeneration || !alive()) return;
          var st = (d && d.status) || {};
          var healthy = st.healthy ? '<span class="owo-fleet-badge ok">健康</span>' : '<span class="owo-fleet-badge bad">异常</span>';
          render(
            ".owo-fleet-node-result",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head">' +
              '<span class="owo-fleet-badge ok">已注册</span>' +
              "<strong>" + H.esc((d && d.node_id) || nodeId) + "</strong>" +
              healthy +
              "</div>" +
              '<div class="owo-fleet-meta">租约纪元 ' +
              H.esc(d && d.lease_epoch != null ? "#" + d.lease_epoch : "—") +
              "</div>" +
              "</div>"
          );
          listNodes();
        })
        .catch(function (e) {
          if (owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-node-result", errCard("注册失败：" + H.friendlyError(e)));
        })
        .finally(function () {
          actionBusy.register = false;
          syncWriteButtons();
        });
    }

    // ---------- 节点列表 ----------

    function nodeCard(n) {
      var card = (n && n.card) || {};
      var state = n.lost
        ? '<span class="owo-fleet-badge bad">失联</span>'
        : n.healthy
        ? '<span class="owo-fleet-badge ok">健康</span>'
        : '<span class="owo-fleet-badge bad">异常</span>';
      var chips = "";
      if (card.worker) chips += chip("worker：" + card.worker);
      var actions = card.actions || [];
      for (var i = 0; i < actions.length; i++) chips += chip(actions[i]);
      var meta =
        "心跳 " + H.esc(n.heartbeats || 0) + " · 重启 " + H.esc(n.restarts || 0) +
        " · 租约 " + H.esc(n.lease_epoch != null ? "#" + n.lease_epoch : "—") +
        (n.registered ? " · 已注册" : "");
      return (
        '<div class="owo-fleet-card">' +
        '<div class="owo-fleet-card-head"><strong>' + H.esc(n.id || "?") + "</strong>" + state + "</div>" +
        (chips ? '<div class="owo-fleet-chips">' + chips + "</div>" : "") +
        '<div class="owo-fleet-meta">' + meta + "</div>" +
        "</div>"
      );
    }

    function listNodes() {
      var request = ++nodesRequestGeneration;
      var owner = panelGeneration;
      return H.get("/fleet/nodes")
        .then(function (d) {
          if (request !== nodesRequestGeneration || owner !== panelGeneration || !alive()) return;
          var nodes = (d && d.nodes) || [];
          if (!nodes.length) {
            render(".owo-fleet-nodes", '<div class="owo-fleet-meta">暂无节点，先在上方注册</div>');
            return;
          }
          var html = '<div class="owo-fleet-meta">共 ' + nodes.length + " 个节点</div>";
          html += '<div class="owo-fleet-grid">';
          for (var i = 0; i < nodes.length; i++) html += nodeCard(nodes[i] || {});
          html += "</div>";
          render(".owo-fleet-nodes", html);
        })
        .catch(function (e) {
          if (request !== nodesRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-nodes", errCard("列表失败：" + H.friendlyError(e)));
        });
    }

    // ---------- 任务提交 ----------

    function doSubmit(taskId, worker, inputText, approvalRequired) {
      var owner = panelGeneration;
      if (!taskId || !worker) return render(".owo-fleet-task-submit-result", errCard("请填写 task_id 与 worker"));
      var input = {};
      try {
        input = inputText.trim() ? JSON.parse(inputText) : { q: 1 };
      } catch (e) {
        return render(".owo-fleet-task-submit-result", errCard("input 不是合法 JSON：" + H.friendlyError(e)));
      }
      if (actionBusy.submit) return;
      actionBusy.submit = true;
      syncWriteButtons();
      safePost("/fleet/tasks/submit", {
        task_id: taskId,
        worker: worker,
        input: input,
        approval_required: !!approvalRequired,
      })
        .then(function (d) {
          if (owner !== panelGeneration || !alive()) return;
          render(
            ".owo-fleet-task-submit-result",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head">' +
              statusBadge(d && d.status) +
              "<strong>" + H.esc((d && d.task_id) || taskId) + "</strong>" +
              "</div>" +
              '<div class="owo-fleet-meta">幂等键 ' + H.esc((d && d.idempotency_key) || "—") + "</div>" +
              "</div>"
          );
        })
        .catch(function (e) {
          if (owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-submit-result", errCard("提交失败：" + H.friendlyError(e)));
        })
        .finally(function () {
          actionBusy.submit = false;
          syncWriteButtons();
        });
    }

    // ---------- 任务查询 / 取消 / 事件 ----------

    function eventLine(ev) {
      var text = brief(ev && ev.payload);
      var lineage = (ev && ev.lineage) || [];
      if (lineage.length) text += "（血缘 " + lineage.join(" → ") + "）";
      return (
        '<div class="owo-fleet-event">' +
        '<span class="owo-fleet-event-kind">' + H.esc(eventKindLabel(ev && ev.kind)) + "</span>" +
        '<span class="owo-fleet-event-text">' + H.esc(text) + "</span>" +
        "</div>"
      );
    }

    function eventsTimeline(events) {
      if (!events.length) return '<div class="owo-fleet-meta">暂无事件</div>';
      var html = "";
      for (var i = 0; i < events.length; i++) html += eventLine(events[i]);
      return '<div class="owo-fleet-events">' + html + "</div>";
    }

    function approvalCard(a) {
      if (!a) return "";
      var decided = a.decided;
      var badge = decided
        ? a.decision === "approved"
          ? '<span class="owo-fleet-badge ok">已批准</span>'
          : '<span class="owo-fleet-badge bad">已拒绝</span>'
        : '<span class="owo-fleet-badge warn">待裁决</span>';
      var evidence = "";
      var items = a.evidence || [];
      for (var i = 0; i < items.length; i++) {
        evidence += chip((items[i].kind || "evidence") + "：" + (items[i].summary || ""));
      }
      return (
        '<div class="owo-fleet-approval">' +
        '<div class="owo-fleet-card-head"><strong>审批</strong>' + badge + "</div>" +
        '<div class="owo-fleet-kv"><strong>步骤</strong>' + H.esc(a.step_id || "—") + "</div>" +
        (a.summary ? '<div class="owo-fleet-kv"><strong>摘要</strong>' + H.esc(a.summary) + "</div>" : "") +
        (a.impact_preview
          ? '<div class="owo-fleet-kv"><strong>影响预览</strong>' + H.esc(a.impact_preview) + "</div>"
          : "") +
        (a.owner_device
          ? '<div class="owo-fleet-kv"><strong>所有者设备</strong>' + H.esc(a.owner_device) + "</div>"
          : "") +
        (a.approved_by
          ? '<div class="owo-fleet-kv"><strong>裁决人</strong>' + H.esc(a.approved_by) + "</div>"
          : "") +
        (evidence ? '<div class="owo-fleet-chips">' + evidence + "</div>" : "") +
        "</div>"
      );
    }

    function taskCard(view) {
      return (
        '<div class="owo-fleet-card">' +
        '<div class="owo-fleet-card-head">' +
        "<strong>" + H.esc(view.task_id || "?") + "</strong>" +
        statusBadge(view.status) +
        "</div>" +
        '<div class="owo-fleet-chips">' +
        (view.worker ? chip("worker：" + view.worker) : "") +
        (view.correlation_id ? chip("correlation：" + view.correlation_id) : "") +
        "</div>" +
        eventsTimeline(view.events || []) +
        approvalCard(view.approval) +
        "</div>"
      );
    }

    function getTask(taskId) {
      if (!taskId) return render(".owo-fleet-task-view", errCard("请填写 task_id"));
      var request = ++taskViewRequestGeneration;
      var owner = panelGeneration;
      H.get("/fleet/tasks/" + encodeURIComponent(taskId))
        .then(function (d) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-view", taskCard(d || { task_id: taskId }));
        })
        .catch(function (e) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-view", errCard("查询失败：" + H.friendlyError(e)));
        });
    }

    function cancelTask(taskId) {
      if (!taskId) return render(".owo-fleet-task-view", errCard("请填写 task_id"));
      var request = ++taskViewRequestGeneration;
      var owner = panelGeneration;
      H.post("/fleet/tasks/" + encodeURIComponent(taskId) + "/cancel", {})
        .then(function (d) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(
            ".owo-fleet-task-view",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head">' +
              statusBadge(d && d.status) +
              "<strong>" + H.esc((d && d.task_id) || taskId) + "</strong>" +
              "</div>" +
              '<div class="owo-fleet-meta">任务已取消</div>' +
              "</div>"
          );
        })
        .catch(function (e) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-view", errCard("取消失败：" + H.friendlyError(e)));
        });
    }

    function taskEvents(taskId) {
      if (!taskId) return render(".owo-fleet-task-view", errCard("请填写 task_id"));
      var request = ++taskViewRequestGeneration;
      var owner = panelGeneration;
      H.get("/fleet/tasks/" + encodeURIComponent(taskId) + "/events?format=json")
        .then(function (d) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          var events = Array.isArray(d) ? d : (d && d.events) || [];
          render(
            ".owo-fleet-task-view",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head"><strong>' + H.esc(taskId) + '</strong><span class="owo-fleet-badge">事件流</span></div>' +
              eventsTimeline(events) +
              "</div>"
          );
        })
        .catch(function (e) {
          if (request !== taskViewRequestGeneration || owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-view", errCard("事件拉取失败：" + H.friendlyError(e)));
        });
    }

    // ---------- 审批响应 ----------

    function respondApproval(taskId, decision, approvedBy) {
      var owner = panelGeneration;
      if (!taskId) return render(".owo-fleet-approval-result", errCard("请填写审批任务 task_id"));
      if (actionBusy.approval) return;
      actionBusy.approval = true;
      syncWriteButtons();
      safePost("/fleet/approvals/" + encodeURIComponent(taskId) + "/respond", {
        decision: decision,
        approved_by: approvedBy || "workbench",
      })
        .then(function (d) {
          if (owner !== panelGeneration || !alive()) return;
          var dec = d && d.decision;
          var badge =
            dec === "approved"
              ? '<span class="owo-fleet-badge ok">已批准</span>'
              : dec === "rejected"
              ? '<span class="owo-fleet-badge bad">已拒绝</span>'
              : statusBadge(dec);
          render(
            ".owo-fleet-approval-result",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head">' +
              badge +
              "<strong>" + H.esc((d && d.task_id) || taskId) + "</strong>" +
              statusBadge(d && d.status) +
              "</div>" +
              '<div class="owo-fleet-meta">裁决人 ' + H.esc(approvedBy || "workbench") + "</div>" +
              "</div>"
          );
        })
        .catch(function (e) {
          if (owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-approval-result", errCard("裁决失败：" + H.friendlyError(e)));
        })
        .finally(function () {
          actionBusy.approval = false;
          syncWriteButtons();
        });
    }

    function dispose() {
      panelGeneration += 1;
      nodesRequestGeneration += 1;
      taskViewRequestGeneration += 1;
      sectionEl = null;
    }

    return {
      id: "fleet",
      title: "控制面（Fleet）",
      nav: nav,
      mount: mount,
      refresh: refresh,
      dispose: dispose,
    };
  })();
})();
