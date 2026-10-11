// R11:observability 面板质量收尾完成。
// R12:observability 面板复核完成（SLO/用量/告警/周报/遥测展示，无需改动）。
/* R5 Agent 3 面板：可观测性（概览卡/回合耗时折线/工具排行/健康清单）。
 * R8 增：SLO 基线 + 用量与预算；R9 增：告警规则/最近告警 + 周期报告入口；
 * R10 增：可选遥测状态展示（默认关，仅聚合指标）。
 * 纯脚本 IIFE，注册 window.OwoPanels.observability；helpers 防御性降级。
 * 折线用内联 SVG 绘制（无外部依赖）。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.observability = (function () {
  "use strict";

  var id = "observability";
  var panelGeneration = 0;
  var refreshGeneration = 0;
  var reportGeneration = 0;
  var reportPending = false;
  var sectionEl = null;

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) return r.json().catch(function () { return {}; }).then(function (body) {
          throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
        });
        return r.status === 204 ? null : r.json();
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
    return { baseUrl: baseUrl, get: get, esc: esc, friendlyError: friendlyError };
  }

  var H = defaultHelpers();
  var state = { overview: null, turns: [], tools: [], health: null, runtime: null, slo: null, usage: null, alerts: null, report: null, telemetry: null };

  function finiteMetric(value) {
    if (typeof value !== "number" && !(typeof value === "string" && value.trim())) return null;
    var number = Number(value);
    return Number.isFinite(number) ? number : null;
  }

  function metricText(value, fallback) {
    var number = finiteMetric(value);
    return number == null ? (fallback == null ? "—" : fallback) : H.esc(String(number));
  }

  function percentageText(value, digits) {
    var number = finiteMetric(value);
    return number == null ? "—" : H.esc((number * 100).toFixed(digits) + "%");
  }

  function nav() {
    return (
      '<section data-panel="' + id + '">' +
      '<style>' +
      '.owo-mtr-row{display:flex;gap:8px;align-items:center;padding:4px 0;border-bottom:1px solid var(--border)}' +
      // KPI 卡片：自适应网格（此前 8 张 inline-block 挤成一行，数字与标签贴在一起）
      '#owo-mtr-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(116px,1fr));gap:8px}' +
      '.owo-mtr-card{display:flex;flex-direction:column;gap:2px;justify-content:center;padding:9px 12px;background:var(--surface-2);border:1px solid var(--border);border-radius:var(--r-md);text-align:left}' +
      '.owo-mtr-card b{font-size:19px;font-variant-numeric:tabular-nums;line-height:1.1;color:var(--text);overflow-wrap:anywhere}' +
      '.owo-mtr-card span{font-size:12px;color:var(--text-2)}' +
      '.owo-mtr-table{width:100%;border-collapse:collapse;font-size:12px}' +
      '.owo-mtr-table td,.owo-mtr-table th{border:0;border-bottom:1px solid var(--border);padding:5px 9px;text-align:left}' +
      '.owo-mtr-table th{background:var(--surface-2);color:var(--text-2);font-weight:650;white-space:nowrap}' +
      '.owo-mtr-table tbody tr:nth-child(even){background:var(--surface-2)}' +
      '.owo-mtr-table tbody tr:hover{background:var(--accent-soft)}' +
      '.owo-mtr-table tbody tr:last-child td{border-bottom:0}' +
      '.owo-mtr-table td:first-child{width:38%;color:var(--text-2)}' +
      '.owo-mtr-health b.ok{color:var(--green)}.owo-mtr-health b.bad{color:var(--red)}' +
      '</style>' +
      '<div class="stack">' +
      '<div class="sub">可观测性 / 性能护栏</div>' +
      '<div class="owo-mtr-row"><button class="primary" id="owo-mtr-refresh">刷新</button>' +
      '<span id="owo-mtr-updated" class="sub">—</span></div>' +
      '<div id="owo-mtr-cards"></div>' +
      '<div class="sub">运行时韧性指标（Wave 1/2）</div>' +
      '<div id="owo-mtr-runtime" class="owo-mtr-runtime">—</div>' +
      '<div class="sub">SLO 基线（Wave 2）</div>' +
      '<div id="owo-mtr-slo" class="owo-mtr-slo">—</div>' +
      '<div class="sub">用量与预算（R8）</div>' +
      '<div id="owo-mtr-usage" class="owo-mtr-usage">—</div>' +
      '<div class="sub">SLO 告警（R9）</div>' +
      '<div id="owo-mtr-alerts" class="owo-mtr-alerts">—</div>' +
      '<div class="sub">周期报告（R9，7 天窗口）</div>' +
      '<div id="owo-mtr-report" class="owo-mtr-report"><button class="primary" id="owo-mtr-report-refresh">加载周报</button></div>' +
      '<div class="sub">可选遥测（R10，默认关，仅聚合指标）</div>' +
      '<div id="owo-mtr-telemetry" class="owo-mtr-telemetry">—</div>' +
      '<div class="sub">回合耗时（最近 50 次，ms）</div>' +
      '<div id="owo-mtr-chart">—</div>' +
      '<div class="sub">工具调用排行</div>' +
      '<table class="owo-mtr-table" id="owo-mtr-tools"><thead><tr><th>工具</th><th>调用</th><th>失败</th><th>失败率</th></tr></thead><tbody></tbody></table>' +
      '<div class="sub">组件健康</div>' +
      '<div id="owo-mtr-health" class="owo-mtr-health">—</div>' +
      '</div>' +
      '</section>'
    );
  }

  function mount(root, helpers) {
    dispose();
    H = helpers || defaultHelpers();
    root.innerHTML = nav();
    sectionEl = root.querySelector('[data-panel="observability"]');
    root.querySelector("#owo-mtr-refresh").addEventListener("click", refresh);
    bindReportReload(root.querySelector("#owo-mtr-report"));
    refresh();
  }

  function showMetricError(selector, error) {
    var target = sectionEl && sectionEl.querySelector(selector);
    if (!target) return;
    var message = "加载失败：" + H.friendlyError(error);
    if (selector === "#owo-mtr-tools tbody") {
      target.innerHTML = '<tr><td colspan="4" style="color:var(--red)">' + H.esc(message) + "</td></tr>";
    } else {
      target.textContent = message;
      target.style.color = "var(--red)";
    }
  }

  function refresh() {
    var request = ++refreshGeneration;
    var owner = panelGeneration;
    var refreshButton = sectionEl && sectionEl.querySelector("#owo-mtr-refresh");
    if (refreshButton) {
      refreshButton.disabled = true;
      refreshButton.setAttribute("aria-busy", "true");
      refreshButton.textContent = "刷新中…";
    }
    var endpoints = [
      ["/metrics/overview", function (data) { state.overview = data; renderCards(); }, "#owo-mtr-cards"],
      ["/metrics/turns?limit=50", function (data) { state.turns = Array.isArray(data && data.turns) ? data.turns : []; renderChart(); }, "#owo-mtr-chart"],
      ["/metrics/tools", function (data) { state.tools = Array.isArray(data && data.tools) ? data.tools.filter(Boolean) : []; renderTools(); }, "#owo-mtr-tools tbody"],
      ["/metrics/health", function (data) { state.health = data; renderHealth(); }, "#owo-mtr-health"],
      ["/metrics/runtime", function (data) { state.runtime = data; renderRuntime(); }, "#owo-mtr-runtime"],
      ["/metrics/slo", function (data) { state.slo = data && typeof data === "object" ? data : {}; renderSlo(); }, "#owo-mtr-slo"],
      ["/usage/summary", function (data) { state.usage = data; renderUsage(); }, "#owo-mtr-usage"],
      ["/metrics/slo/alerts", function (data) { state.alerts = data && typeof data === "object" ? data : {}; renderAlerts(); }, "#owo-mtr-alerts"],
      ["/metrics/telemetry/status", function (data) { state.telemetry = data && typeof data === "object" ? data : {}; renderTelemetry(); }, "#owo-mtr-telemetry"],
    ];
    return Promise.all(endpoints.map(function (endpoint) {
      return H.get(endpoint[0])
        .then(function (data) {
          if (request !== refreshGeneration || owner !== panelGeneration) return;
          endpoint[1](data || {});
        })
        .catch(function (error) {
          if (request !== refreshGeneration || owner !== panelGeneration) return;
          showMetricError(endpoint[2], error);
        });
    })).finally(function () {
      if (request !== refreshGeneration || owner !== panelGeneration) return;
      var button = sectionEl && sectionEl.querySelector("#owo-mtr-refresh");
      if (button) {
        button.disabled = false;
        button.removeAttribute("aria-busy");
        button.textContent = "刷新";
      }
    });
  }

  function bindReportReload(container) {
    if (!container) return;
    var button = container.querySelector("#owo-mtr-report-refresh");
    if (button) button.addEventListener("click", loadReport);
  }

  function renderReportError(error) {
    var el = sectionEl && sectionEl.querySelector("#owo-mtr-report");
    if (!el) return;
    el.innerHTML = '<div role="alert" style="color:var(--red)">周报加载失败：' + H.esc(H.friendlyError(error)) +
      '</div><button class="primary" id="owo-mtr-report-refresh">重试</button>';
    bindReportReload(el);
    var retry = el.querySelector("#owo-mtr-report-refresh");
    if (retry) retry.textContent = "重试";
  }

  function loadReport() {
    if (reportPending) return Promise.resolve(null);
    var request = ++reportGeneration;
    var owner = panelGeneration;
    var reportEl = sectionEl && sectionEl.querySelector("#owo-mtr-report");
    reportPending = true;
    if (reportEl) {
      reportEl.innerHTML = '<span role="status">正在加载周报…</span><button id="owo-mtr-report-refresh" disabled aria-busy="true">正在加载…</button>';
    }
    return Promise.resolve()
      .then(function () { return H.get("/metrics/slo/report?days=7"); })
      .then(function (data) {
        if (request !== reportGeneration || owner !== panelGeneration) return null;
        state.report = data || {};
        renderReport();
        return data;
      })
      .catch(function (error) {
        if (request === reportGeneration && owner === panelGeneration) renderReportError(error);
        return null;
      })
      .finally(function () {
        if (request !== reportGeneration || owner !== panelGeneration) return;
        reportPending = false;
        var current = sectionEl && sectionEl.querySelector("#owo-mtr-report-refresh");
        if (current) {
          current.disabled = false;
          current.removeAttribute("aria-busy");
        }
      });
  }

  function renderCards() {
    var el = document.getElementById("owo-mtr-cards");
    if (!el || !state.overview) return;
    var o = state.overview;
    el.innerHTML =
      card(o.traces_count, "traces") +
      card(o.avg_turn_ms, "均耗时 ms") +
      card(o.p50_ms, "p50 ms") +
      card(o.p95_ms, "p95 ms") +
      card(o.tool_calls_total, "工具调用") +
      card(o.approvals_total, "审批") +
      card(o.denied, "拒绝") +
      card(o.failures, "失败");
    var updated = document.getElementById("owo-mtr-updated");
    var updatedAt = typeof o.updated_at === "string" ? o.updated_at : "";
    if (updated) updated.textContent = updatedAt ? "更新于 " + updatedAt.slice(0, 19).replace("T", " ") : "更新时间未知";
  }

  function card(value, label) {
    return '<div class="owo-mtr-card"><b>' + (value == null ? "—" : H.esc(String(value))) + "</b><span>" + H.esc(label) + "</span></div>";
  }

  function renderChart() {
    var el = document.getElementById("owo-mtr-chart");
    if (!el) return;
    var data = state.turns;
    if (!data.length) {
      el.textContent = "暂无回合数据";
      return;
    }
    var values = data
      .filter(function (turn) { return turn && typeof turn === "object"; })
      .map(function (turn) { return finiteMetric(turn.duration_ms); })
      .filter(function (duration) { return duration != null && duration >= 0; })
      .slice(0, 50)
      .reverse();
    if (!values.length) {
      el.textContent = "暂无有效回合耗时数据";
      return;
    }
    var width = 560;
    var height = 120;
    var max = Math.max.apply(null, values.concat([1]));
    var points = values
      .map(function (v, i) {
        var x = (i / Math.max(1, values.length - 1)) * (width - 8) + 4;
        var y = height - 8 - (v / max) * (height - 20);
        return x.toFixed(1) + "," + y.toFixed(1);
      })
      .join(" ");
    el.innerHTML =
      '<svg width="100%" viewBox="0 0 ' + width + " " + height + '" style="max-width:560px">' +
      '<polyline points="' + points + '" fill="none" style="stroke:var(--green)" stroke-width="1.5"></polyline>' +
      "<text x=\"4\" y=\"14\" font-size=\"10\" style=\"fill:var(--text-3)\">峰值 " + max + " ms（最近 " + values.length + " 次）</text>" +
      "</svg>";
  }

  function renderTools() {
    var el = document.querySelector("#owo-mtr-tools tbody");
    if (!el) return;
    if (!state.tools.length) {
      el.innerHTML = '<tr><td colspan="4" class="sub">暂无工具调用</td></tr>';
      return;
    }
    el.innerHTML = state.tools
      .map(function (t) {
        return (
          "<tr><td>" + H.esc(t.tool) + "</td><td>" + metricText(t.calls, "0") + "</td><td>" + metricText(t.failures, "0") +
          "</td><td>" + percentageText(t.failure_rate, 1) + "</td></tr>"
        );
      })
      .join("");
  }

  function renderHealth() {
    var el = document.getElementById("owo-mtr-health");
    if (!el || !state.health) return;
    var c = state.health.components || {};
    var stt = c.stt && c.stt.ready;
    el.innerHTML =
      "STT：<b class=\"" + (stt ? "ok" : "bad") + "\">" + (stt ? "就绪" : "未就绪") + "</b> ｜ " +
      "云端传输：<b class=\"ok\">" + H.esc((c.cloud_transport && c.cloud_transport.kind) || "?") + "</b> ｜ " +
      "插件：" + metricText(c.plugins && c.plugins.count, "0") + " ｜ " +
      "笔记：" + metricText(c.notes && c.notes.count, "0") + " ｜ " +
      "traces：" + metricText(c.traces && c.traces.count, "0");
  }

  function renderRuntime() {
    var el = document.getElementById("owo-mtr-runtime");
    if (!el || !state.runtime) return;
    var r = state.runtime;
    var tool = r.tool || {};
    var approval = r.approval || {};
    var sse = r.sse || {};
    var events = r.events || {};
    var pct = function (v) { return percentageText(v, 1); };
    var ms = function (v) {
      return v == null ? "—" : H.esc(String(v)) + " ms";
    };
    el.innerHTML =
      '<table class="owo-mtr-table">' +
      "<tr><td>工具调度 p95 / p50</td><td>" + ms(tool.p95_ms) + " / " + ms(tool.p50_ms) + "（样本 " + metricText(tool.samples, "0") + "）</td></tr>" +
      "<tr><td>审批通过率</td><td>" + pct(approval.pass_rate) + "</td></tr>" +
      "<tr><td>审批拦截率</td><td>" + pct(approval.intercept_rate) + "（通过 " + metricText(approval.approved, "0") + " / 拦截 " + metricText(approval.denied, "0") + " / 共 " + metricText(approval.total, "0") + "）</td></tr>" +
      "<tr><td>事件队列深度</td><td>" + metricText(r.queue_depth, "0") + "</td></tr>" +
      "<tr><td>SSE 活跃连接</td><td>" + metricText(sse.active_connections, "0") + "（累计 " + metricText(sse.total_connections, "0") + "，慢消费者断开 " + metricText(sse.lagged_total, "0") + "）</td></tr>" +
      "<tr><td>事件流发布/丢弃</td><td>" + metricText(events.published, "0") + " / " + metricText(events.dropped, "0") + "</td></tr>" +
      "</table>";
  }

  function renderSlo() {
    var el = document.getElementById("owo-mtr-slo");
    if (!el || !state.slo) return;
    var items = (Array.isArray(state.slo.slo) ? state.slo.slo : []).filter(function (item) { return item && typeof item === "object"; }).slice().sort(function (a, b) {
      return String(a.name || "").localeCompare(String(b.name || ""));
    });
    if (!items.length) {
      el.innerHTML = '<span class="sub">暂无 SLO 数据（服务端未注册报告探针）</span>';
      return;
    }
    var rows = items
      .map(function (item) {
        var budget = item.error_budget && typeof item.error_budget === "object" ? item.error_budget : {};
        var target = item.target_ms == null ? percentageText(item.success_floor, 1) : metricText(item.target_ms) + " ms";
        var p95Value = finiteMetric(item.p95_ms);
        var p95 = p95Value == null ? "—" : metricText(p95Value) + " ms";
        var rate = percentageText(item.success_rate, 2);
        // 样本为 0 时不能报"达标"：没有观测数据就无达标可言，显示灰色"样本不足"避免误判。
        var sampleCount = finiteMetric(item.samples);
        var status = (sampleCount || 0) === 0
          ? '<b style="color:var(--text-3)">样本不足</b>'
          : item.achieving
            ? '<b class="ok">达标</b>'
            : '<b class="bad">未达标</b>';
        return (
          "<tr><td>" + H.esc(item.name) + "</td><td>" + H.esc(target) +
          "</td><td>" + p95 + "</td><td>" + rate +
          "</td><td>" + metricText(sampleCount, "0") +
          "</td><td>" + metricText(budget.bad, "0") + " / " + metricText(budget.allowed_bad, "0") +
          "</td><td>" + status + "</td></tr>"
        );
      })
      .join("");
    el.innerHTML =
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>SLO</th><th>目标</th><th>p95</th><th>成功率</th><th>样本</th><th>违规/预算</th><th>状态</th></tr></thead>" +
      "<tbody>" + rows + "</tbody></table>";
  }

  function renderUsage() {
    var el = document.getElementById("owo-mtr-usage");
    if (!el || !state.usage) return;
    var u = state.usage;
    if (u.error) {
      el.innerHTML = '<span class="sub">用量端点未就绪（主控接线后可用）</span>';
      return;
    }
    var dims = Array.isArray(u.dimensions) ? u.dimensions.filter(function (item) { return item && typeof item === "object"; }) : [];
    var rows = dims
      .map(function (d) {
        var budget = d.budget ? "，预算 " + H.esc(String(d.budget.limit_usd)) + " USD" : "";
        var exceeded = d.budget && d.budget.exceeded ? ' <b class="bad">超限</b>' : "";
        return (
          "<tr><td>" + H.esc(d.dimension) + "</td><td>" + metricText(d.calls, "0") +
          "</td><td>" + metricText(d.total_tokens) +
          "</td><td>" + metricText(d.cost_usd) + (d.cost_usd == null ? "" : " USD") +
          "</td><td>" + metricText(d.budget ? d.budget.spent_usd : null) + " / " +
          H.esc(String(d.budget ? d.budget.limit_usd : "—")) + budget + exceeded + "</td></tr>"
        );
      })
      .join("");
    var stop = u.hard_stop
      ? ' <b class="bad">硬熔断中</b>' + (u.hard_stop_reason ? "（" + H.esc(u.hard_stop_reason) + "）" : "")
      : "";
    el.innerHTML =
      "<div class=\"owo-mtr-row\">记录 " + metricText(u.count, "0") + " 条，单价 " + metricText(u.price_per_mtok) + " $/Mtok" + stop + "</div>" +
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>维度</th><th>调用</th><th>Token</th><th>成本</th><th>花费/预算</th></tr></thead>" +
      "<tbody>" + (rows || '<tr><td colspan="5" class="sub">暂无用量记录</td></tr>') + "</tbody></table>";
  }

  function renderAlerts() {
    var el = document.getElementById("owo-mtr-alerts");
    if (!el || !state.alerts) return;
    var data = state.alerts;
    if (data.note) {
      el.innerHTML = '<span class="sub">告警探针未注册（主控接线后可用）</span>';
      return;
    }
    var rules = Array.isArray(data.rules) ? data.rules.filter(function (item) { return item && typeof item === "object"; }) : [];
    var ruleHtml = rules
      .map(function (r) {
        return "<tr><td>" + H.esc(r.name) + "</td><td>" + H.esc(r.slo_name) +
          "</td><td>" + H.esc(String(r.kind)) + " &gt; " + H.esc(String(r.threshold)) +
          "</td><td>连续 " + metricText(r.consecutive, "0") + " 次</td><td>" +
          H.esc(r.severity || "") + "</td></tr>";
      })
      .join("");
    var alerts = (Array.isArray(data.alerts) ? data.alerts : []).filter(function (item) { return item && typeof item === "object"; }).slice(0, 8);
    var alertHtml = alerts
      .map(function (a) {
        var color = a.kind === "recovered" ? "var(--green)" : a.severity === "critical" ? "var(--red)" : "var(--yellow)";
        return "<div class=\"owo-mtr-row\"><span style=\"color:" + color + "\">[" + H.esc(a.kind) +
          "] " + H.esc(a.rule) + "</span><span class=\"sub\">" +
          H.esc(String(a.at || "").slice(11, 19)) + "</span></div><div class=\"sub\">" +
          H.esc(a.detail || "") + "</div>";
      })
      .join("");
    el.innerHTML =
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>规则</th><th>SLO</th><th>判定</th><th>连续</th><th>级别</th></tr></thead>" +
      "<tbody>" + (ruleHtml || '<tr><td colspan="5" class="sub">暂无规则</td></tr>') + "</tbody></table>" +
      '<div class="sub">最近告警（' + metricText(data.count, "0") + '）</div>' +
      (alertHtml || '<div class="sub">暂无告警</div>');
  }

  function renderReport() {
    var el = sectionEl && sectionEl.querySelector("#owo-mtr-report");
    if (!el || !state.report) return;
    var data = state.report;
    if (data.note) {
      el.innerHTML = '<span class="sub">周期报告探针未注册（主控接线后可用）</span><button id="owo-mtr-report-refresh">重新检查</button>';
      bindReportReload(el);
      return;
    }
    var items = (Array.isArray(data.slo) ? data.slo : []).filter(function (item) { return item && typeof item === "object"; }).slice().sort(function (a, b) {
      return String(a.name || "").localeCompare(String(b.name || ""));
    });
    var rows = items
      .map(function (item) {
        var p95Value = finiteMetric(item.p95_ms);
        var p95 = p95Value == null ? "—" : metricText(p95Value) + " ms";
        var rate = percentageText(item.success_rate, 2);
        // 样本为 0 时不能报"达标"：没有观测数据就无达标可言，显示灰色"样本不足"避免误判。
        var sampleCount = finiteMetric(item.samples);
        var status = (sampleCount || 0) === 0
          ? '<b style="color:var(--text-3)">样本不足</b>'
          : item.achieving
            ? '<b class="ok">达标</b>'
            : '<b class="bad">未达标</b>';
        return (
          "<tr><td>" + H.esc(item.name) + "</td><td>" + p95 +
          "</td><td>" + rate + "</td><td>" + metricText(sampleCount, "0") +
          "</td><td>" + metricText(item.violations_in_window, "0") + "</td><td>" + status + "</td></tr>"
        );
      })
      .join("");
    el.innerHTML =
      '<div class="owo-mtr-row">周期 ' + metricText(data.period_days, "7") + " 天，共 " + items.length + " 项</div>" +
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>SLO</th><th>p95</th><th>成功率</th><th>样本</th><th>违规</th><th>状态</th></tr></thead>" +
      "<tbody>" + (rows || '<tr><td colspan="6" class="sub">暂无周期数据</td></tr>') + "</tbody></table>" +
      '<div class="sub"><button class="primary" id="owo-mtr-report-refresh">重新加载</button></div>';
    var btn = el.querySelector("#owo-mtr-report-refresh");
    if (btn) btn.addEventListener("click", loadReport);
  }

  function renderTelemetry() {
    var el = document.getElementById("owo-mtr-telemetry");
    if (!el || !state.telemetry) return;
    var t = state.telemetry;
    if (t.error) {
      el.innerHTML = '<span class="sub">遥测端点未就绪（主控接线后可用）</span>';
      return;
    }
    var enabled = !!t.enabled;
    var status = enabled
      ? '<b style="color:var(--yellow)">开（仅聚合指标，不含内容）</b>'
      : '<b class="ok">关（默认）</b>';
    var counters = t.counters && typeof t.counters === "object" ? t.counters : {};
    var codes = t.error_codes && typeof t.error_codes === "object" ? t.error_codes : {};
    var perf = t.performance && typeof t.performance === "object" ? t.performance : {};
    var counterSummary = Object.keys(counters)
      .map(function (k) { return H.esc(k) + "=" + metricText(counters[k]); })
      .join("，") || "无";
    var codeSummary = Object.keys(codes)
      .map(function (k) { return H.esc(k) + "×" + metricText(codes[k]); })
      .join("，") || "无";
    var dict = t.data_dictionary && typeof t.data_dictionary === "object" ? t.data_dictionary : {};
    el.innerHTML =
      '<table class="owo-mtr-table">' +
      "<tr><td>开关</td><td>" + status + "</td></tr>" +
      "<tr><td>功能计数</td><td>" + counterSummary + "</td></tr>" +
      "<tr><td>错误码分布</td><td>" + codeSummary + "</td></tr>" +
      "<tr><td>性能分位</td><td>工具 p50=" + metricText(perf.tool_p50_ms) +
      " ms，p95=" + metricText(perf.tool_p95_ms) + " ms</td></tr>" +
      "<tr><td>数据字典</td><td class=\"sub\">" + H.esc(t.note || "") +
      (dict.note ? "；" + H.esc(dict.note) : "") + "</td></tr>" +
      "</table>";
  }

  function dispose() {
    panelGeneration += 1;
    refreshGeneration += 1;
    reportGeneration += 1;
    reportPending = false;
    sectionEl = null;
  }

  return {
    id: id,
    title: "可观测性",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { loadReport: loadReport },
  };
})();
