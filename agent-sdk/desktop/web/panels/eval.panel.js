// 模型评测：服务执行、历史报告和可核对的结果详情。
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.eval = (function () {
  "use strict";
  var id = "eval", H = null, sectionEl = null, state = null;
  var scopes = Object.create(null), tabs = ["run", "history", "detail"];
  var lifecycleGeneration = 0, historyGeneration = 0, detailGeneration = 0;
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
    return { baseUrl: baseUrl, get: get, post: post, esc: esc, friendlyError: friendlyError };
  }

  function alive() { return sectionEl && document.contains(sectionEl); }
  function current(generation, owner) { return generation === lifecycleGeneration && (!owner || owner === state) && alive(); }
  function $(selector) { return alive() ? sectionEl.querySelector(selector) : null; }
  function nav() {
    return '<section data-panel="eval" data-layout="custom" class="owo-eval-panel owo-insight">' +
      '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="模型评测">' +
      tabs.map(function (tab, i) {
        return '<button type="button" role="tab" id="eval-tab-' + tab + '" data-eval-tab="' + tab +
          '" aria-controls="eval-pane-' + tab + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' + ["运行评测", "历史报告", "报告详情"][i] + '</button>';
      }).join("") + '</div><button type="button" id="owo-eval-refresh">刷新历史</button></div>' +
      '<section role="tabpanel" id="eval-pane-run" aria-labelledby="eval-tab-run"><h3>运行评测</h3>' +
      '<p class="sub">运行会调用已配置的模型并可能产生费用；凭据缺失时由服务跳过。</p>' +
      '<label for="owo-eval-suite">套件路径</label><input id="owo-eval-suite" placeholder="留空使用内置套件">' +
      '<label for="owo-eval-model">模型（可选）</label><input id="owo-eval-model" placeholder="留空使用核心默认">' +
      '<div class="owo-insight-actions"><button type="button" class="primary" id="owo-eval-run" data-core-action>运行评测</button></div>' +
      '<section class="owo-insight-section"><h3>运行状态</h3><div id="owo-eval-status" class="sub" role="status" aria-live="polite">尚未运行</div></section></section>' +
      '<section role="tabpanel" id="eval-pane-history" aria-labelledby="eval-tab-history" hidden><h3>历史报告</h3>' +
      '<label for="owo-eval-search">搜索报告</label><input type="search" id="owo-eval-search" placeholder="按套件、模型或文件名搜索">' +
      '<div id="owo-eval-history-status" class="sub" role="status"></div><div id="owo-eval-list" class="owo-eval-list"></div></section>' +
      '<section role="tabpanel" id="eval-pane-detail" aria-labelledby="eval-tab-detail" hidden><h3>报告详情</h3>' +
      '<div class="owo-eval-detail" id="owo-eval-detail"><div class="owo-insight-empty">选择一份历史报告查看详情</div></div></section></section>';
  }
  function selectTab(tab) {
    if (!alive() || tabs.indexOf(tab) < 0) return;
    state.tab = tab;
    tabs.forEach(function (id) {
      var btn = $('[data-eval-tab="' + id + '"]');
      if (btn && btn.setAttribute) { btn.setAttribute("aria-selected", String(tab === id)); btn.setAttribute("tabindex", tab === id ? "0" : "-1"); }
      var pane = $("#eval-pane-" + id); if (pane) pane.hidden = tab !== id;
    });
  }
  function status(message, error) {
    state.status = message; state.error = !!error;
    var el = $("#owo-eval-status"); if (el) { el.textContent = message; el.className = error ? "owo-insight-error" : "sub"; }
  }
  function runUi() {
    var button = $("#owo-eval-run"); if (!button) return;
    button.disabled = !!state.running;
    if (button.setAttribute) button.setAttribute("aria-busy", String(!!state.running));
    button.textContent = state.running ? "评测运行中…" : "运行评测";
    if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, !!state.running);
  }
  function mount(root, helpers) {
    dispose();
    H = Object.assign(defaultHelpers(), helpers || {});
    var scope = (H.baseUrl || window.location.origin) + "#" + (H.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
    state = scopes[scope] || (scopes[scope] = {reports:[],current:null,currentFile:"",running:null,suite:"",model:"",search:"",tab:"run",status:"尚未运行"});
    root.innerHTML = nav(); sectionEl = root.querySelector(".owo-eval-panel");
    var generation = lifecycleGeneration, owner = state;
    function bind(selector, event, action) {
      var el = $(selector); if (el) el.addEventListener(event, function (e) { if (current(generation, owner)) return action(e); });
    }
    $("#owo-eval-suite").value = state.suite; $("#owo-eval-model").value = state.model; $("#owo-eval-search").value = state.search;
    bind("#owo-eval-run", "click", run);
    bind("#owo-eval-refresh", "click", refresh);
    bind("#owo-eval-search", "input", function () { state.search = $("#owo-eval-search").value; renderList(); });
    tabs.forEach(function (tab, i) {
      bind('[data-eval-tab="' + tab + '"]', "click", function () { selectTab(tab); });
      bind('[data-eval-tab="' + tab + '"]', "keydown", function (e) {
        var next = e.key === "ArrowRight" ? (i + 1) % 3 : e.key === "ArrowLeft" ? (i + 2) % 3 : e.key === "Home" ? 0 : e.key === "End" ? 2 : -1;
        if (next < 0) return; e.preventDefault(); selectTab(tabs[next]); $('[data-eval-tab="' + tabs[next] + '"]').focus();
      });
    });
    selectTab(state.tab); runUi(); status(state.running ? "评测仍在运行…" : state.status, state.error);
    if (state.current) renderDetail();
    renderList(); refresh();
  }
  function prettyTs(ts) {
    var raw = String(ts || ""), m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/.exec(raw);
    if (m) raw = m[1] + "-" + m[2] + "-" + m[3] + "T" + m[4] + ":" + m[5] + ":" + m[6] + "Z";
    var d = new Date(raw); return isNaN(d.getTime()) ? String(ts || "—") : d.toLocaleString();
  }
  function count(value) { return typeof value === "number" && isFinite(value) && value >= 0 && Math.floor(value) === value ? value : null; }
  function rate(r) { return typeof r.pass_rate === "number" && isFinite(r.pass_rate) && r.pass_rate >= 0 && r.pass_rate <= 1 ? r.pass_rate : null; }
  function rateText(r) { var total = count(r.total), passed = count(r.passed); return total == null || total === 0 || passed == null || passed > total || rate(r) == null || Math.abs(rate(r) - passed / total) > 0.000001 ? "—" : (rate(r) * 100).toFixed(1) + "%"; }
  function valueText(value) { return value == null ? "—" : String(value); }
  function summary(r) {
    return "套件 " + (r.suite || "—") + " · 通过率 " + rateText(r) + "（" + valueText(count(r.passed)) + "/" + valueText(count(r.total)) +
      "）· 耗时 " + (typeof r.total_duration_ms === "number" && r.total_duration_ms >= 0 ? (r.total_duration_ms / 1000).toFixed(1) + "s" : "—") + " · 模型 " + (r.model || "—");
  }
  function run() {
    if (!alive() || state.running) return Promise.resolve();
    var suite = $("#owo-eval-suite").value.trim(), model = $("#owo-eval-model").value.trim();
    state.suite = suite; state.model = model;
    var generation = lifecycleGeneration, owner = state, token = {}, detailAtStart = detailGeneration;
    var body = {}; if (suite) body.suite = suite; if (model) body.model = model;
    owner.running = token; runUi(); status("正在运行评测…");
    var pending;
    try { pending = H.post("/eval/gate/run", body); } catch (error) { pending = Promise.reject(error); }
    return Promise.resolve(pending).then(function (data) {
      if (!data || typeof data !== "object") throw new Error("评测响应不完整");
      if (data.skipped === true) { owner.status = "已跳过：" + (data.reason || "服务未说明原因"); owner.error = false; }
      else {
        if (!data.report || typeof data.report !== "object" || Array.isArray(data.report)) throw new Error("服务未返回评测报告，无法确认结果");
        owner.status = summary(data.report); owner.error = false;
        if (detailAtStart === detailGeneration && owner === state) { owner.current = data.report; owner.currentFile = data.report.file || ""; }
      }
      if (owner !== state || !alive()) return;
      status((generation === lifecycleGeneration ? "" : "评测已结束：") + owner.status);
      if (data.report && detailAtStart === detailGeneration) {
        renderDetail(); if (generation === lifecycleGeneration) selectTab("detail");
      }
      return refresh();
    }).catch(function (error) {
      owner.status = "评测失败：" + H.friendlyError(error); owner.error = true;
      if (owner === state && alive()) status(owner.status, true);
    }).finally(function () {
      if (owner.running !== token) return;
      owner.running = null;
      if (owner === state && alive()) runUi();
    });
  }
  function refresh() {
    if (!alive()) return Promise.resolve();
    var generation = lifecycleGeneration, owner = state, request = ++historyGeneration;
    $("#owo-eval-history-status").textContent = "正在读取历史报告…";
    return H.get("/eval/gate/reports").then(function (data) {
      if (!current(generation, owner) || request !== historyGeneration) return;
      if (!data || !Array.isArray(data.reports) || data.reports.some(function (r) { return !r || typeof r !== "object"; })) throw new Error("历史报告响应不完整");
      state.reports = data.reports; $("#owo-eval-history-status").textContent = ""; renderList();
    }).catch(function (error) {
      if (!current(generation, owner) || request !== historyGeneration) return;
      $("#owo-eval-history-status").textContent = "历史报告读取失败：" + H.friendlyError(error) + (state.reports.length ? "（保留上次结果）" : "。可刷新重试。");
    });
  }
  function renderList() {
    var el = $("#owo-eval-list"); if (!el) return;
    var query = String(state.search || "").trim().toLowerCase(), generation = lifecycleGeneration, owner = state;
    var reports = state.reports.filter(function (r) { return !query || [r.suite,r.model,r.file].join(" ").toLowerCase().indexOf(query) >= 0; });
    el.innerHTML = reports.length ? reports.map(function (r) {
      return '<button type="button" class="owo-eval-row owo-eval-item" data-file="' + H.esc(r.file || "") + '" aria-pressed="' + (state.currentFile === r.file) + '">' +
        '<span class="owo-eval-badge">' + H.esc(rateText(r)) + '</span><span><strong>' + H.esc(r.suite || "未命名套件") +
        '</strong><span class="sub">' + H.esc(valueText(count(r.passed)) + "/" + valueText(count(r.total)) + " · " + prettyTs(r.timestamp) + " · " + (r.model || "—")) + '</span></span></button>';
    }).join("") : '<div class="owo-insight-empty">' + (query ? "没有匹配的报告" : "暂无历史报告") + '</div>';
    var buttons = el.querySelectorAll(".owo-eval-item");
    for (var i = 0; i < buttons.length; i++) buttons[i].addEventListener("click", function () {
      if (!current(generation, owner)) return;
      return loadDetail(this.getAttribute("data-file") || "");
    });
  }
  function loadDetail(file) {
    if (!alive() || !file) return Promise.resolve();
    var generation = lifecycleGeneration, owner = state, request = ++detailGeneration;
    selectTab("detail"); $("#owo-eval-detail").textContent = "正在读取 " + file + "…";
    return H.get("/eval/gate/report?file=" + encodeURIComponent(file)).then(function (data) {
      if (!current(generation, owner) || request !== detailGeneration) return;
      if (!data || !data.report || typeof data.report !== "object" || Array.isArray(data.report)) throw new Error("报告响应不完整");
      state.current = data.report; state.currentFile = data.file || file; renderDetail(); renderList();
    }).catch(function (error) {
      if (!current(generation, owner) || request !== detailGeneration) return;
      $("#owo-eval-detail").innerHTML = '<p class="owo-insight-error">' + H.esc(H.friendlyError(error)) + '</p><button type="button" id="owo-eval-detail-retry">重新读取报告</button>';
      var retry = $("#owo-eval-detail-retry");
      if (retry) retry.addEventListener("click", function () { if (current(generation, owner)) return loadDetail(file); });
    });
  }
  function renderDetail() {
    var el = $("#owo-eval-detail"), r = state.current; if (!el || !r) return;
    var esc = H.esc, total = count(r.total), passed = count(r.passed), failures = Array.isArray(r.failures) ? r.failures : null;
    var html = '<div class="owo-eval-overview"><h3>' + esc(r.suite || "未命名套件") + '</h3><p>' + esc(summary(r)) + '</p><p class="sub">' +
      esc(prettyTs(r.timestamp)) + (state.currentFile ? " · " + esc(state.currentFile) : "") + '</p></div>';
    var verdict, okay = false;
    if (total === 0) verdict = "报告未执行用例，不计算通过率。";
    else if (total == null || passed == null || passed > total) verdict = "报告统计不完整，无法判断通过情况。";
    else if (rate(r) != null && Math.abs(rate(r) - passed / total) > 0.000001) verdict = "通过率与用例计数不一致，请检查报告。";
    else if (!failures) verdict = "报告未提供失败明细。";
    else if (passed === total && !failures.length) { verdict = "全部用例通过"; okay = true; }
    else if (!failures.length) verdict = "有未通过用例，报告未提供失败明细。";
    else verdict = "未通过用例 " + failures.length + " 项";
    html += '<p class="' + (okay ? "owo-eval-allok" : "sub") + '">' + esc(verdict) + '</p>';
    (failures || []).forEach(function (f) {
      f = f || {};
      html += '<section class="owo-eval-failure"><div class="owo-eval-failure-head"><strong>' + esc(f.name || "未命名用例") +
        '</strong><span class="sub">' + esc(typeof f.duration_ms === "number" ? f.duration_ms + "ms" : "耗时未知") + '</span></div>' +
        (f.error ? '<p class="owo-insight-error">' + esc(f.error) + '</p>' : "") + (f.output ? '<pre>' + esc(f.output) + '</pre>' : "") + '</section>';
    });
    el.innerHTML = html;
  }
  function dispose() {
    if (alive() && state) { state.suite = $("#owo-eval-suite").value; state.model = $("#owo-eval-model").value; state.search = $("#owo-eval-search").value; }
    lifecycleGeneration += 1; historyGeneration += 1; detailGeneration += 1; sectionEl = null;
  }
  return {id:id,title:"模型评测",nav:nav,mount:mount,refresh:refresh,dispose:dispose,_test:{loadDetail:loadDetail,renderDetail:renderDetail}};
})();
