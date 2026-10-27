// 工具命令：输入、执行结果和审计共享当前 Daemon 的认证传输。
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.command = (function () {
  "use strict";
  var id = "command", H = null, rootEl = null, S = null;
  var panelGeneration = 0, auditGeneration = 0;
  var scopes = Object.create(null);
  var tabs = ["input", "results", "audit"];
  function $(selector) { return rootEl && rootEl.querySelector(selector); }
  function current(owner, state) {
    return owner === panelGeneration && state === S && !!rootEl && (typeof rootEl.isConnected !== "boolean" || rootEl.isConnected);
  }
  function text(selector, value) { var el = $(selector); if (el) el.textContent = value; }
  function setStatus(value, error) {
    if (!S) return;
    S.status = value; S.error = !!error;
    text("#owo-command-status", value);
    var el = $("#owo-command-status"); if (el) el.className = error ? "owo-insight-error" : "sub";
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
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) {
          return r.json().catch(function () { return {}; }).then(function (body) {
            throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
          });
        }
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
    function renderMarkdown(text) {
      return esc(text);
    }
    return { baseUrl: baseUrl, get: get, post: post, esc: esc, friendlyError: friendlyError, renderMarkdown: renderMarkdown };
  }

  function nav() {
    return '<section data-panel="command" data-layout="custom" class="owo-command-panel owo-insight">' +
      '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="工具命令">' +
      tabs.map(function (tab, i) {
        return '<button type="button" role="tab" id="command-tab-' + tab + '" data-command-tab="' + tab +
          '" aria-controls="command-pane-' + tab + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' + ["输入命令", "执行结果", "审计"][i] + '</button>';
      }).join("") + '</div><button type="button" id="owo-command-refresh"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1"/></svg>刷新审计</button></div>' +
      '<section role="tabpanel" id="command-pane-input" aria-labelledby="command-tab-input"><h3>输入命令</h3>' +
      '<p class="sub">文本、WAV 语音或屏幕区域识别。</p><label for="owo-command-mode">输入方式</label>' +
      '<select id="owo-command-mode"><option value="text">文本</option><option value="voice">语音（WAV）</option><option value="region">区域（OCR）</option></select>' +
      '<div id="owo-command-text-field"><label for="owo-command-text">命令内容</label><textarea id="owo-command-text" rows="7" placeholder="例如：搜索记忆：前端验收"></textarea></div>' +
      '<div id="owo-command-voice-field" hidden><label for="owo-command-wav">WAV 文件</label><input type="file" id="owo-command-wav" accept="audio/wav,.wav">' +
      '<p class="sub">选择文件后执行；核心需已配置语音识别。</p></div>' +
      '<p id="owo-command-region-hint" class="sub" hidden>识别指定屏幕区域，文字会回填供你检查；再次执行前不会提交命令。需已授权视觉识别。</p>' +
      '<div class="owo-command-submit"><p class="sub">命令执行仍受权限约束。</p><button type="button" class="primary" id="owo-command-run" data-core-action>执行</button></div>' +
      '<section class="owo-insight-section owo-command-status-line"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/></svg><div id="owo-command-status" class="sub" role="status" aria-live="polite">尚未执行命令</div></section></section>' +
      '<section role="tabpanel" id="command-pane-results" aria-labelledby="command-tab-results" hidden><h3>执行结果</h3>' +
      '<div id="owo-command-intent" class="sub"></div><div id="owo-command-results"><div class="owo-insight-empty">尚未执行命令</div></div></section>' +
      '<section role="tabpanel" id="command-pane-audit" aria-labelledby="command-tab-audit" hidden><h3>命令审计</h3><p class="sub">最近 20 条命令记录。</p>' +
      '<div class="owo-command-log" id="owo-command-audit" role="status">加载中…</div></section></section>';
  }
  function selectTab(tab) {
    if (!rootEl || tabs.indexOf(tab) < 0) return;
    S.tab = tab;
    tabs.forEach(function (id) {
      var btn = $('[data-command-tab="' + id + '"]');
      if (btn && btn.setAttribute) { btn.setAttribute("aria-selected", String(tab === id)); btn.setAttribute("tabindex", tab === id ? "0" : "-1"); }
      var pane = $("#command-pane-" + id); if (pane) pane.hidden = tab !== id;
    });
  }
  function busyButton() {
    var button = $("#owo-command-run"); if (!button) return;
    var busy = !!S.pending;
    button.disabled = busy;
    if (button.setAttribute) button.setAttribute("aria-busy", String(busy));
    button.textContent = busy ? "执行中…" : S.mode === "region" ? "识别区域" : "执行";
    if (window.OwoCoreActionAvailability) window.OwoCoreActionAvailability.setBusy(button, busy);
  }
  function updateMode() {
    if (!rootEl) return;
    S.mode = $("#owo-command-mode").value || "text";
    $("#owo-command-voice-field").hidden = S.mode !== "voice";
    $("#owo-command-text-field").hidden = S.mode === "voice" || S.mode === "region";
    $("#owo-command-region-hint").hidden = S.mode !== "region";
    busyButton();
  }
  function mount(root, helpers) {
    dispose();
    H = Object.assign(defaultHelpers(), helpers || {});
    var scope = (H.baseUrl || window.location.origin) + "#" + (H.coreInstanceId || (window.OwoApi && window.OwoApi.coreInstanceId) || "");
    S = scopes[scope] || (scopes[scope] = { text: "", mode: "text", tab: "input", result: null, status: "尚未执行命令", pending: null });
    rootEl = root; root.innerHTML = nav();
    var owner = panelGeneration, state = S;
    function bind(selector, event, handler) {
      var el = $(selector); if (el) el.addEventListener(event, function (e) { if (current(owner, state)) return handler(e); });
    }
    $("#owo-command-text").value = S.text; $("#owo-command-mode").value = S.mode;
    bind("#owo-command-run", "click", runCommand);
    bind("#owo-command-refresh", "click", refresh);
    bind("#owo-command-text", "input", function () { S.text = $("#owo-command-text").value; });
    bind("#owo-command-text", "keydown", function (e) {
      if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { if (e.preventDefault) e.preventDefault(); return runCommand(); }
    });
    bind("#owo-command-mode", "change", updateMode);
    tabs.forEach(function (tab, i) {
      bind('[data-command-tab="' + tab + '"]', "click", function () { selectTab(tab); });
      bind('[data-command-tab="' + tab + '"]', "keydown", function (e) {
        var next = e.key === "ArrowRight" ? (i + 1) % 3 : e.key === "ArrowLeft" ? (i + 2) % 3 : e.key === "Home" ? 0 : e.key === "End" ? 2 : -1;
        if (next < 0) return;
        e.preventDefault(); selectTab(tabs[next]); $('[data-command-tab="' + tabs[next] + '"]').focus();
      });
    });
    selectTab(S.tab); updateMode(); setStatus(S.pending ? "命令仍在处理中…" : S.status, S.error);
    if (S.result) renderCommand(S.result);
    refresh();
  }
  function refresh() {
    if (!rootEl) return Promise.resolve();
    var owner = panelGeneration, state = S, request = ++auditGeneration;
    return H.get("/command/audit").then(function (data) {
      if (!current(owner, state) || request !== auditGeneration) return;
      if (!data || !Array.isArray(data.audit)) throw new Error("审计响应不完整");
      var entries = data.audit.slice(0, 20);
      // Keep raw event names searchable; they are authoritative audit labels.
      $("#owo-command-audit").textContent = entries.length ? entries.map(function (a) {
        return (a.ts ? String(a.ts) + " " : "") + String(a.event || "") + " — " + String(a.detail || "");
      }).join("\n") : "暂无审计记录";
    }).catch(function (error) {
      if (current(owner, state) && request === auditGeneration) text("#owo-command-audit", "审计记录加载失败：" + H.friendlyError(error) + "。可刷新重试。");
    });
  }
  function readWavBase64(file) {
    return new Promise(function (resolve, reject) {
      var reader = new FileReader();
      reader.onload = function () { resolve(String(reader.result).split(",")[1] || ""); };
      reader.onerror = function () { reject(new Error("无法读取 WAV 文件")); };
      reader.readAsDataURL(file);
    });
  }
  function captureRegionToText(owner, state, transport, prompt) {
    var options = {title: "区域 OCR", label: "输入屏幕像素区域 x,y,width,height（例如 100,200,600,80）", value: "0,0,800,200", confirmText: "开始识别"};
    return Promise.resolve().then(function () {
      return prompt ? prompt(options) : window.prompt(options.label, options.value);
    }).then(function (raw) {
      if (!raw || !current(owner, state)) return null;
      var parts = String(raw).split(/[，,\s]+/).filter(Boolean).map(Number);
      if (parts.length !== 4 || parts.some(function (n) { return !isFinite(n) || n < 0; }) || parts[2] <= 0 || parts[3] <= 0) throw new Error("请输入有效的 x,y,width,height；坐标不能为负，宽度和高度必须大于 0。");
      return transport("/perception/ocr/region", { x:parts[0], y:parts[1], width:parts[2], height:parts[3] }).then(function (data) {
        if (!data || typeof data !== "object") throw new Error("识别响应不完整");
        var text = data.text || (Array.isArray(data.lines) ? data.lines.join("\n") : "");
        if (!text) throw new Error("该区域未识别到文字");
        return text;
      });
    });
  }
  function runCommand() {
    if (!rootEl || !S || S.pending) return Promise.resolve();
    var owner = panelGeneration, state = S, transport = H.post, prompt = H.prompt, friendly = H.friendlyError;
    var modeEl = $("#owo-command-mode"), textEl = $("#owo-command-text");
    state.mode = modeEl.value || "text"; state.text = textEl.value;
    var mode = state.mode, token = {};
    state.pending = token; busyButton(); setStatus(mode === "region" ? "正在识别区域…" : "正在执行命令…");
    function execute(body) {
      if (!current(owner, state)) return Promise.resolve();
      return transport("/command/run", body).then(function (data) {
        if (!data || typeof data.results !== "object" || !data.results || Array.isArray(data.results)) throw new Error("命令响应不完整，无法确认执行结果");
        state.result = data;
        state.status = data.results.blocked ? "命令已被阻止：" + (data.results.reason || "请查看执行结果") : "命令已返回结果";
        state.error = !!data.results.blocked;
        if (S === state && rootEl) {
          renderCommand(data); setStatus(state.status, state.error);
          if (owner === panelGeneration) selectTab("results");
        }
        if (S === state && rootEl) refresh();
      });
    }
    function prepare() {
      if (mode === "region") return captureRegionToText(owner, state, transport, prompt).then(function (recognized) {
        if (!current(owner, state)) return;
        if (!recognized) { setStatus("已取消区域识别"); return; }
        state.mode = "text"; state.text = recognized;
        modeEl.value = "text"; textEl.value = recognized; updateMode();
        setStatus("识别文字已回填，请检查后再执行命令。");
      });
      if (mode === "voice") {
        var input = $("#owo-command-wav");
        if (!input || !input.files || !input.files.length) throw new Error("语音模式请先选择 WAV 文件。");
        return readWavBase64(input.files[0]).then(function (b64) { return execute({mode:"voice",wav_b64:b64}); });
      }
      if (!String(state.text || "").trim()) throw new Error("请先输入要执行的命令。");
      return execute({mode:"text",text:state.text});
    }
    var operation;
    try { operation = prepare(); } catch (error) { operation = Promise.reject(error); }
    return Promise.resolve(operation).catch(function (error) {
      state.status = (mode === "region" ? "区域识别失败：" : "命令失败：") + friendly(error); state.error = true;
      if (S === state && rootEl) setStatus(state.status, true);
    }).finally(function () {
      if (state.pending !== token) return;
      state.pending = null;
      if (S === state && rootEl) busyButton();
    });
  }
  function renderCommand(data) {
    var confidence = typeof data.confidence === "number" && isFinite(data.confidence) ? data.confidence.toFixed(2) : "—";
    var intent = $("#owo-command-intent");
    if (intent) intent.textContent = String(data.intent || "命令") + " · 置信度 " + confidence + (data.text ? " · " + data.text : "");
    var results = $("#owo-command-results"); if (results) results.innerHTML = renderResults(data.results);
  }
  function renderResults(results) {
    if (!results || typeof results !== "object") return '<div class="owo-insight-empty">服务未返回结果内容</div>';
    if (results.blocked) return '<div class="owo-command-result owo-insight-error"><h3>已拦截</h3><p>' + H.esc(results.reason || "服务未说明原因") + '</p></div>';
    var labels = {query:"查询",count:"匹配数量",goal_id:"目标 ID",objective:"目标内容",model:"模型",reply:"回复",run_id:"运行 ID",workflow:"流程",hits:"命中记录"};
    var lines = Object.keys(results).map(function (key) {
      var value = results[key], rendered;
      if (Array.isArray(value)) {
        rendered = value.length ? value.map(function (item) {
          if (item && typeof item === "object" && (item.title || item.summary)) {
            return '<div class="owo-command-hit">' + H.esc(item.title || item.summary) +
              '<span class="sub">' + H.esc([item.id, item.ts, item.app_id, typeof item.score === "number" ? "匹配分数 " + item.score.toFixed(2) : ""].filter(Boolean).join(" · ")) + '</span></div>';
          }
          var content = typeof item === "object" && item !== null ? JSON.stringify(item) : String(item);
          return '<div class="owo-command-hit">' + H.esc(content) + '</div>';
        }).join("") : '<span class="sub">没有匹配结果</span>';
      } else if (typeof value === "object" && value !== null) {
        rendered = '<pre>' + H.esc(JSON.stringify(value, null, 2)) + '</pre>';
      } else rendered = H.esc(String(value == null ? "—" : value));
      return '<div class="owo-command-result"><strong>' + H.esc(labels[key] || key) + '</strong><div>' + rendered + '</div></div>';
    });
    return lines.join("") || '<div class="owo-insight-empty">服务未返回结果内容</div>';
  }
  function dispose() {
    if (rootEl && S) { var input = $("#owo-command-text"); if (input) S.text = input.value; var mode = $("#owo-command-mode"); if (mode && mode.value) S.mode = mode.value; }
    panelGeneration += 1; auditGeneration += 1; rootEl = null;
  }
  return { id:id,title:"工具命令",nav:nav,mount:mount,refresh:refresh,dispose:dispose,_test:{renderResults:renderResults,runCommand:runCommand} };
})();
