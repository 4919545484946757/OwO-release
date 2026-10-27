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
  var activeTab = "overview";
  function el(id) { return sectionEl && sectionEl.querySelector("#"+id); }
  function selectTab(value) {
    var names=["overview","usage","quality","telemetry"];
    activeTab=names.indexOf(value)<0?"overview":value;
    names.forEach(function(name){
      var button=el("owo-mtr-tab-"+name), pane=el("owo-mtr-pane-"+name);
      if(button && button.setAttribute){button.setAttribute("aria-selected",String(name===activeTab));button.setAttribute("tabindex",name===activeTab?"0":"-1");}
      if(pane)pane.hidden=name!==activeTab;
    });
  }
  function localTime(value) {
    if(!value)return "更新时间未知";
    var date=new Date(value);
    return Number.isNaN(date.getTime())?"更新时间未知":"更新于 "+date.toLocaleString("zh-CN",{hour12:false})+"（本地时间）";
  }
  function validObject(data) {
    if(!data || typeof data!=="object" || Array.isArray(data))throw new Error("指标响应格式不完整");
    if(data.error)throw new Error(typeof data.error==="string"?data.error:"指标读取失败");
    return data;
  }
  function validArray(data,key) {
    validObject(data);
    if(!Array.isArray(data[key]))throw new Error("指标响应格式不完整");
    return data[key];
  }

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

  function refreshLabel() { return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6v6h-6"/><path d="M20 12a8 8 0 1 0-2.4 5.7"/></svg>刷新'; }

  function nav() {
    var names=["overview","usage","quality","telemetry"], labels=["概览","用量与预算","服务质量","遥测"];
    function section(title,body,caption) {
      return '<section class="owo-insight-section"><h3>'+title+'</h3>'+(caption?'<p class="sub">'+caption+'</p>':"")+body+'</section>';
    }
    function box(id) { return '<div id="owo-mtr-'+id+'" class="owo-mtr-box" aria-live="polite">正在加载…</div>'; }
    return '<section data-panel="observability" data-layout="custom" class="owo-insight owo-mtr">'+
      '<div class="owo-insight-toolbar"><div role="tablist" aria-label="指标分区">'+names.map(function(name,index){return '<button type="button" role="tab" id="owo-mtr-tab-'+name+'" aria-controls="owo-mtr-pane-'+name+'">'+labels[index]+'</button>';}).join("")+
      '</div><button type="button" id="owo-mtr-refresh">'+refreshLabel()+'</button></div><p id="owo-mtr-updated" class="sub" role="status">正在加载指标…</p>'+
      '<div role="tabpanel" id="owo-mtr-pane-overview" aria-labelledby="owo-mtr-tab-overview">'+box("cards")+
      section("回合耗时",box("chart"),"最近 50 次 · ms")+
      section("工具调用排行",'<div class="owo-insight-table-wrap" tabindex="0" role="region" aria-label="工具调用排行，可横向滚动"><table class="owo-mtr-table" id="owo-mtr-tools"><thead><tr><th>工具</th><th>调用</th><th>失败</th><th>失败率</th></tr></thead><tbody></tbody></table></div>')+
      section("组件健康",box("health"))+section("运行时指标",box("runtime"))+'</div>'+
      '<div role="tabpanel" id="owo-mtr-pane-usage" aria-labelledby="owo-mtr-tab-usage" hidden>'+section("用量与预算",box("usage"))+'</div>'+
      '<div role="tabpanel" id="owo-mtr-pane-quality" aria-labelledby="owo-mtr-tab-quality" hidden>'+section("SLO 基线",box("slo"))+
      section("SLO 告警",box("alerts"))+section("周期报告",'<div id="owo-mtr-report"><button type="button" id="owo-mtr-report-refresh">加载周报</button></div>',"最近 7 天窗口")+'</div>'+
      '<div role="tabpanel" id="owo-mtr-pane-telemetry" aria-labelledby="owo-mtr-tab-telemetry" hidden>'+section("可选遥测",box("telemetry"),"仅显示核心返回的聚合指标状态。")+'</div></section>';
  }

  function mount(root, helpers) {
    dispose();
    H = helpers || defaultHelpers();
    root.innerHTML = nav();
    sectionEl = root.querySelector('[data-panel="observability"]');
    root.querySelector("#owo-mtr-refresh").addEventListener("click", refresh);
    bindReportReload(root.querySelector("#owo-mtr-report"));
    var owner=panelGeneration;
    ["overview","usage","quality","telemetry"].forEach(function(name,index,names) {
      var button=el("owo-mtr-tab-"+name);
      if(!button)return;
      button.addEventListener("click",function(){if(owner===panelGeneration)selectTab(name);});
      button.addEventListener("keydown",function(event){
        if(owner!==panelGeneration)return;
        var next=event.key==="ArrowRight"?(index+1)%names.length:event.key==="ArrowLeft"?(index+names.length-1)%names.length:event.key==="Home"?0:event.key==="End"?names.length-1:-1;
        if(next<0)return;event.preventDefault();selectTab(names[next]);var target=el("owo-mtr-tab-"+names[next]);if(target.focus)target.focus();
      });
    });
    selectTab(activeTab);refresh();
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
    if(!sectionEl)return Promise.resolve();
    var request = ++refreshGeneration;
    var owner = panelGeneration;
    var refreshButton = sectionEl && sectionEl.querySelector("#owo-mtr-refresh");
    if (refreshButton) {
      refreshButton.disabled = true;
      refreshButton.setAttribute("aria-busy", "true");
      refreshButton.textContent = "刷新中…";
    }
    var endpoints = [
      ["/metrics/overview", function (data) { validObject(data); if(data.traces_count===undefined)throw new Error("概览响应缺少回合记录"); state.overview = data; renderCards(); }, "#owo-mtr-cards"],
      ["/metrics/turns?limit=50", function (data) { state.turns = validArray(data,"turns"); renderChart(); }, "#owo-mtr-chart"],
      ["/metrics/tools", function (data) { state.tools = validArray(data,"tools").filter(Boolean); renderTools(); }, "#owo-mtr-tools tbody"],
      ["/metrics/health", function (data) { validObject(data);validObject(data.components);state.health = data; renderHealth(); }, "#owo-mtr-health"],
      ["/metrics/runtime", function (data) { state.runtime = validObject(data); renderRuntime(); }, "#owo-mtr-runtime"],
      ["/metrics/slo", function (data) { validArray(data,"slo");state.slo=data; renderSlo(); }, "#owo-mtr-slo"],
      ["/usage/summary", function (data) { validArray(data,"dimensions");state.usage = data; renderUsage(); }, "#owo-mtr-usage"],
      ["/metrics/slo/alerts", function (data) { validArray(data,"rules");validArray(data,"alerts");state.alerts=data; renderAlerts(); }, "#owo-mtr-alerts"],
      ["/metrics/telemetry/status", function (data) { validObject(data);if(typeof data.enabled!=="boolean")throw new Error("遥测开关状态未知");state.telemetry=data; renderTelemetry(); }, "#owo-mtr-telemetry"],
    ];
    return Promise.all(endpoints.map(function (endpoint) {
      return H.get(endpoint[0])
        .then(function (data) {
          if (request !== refreshGeneration || owner !== panelGeneration) return;
          var target=sectionEl && sectionEl.querySelector(endpoint[2]);
          if(target && target.style)target.style.color="";
          endpoint[1](data);
        })
        .catch(function (error) {
          if (request !== refreshGeneration || owner !== panelGeneration) return;
          showMetricError(endpoint[2], error);
          if(endpoint[0]==="/metrics/overview"){var updated=el("owo-mtr-updated");if(updated)updated.textContent="概览读取失败，可刷新重试。";}
        });
    })).finally(function () {
      if (request !== refreshGeneration || owner !== panelGeneration) return;
      var button = sectionEl && sectionEl.querySelector("#owo-mtr-refresh");
      if (button) {
        button.disabled = false;
        button.removeAttribute("aria-busy");
        button.innerHTML = refreshLabel();
      }
    });
  }

  function bindReportReload(container) {
    if (!container) return;
    var button = container.querySelector("#owo-mtr-report-refresh");
    if (button) button.addEventListener("click", loadReport);
  }

  function renderReportError(error) {
    var target = sectionEl && sectionEl.querySelector("#owo-mtr-report");
    if (!target) return;
    target.innerHTML = '<div role="alert" style="color:var(--red)">周报加载失败：' + H.esc(H.friendlyError(error)) +
      '</div><button class="primary" id="owo-mtr-report-refresh">重试</button>';
    bindReportReload(target);
    var retry = target.querySelector("#owo-mtr-report-refresh");
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
      .then(function () { return owner===panelGeneration ? H.get("/metrics/slo/report?days=7") : null; })
      .then(function (data) {
        if (request !== reportGeneration || owner !== panelGeneration) return null;
        validArray(data,"slo");state.report = data;
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
    var target = el("owo-mtr-cards");
    if (!target || !state.overview) return;
    var o = state.overview;
    target.innerHTML =
      card(o.traces_count, "回合记录") +
      card(o.avg_turn_ms, "平均耗时", " ms") +
      card(o.p50_ms, "p50", " ms") +
      card(o.p95_ms, "p95", " ms") +
      card(o.tool_calls_total, "工具调用") +
      card(o.approvals_total, "审批") +
      card(o.denied, "拒绝") +
      card(o.failures, "失败");
    var updated = el("owo-mtr-updated");
    if(updated)updated.textContent=localTime(o.updated_at);
  }

  function card(value, label, unit) {
    var number=finiteMetric(value);
    return '<div class="owo-mtr-card"><span>'+H.esc(label)+'</span><b>'+metricText(value)+(number==null?"":H.esc(unit || ""))+'</b></div>';
  }

  function renderChart() {
    var target = el("owo-mtr-chart");
    if (!target) return;
    var data = state.turns;
    if (!data.length) {
      target.textContent = "暂无回合数据";
      return;
    }
    var values = data
      .filter(function (turn) { return turn && typeof turn === "object"; })
      .map(function (turn) { return finiteMetric(turn.duration_ms); })
      .filter(function (duration) { return duration != null && duration >= 0; })
      .slice(0, 50)
      .reverse();
    if (!values.length) {
      target.textContent = "暂无有效回合耗时数据";
      return;
    }
    var width=720,height=168,left=48,right=22,top=24,bottom=28;
    var maximum=Math.max.apply(null,values.concat([1]));
    var magnitude=Math.pow(10,Math.floor(Math.log10(maximum)));
    var ceiling=Math.ceil(maximum/magnitude)*magnitude;
    var innerWidth=width-left-right,innerHeight=height-top-bottom;
    var points=values.map(function(value,index){return [left+(values.length===1?innerWidth/2:index/(values.length-1)*innerWidth),top+innerHeight-value/ceiling*innerHeight];});
    var ticks="";
    for(var i=0;i<=4;i++) {
      var y=top+innerHeight-i/4*innerHeight;
      ticks+='<line x1="'+left+'" x2="'+(width-right)+'" y1="'+y+'" y2="'+y+'" class="owo-mtr-grid"/>'+
        '<text x="'+(left-8)+'" y="'+(y+4)+'" text-anchor="end">'+H.esc(String(ceiling*i/4))+'</text>';
    }
    target.innerHTML='<div class="owo-insight-table-wrap" tabindex="0" role="region" aria-label="回合耗时图，可横向滚动">'+
      '<svg class="owo-mtr-chart" viewBox="0 0 '+width+' '+height+'" role="img" aria-label="'+H.esc("最近 "+values.length+" 次回合耗时，峰值 "+maximum+" ms")+'">'+
      '<title>'+H.esc(values.map(function(v,i){return "第"+(i+1)+"次："+v+" ms";}).join("；"))+'</title>'+ticks+
      '<polyline points="'+points.map(function(point){return point.map(function(v){return v.toFixed(1);}).join(",");}).join(" ")+'" fill="none" class="owo-mtr-line"/>'+
      points.map(function(point,index){return '<circle cx="'+point[0].toFixed(1)+'" cy="'+point[1].toFixed(1)+'" r="3" class="owo-mtr-dot"><title>'+H.esc(String(values[index]))+' ms</title></circle>'+
        (values.length<=6?'<text x="'+point[0].toFixed(1)+'" y="'+(point[1]-9).toFixed(1)+'" text-anchor="middle" class="owo-mtr-value">'+H.esc(String(values[index]))+'</text>':"");}).join("")+
      '<text x="'+left+'" y="'+(height-4)+'">较早</text><text x="'+(width-right)+'" y="'+(height-4)+'" text-anchor="end">最近</text></svg></div>';
  }

  function renderTools() {
    var target = sectionEl && sectionEl.querySelector("#owo-mtr-tools tbody");
    if (!target) return;
    if (!state.tools.length) {
      target.innerHTML = '<tr><td colspan="4" class="sub">暂无工具调用</td></tr>';
      return;
    }
    target.innerHTML = state.tools
      .map(function (t) {
        return (
          "<tr><td>" + H.esc(t.tool) + "</td><td>" + metricText(t.calls) + "</td><td>" + metricText(t.failures) +
          "</td><td>" + percentageText(t.failure_rate, 1) + "</td></tr>"
        );
      })
      .join("");
  }

  function renderHealth() {
    var target = el("owo-mtr-health");
    if (!target || !state.health) return;
    var c = state.health.components || {};
    var stt=c.stt && c.stt.ready;
    var rows=[["STT",stt===true?"就绪":stt===false?"未就绪":"状态未知"],["云端传输",c.cloud_transport && c.cloud_transport.kind || "状态未知"],
      ["插件",metricText(c.plugins && c.plugins.count)],["笔记",metricText(c.notes && c.notes.count)],["回合记录",metricText(c.traces && c.traces.count)]];
    target.innerHTML='<dl class="owo-mtr-health-list">'+rows.map(function(row){return '<div><dt>'+H.esc(row[0])+'</dt><dd>'+H.esc(row[1])+'</dd></div>';}).join("")+'</dl>';
  }

  function renderRuntime() {
    var target = el("owo-mtr-runtime");
    if (!target || !state.runtime) return;
    var r = state.runtime;
    var tool = r.tool || {};
    var approval = r.approval || {};
    var sse = r.sse || {};
    var events = r.events || {};
    var pct = function (v) { return percentageText(v, 1); };
    var ms = function (v) {
      return v == null ? "—" : H.esc(String(v)) + " ms";
    };
    target.innerHTML =
      '<table class="owo-mtr-table">' +
      "<tr><td>工具调度 p95 / p50</td><td>" + ms(tool.p95_ms) + " / " + ms(tool.p50_ms) + "（样本 " + metricText(tool.samples) + "）</td></tr>" +
      "<tr><td>审批通过率</td><td>" + pct(approval.pass_rate) + "</td></tr>" +
      "<tr><td>审批拦截率</td><td>" + pct(approval.intercept_rate) + "（通过 " + metricText(approval.approved) + " / 拦截 " + metricText(approval.denied) + " / 共 " + metricText(approval.total) + "）</td></tr>" +
      "<tr><td>事件队列深度</td><td>" + metricText(r.queue_depth) + "</td></tr>" +
      "<tr><td>SSE 活跃连接</td><td>" + metricText(sse.active_connections) + "（累计 " + metricText(sse.total_connections) + "，滞后事件 " + metricText(sse.lagged_total) + "）</td></tr>" +
      "<tr><td>事件流发布/丢弃</td><td>" + metricText(events.published) + " / " + metricText(events.dropped) + "</td></tr>" +
      "</table>";
  }

  function renderSlo() {
    var target = el("owo-mtr-slo");
    if (!target || !state.slo) return;
    var items = (Array.isArray(state.slo.slo) ? state.slo.slo : []).filter(function (item) { return item && typeof item === "object"; }).slice().sort(function (a, b) {
      return String(a.name || "").localeCompare(String(b.name || ""));
    });
    if (!items.length) {
      target.innerHTML = '<span class="sub">暂无 SLO 数据</span>';
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
          "</td><td>" + metricText(sampleCount) +
          "</td><td>" + metricText(budget.bad) + " / " + metricText(budget.allowed_bad) +
          "</td><td>" + status + "</td></tr>"
        );
      })
      .join("");
    target.innerHTML =
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>SLO</th><th>目标</th><th>p95</th><th>成功率</th><th>样本</th><th>违规/预算</th><th>状态</th></tr></thead>" +
      "<tbody>" + rows + "</tbody></table>";
  }

  function renderUsage() {
    var target = el("owo-mtr-usage");
    if (!target || !state.usage) return;
    var u = state.usage;
    if (u.error) {
      target.innerHTML = '<span class="sub">用量端点未就绪（主控接线后可用）</span>';
      return;
    }
    var priced=finiteMetric(u.price_per_mtok)>0;
    var dims = Array.isArray(u.dimensions) ? u.dimensions.filter(function (item) { return item && typeof item === "object"; }) : [];
    var rows = dims
      .map(function (d) {
        var budget = d.budget ? "，预算 " + H.esc(String(d.budget.limit_usd)) + " USD" : "";
        var exceeded = d.budget && d.budget.exceeded ? ' <b class="bad">超限</b>' : "";
        return (
          "<tr><td>" + H.esc(d.dimension) + "</td><td>" + metricText(d.calls) +
          "</td><td>" + metricText(d.total_tokens) +
          "</td><td>" + (priced?metricText(d.cost_usd)+(d.cost_usd==null?"":" USD"):"—") +
          "</td><td>" + metricText(d.budget ? d.budget.spent_usd : null) + " / " +
          H.esc(String(d.budget ? d.budget.limit_usd : "—")) + budget + exceeded + "</td></tr>"
        );
      })
      .join("");
    var stop = u.hard_stop
      ? ' <b class="bad">硬熔断中</b>' + (u.hard_stop_reason ? "（" + H.esc(u.hard_stop_reason) + "）" : "")
      : "";
    target.innerHTML =
      "<div class=\"owo-mtr-row\">记录 " + metricText(u.count) + " 条，" + (priced ? "配置单价 "+metricText(u.price_per_mtok)+" $/Mtok（估算）" : "未配置有效单价，成本估算不可用") + stop + "</div>" +
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>维度</th><th>调用</th><th>Token</th><th>估算成本</th><th>花费/预算</th></tr></thead>" +
      "<tbody>" + (rows || '<tr><td colspan="5" class="sub">暂无用量记录</td></tr>') + "</tbody></table>";
  }

  function renderAlerts() {
    var target = el("owo-mtr-alerts");
    if (!target || !state.alerts) return;
    var data = state.alerts;
    if (data.note) {
      target.innerHTML = '<span class="sub">告警探针未注册（主控接线后可用）</span>';
      return;
    }
    var rules = Array.isArray(data.rules) ? data.rules.filter(function (item) { return item && typeof item === "object"; }) : [];
    var ruleHtml = rules
      .map(function (r) {
        return "<tr><td>" + H.esc(r.name) + "</td><td>" + H.esc(r.slo_name) +
          "</td><td>" + H.esc(String(r.kind)) + " &gt; " + H.esc(String(r.threshold)) +
          "</td><td>连续 " + metricText(r.consecutive) + " 次</td><td>" +
          H.esc(r.severity || "") + "</td></tr>";
      })
      .join("");
    var alerts = (Array.isArray(data.alerts) ? data.alerts : []).filter(function (item) { return item && typeof item === "object"; });
    var alertHtml = alerts
      .map(function (a) {
        var color = a.kind === "recovered" ? "var(--green)" : a.severity === "critical" ? "var(--red)" : "var(--yellow)";
        return "<div class=\"owo-mtr-row\"><span style=\"color:" + color + "\">[" + H.esc(a.kind) +
          "] " + H.esc(a.rule) + "</span><span class=\"sub\">" +
          H.esc(localTime(a.at).replace(/^更新于 /,"")) + "</span></div><div class=\"sub\">" +
          H.esc(a.detail || "") + "</div>";
      })
      .join("");
    target.innerHTML =
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>规则</th><th>SLO</th><th>判定</th><th>连续</th><th>级别</th></tr></thead>" +
      "<tbody>" + (ruleHtml || '<tr><td colspan="5" class="sub">暂无规则</td></tr>') + "</tbody></table>" +
      '<div class="sub">最近告警（' + metricText(data.count) + '）</div>' +
      (alertHtml || '<div class="sub">暂无告警</div>');
  }

  function renderReport() {
    var target = sectionEl && sectionEl.querySelector("#owo-mtr-report");
    if (!target || !state.report) return;
    var data = state.report;
    if (data.note) {
      target.innerHTML = '<span class="sub">周期报告探针未注册（主控接线后可用）</span><button id="owo-mtr-report-refresh">重新检查</button>';
      bindReportReload(target);
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
          "</td><td>" + rate + "</td><td>" + metricText(sampleCount) +
          "</td><td>" + metricText(item.violations_in_window) + "</td><td>" + status + "</td></tr>"
        );
      })
      .join("");
    target.innerHTML =
      '<div class="owo-mtr-row">周期 ' + metricText(data.period_days, "7") + " 天，共 " + items.length + " 项</div>" +
      '<table class="owo-mtr-table">' +
      "<thead><tr><th>SLO</th><th>p95</th><th>成功率</th><th>样本</th><th>违规</th><th>状态</th></tr></thead>" +
      "<tbody>" + (rows || '<tr><td colspan="6" class="sub">暂无周期数据</td></tr>') + "</tbody></table>" +
      '<div class="sub"><button class="primary" id="owo-mtr-report-refresh">重新加载</button></div>';
    var btn = target.querySelector("#owo-mtr-report-refresh");
    if (btn) btn.addEventListener("click", loadReport);
  }

  function renderTelemetry() {
    var target = el("owo-mtr-telemetry");
    if (!target || !state.telemetry) return;
    var t = state.telemetry;
    if (t.error) {
      target.innerHTML = '<span class="sub">遥测端点未就绪（主控接线后可用）</span>';
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
    target.innerHTML =
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
    _test: { loadReport: loadReport, selectTab:selectTab, localTime:localTime },
  };
})();
