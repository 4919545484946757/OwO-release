/* R5 Agent 3 面板：eval 护栏（历史列表/报告详情/运行按钮）。
 * 纯脚本 IIFE，注册 window.OwoPanels.eval；helpers 防御性降级。
 * 报告详情为结构化卡片（概览 + 失败用例列表），无裸 JSON 直出；
 * 面板内查找一律限定 sectionEl（挂载后赋值），异步渲染带"已卸载则跳过"防御。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.eval = (function () {
  "use strict";

  var id = "eval";
  var sectionEl = null;

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

  var H = defaultHelpers();
  var state = { reports: [], current: null, currentFile: "", running: false };
  var lifecycleGeneration = 0;
  var historyGeneration = 0;
  var detailGeneration = 0;

  function nav() {
    return (
      '<section data-panel="' + id + '" class="owo-eval-panel">' +
      '<style>' +
      '.owo-eval-row{display:flex;gap:8px;align-items:center;padding:4px 0;border-bottom:1px solid var(--border)}' +
      '.owo-eval-item{cursor:pointer}' +
      '.owo-eval-item{width:100%;text-align:left;color:inherit;background:transparent;border:0;font:inherit}' +
      '.owo-eval-item:hover{background:var(--surface-2)}' +
      '.owo-eval-badge{display:inline-block;padding:1px 6px;border-radius:8px;font-size:12px;color:var(--accent-ink)}' +
      '.owo-eval-badge.ok{background:var(--green)}.owo-eval-badge.bad{background:var(--red)}' +
      '.owo-eval-badge.warn{background:var(--yellow)}' +
      // 报告详情卡（原 pre JSON 直出 → 结构化卡片）
      '.owo-eval-detail{display:flex;flex-direction:column;gap:8px}' +
      '.owo-eval-detail-card{border:1px solid var(--border);border-radius:8px;padding:8px 10px;background:var(--surface-2);display:flex;flex-direction:column;gap:4px}' +
      '.owo-eval-detail-head{display:flex;gap:8px;align-items:center;flex-wrap:wrap}' +
      '.owo-eval-meta{color:var(--text-3);font-size:12px}' +
      '.owo-eval-failure{border:1px solid var(--red);border-radius:8px;padding:8px 10px;background:var(--red-soft);display:flex;flex-direction:column;gap:4px}' +
      '.owo-eval-failure-head{display:flex;gap:8px;align-items:baseline;justify-content:space-between}' +
      '.owo-eval-failure-err{font-size:12px;color:var(--red)}' +
      '.owo-eval-failure-out{font-size:12px;color:var(--text-2);word-break:break-all}' +
      '.owo-eval-allok{border:1px solid var(--green);border-radius:8px;padding:8px 10px;background:var(--green-soft);color:var(--green);font-size:12px}' +
      '</style>' +
      '<div class="stack">' +
      '<div class="sub">eval 护栏（真实模型，缺 OPENAI_API_KEY 自动跳过）</div>' +
      '<div class="owo-eval-row">' +
      '<input id="owo-eval-suite" placeholder="套件（留空=内置 builtin；可填 .json 路径）" style="flex:1">' +
      '<button class="primary" id="owo-eval-run" data-core-action>运行</button></div>' +
      '<div id="owo-eval-status" class="sub">—</div>' +
      '<div class="sub">历史报告</div>' +
      '<div id="owo-eval-list" class="list"></div>' +
      '<details><summary>最新报告详情</summary><div class="owo-eval-detail" id="owo-eval-detail"></div></details>' +
      '</div>' +
      '</section>'
    );
  }

  function mount(root, helpers) {
    lifecycleGeneration += 1;
    historyGeneration += 1;
    detailGeneration += 1;
    if (helpers) H = helpers;
    root.innerHTML = nav();
    sectionEl = root.querySelector(".owo-eval-panel");
    var runButton = sectionEl.querySelector("#owo-eval-run");
    runButton.disabled = state.running;
    runButton.addEventListener("click", run);
    if (state.running) $("#owo-eval-status").textContent = "评测仍在运行…";
    refresh();
  }

  // 面板可能已被切换/卸载（异步返回时），此时静默跳过渲染
  function alive() {
    return sectionEl && document.contains(sectionEl);
  }
  function current(generation) {
    return generation === lifecycleGeneration && alive();
  }
  function $(sel) {
    return alive() ? sectionEl.querySelector(sel) : null;
  }

  // "20260924T035000Z" → "2026-09-24 03:50:00"（无法解析时原样返回）
  function prettyTs(ts) {
    var m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/.exec(String(ts || ""));
    if (!m) return String(ts || "");
    return m[1] + "-" + m[2] + "-" + m[3] + " " + m[4] + ":" + m[5] + ":" + m[6];
  }

  function run() {
    if (state.running) return;
    var suite = $("#owo-eval-suite").value.trim();
    var btn = $("#owo-eval-run");
    if (!btn) return;
    var generation = lifecycleGeneration;
    btn.disabled = true;
    state.running = true;
    $("#owo-eval-status").textContent = "正在运行评测…";
    H.post("/eval/gate/run", suite ? { suite: suite } : {})
      .then(function (data) {
        if (!current(generation)) return;
        var status = $("#owo-eval-status");
        if (data.skipped) {
          status.textContent = "已跳过：" + (data.reason || "无凭据");
          status.className = "sub";
        } else {
          var r = data.report || {};
          status.textContent =
            "套件 " + (r.suite || "?") + " 通过率 " + (r.pass_rate * 100).toFixed(1) + "%（" +
            r.passed + "/" + r.total + "）耗时 " + (r.total_duration_ms / 1000).toFixed(1) + "s 模型 " + (r.model || "?");
          status.className = "sub";
          state.current = r;
          state.currentFile = r.file || "";
          renderDetail();
        }
        return refresh();
      })
      .catch(function (e) {
        if (!current(generation)) return;
        var status = $("#owo-eval-status");
        status.textContent = H.friendlyError(e);
        status.className = "owo-eval-badge bad";
      })
      .finally(function () {
        state.running = false;
        if (alive()) {
          $("#owo-eval-run").disabled = false;
          if (generation !== lifecycleGeneration) {
            $("#owo-eval-status").textContent = "评测已结束；历史报告已刷新";
            refresh();
          }
        }
      });
  }

  function refresh() {
    var generation = lifecycleGeneration;
    var request = ++historyGeneration;
    return H.get("/eval/gate/reports")
      .then(function (data) {
        if (!current(generation) || request !== historyGeneration) return;
        state.reports = (data && data.reports) || [];
        renderList();
      })
      .catch(function (e) {
        if (!current(generation) || request !== historyGeneration) return;
        var el = $("#owo-eval-list");
        if (el) el.innerHTML = '<div class="owo-eval-badge bad">' + H.esc(H.friendlyError(e)) + "</div>";
      });
  }

  function renderList() {
    var el = $("#owo-eval-list");
    if (!el) return;
    if (!state.reports.length) {
      el.innerHTML = '<div class="sub">暂无报告（点"运行"生成；无凭据会提示跳过原因）</div>';
      return;
    }
    el.innerHTML = state.reports
      .map(function (r) {
        var badge = (r.pass_rate || 0) >= 0.8 ? "ok" : (r.pass_rate || 0) > 0 ? "warn" : "bad";
        return (
          '<button type="button" class="owo-eval-row owo-eval-item" data-file="' + H.esc(r.file) + '">' +
          '<span class="owo-eval-badge ' + badge + '">' + Math.round((r.pass_rate || 0) * 100) + "%</span> " +
          "<strong>" + H.esc(r.suite || "?") + "</strong>" +
          '<span class="sub">' + (r.passed || 0) + "/" + (r.total || 0) + " ｜ " +
          H.esc(prettyTs((r.timestamp || "").slice(0, 19))) + " ｜ " + H.esc(r.model || "") + "</span>" +
          "</button>"
        );
      })
      .join("");
    var buttons = el.querySelectorAll(".owo-eval-item");
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].addEventListener("click", function () {
        var file = this.getAttribute("data-file") || "";
        if (!file) return;
        var generation = lifecycleGeneration;
        var request = ++detailGeneration;
        H.get("/eval/gate/report?file=" + encodeURIComponent(file))
          .then(function (data) {
            if (!current(generation) || request !== detailGeneration) return;
            state.current = data.report || null;
            state.currentFile = data.file || "";
            renderDetail();
          })
          .catch(function (e) {
            if (!current(generation) || request !== detailGeneration) return;
            var el = $("#owo-eval-detail");
            if (el) el.innerHTML = '<div class="owo-eval-badge bad">' + H.esc(H.friendlyError(e)) + "</div>";
          });
      });
    }
  }

  function renderDetail() {
    var el = $("#owo-eval-detail");
    if (!el) return;
    var r = state.current;
    if (!r) {
      el.innerHTML = '<div class="sub">—（点上方任一报告查看）</div>';
      return;
    }
    var rate = (r.pass_rate || 0) * 100;
    var badge = rate >= 80 ? "ok" : rate > 0 ? "warn" : "bad";
    var html =
      '<div class="owo-eval-detail-card">' +
      '<div class="owo-eval-detail-head">' +
      '<span class="owo-eval-badge ' + badge + '">' + rate.toFixed(1) + "%</span>" +
      "<strong>" + H.esc(r.suite || "?") + "</strong>" +
      '<span class="sub">' + H.esc(r.passed || 0) + "/" + H.esc(r.total || 0) + " 通过 ｜ " +
      ((r.total_duration_ms || 0) / 1000).toFixed(1) + "s ｜ " + H.esc(r.model || "?") + "</span>" +
      "</div>" +
      '<div class="owo-eval-meta">' + H.esc(prettyTs(r.timestamp)) +
      (state.currentFile ? " ｜ 文件 " + H.esc(state.currentFile) : "") + "</div>" +
      "</div>";
    var failures = r.failures || [];
    if (!failures.length) {
      html += '<div class="owo-eval-allok">全部用例通过</div>';
    } else {
      for (var i = 0; i < failures.length; i++) {
        var f = failures[i] || {};
        html +=
          '<div class="owo-eval-failure">' +
          '<div class="owo-eval-failure-head">' +
          "<strong>" + H.esc(f.name || "?") + "</strong>" +
          '<span class="sub">' + H.esc(f.duration_ms || 0) + "ms</span>" +
          "</div>" +
          (f.error ? '<div class="owo-eval-failure-err">' + H.esc(f.error) + "</div>" : "") +
          (f.output ? '<div class="owo-eval-failure-out">' + H.esc(f.output) + "</div>" : "") +
          "</div>";
      }
    }
    el.innerHTML = html;
  }

  return {
    id: id,
    title: "eval 护栏",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: function () {
      lifecycleGeneration += 1;
      historyGeneration += 1;
      detailGeneration += 1;
      sectionEl = null;
    },
  };
})();
