/* 帮助与关于面板：服务与版本 / 能力面速览 / 快捷键 / 斜杠命令 / 文档与契约 / 许可与致谢 / 诊断。
 * 纯脚本 IIFE，注册 window.OwoPanels.about；helpers 缺失时复用统一 ApiClient。
 * 版本号等运行时信息一律从 /health 与 /server/status 读取，不在前端硬编码。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.about = (function () {
  "use strict";

  var id = "about";
  var H = null;
  var rootEl = null;
  var refreshGeneration = 0;
  var mountGeneration = 0;
  var activeTab = "overview";
  var tabs = ["overview", "guide", "docs", "diagnostics"];

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
    return { baseUrl: baseUrl, get: get, esc: esc };
  }

  var SHORTCUTS = [
    ["Ctrl + N", "新对话"],
    ["Ctrl + K", "搜索与切换功能"],
    ["Ctrl + O", "打开文件夹（选择工作区）"],
    ["Ctrl + B", "显示 / 隐藏会话栏"],
    ["Ctrl + ,", "打开设置"],
    ["Ctrl + L", "聚焦输入框"],
    ["F11", "全屏 / 窗口"],
    ["Enter", "发送（Shift + Enter 换行）"],
    ["Esc", "关闭菜单 / 弹窗"],
  ];

  var SLASH = [
    ["/new", "新建会话，沿用当前模式"],
    ["/chat", "新建对话：可联网检索，不读写本地文件"],
    ["/agent", "新建 Work：可读写工作区、执行命令"],
    ["/undo", "回退最近一个回合"],
    ["/redo", "重做被回退的回合"],
    ["/export", "导出当前会话"],
    ["/stop", "中止正在跑的回合"],
    ["/settings", "打开设置页"],
    ["/help", "查看全部斜杠命令"],
  ];

  var DOCS = [
    ["OpenAPI 3.1 契约", "openapi.json", "全部 HTTP 端点与 SSE 事件定义"],
    ["隐私声明", "privacy.md", "本地优先、数据出境与感知分层的边界"],
    ["接口形态速查", "schemas", "按版本查询 wire 契约"],
  ];

  var THIRD_PARTY = [
    "axum（HTTP/SSE）",
    "tokio（异步运行时）",
    "serde / serde_json（序列化）",
    "reqwest（模型网关）",
    "clap（命令行）",
    "tracing / tracing-subscriber（日志）",
    "ed25519-dalek / sha2（插件签名与校验）",
    "chrono / uuid / regex / zip / base64",
  ];

  function nav() {
    function table(rows, headings) {
      return '<div class="owo-about-table-wrap"><table class="owo-about-table"><thead><tr>' + headings.map(function (v) { return '<th>' + v + '</th>'; }).join('') + '</tr></thead><tbody>' + rows.map(function (row) {
        return '<tr><td><code>' + escapeHtml(row[0]) + '</code></td><td>' + escapeHtml(row[1]) + '</td></tr>';
      }).join('') + '</tbody></table></div>';
    }
    var base = (H && H.baseUrl || '').replace(/\/$/, '');
    return '<section data-panel="about" data-layout="custom" class="owo-about-panel owo-insight">' +
      '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="帮助与关于">' + tabs.map(function (tab, i) {
        return '<button type="button" role="tab" id="about-tab-' + tab + '" data-about-tab="' + tab + '" aria-controls="about-pane-' + tab + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? '0' : '-1') + '">' + ['概览', '使用指南', '文档与许可', '诊断'][i] + '</button>';
      }).join('') + '</div><button type="button" id="owo-about-refresh"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1.5L20 9M4 15l2.3 3.5A7 7 0 0 0 17.9 17"/></svg>刷新状态</button></div>' +
      '<section role="tabpanel" id="about-pane-overview" aria-labelledby="about-tab-overview"><h3>服务与版本</h3>' +
      '<div id="owo-about-health" class="owo-about-health" role="status" aria-live="polite">加载中…</div>' +
      '<section class="owo-insight-section"><h3>能力速览</h3><div class="owo-about-cards" id="owo-about-caps" role="status" aria-live="polite">正在读取能力状态…</div>' +
      '<button type="button" id="owo-about-capabilities">查看能力与成熟度 →</button></section></section>' +
      '<section role="tabpanel" id="about-pane-guide" aria-labelledby="about-tab-guide" hidden><h3>快捷键</h3>' + table(SHORTCUTS, ['快捷键', '功能']) +
      '<section class="owo-insight-section"><h3>斜杠命令</h3>' + table(SLASH, ['命令', '作用']) + '</section></section>' +
      '<section role="tabpanel" id="about-pane-docs" aria-labelledby="about-tab-docs" hidden><h3>文档与契约</h3>' + DOCS.map(function (row) {
        return '<div class="owo-about-doc"><div><strong>' + escapeHtml(row[0]) + '</strong><p class="sub">' + escapeHtml(row[2]) + '</p></div>' +
          (row[1] === 'schemas' ? '<button type="button" id="owo-about-schemas">读取契约</button>' : '<a href="' + escapeHtml(base + '/' + row[1]) + '" target="_blank" rel="noopener">打开 ↗</a>') + '</div>';
      }).join('') + '<div id="owo-about-schema-result" role="status" aria-live="polite"></div>' +
      '<section class="owo-insight-section"><h3>许可与致谢</h3><p>GPL-3.0-only（Agent SDK workspace 的 Cargo.toml）</p><p class="sub">主要依赖；完整清单见 Cargo.lock。</p><ul class="owo-about-credits">' +
      THIRD_PARTY.map(function (v) { return '<li>' + escapeHtml(v) + '</li>'; }).join('') + '</ul></section></section>' +
      '<section role="tabpanel" id="about-pane-diagnostics" aria-labelledby="about-tab-diagnostics" hidden><h3>诊断</h3>' +
      '<p class="sub">配置和状态不代表已完成推理验证；请结合实际任务判断能力是否可用。</p>' +
      '<div id="owo-about-diag" class="owo-about-pre" role="status" aria-live="polite">正在读取诊断…</div>' +
      '<div class="owo-insight-actions"><button type="button" id="owo-about-copy">复制诊断信息</button></div>' +
      '<p id="owo-about-copy-status" class="sub" role="status" aria-live="polite"></p><textarea id="owo-about-copy-fallback" aria-label="待复制的诊断信息" readonly hidden></textarea></section></section>';
  }
  function selectTab(tab) {
    if (!rootEl || tabs.indexOf(tab) < 0) return;
    activeTab = tab;
    tabs.forEach(function (key) {
      var btn = rootEl.querySelector('[data-about-tab="' + key + '"]');
      if (btn && btn.setAttribute) { btn.setAttribute('aria-selected', String(tab === key)); btn.setAttribute('tabindex', tab === key ? '0' : '-1'); }
      var pane = rootEl.querySelector('#about-pane-' + key); if (pane) pane.hidden = tab !== key;
    });
  }
  function facts(rows) {
    return '<dl class="owo-about-facts">' + rows.map(function (row) { return '<dt>' + escapeHtml(row[0]) + '</dt><dd>' + escapeHtml(row[1]) + '</dd>'; }).join('') + '</dl>';
  }
  function text(id, value, targetRoot) {
    var el = targetRoot && targetRoot.querySelector("#" + id);
    if (el) el.textContent = value;
  }

  function isCurrentRefresh(generation, targetRoot) {
    return generation === refreshGeneration && rootEl === targetRoot;
  }

  function countOf(data, key) {
    if (data == null) return "—";
    if (Object.prototype.toString.call(data) === "[object Array]") return String(data.length);
    if (typeof data === "object") {
      if (Number.isInteger(data.count) && data.count >= 0) return String(data.count);
      if (data[key] && Object.prototype.toString.call(data[key]) === "[object Array]") return String(data[key].length);
    }
    return "—";
  }

  function escapeHtml(value) {
    if (H && typeof H.esc === "function") return H.esc(value);
    return defaultHelpers().esc(value);
  }

  function card(label, value, hint) {
    return (
      '<div class="owo-about-card"><span>' +
      escapeHtml(label) +
      "</span><b>" +
      escapeHtml(value) +
      "</b><span>" +
      escapeHtml(hint || "") +
      "</span></div>"
    );
  }

  function refresh() {
    var generation = ++refreshGeneration;
    var targetRoot = rootEl;
    var base = (H && H.baseUrl) || "";
    var current = function () { return isCurrentRefresh(generation, targetRoot); };
    // 1) 服务与版本；失败单独显示，避免把“未读取”伪装成服务的实际状态。
    var healthRequest = Promise.all([
      H.get("/health").catch(function (error) { return { error: String(error && error.message || error) }; }),
      H.get("/server/status").catch(function (error) { return { error: String(error && error.message || error) }; }),
    ]).then(function (result) {
      if (!current()) return;
      var health = result[0] || {};
      var status = result[1] || {};
      var readOnly = status.storage && status.storage.read_only;
      if (typeof readOnly !== 'boolean') readOnly = status.read_only;
      var rows = [
        ['版本', health.error ? '不可用（' + health.error + '）' : health.version || '未知'],
        ['API', health.error ? '未读取' : health.api_version || '未知'],
        ['健康状态', health.error ? '未读取' : health.healthy === true ? '正常' : health.healthy === false ? '异常' : '未知'],
        ['审批方式', health.error ? '未读取' : health.auto_approve === true ? '自动审批已开启' : health.auto_approve === false ? '逐次审批' : '未知'],
        ['存储模式', status.error ? '不可用（' + status.error + '）' : readOnly === true ? '只读' : readOnly === false ? '可写' : '未知'],
        ['核心阶段', health.error ? '未读取' : health.stage || '未知']
      ];
      if (status.workspace) rows.push(['工作区', status.workspace]);
      if (status.model) rows.push(['模型', status.model]);
      rows.push(['API Base', base || '同源']);
      var el = targetRoot && targetRoot.querySelector('#owo-about-health');
      if (el) el.innerHTML = facts(rows);
    });

    // 2) 能力速览。未读取与真实 0 个明确区分；移除未展示的用量请求。
    var capsRequest = Promise.all([
      H.get("/skills").catch(function () { return null; }),
      H.get("/plugins").catch(function () { return null; }),
      H.get("/mcp").catch(function () { return null; }),
      H.get("/automations").catch(function () { return null; }),
      H.get("/notes").catch(function () { return null; }),
      H.get("/sessions").catch(function () { return null; }),
    ]).then(function (r) {
      if (!current()) return;
      var skills = r[0];
      var plugins = r[1];
      var mcp = r[2];
      var connected = mcp && Array.isArray(mcp.connected) ? mcp.connected.filter(function (v) { return typeof v === "string"; }) : [];
      var mcpCount = mcp == null ? "不可用" : countOf(mcp, "servers");
      var hint = !mcp
        ? "无法读取服务器状态；点击刷新重试"
        : connected.length
          ? "已连接：" + connected.join("、")
          : mcpCount === "—"
            ? "服务未返回可识别的连接状态；点击刷新重试"
            : Number(mcpCount) > 0
              ? "仅配置、未连接"
              : "无（插件声明的服务器启动后在此显示）";
      // 大数字优先显示实际连接数；读取失败或响应形状未知都不误报为 0。
      var mcpValue = !mcp ? "不可用" : connected.length ? String(connected.length) : mcpCount;
      var capabilityCard = function (label, data, key, hintText) {
        if (data == null) return card(label, "不可用", "读取失败；点击上方刷新重试");
        var value = countOf(data, key);
        return value === "—"
          ? card(label, "未知", "服务未返回可识别的数量；点击上方刷新重试")
          : card(label, value, hintText);
      };
      var caps = []
        .concat([capabilityCard("技能", skills, "skills", "")])
        .concat([capabilityCard("插件", plugins, "plugins", "")])
        .concat([card("MCP 服务器", mcpValue === "—" ? "未知" : mcpValue, hint)])
        .concat([capabilityCard("定时任务", r[3], null, "")])
        .concat([capabilityCard("笔记", r[4], "notes", "")])
        .concat([capabilityCard("会话", r[5], "sessions", "")]);
      var el = targetRoot && targetRoot.querySelector("#owo-about-caps");
      if (el) el.innerHTML = caps.join("");
    });

    // 3) 诊断（感知 / 视觉 / 模型链路就绪度）
    var diagRequest = Promise.all([
      H.get("/perception/ocr/status").catch(function (e) { return { error: String(e && e.message) }; }),
      H.get("/vision/status").catch(function (e) { return { error: String(e && e.message) }; }),
      H.get("/plugins/market").catch(function (e) { return { error: String(e && e.message) }; }),
    ]).then(function (r) {
      if (!current()) return;
      var ocr = r[0] || {};
      var vision = r[1] || {};
      var market = r[2] || {};
      var lines = [];
      var ocrState = ocr.onnx_models_present === false ? '模型文件缺失' : ocr.ready === false ? '未就绪' : ocr.ready === true ? '服务报告就绪（未验证本次识别）' : '识别就绪度未确认';
      lines.push('OCR：' + (ocr.error ? '不可用（' + ocr.error + '）' : ocrState));
      if (!ocr.error && typeof ocr.engine_created === 'boolean') lines.push('OCR 引擎对象：' + (ocr.engine_created ? '已创建' : '未创建'));
      lines.push('视觉：' + (vision.error ? '不可用（' + vision.error + '）' : vision.ready === false ? '未就绪' : vision.ready === true ? '服务报告就绪（未验证本次推理）' : '推理就绪度未确认'));
      if (!vision.error) {
        if (vision.provider) lines.push('视觉提供方：' + vision.provider);
        if (vision.model) lines.push('视觉模型配置：' + vision.model);
        if (Array.isArray(vision.ollama_models)) lines.push('本地模型列表：' + (vision.ollama_models.length ? vision.ollama_models.map(function (v) { return typeof v === 'string' ? v : v && v.name || '未命名'; }).join('、') : '空'));
      }
      lines.push('插件市场：' + (market.error ? '不可用（' + market.error + '）' : '已读取目录，条目 ' + countOf(market, 'catalog')));
      lines.push("采样时间：" + new Date().toLocaleString());
      text("owo-about-diag", lines.join("\n"), targetRoot);
    });
    return Promise.allSettled([healthRequest, capsRequest, diagRequest]);
  }

  function copyDiagnostics() {
    var target = rootEl, generation = mountGeneration;
    var health = target && target.querySelector('#owo-about-health'), diag = target && target.querySelector('#owo-about-diag');
    var payload = ['# OwO Agent 诊断信息', '', health ? health.textContent : '', '', diag ? diag.textContent : '', 'userAgent: ' + navigator.userAgent].join('\n');
    function current() { return rootEl === target && generation === mountGeneration; }
    function fallback() {
      if (!current()) return;
      text('owo-about-copy-status', '无法访问剪贴板，请在下方选择文本后手动复制。', target);
      var area = target.querySelector('#owo-about-copy-fallback');
      if (area) { area.hidden = false; area.value = payload; if (area.focus) area.focus(); if (area.select) area.select(); }
    }
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return Promise.resolve().then(function () { return navigator.clipboard.writeText(payload); }).then(function () {
        if (current()) text('owo-about-copy-status', '诊断信息已复制。', target);
      }).catch(fallback);
    }
    fallback();
  }
  function mount(root, helpers) {
    dispose(); H = Object.assign(defaultHelpers(), helpers || {}); rootEl = root;
    var generation = mountGeneration;
    root.innerHTML = nav();
    function bind(selector, action, event) {
      var el = root.querySelector(selector);
      if (el) el.addEventListener(event || 'click', function (e) { if (rootEl === root && generation === mountGeneration) return action(e); });
    }
    bind('#owo-about-refresh', refresh);
    bind('#owo-about-copy', copyDiagnostics);
    var cap = root.querySelector('#owo-about-capabilities');
    if (cap) cap.hidden = typeof H.openPanel !== 'function';
    bind('#owo-about-capabilities', function () { if (H.openPanel) H.openPanel('capabilities'); });
    var schemasPending = false;
    bind('#owo-about-schemas', function () {
      if (schemasPending) return;
      schemasPending = true; var button = root.querySelector('#owo-about-schemas'); button.disabled = true;
      text('owo-about-schema-result', '正在读取契约…', root);
      return Promise.resolve().then(function () { return H.get('/schemas'); }).then(function (value) {
        if (rootEl !== root || generation !== mountGeneration) return;
        var result = root.querySelector('#owo-about-schema-result');
        if (result) result.innerHTML = '<details open><summary>接口形态</summary><pre>' + escapeHtml(JSON.stringify(value, null, 2)) + '</pre></details>';
      }).catch(function (error) {
        if (rootEl === root && generation === mountGeneration) text('owo-about-schema-result', '读取失败：' + (error.message || error), root);
      }).finally(function () { schemasPending = false; if (rootEl === root && generation === mountGeneration) button.disabled = false; });
    });
    tabs.forEach(function (tab, i) {
      bind('[data-about-tab="' + tab + '"]', function () { selectTab(tab); });
      bind('[data-about-tab="' + tab + '"]', function (e) {
        if (e.ctrlKey || e.metaKey || e.altKey) return;
        var next = e.key === 'ArrowRight' ? (i + 1) % 4 : e.key === 'ArrowLeft' ? (i + 3) % 4 : e.key === 'Home' ? 0 : e.key === 'End' ? 3 : -1;
        if (next < 0) return; e.preventDefault(); selectTab(tabs[next]); root.querySelector('[data-about-tab="' + tabs[next] + '"]').focus();
      }, 'keydown');
    });
    selectTab(activeTab); return refresh();
  }
  function dispose() { refreshGeneration += 1; mountGeneration += 1; rootEl = null; }

  return { id: id, title: "帮助与关于", mount: mount, dispose: dispose, refresh: refresh };
})();
