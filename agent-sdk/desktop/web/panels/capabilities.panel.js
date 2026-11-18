// 我现在能做什么（§8.3 单一功能目录的 UI 面）：读取 GET /capabilities，
// 按「任务/能力」展示卡片（不按内部模块）；普通模式隐藏高级能力，
// 开发者模式（body.dev-mode）展开全部并显示技术入口 route。
// IIFE 注册 window.OwoPanels.capabilities；helpers 缺省时自建（fetch + esc）。
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels.capabilities = (function () {
    var ID = "capabilities";
    var H = {};
    var rootEl = null;
    var lifecycleGeneration = 0;
    var refreshGeneration = 0;
    var hasSnapshot = false;

    function defaultGet(path) {
      return window.OwoApi.get(path);
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

    function esc(s) {
      return H.esc ? H.esc(s) : defaultEsc(s);
    }

    /// 成熟度徽标（§8.3：只有四要素齐备才标「稳定」）。
    function maturityBadge(m) {
      var map = {
        stable: ["稳定", "owo-cap-stable"],
        beta: ["试用", "owo-cap-beta"],
        experimental: ["实验", "owo-cap-exp"],
      };
      var hit = map[String(m || "")] || [String(m || "—"), "owo-cap-exp"];
      return '<span class="owo-cap-badge ' + hit[1] + '">' + esc(hit[0]) + "</span>";
    }

    /// 影响范围文案映射（服务端 EffectClass 标签 → 用户语义）。
    function effectLabel(cls) {
      var map = {
        read: "读取文件",
        write: "写入文件",
        execute: "执行命令",
        network: "访问网络",
      };
      var key = String(cls || "").toLowerCase();
      return map[key] || String(cls || "");
    }

    /// 成熟度环形图（SVG，零依赖）：一眼看清稳定/试用/实验的分布。
    function maturityDonut(m, total) {
      var stable = m.stable || 0;
      var beta = m.beta || 0;
      var experimental = m.experimental || 0;
      if (!total) return "";
      var segments = [
        { count: stable, color: "var(--green)", label: "稳定" },
        { count: beta, color: "var(--accent)", label: "试用" },
        { count: experimental, color: "var(--yellow)", label: "实验" },
      ].filter(function (s) { return s.count > 0; });
      var radius = 15.9155; // 周长=100，方便百分比
      var circumference = 100;
      var offset = 25; // 从 12 点钟开始
      var arcs = segments
        .map(function (s) {
          var portion = (s.count / total) * circumference;
          var dash = portion + " " + (circumference - portion);
          var arc =
            '<circle r="' + radius + '" cx="21" cy="21" fill="none" stroke="' + s.color +
            '" stroke-width="5" stroke-dasharray="' + dash +
            '" stroke-dashoffset="' + offset + '"/>';
          offset -= portion; // 顺时针续接
          return arc;
        })
        .join("");
      var legend = segments
        .map(function (s) {
          return (
            '<span><i style="background:' + s.color + '"></i>' + s.label + " " + s.count + "</span>"
          );
        })
        .join("");
      return (
        '<div class="owo-cap-donut">' +
        '<svg viewBox="0 0 42 42" width="64" height="64" role="img" aria-label="成熟度分布">' +
        '<circle r="' + radius + '" cx="21" cy="21" fill="none" stroke="var(--surface-3)" stroke-width="5"/>' +
        arcs +
        '<text x="21" y="24" text-anchor="middle" font-size="11" font-weight="600" fill="var(--text)">' + total + "</text>" +
        "</svg>" +
        '<div class="owo-cap-donut-meta">' + legend + "</div></div>"
      );
    }

    /// 影响范围徽章（色点 + 文案，替代纯文本行）。
    function effectChip(cls) {
      var map = {
        read: ["读取文件", "owo-cap-eff-read"],
        write: ["写入文件", "owo-cap-eff-write"],
        execute: ["执行命令", "owo-cap-eff-exec"],
        network: ["访问网络", "owo-cap-eff-net"],
      };
      var key = String(cls || "").toLowerCase();
      var hit = map[key] || [String(cls || ""), ""];
      return '<span class="owo-cap-eff ' + hit[1] + '"><i aria-hidden="true"></i>' + esc(hit[0]) + "</span>";
    }

    /// 入口类型标记：界面/命令行/HTTP/服务 各有底色。
    function kindMark(kind) {
      var map = {
        ui: ["界面", "owo-cap-kind-ui"],
        cli: ["命令行", "owo-cap-kind-cli"],
        http: ["HTTP", "owo-cap-kind-http"],
        service: ["服务", "owo-cap-kind-service"],
      };
      var key = String(kind || "").toLowerCase();
      var hit = map[key] || [kind || "—", ""];
      return '<span class="owo-cap-kind ' + hit[1] + '">' + esc(hit[0]) + "</span>";
    }

    /// 能力卡片：名称 + 成熟度 + 说明 + 影响范围 + 用户入口。
    /// 技术入口 route 仅在开发者模式显示（§8.2：普通路由不泄漏 API path）。
    function card(c, dev) {
      var entrypoints = (c.entrypoints || []).map(function (e) {
        var tech = dev && e.target ? ' <code class="hint">' + esc(e.target) + "</code>" : "";
        return (
          "<li>" + kindMark(e.kind) + " " +
          esc(e.label || "") + '<span class="hint" title="技术入口：' + esc(e.target || "") + '">' +
          (dev ? "" : "（悬停查看技术入口）") + "</span>" + tech + "</li>"
        );
      }).join("");
      var effects = (c.required_effects || []).map(effectChip).join("");
      var deps = (c.dependencies || []).join("、");
      return (
        '<div class="owo-cap-card">' +
        '<div class="owo-cap-head"><b>' + esc(c.user_name || "") + "</b>" +
        maturityBadge(c.maturity) +
        (c.advanced ? '<span class="owo-cap-badge owo-cap-adv">高级</span>' : "") +
        "</div>" +
        '<div class="owo-cap-sum">' + esc(c.summary || "") + "</div>" +
        (effects ? '<div class="owo-cap-effects">' + effects + "</div>" : "") +
        (deps ? '<div class="hint">依赖能力：' + esc(deps) + "</div>" : "") +
        (entrypoints ? '<ul class="owo-cap-entries">' + entrypoints + "</ul>" : "") +
        "</div>"
      );
    }

    function nav() {
      return (
        '<section data-panel="capabilities" data-layout="custom" class="owo-cap-panel">' +
        '<div class="owo-cap-tools"><h3>我现在能做什么</h3>' +
        '<button type="button" class="owo-cap-refresh" aria-label="刷新能力目录">刷新</button></div>' +
        '<div class="hint">按任务展示产品能力与入口；标记「高级」的能力在开发者模式下显示。</div>' +
        '<div class="owo-cap-meta sub" role="status" aria-live="polite">加载中…</div>' +
        '<div class="owo-cap-summary" hidden></div>' +
        '<div class="owo-cap-list"></div>' +
        '<div class="owo-cap-error sub" role="alert" hidden></div>' +
        "</section>"
      );
    }

    function isCurrent(root, lifecycle, request) {
      return root === rootEl && lifecycle === lifecycleGeneration &&
        request === refreshGeneration &&
        (typeof root.isConnected !== "boolean" || root.isConnected);
    }

    function developerMode() {
      return typeof document !== "undefined" && document.body &&
        document.body.classList.contains("dev-mode");
    }

    /// 纯渲染（Node 可测）：dev=false 时过滤 advanced 能力并隐藏技术入口。
    function paint(data, root, dev) {
      if (!root) return { shown: 0, hidden: 0, total: 0 };
      var list = root.querySelector(".owo-cap-list");
      var meta = root.querySelector(".owo-cap-meta");
      var caps = (data && data.capabilities) || [];
      var shown = caps.filter(function (c) {
        return dev || !c.advanced;
      });
      var m = (data && data.maturity) || {};
      var hiddenCount = caps.length - shown.length;
      if (meta) {
        meta.innerHTML =
          esc("共 " + (data && data.count != null ? data.count : caps.length) + " 项能力") +
          (hiddenCount > 0 && !dev ? "（另有 " + hiddenCount + " 项高级能力在开发者模式下显示）" : "");
      }
      var summary = root.querySelector(".owo-cap-summary");
      if (summary) {
        var donut = maturityDonut(m, m.stable + m.beta + m.experimental);
        summary.innerHTML = donut || "";
        summary.hidden = !donut;
      }
      if (list) {
        list.innerHTML = shown.length
          ? shown.map(function (c) { return card(c, dev); }).join("")
          : '<div class="hint">暂无已登记能力</div>';
      }
      return { shown: shown.length, hidden: hiddenCount, total: caps.length };
    }

    function refresh(root) {
      root = root || rootEl;
      if (!root || (rootEl && root !== rootEl)) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      var request = ++refreshGeneration;
      var errBox = root.querySelector(".owo-cap-error");
      var meta = root.querySelector(".owo-cap-meta");
      if (errBox) {
        errBox.hidden = true;
        errBox.textContent = "";
      }
      if (meta) meta.textContent = hasSnapshot ? "正在更新能力目录…（保留当前结果）" : "加载中…";
      return H.get("/capabilities")
        .then(function (data) {
          if (!isCurrent(root, lifecycle, request)) return;
          hasSnapshot = true;
          paint(data, root, developerMode());
        })
        .catch(function (e) {
          if (!isCurrent(root, lifecycle, request)) return;
          if (meta) meta.textContent = hasSnapshot
            ? "更新失败，已保留上次读取结果"
            : "能力目录加载失败，可重试";
          if (errBox) {
            errBox.hidden = false;
            errBox.textContent = "功能目录加载失败：" + H.friendlyError(e);
          }
        });
    }

    function mount(root, helpers) {
      dispose();
      H = {
        get: (helpers && helpers.get) || defaultGet,
        friendlyError: (helpers && helpers.friendlyError) || defaultFriendlyError,
        esc: (helpers && helpers.esc) || defaultEsc,
      };
      if (!root) return;
      rootEl = root;
      hasSnapshot = false;
      root.innerHTML = nav();
      var btn = root.querySelector(".owo-cap-refresh");
      if (btn)
        btn.addEventListener("click", function () {
          refresh(root);
        });
      refresh(root);
    }

    function dispose() {
      lifecycleGeneration += 1;
      refreshGeneration += 1;
      rootEl = null;
      hasSnapshot = false;
    }

    var TEST_API = {
      maturityBadge: maturityBadge,
      effectLabel: effectLabel,
      effectChip: effectChip,
      kindMark: kindMark,
      maturityDonut: maturityDonut,
      card: card,
      paint: paint,
    };

    return {
      id: ID,
      title: "我现在能做什么",
      nav: nav,
      mount: mount,
      refresh: refresh,
      dispose: dispose,
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效、零运行时差异。
if (typeof module !== "undefined" && module.exports) {
  var __capWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __capWin.OwoPanels.capabilities;
}
