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
    var scopes = Object.create(null), scopeState = null, taskSnapshot = null, approvalSnapshot = null;
    var approvalRequestGeneration = 0;
    var tabs = ["nodes", "submit", "tasks", "approval"];
    var activeStates = ["pending", "running", "awaiting_approval"];
    var states = activeStates.concat(["succeeded", "failed", "cancelled"]);
    var draftFields = ["node-id", "node-worker", "node-os", "node-arch", "node-actions", "task-id", "task-worker", "task-input", "task-get-id", "approval-id", "approval-decision", "approval-by"];

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
      function field(name, label, placeholder) { return '<label for="fleet-' + name + '">' + label + '</label><input id="fleet-' + name + '" class="owo-fleet-' + name + '" placeholder="' + placeholder + '">'; }
      return '<section data-panel="fleet" data-layout="custom" class="owo-fleet-panel owo-insight">' +
        '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="协作设备">' + tabs.map(function (tab, i) {
          return '<button type="button" role="tab" id="fleet-tab-' + tab + '" data-fleet-tab="' + tab + '" aria-controls="fleet-pane-' + tab + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' + ['设备', '提交任务', '任务与事件', '审批'][i] + '</button>';
        }).join('') + '</div><button type="button" class="owo-fleet-nodes-refresh"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1.5L20 9M4 15l2.3 3.5A7 7 0 0 0 17.9 17"/></svg>刷新设备</button></div>' +
        '<section role="tabpanel" id="fleet-pane-nodes" aria-labelledby="fleet-tab-nodes"><h3>已登记设备</h3><div class="owo-fleet-nodes" role="status" aria-live="polite">正在读取设备…</div>' +
        '<section class="owo-insight-section"><h3>登记设备</h3><p class="sub">显式填写设备身份和能力；不会在此建立远程连接。按不可信设备登记，禁止出网。</p>' +
        '<div class="owo-fleet-two"><div>' + field('node-id','设备 ID','例如 node-a') + '</div><div>' + field('node-worker','工作器名称','留空使用设备 ID') + '</div><div>' +
        field('node-os','系统','windows / linux / mac_os / other') + '</div><div>' + field('node-arch','架构','x86_64 / aarch64 / other') + '</div></div>' +
        field('node-actions','允许的动作（逗号分隔，可留空）','例如 shell；仅声明该设备实际支持的动作') +
        '<div class="owo-insight-actions"><button type="button" class="owo-fleet-node-register primary" data-core-action>登记设备</button></div><div class="owo-fleet-node-result" role="status" aria-live="polite"></div></section></section>' +
        '<section role="tabpanel" id="fleet-pane-submit" aria-labelledby="fleet-tab-submit" hidden><h3>提交任务</h3><p class="sub">提交到控制面不代表节点已领取或执行成功。实际执行仍受节点、策略与审批约束。</p>' +
        '<div class="owo-fleet-two"><div>' + field('task-id','任务 ID','例如 task-1') + '</div><div>' + field('task-worker','工作器名称','例如 node-a') + '</div></div>' +
        '<label for="fleet-task-input">任务输入（JSON）</label><textarea id="fleet-task-input" class="owo-fleet-task-input" rows="6" spellcheck="false" placeholder="留空使用空对象 {}"></textarea>' +
        '<label class="owo-fleet-check"><input type="checkbox" class="owo-fleet-task-approval"> 需要审批</label>' +
        '<div class="owo-insight-actions"><button type="button" class="owo-fleet-task-submit primary" data-core-action>提交任务</button></div><div class="owo-fleet-task-submit-result" role="status" aria-live="polite"></div></section>' +
        '<section role="tabpanel" id="fleet-pane-tasks" aria-labelledby="fleet-tab-tasks" hidden><h3>查询任务</h3>' + field('task-get-id','任务 ID','输入要查询的任务 ID') +
        '<div class="owo-insight-actions owo-fleet-task-actions"><button type="button" class="owo-fleet-task-get primary">查询</button><button type="button" class="owo-fleet-task-events">读取事件</button><button type="button" class="owo-fleet-task-cancel" data-core-action disabled>取消任务</button></div>' +
        '<section class="owo-insight-section"><h3>任务详情与事件</h3><p class="sub">事件按需读取；取消结果以控制面状态为准，不代表节点已停止。</p><div class="owo-fleet-task-view" role="status" aria-live="polite">输入任务 ID 后查询。</div></section></section>' +
        '<section role="tabpanel" id="fleet-pane-approval" aria-labelledby="fleet-tab-approval" hidden><h3>审批</h3><p class="sub">先读取待审批任务的摘要、影响与证据，再选择裁决。批准仍由服务验证材料与权限。</p>' +
        field('approval-id','审批任务 ID','输入待审批任务 ID') + '<div class="owo-insight-actions owo-fleet-task-actions"><button type="button" class="owo-fleet-approval-load">读取审批</button></div>' +
        '<div class="owo-fleet-approval-context" role="status" aria-live="polite">尚未读取审批材料。</div>' +
        '<label for="fleet-approval-decision">裁决</label><select id="fleet-approval-decision" class="owo-fleet-approval-decision"><option value="">请选择裁决</option><option value="approve">批准</option><option value="reject">拒绝</option></select>' + field('approval-by','裁决人（可选）','留空标记为 workbench') +
        '<div class="owo-insight-actions"><button type="button" class="owo-fleet-approval-respond primary" data-core-action disabled>提交裁决</button></div><div class="owo-fleet-approval-result" role="status" aria-live="polite"></div></section></section>';
    }
    function selectTab(tab) {
      if (!alive() || tabs.indexOf(tab) < 0) return; scopeState.tab = tab;
      tabs.forEach(function (key) {
        var button = sectionEl.querySelector('[data-fleet-tab="' + key + '"]');
        if (button && button.setAttribute) { button.setAttribute('aria-selected', String(tab === key)); button.setAttribute('tabindex', tab === key ? '0' : '-1'); }
        var pane = sectionEl.querySelector('#fleet-pane-' + key); if (pane) pane.hidden = tab !== key;
      });
    }
    function mount(root, helpers) {
      dispose(); H = Object.assign({get:defaultGet,post:defaultPost,esc:defaultEsc,friendlyError:defaultFriendlyError}, helpers || {});
      var scope = (H.baseUrl || (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin) + '#' + (H.coreInstanceId || window.OwoApi && window.OwoApi.coreInstanceId || '');
      scopeState = scopes[scope] || (scopes[scope] = {busy:{register:false,submit:false,approval:false},locks:Object.create(null),draft:{},tab:'nodes',approvalRequired:false});
      actionBusy = scopeState.busy;
      root.innerHTML = nav(); sectionEl = root.querySelector('.owo-fleet-panel');
      var generation = panelGeneration, ownerState = scopeState;
      var $ = function (sel) { return root.querySelector(sel); };
      function bind(sel, event, action) { var el = $(sel); if (el && el.addEventListener) el.addEventListener(event, function (e) { if (generation === panelGeneration && ownerState === scopeState && alive()) return action(e); }); }
      draftFields.forEach(function (key) {
        var selector = '.owo-fleet-' + key, el = $(selector);
        if (Object.prototype.hasOwnProperty.call(scopeState.draft,key)) el.value = scopeState.draft[key];
        bind(selector, 'input', function () {
          scopeState.draft[key] = $(selector).value;
          if (key === 'task-get-id') { taskSnapshot = null; taskViewRequestGeneration++; }
          if (key === 'approval-id') { approvalSnapshot = null; approvalRequestGeneration++; render('.owo-fleet-approval-context', '<p class="sub">任务已更改，请重新读取审批材料。</p>'); }
          syncWriteButtons();
        });
        bind(selector, 'change', function () { scopeState.draft[key] = $(selector).value; syncWriteButtons(); });
      });
      $('.owo-fleet-task-approval').checked = scopeState.approvalRequired;
      bind('.owo-fleet-task-approval','change',function () { scopeState.approvalRequired = $('.owo-fleet-task-approval').checked; });
      bind('.owo-fleet-node-register','click',function () { return doRegister($('.owo-fleet-node-id').value,$('.owo-fleet-node-worker').value,$('.owo-fleet-node-os').value,$('.owo-fleet-node-arch').value,$('.owo-fleet-node-actions').value); });
      bind('.owo-fleet-nodes-refresh','click',listNodes);
      bind('.owo-fleet-task-submit','click',function () { return doSubmit($('.owo-fleet-task-id').value,$('.owo-fleet-task-worker').value,$('.owo-fleet-task-input').value,$('.owo-fleet-task-approval').checked); });
      bind('.owo-fleet-task-get','click',function () { return getTask($('.owo-fleet-task-get-id').value); });
      bind('.owo-fleet-task-events','click',function () { return taskEvents($('.owo-fleet-task-get-id').value); });
      bind('.owo-fleet-task-cancel','click',function () { return cancelTask($('.owo-fleet-task-get-id').value); });
      bind('.owo-fleet-approval-load','click',function () { return loadApproval($('.owo-fleet-approval-id').value); });
      bind('.owo-fleet-approval-respond','click',function () { return respondApproval($('.owo-fleet-approval-id').value,$('.owo-fleet-approval-decision').value,$('.owo-fleet-approval-by').value); });
      tabs.forEach(function (tab, i) {
        bind('[data-fleet-tab="' + tab + '"]','click',function () { selectTab(tab); });
        bind('[data-fleet-tab="' + tab + '"]','keydown',function (e) { if (e.ctrlKey || e.metaKey || e.altKey) return; var next = e.key === 'ArrowRight' ? (i+1)%4 : e.key === 'ArrowLeft' ? (i+3)%4 : e.key === 'Home' ? 0 : e.key === 'End' ? 3 : -1; if (next<0) return; e.preventDefault(); selectTab(tabs[next]); $('[data-fleet-tab="' + tabs[next] + '"]').focus(); });
      });
      selectTab(scopeState.tab); syncWriteButtons(); return listNodes();
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
    function approvalMaterial(approval) { return !!approval && typeof approval.impact_preview === "string" && !!approval.impact_preview.trim() && Array.isArray(approval.evidence) && approval.evidence.length > 0; }
    function syncWriteButtons() {
      if (!sectionEl) return;
      var query = sectionEl.querySelector('.owo-fleet-task-get-id'), approval = sectionEl.querySelector('.owo-fleet-approval-id'), decision = sectionEl.querySelector('.owo-fleet-approval-decision');
      var taskId = query && query.value && query.value.trim(), approvalId = approval && approval.value && approval.value.trim();
      var canCancel = taskSnapshot && taskSnapshot.task_id === taskId && activeStates.indexOf(taskSnapshot.status) >= 0;
      var canApprove = approvalSnapshot && approvalSnapshot.task_id === approvalId && approvalSnapshot.status === 'awaiting_approval' && approvalSnapshot.approval && approvalSnapshot.approval.decided === false && ['approve','reject'].indexOf(decision && decision.value) >= 0 && (decision.value === 'reject' || approvalMaterial(approvalSnapshot.approval));
      var buttons = [
        ['.owo-fleet-node-register', actionBusy.register, false, '登记中…', '登记设备'],
        ['.owo-fleet-task-submit', actionBusy.submit, false, '提交中…', '提交任务'],
        ['.owo-fleet-approval-respond', actionBusy.approval || !!scopeState.locks[approvalId], !canApprove, '处理中…', '提交裁决'],
        ['.owo-fleet-task-cancel', !!scopeState.locks[taskId], !canCancel, '取消中…', '取消任务']
      ];
      buttons.forEach(function (row) {
        var button = sectionEl.querySelector(row[0]); if (!button) return;
        button.disabled = !!row[1] || row[2]; button.textContent = row[1] ? row[3] : row[4];
        if (button.setAttribute) button.setAttribute('aria-busy',String(!!row[1]));
        if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button,!!row[1] || row[2]);
      });
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
      if (s === "running") return '<span class="owo-fleet-badge run">处理中</span>';
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

    // Capability declarations describe the requested node, not the browser's host.
    function doRegister(nodeId, worker, os, arch, actionsText) {
      nodeId = String(nodeId || '').trim(); worker = String(worker || '').trim(); os = String(os || '').trim(); arch = String(arch || '').trim();
      var owner = panelGeneration, busy = actionBusy;
      if (!nodeId) return render('.owo-fleet-node-result',errCard('请填写设备 ID'));
      if (['windows','linux','mac_os','other'].indexOf(os) < 0 || ['x86_64','aarch64','other'].indexOf(arch) < 0) return render('.owo-fleet-node-result',errCard('请填写设备的系统和架构，取值见输入提示。'));
      if (busy.register) return; busy.register = true; syncWriteButtons();
      var actions = String(actionsText || '').split(/[,，]/).map(function(v){return v.trim();}).filter(Boolean);
      return safePost('/fleet/nodes/register',{node_id:nodeId,card:{worker:worker || nodeId,os:os,arch:arch,actions:Array.from(new Set(actions)),trust:'untrusted',egress:'none'}}).then(function (d) {
        if (owner !== panelGeneration || !alive()) return;
        if (!d || d.node_id !== nodeId || !d.status || typeof d.status !== 'object') throw new Error('登记响应的设备身份或状态无效');
        render('.owo-fleet-node-result','<div class="owo-fleet-card"><div class="owo-fleet-card-head"><strong>' + H.esc(d.node_id) + '</strong><span class="owo-fleet-badge">已登记</span></div><p class="sub">租约纪元 ' + H.esc(d.lease_epoch == null ? '未知' : d.lease_epoch) + '；登记不代表已连接或可执行。</p></div>');
        return listNodes();
      }).catch(function(e){if(owner === panelGeneration && alive()) render('.owo-fleet-node-result',errCard('登记失败：'+H.friendlyError(e)));}).finally(function(){busy.register=false;if(busy===actionBusy)syncWriteButtons();});
    }

    // ---------- 节点列表 ----------

    function nodeCard(n) {
      var card = (n && n.card) || {};
      var state = n.lost === true
        ? '<span class="owo-fleet-badge bad">失联</span>'
        : n.healthy === true
        ? '<span class="owo-fleet-badge ok">健康</span>'
        : n.healthy === false ? '<span class="owo-fleet-badge bad">异常</span>' : '<span class="owo-fleet-badge">健康状态未知</span>';
      var chips = "";
      if (card.worker) chips += chip("worker：" + card.worker);
      var actions = Array.isArray(card.actions) ? card.actions : [];
      for (var i = 0; i < actions.length; i++) chips += chip(actions[i]);
      var meta =
        "心跳 " + H.esc(Number.isInteger(n.heartbeats) && n.heartbeats >= 0 ? n.heartbeats : "未知") + " · 重启 " + H.esc(Number.isInteger(n.restarts) && n.restarts >= 0 ? n.restarts : "未知") +
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
      return safeGet("/fleet/nodes")
        .then(function (d) {
          if (request !== nodesRequestGeneration || owner !== panelGeneration || !alive()) return;
          if (!d || !Array.isArray(d.nodes)) throw new Error("服务返回的设备列表无效");
          var nodes = d.nodes;
          if (!nodes.length) {
            render(".owo-fleet-nodes", '<div class="owo-fleet-empty"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="m12 2 9 5v10l-9 5-9-5V7l9-5Zm0 10 9-5M12 12 3 7m9 5v10"/></svg><strong>当前没有登记设备。</strong><p class="sub">登记信息不代表设备已连接或能执行任务。</p></div>');
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
      taskId = String(taskId || "").trim(); worker = String(worker || "").trim();
      var owner = panelGeneration, busy = actionBusy;
      if (!taskId || !worker) return render(".owo-fleet-task-submit-result", errCard("请填写 task_id 与 worker"));
      var input = {};
      try {
        input = inputText.trim() ? JSON.parse(inputText) : {};
      } catch (e) {
        return render(".owo-fleet-task-submit-result", errCard("input 不是合法 JSON：" + H.friendlyError(e)));
      }
      if (busy.submit) return;
      busy.submit = true;
      syncWriteButtons();
      return safePost("/fleet/tasks/submit", {
        task_id: taskId,
        worker: worker,
        input: input,
        approval_required: !!approvalRequired,
      })
        .then(function (d) {
          if (owner !== panelGeneration || !alive()) return;
          if (!d || d.task_id !== taskId || states.indexOf(d.status) < 0) throw new Error("提交响应身份或状态无效");
          render(
            ".owo-fleet-task-submit-result",
            '<div class="owo-fleet-card">' +
              '<div class="owo-fleet-card-head">' +
              statusBadge(d && d.status) +
              "<strong>" + H.esc((d && d.task_id) || taskId) + "</strong>" +
              "</div>" +
              '<div class="owo-fleet-meta">幂等键 ' + H.esc((d && d.idempotency_key) || "—") + '</div><p class="sub">提交到控制面不代表节点已领取或执行成功。</p>' +
              "</div>"
          );
        })
        .catch(function (e) {
          if (owner !== panelGeneration || !alive()) return;
          render(".owo-fleet-task-submit-result", errCard("提交失败：" + H.friendlyError(e)));
        })
        .finally(function () {
          busy.submit = false;
          if (busy === actionBusy) syncWriteButtons();
        });
    }

    // ---------- 任务查询 / 取消 / 事件 ----------

    function eventLine(ev) {
      var text = brief(ev && ev.payload);
      var lineage = ev && Array.isArray(ev.lineage) ? ev.lineage : [];
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
      var badge = decided === true && a.decision === "approved" ? '<span class="owo-fleet-badge ok">已批准</span>' : decided === true && a.decision === "rejected" ? '<span class="owo-fleet-badge bad">已拒绝</span>' : decided === false ? '<span class="owo-fleet-badge warn">待裁决</span>' : '<span class="owo-fleet-badge">裁决状态未知</span>';
      var evidence = "";
      var items = Array.isArray(a.evidence) ? a.evidence.filter(Boolean) : [];
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
        (evidence ? '<div class="owo-fleet-chips">' + evidence + "</div>" : '<p class="sub">结构化证据：未提供</p>') +
        (!a.impact_preview ? '<p class="sub">影响预览：未提供。批准需要 impact_preview 和 evidence 齐备。</p>' : "") +
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
        eventsTimeline(Array.isArray(view.events) ? view.events : []) +
        approvalCard(view.approval) +
        "</div>"
      );
    }

    function safeGet(path) { try { return Promise.resolve(H.get(path)); } catch(error) { return Promise.reject(error); } }
    function validTask(d, taskId) { if (!d || d.task_id !== taskId || states.indexOf(d.status) < 0 || !Array.isArray(d.events)) throw new Error('任务响应的身份、状态或事件无效'); return d; }
    function getTask(taskId) {
      taskId = String(taskId || '').trim(); if(!taskId) return render('.owo-fleet-task-view',errCard('请填写任务 ID'));
      var request = ++taskViewRequestGeneration, owner = panelGeneration, locks = scopeState.locks;
      taskSnapshot = null; syncWriteButtons();
      return safeGet('/fleet/tasks/'+encodeURIComponent(taskId)).then(function(d){
        if(request!==taskViewRequestGeneration || owner!==panelGeneration || !alive() || locks[taskId])return;
        taskSnapshot = validTask(d,taskId); render('.owo-fleet-task-view',taskCard(d)); syncWriteButtons();
      }).catch(function(e){if(request===taskViewRequestGeneration && owner===panelGeneration && alive())render('.owo-fleet-task-view',errCard('查询失败：'+H.friendlyError(e)));});
    }
    function cancelTask(taskId) {
      taskId = String(taskId || '').trim();
      if(!taskSnapshot || taskSnapshot.task_id!==taskId || activeStates.indexOf(taskSnapshot.status)<0) return render('.owo-fleet-task-view',errCard('请先查询仍在处理的任务，再取消。'));
      var owner=panelGeneration, ownerState=scopeState, locks=ownerState.locks;
      if(locks[taskId])return; locks[taskId]='cancel'; var request=++taskViewRequestGeneration; syncWriteButtons();
      return safeGet('/fleet/tasks/'+encodeURIComponent(taskId)).then(function(d){
        if(owner!==panelGeneration || request!==taskViewRequestGeneration || !alive() || String(sectionEl.querySelector('.owo-fleet-task-get-id').value).trim()!==taskId)return;
        validTask(d,taskId); if(activeStates.indexOf(d.status)<0){taskSnapshot=d;render('.owo-fleet-task-view',taskCard(d));return;}
        return safePost('/fleet/tasks/'+encodeURIComponent(taskId)+'/cancel',{}).then(function(result){
          if(owner!==panelGeneration || request!==taskViewRequestGeneration || !alive())return;
          if(!result || result.task_id!==taskId || states.indexOf(result.status)<0)throw new Error('取消响应的身份或状态无效');
          taskSnapshot=Object.assign({},d,{status:result.status});
          render('.owo-fleet-task-view',taskCard(taskSnapshot)+'<p class="sub">控制面返回：'+H.esc(result.status)+'。请结合节点事件确认实际停止状态。</p>');
        });
      }).catch(function(e){if(owner===panelGeneration && request===taskViewRequestGeneration && alive())render('.owo-fleet-task-view',errCard('取消失败：'+H.friendlyError(e)));}).finally(function(){delete locks[taskId];if(ownerState===scopeState)syncWriteButtons();});
    }
    function taskEvents(taskId) {
      taskId=String(taskId||'').trim();if(!taskId)return render('.owo-fleet-task-view',errCard('请填写任务 ID'));
      var request=++taskViewRequestGeneration,owner=panelGeneration;
      return safeGet('/fleet/tasks/'+encodeURIComponent(taskId)+'/events?format=json').then(function(d){
        if(request!==taskViewRequestGeneration || owner!==panelGeneration || !alive())return;
        var events=Array.isArray(d)?d:d&&d.events;if(!Array.isArray(events))throw new Error('服务返回的任务事件无效');
        render('.owo-fleet-task-view','<div class="owo-fleet-card"><div class="owo-fleet-card-head"><strong>'+H.esc(taskId)+'</strong><span class="owo-fleet-badge">事件快照</span></div>'+eventsTimeline(events)+'</div>');
      }).catch(function(e){if(request===taskViewRequestGeneration && owner===panelGeneration && alive())render('.owo-fleet-task-view',errCard('事件读取失败：'+H.friendlyError(e)));});
    }
    function loadApproval(taskId) {
      taskId=String(taskId||'').trim();approvalSnapshot=null;syncWriteButtons();
      if(!taskId)return render('.owo-fleet-approval-context',errCard('请填写审批任务 ID'));
      var owner=panelGeneration,request=++approvalRequestGeneration;
      render('.owo-fleet-approval-context','<p class="sub">正在读取审批材料…</p>');
      return safeGet('/fleet/tasks/'+encodeURIComponent(taskId)).then(function(d){
        if(owner!==panelGeneration || request!==approvalRequestGeneration || !alive())return;
        validTask(d,taskId);approvalSnapshot=d;
        render('.owo-fleet-approval-context',taskCard(d)+(d.status==='awaiting_approval'&&d.approval&&d.approval.decided===false?'':'<p class="sub">此任务当前没有待裁决审批。</p>'));syncWriteButtons();
      }).catch(function(e){if(owner===panelGeneration && request===approvalRequestGeneration && alive())render('.owo-fleet-approval-context',errCard('审批读取失败：'+H.friendlyError(e)));});
    }
    function respondApproval(taskId,decision,approvedBy) {
      taskId=String(taskId||'').trim();
      if(!approvalSnapshot || approvalSnapshot.task_id!==taskId || approvalSnapshot.status!=='awaiting_approval' || !approvalSnapshot.approval || approvalSnapshot.approval.decided!==false)return render('.owo-fleet-approval-result',errCard('请先读取待审批任务的材料。'));
      if(['approve','reject'].indexOf(decision)<0)return render('.owo-fleet-approval-result',errCard('请明确选择批准或拒绝。'));
      if(decision==='approve'&&!approvalMaterial(approvalSnapshot.approval))return render('.owo-fleet-approval-result',errCard('缺少影响预览或结构化证据，当前不能批准。'));
      var owner=panelGeneration,ownerState=scopeState,busy=actionBusy,locks=ownerState.locks;
      if(busy.approval||locks[taskId])return;busy.approval=true;locks[taskId]='approval';taskViewRequestGeneration++;var request=++approvalRequestGeneration;syncWriteButtons();
      return safeGet('/fleet/tasks/'+encodeURIComponent(taskId)).then(function(d){
        if(owner!==panelGeneration || request!==approvalRequestGeneration || !alive() || String(sectionEl.querySelector('.owo-fleet-approval-id').value).trim()!==taskId)return;
        validTask(d,taskId);if(decision==='approve'&&!approvalMaterial(d.approval))throw new Error('当前审批材料不完整，不能批准');if(d.status!=='awaiting_approval'||!d.approval||d.approval.decided!==false){approvalSnapshot=null;render('.owo-fleet-approval-context',taskCard(d));throw new Error('任务已不在待审批状态，请重新读取。');}
        return safePost('/fleet/approvals/'+encodeURIComponent(taskId)+'/respond',{decision:decision,approved_by:String(approvedBy||'').trim()||'workbench'}).then(function(result){
          if(owner!==panelGeneration||request!==approvalRequestGeneration||!alive())return;
          if(!result||result.task_id!==taskId||['approved','rejected'].indexOf(result.decision)<0||states.indexOf(result.status)<0)throw new Error('裁决响应身份、决定或状态无效');
          approvalSnapshot=null;
          render('.owo-fleet-approval-result','<div class="owo-fleet-card"><div class="owo-fleet-card-head"><strong>'+H.esc(taskId)+'</strong><span class="owo-fleet-badge">'+(result.decision==='approved'?'已批准':'已拒绝')+'</span>'+statusBadge(result.status)+'</div><p class="sub">裁决人 '+H.esc(String(approvedBy||'').trim()||'workbench')+'；执行结果仍以节点上报为准。</p></div>');
          return loadApproval(taskId);
        });
      }).catch(function(e){if(owner===panelGeneration&&request===approvalRequestGeneration&&alive()){approvalSnapshot=null;render('.owo-fleet-approval-result',errCard('裁决失败：'+H.friendlyError(e)+'；请重新读取当前审批状态。'));}}).finally(function(){busy.approval=false;delete locks[taskId];if(ownerState===scopeState)syncWriteButtons();});
    }

    function dispose() {
      panelGeneration += 1;
      nodesRequestGeneration += 1;
      taskViewRequestGeneration += 1;
      approvalRequestGeneration += 1; taskSnapshot = null; approvalSnapshot = null;
      sectionEl = null;
    }

    return {
      id: "fleet",
      title: "协作设备",
      nav: nav,
      mount: mount,
      refresh: refresh,
      dispose: dispose,
    };
  })();
})();
