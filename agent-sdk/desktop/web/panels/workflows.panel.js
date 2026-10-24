// 用户自定义多 Agent 工作流面板（团队模板的用户/社区通道 UI）：
// - 我的工作流：GET /workflows/user（导出/删除）
// - 新建/编辑：JSON 编辑器 + 本地轻量检查 + 服务端校验 + 保存
// - AI 生成：自然语言 → POST /workflows/draft（预览草稿，采纳/丢弃）
// - 社区包：粘贴 base64 → 评审（不落盘）/导入
// IIFE 注册 window.OwoPanels.workflows；helpers 缺省时自建（OwoApi + esc）。
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels["custom-workflows"] = (function () {
    var ID = "custom-workflows";
    var H = {};
    var rootEl = null;
    var lifecycleGeneration = 0;
    var editor = function () {
      return win.OwoWorkflowEditor || null;
    };

    function defaultEsc(value) {
      return String(value == null ? "" : value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function defaultFriendlyError(error) {
      return String((error && error.message) || error || "未知错误");
    }
    function defaultGet(path) {
      return win.OwoApi.get(path);
    }
    function defaultPost(path, body) {
      return win.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(readResponse);
    }
    function defaultDelete(path) {
      return win.OwoApi.stream(path, { method: "DELETE" }).then(readResponse);
    }
    function readResponse(response) {
      if (!response.ok) {
        return response
          .json()
          .catch(function () {
            return {};
          })
          .then(function (body) {
            var message = (body && (body.message || body.error)) || "HTTP " + response.status;
            if (body && Array.isArray(body.errors) && body.errors.length) {
              message += "：" + body.errors.join("；");
            }
            throw new Error(message);
          });
      }
      return response.status === 204 ? null : response.json();
    }
    function esc(value) {
      return H.esc ? H.esc(value) : defaultEsc(value);
    }
        function nav() {
      return [
        '<div class="owo-wf">',
        '  <section class="owo-wf-brief" aria-labelledby="owoWfBriefTitle">',
        '    <div class="owo-wf-brief-head">',
        '      <div><p class="owo-wf-eyebrow">团队协作 · 一句话编排</p><h3 id="owoWfBriefTitle">说一句，生成一张可拖拽的团队画布</h3>',
        '      <p class="hint">模型会把目标拆成角色、依赖、工具与交付契约；生成后直接拖动节点调整布局。采纳只保存配置，不会自动执行。</p></div>',
        '      <span class="owo-wf-flow-badge">对话 → 画布 → 团队</span>',
        '    </div>',
        '    <div class="owo-wf-brief-input">',
        '      <textarea class="owo-wf-desc" rows="2" placeholder="例如：把一段素材做成可发布的竖屏短片，最后要人工验收"></textarea>',
        '      <button type="button" class="owo-wf-draft primary">生成工作流</button>',
        '    </div>',
        '    <div class="owo-wf-brief-extra">',
        '      <input class="owo-wf-constraints" type="text" placeholder="附加约束（可选）：必须有人工节点 / 只用本地工具 / 必须用 MCP 渲染工具">',
        '      <label class="owo-wf-rounds-wrap hint">收敛轮次 <input class="owo-wf-rounds" type="number" min="1" max="5" value="1" title=">1 走多轮收敛（按校验错误迭代，最多 5 轮）"></label>',
        '    </div>',
        '    <details class="owo-wf-catalog">',
        '      <summary class="owo-wf-catalog-head"><span>本次可用工具与 MCP</span><svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="m6 8 4 4 4-4"/></svg></summary>',
        '      <div class="owo-wf-catalog-actions"><button type="button" class="owo-wf-catalog-refresh">刷新工具列表</button></div>',
        '      <div class="owo-wf-catalog-list hint">正在读取当前工具面…</div>',
        '    </details>',
        '    <div class="owo-wf-graph-box owo-wf-graph-box-hero"></div>',
        '    <div class="owo-wf-graph-info"></div>',
        '    <div class="owo-wf-draft-actions">',
        '      <button type="button" class="owo-wf-adopt" disabled>采纳草稿</button>',
        '      <button type="button" class="owo-wf-discard" disabled>丢弃草稿</button>',
        '      <span class="hint">蓝色节点可拖动；双击「重排」回到自动分层布局。</span>',
        '    </div>',
        '    <pre class="owo-wf-draft-out hint"></pre>',
        '  </section>',
        '  <div class="owo-wf-toolbar">',
        '    <button type="button" class="owo-wf-refresh">刷新我的工作流</button>',
        '    <input class="owo-wf-objective" type="text" placeholder="本次目标（用下面任一条工作流建队时必填）">',
        '    <label title="团队运行没有交互式审批通道：不勾选时，写节点等待审批直到超时被拒，工作流必然失败"><input class="owo-wf-allow-writes" type="checkbox" checked>允许节点按各自声明的 write_paths 写入当前工作区<span class="hint">（不勾选 = 写节点会被拒绝）</span></label>',
        '    <span class="owo-wf-meta hint"></span>',
        "  </div>",
        '  <ul class="owo-wf-list"></ul>',
        '  <div class="owo-wf-error hint" hidden></div>',
        '  <details class="owo-wf-section">',
        "    <summary>新建 / 编辑（JSON）</summary>",
        '    <textarea class="owo-wf-editor" rows="10" spellcheck="false" placeholder=\'{"template_id":"my-flow","name":"我的工作流","mode":"team","applicability":"关键词","roles":[…]}\'></textarea>',
        '    <div class="owo-wf-actions">',
        '      <button type="button" class="owo-wf-check">本地检查</button>',
        '      <button type="button" class="owo-wf-validate">服务端校验</button>',
        '      <button type="button" class="owo-wf-save">保存为用户工作流</button>',
        "    </div>",
        '    <pre class="owo-wf-preview hint"></pre>',
        "  </details>",
        '  <details class="owo-wf-section">',
        "    <summary>使用统计（跑过几次 / 成败 / 最近失败）</summary>",
        '    <div class="owo-wf-actions">',
        '      <button type="button" class="owo-wf-stats">刷新统计</button>',
        '      <select class="owo-wf-stats-sort" title="排序方式">',
        '        <option value="runs">按运行次数</option>',
        '        <option value="failed">按失败次数</option>',
        '        <option value="success_rate">按成功率</option>',
        '        <option value="last">按最近运行</option>',
        "      </select>",
        "    </div>",
        '    <ul class="owo-wf-stats-list"></ul>',
        "  </details>",
        '  <details class="owo-wf-section">',
        "    <summary>社区包（导入 / 评审）</summary>",
        '    <textarea class="owo-wf-package" rows="4" spellcheck="false" placeholder="粘贴社区包内容（base64，来自 export）"></textarea>',
        '    <div class="owo-wf-actions">',
        '      <button type="button" class="owo-wf-review">只评审</button>',
        '      <button type="button" class="owo-wf-import">导入</button>',
        "    </div>",
        '    <pre class="owo-wf-review-out hint"></pre>',
        "  </details>",
        "</div>",
      ].join("\n");
    }

    /// 列表项渲染（纯函数便于测试）。
    function renderWorkflowRows(listed) {
      var helper = editor();
      var items = (listed && Array.isArray(listed.templates) ? listed.templates : []).slice();
      if (!items.length) return '<li class="sub">还没有自定义工作流：可以粘贴 JSON 保存，或让 AI 生成候选。</li>';
      return items
        .map(function (item) {
          var row = helper ? helper.describeWorkflow(item) : { id: "?", name: "", origin: "?", roles: 0, tag: "" };
          return (
            '<li class="owo-wf-item" data-id="' +
            esc(row.id) +
            '" data-findings="' +
            esc(JSON.stringify(row.findings || [])) +
            '"><strong>' +
            esc(row.name || row.id) +
            "</strong> <code>" +
            esc(row.id) +
            "</code> <span class=\"hint\">" +
            esc(row.origin) +
            " · " +
            row.roles +
            " 个角色 · " +
            esc(row.tag) +
            (row.warnings ? " · " + row.warnings + " 条提示" : "") +
            (row.riskLabel ? " · 风险 " + esc(row.riskLabel) : "") +
            (row.provenanceLabel ? " · " + esc(row.provenanceLabel) : "") +
            (row.runs ? " · 运行 " + row.runs + " 次" : "") +
            (row.checklistFixable
              ? ' · <span class="owo-wf-risk" title="' +
                esc(
                  row.checklist
                    .filter(function (entry) {
                      return entry.level !== "info";
                    })
                    .map(function (entry) {
                      return entry.message;
                    })
                    .join("；")
                ) +
                '">待修 ' +
                row.checklistFixable +
                " 项</span>"
              : "") +
            '</span> <button type="button" class="owo-wf-edit">编辑</button> <button type="button" class="owo-wf-export">导出</button> <button type="button" class="owo-wf-delete">删除</button> <button type="button" class="owo-wf-improve" title="把这条件工作流的真实运行证据交给 AI，生成改进候选">按证据改进</button> <button type="button" class="owo-wf-rerun">再跑</button> <button type="button" class="owo-wf-run">建队</button>' +
            (row.attention.length
              ? '<div class="hint owo-wf-risk">⚠ 需要修复：' + esc(row.attention.join("；")) + "</div>"
              : "") +
            "</li>"
          );
        })
        .join("");
    }

    /// 读取/持久化单条工作流的拖拽布局（坏 JSON 静默回落默认布局）。
    function layoutStorageKey(template) {
      var id = template && template.template_id ? String(template.template_id) : "draft";
      return "owo.wf.canvas.v1:" + id;
    }
    function loadSavedLayout(template) {
      try {
        var raw = win.localStorage && win.localStorage.getItem(layoutStorageKey(template));
        var parsed = raw ? JSON.parse(raw) : null;
        return parsed && typeof parsed === "object" ? parsed : {};
      } catch (error) {
        return {};
      }
    }
    function saveSavedLayout(template, layout) {
      try {
        if (win.localStorage) win.localStorage.setItem(layoutStorageKey(template), JSON.stringify(layout));
      } catch (error) {
        // 隐私模式/配额满：拖拽仍生效，仅本次会话不持久化。
      }
    }

    /// DAG 画布（SVG 字符串）：默认分层布局；`saved` 覆盖节点位置，节点可拖拽。
    function renderGraphSvg(template, saved) {
      var helper = editor();
      if (!helper || !helper.graphPositions) return "";
      var layout = helper.graphPositions(template, saved || {});
      if (!layout.ok) return '<p class="hint">' + esc(layout.error) + "</p>";
      if (!layout.nodes.length) return "";
      var size = helper.graphNodeSize || { width: 124, height: 48 };
      var nodeW = size.width, nodeH = size.height, pad = 18;
      var svg = [
        '<div class="owo-wf-canvas-toolbar"><span>拖动节点调整布局</span><button type="button" class="owo-wf-graph-reset">重排</button></div>',
        '<svg class="owo-wf-graph" viewBox="0 0 ' + layout.width + " " + layout.height + '" width="100%" height="' + layout.height + '" role="img" aria-label="工作流依赖图" style="touch-action:none">',
      ];
      var positions = {};
      layout.nodes.forEach(function (node) {
        positions[node.id] = { x: node.x, y: node.y };
      });
      layout.edges.forEach(function (edge) {
        var from = positions[edge.from];
        var to = positions[edge.to];
        if (!from || !to) return;
        svg.push(
          '<path class="owo-wf-edge" data-edge-from="' + esc(edge.from) + '" data-edge-to="' + esc(edge.to) +
            '" d="' + esc(helper.edgePath(from, to)) + '" fill="none" stroke="currentColor" stroke-width="1.4" opacity="0.55"/>'
        );
      });
      layout.nodes.forEach(function (node) {
        var position = positions[node.id];
        svg.push(
          '<g class="owo-wf-node" data-node-id="' + esc(node.id) + '" transform="translate(' + position.x + " " + position.y +
            ')" tabindex="0" role="button" aria-label="节点 ' + esc(node.id) + '，可拖动">'
        );
        svg.push('<rect width="' + nodeW + '" height="' + nodeH + '" rx="10" class="owo-wf-node-box"/>');
        svg.push('<circle cx="14" cy="16" r="3.5" class="owo-wf-node-dot"/>');
        svg.push('<text x="24" y="20" font-size="11" class="owo-wf-node-title">' + esc(node.id) + "</text>");
        svg.push('<text x="24" y="35" font-size="9" opacity="0.68" class="owo-wf-node-meta">' + esc(node.assignee) + "</text>");
        svg.push("</g>");
      });
      svg.push("</svg>");
      return svg.join("");
    }

    /// 工具/MCP 清单（纯函数便于测试）：mcp__ 单独成组，明确“来自 MCP”。
    function renderToolCatalog(names) {
      var list = Array.isArray(names) ? names.map(function (name) { return String(name || "").trim(); }).filter(Boolean) : [];
      var mcp = list.filter(function (name) { return name.indexOf("mcp__") === 0; });
      var builtin = list.filter(function (name) { return name.indexOf("mcp__") !== 0; });
      function chips(values) {
        return values.map(function (name) { return '<code class="owo-wf-tool">' + esc(name) + "</code>"; }).join("");
      }
      return (
        '<div class="owo-wf-tool-groups">' +
        '<div><span class="owo-wf-tool-label">内置工具 ' + builtin.length + '</span><div class="owo-wf-tool-chips">' +
        (builtin.length ? chips(builtin) : '<span class="hint">当前没有内置工具</span>') +
        "</div></div>" +
        '<div><span class="owo-wf-tool-label">MCP 工具 ' + mcp.length + '</span><div class="owo-wf-tool-chips">' +
        (mcp.length ? chips(mcp) : '<span class="hint">未连接 MCP；接入后可直接写进角色 tools</span>') +
        "</div></div></div>"
      );
    }

    /// 读取当前 Agent 真实工具面，避免把“未连接”的 MCP 画进画布。
    function refreshCatalog(root) {
      root = root || rootEl;
      if (!root) return Promise.resolve();
      var box = root.querySelector(".owo-wf-catalog-list");
      if (box) box.innerHTML = '<span class="hint">正在读取当前工具面…</span>';
      return H.get("/settings")
        .then(function (settings) {
          var names = settings && settings.runtime && Array.isArray(settings.runtime.active_tool_names)
            ? settings.runtime.active_tool_names
            : [];
          if (box) box.innerHTML = renderToolCatalog(names);
        })
        .catch(function () {
          if (box) {
            box.innerHTML = '<span class="hint">工具面读取失败；确认本地核心已连接后点「刷新」。</span>';
          }
        });
    }

    /// 把布局状态挂到画布容器：一次绑定事件，innerHTML 重绘后仍生效。
    function paintGraph(root, template) {
      var box = root.querySelector(".owo-wf-graph-box");
      if (!box || !template) return;
      var helper = editor();
      if (!helper || !helper.graphPositions) return;
      var saved = loadSavedLayout(template);
      var layout = helper.graphPositions(template, saved);
      if (!layout.ok) {
        box.innerHTML = '<p class="hint">' + esc(layout.error) + "</p>";
        return;
      }
      box.setAttribute("data-template-id", String(template.template_id || "draft"));
      box.innerHTML = renderGraphSvg(template, saved);
      var svg = box.querySelector("svg");
      if (!svg) return;
      var current = {};
      layout.nodes.forEach(function (node) {
        current[node.id] = { x: node.x, y: node.y };
      });
      var drag = null;
      function point(event) {
        var matrix = svg.getScreenCTM && svg.getScreenCTM();
        if (matrix) {
          var pt = svg.createSVGPoint();
          pt.x = event.clientX;
          pt.y = event.clientY;
          var local = pt.matrixTransform(matrix.inverse());
          return { x: local.x, y: local.y };
        }
        var rect = svg.getBoundingClientRect();
        return {
          x: ((event.clientX - rect.left) / Math.max(1, rect.width)) * layout.width,
          y: ((event.clientY - rect.top) / Math.max(1, rect.height)) * layout.height,
        };
      }
      function updateEdges() {
        Array.prototype.slice.call(svg.querySelectorAll("[data-edge-from]")).forEach(function (path) {
          var from = current[path.getAttribute("data-edge-from")];
          var to = current[path.getAttribute("data-edge-to")];
          if (from && to) path.setAttribute("d", helper.edgePath(from, to));
        });
      }
      box.onpointerdown = function (event) {
        var node = event.target && event.target.closest ? event.target.closest(".owo-wf-node") : null;
        if (!node || event.button !== 0) return;
        var id = node.getAttribute("data-node-id");
        if (!id || !current[id]) return;
        event.preventDefault();
        var start = point(event);
        drag = { id: id, offsetX: start.x - current[id].x, offsetY: start.y - current[id].y, node: node, moved: false };
        if (node.setPointerCapture) {
          try { node.setPointerCapture(event.pointerId); } catch (error) { /* 忽略合成事件 */ }
        }
        node.classList.add("is-dragging");
      };
      box.onpointermove = function (event) {
        if (!drag) return;
        event.preventDefault();
        var at = point(event);
        var next = helper.clampNodePosition(at.x - drag.offsetX, at.y - drag.offsetY, layout.width, layout.height);
        current[drag.id] = next;
        drag.node.setAttribute("transform", "translate(" + next.x + " " + next.y + ")");
        drag.moved = true;
        updateEdges();
      };
      function finishDrag() {
        if (!drag) return;
        var node = drag.node;
        var id = drag.id;
        var moved = drag.moved;
        node.classList.remove("is-dragging");
        drag = null;
        if (moved) {
          var merged = {};
          Object.keys(current).forEach(function (key) { merged[key] = current[key]; });
          saved[id] = merged[id];
          saveSavedLayout(template, saved);
          box.setAttribute("data-dragged", "1");
          setMeta(root, "已保存「" + (template.name || template.template_id || "工作流") + "」的节点布局（仅界面，不改配置）");
        }
      }
      box.onpointerup = finishDrag;
      box.onpointercancel = finishDrag;
      box.onclick = function (event) {
        var reset = event.target && event.target.closest ? event.target.closest(".owo-wf-graph-reset") : null;
        if (!reset) return;
        event.preventDefault();
        saveSavedLayout(template, {});
        paintGraph(root, template);
        setMeta(root, "已恢复自动分层布局");
      };
    }

    /// 统计列表 HTML（纯函数便于测试）。
    function renderStatsRows(stats) {
      var helper = editor();
      var items = stats && Array.isArray(stats.templates) ? stats.templates : [];
      if (!items.length) return '<li class="sub">还没有用这些工作流创建过团队。</li>';
      return items
        .map(function (item) {
          var row = helper ? helper.describeStatsRow(item) : { id: "?", runs: 0 };
          var line =
            "<strong>" +
            esc(row.name || row.id) +
            "</strong> <code>" +
            esc(row.id) +
            "</code> <span class=\"hint\">" +
            esc(row.origin) +
            (row.riskLabel ? " · 风险 " + esc(row.riskLabel) : "") +
            " · 运行 " +
            row.runs +
            " · 成功 " +
            row.succeeded +
            " · 失败 " +
            row.failed +
            " · 成功率 " +
            esc(row.successRate) +
            (row.lastRunAt ? " · 最后 " + esc(row.lastRunAt) : "") +
            "</span>";
          if (row.attention.length) {
            line += '<div class="hint owo-wf-risk">⚠ 需要修复：' + esc(row.attention.join("；")) + "</div>";
          }
          if (row.lastFailure) {
            line +=
              '<div class="hint owo-wf-risk">最近失败：' +
              esc(row.lastFailure.event) +
              " · " +
              esc(row.lastFailure.detail) +
              "</div>";
          }
          if (row.running || row.awaitingHuman) {
            line +=
              '<div class="hint">进行中 ' +
              row.running +
              " · 等待人工 " +
              row.awaitingHuman +
              "</div>";
          }
          return (
            '<li data-id="' +
            esc(row.id) +
            '">' +
            line +
            ' <button type="button" class="owo-wf-rerun" data-id="' +
            esc(row.id) +
            '">再跑</button></li>'
          );
        })
        .join("");
    }

    function refreshStats(root) {
      root = root || rootEl;
      if (!root) return Promise.resolve();
      var list = root.querySelector(".owo-wf-stats-list");
      if (list) {
        list.innerHTML = '<li class="sub">加载中…</li>';
        // 事件委托：统计行里的「再跑」按钮复用列表行的同一段逻辑（沿用上次目标）。
        if (!list.__owoRerunBound) {
          list.__owoRerunBound = true;
          list.addEventListener("click", function (event) {
            var button = event.target && event.target.closest
              ? event.target.closest(".owo-wf-rerun")
              : null;
            if (!button) return;
            var id = button.getAttribute("data-id") || "";
            if (id) rerunFromLast(root, id);
          });
        }
      }
      var sortSelect = root.querySelector(".owo-wf-stats-sort");
      var helper = editor();
      var uri =
        helper && helper.buildStatsUri
          ? helper.buildStatsUri(sortSelect ? sortSelect.value : "")
          : "/workflows/stats";
      return H.get(uri)
        .then(function (stats) {
          var target = root.querySelector(".owo-wf-stats-list");
          if (target) target.innerHTML = renderStatsRows(stats);
        })
        .catch(function (error) {
          var target = root.querySelector(".owo-wf-stats-list");
          if (target) target.innerHTML = '<li class="sub">统计加载失败：' + esc(H.friendlyError(error)) + "</li>";
        });
    }

    /// 校验/评审输出文本（纯函数便于测试）。
    function renderReviewText(result) {
      var helper = editor();
      if (!helper) return "";
      return helper.describeReview(result).join("\n");
    }

    /// AI 草稿预览文本（纯函数便于测试）。
    function renderDraftText(draft) {
      var helper = editor();
      if (!helper) return "";
      var lines = [];
      var outcome = draft && (draft.verify_outcome || (draft.draft && draft.draft.verify_outcome));
      if (outcome && typeof outcome === "object") {
        lines.push(
          "上次验证：" +
            String(outcome.status || "?") +
            (outcome.compared_status ? "（基线 " + String(outcome.compared_status) + "）" : "") +
            (outcome.team_id ? " · team " + String(outcome.team_id) : "")
        );
      }
      if (draft && typeof draft.run_evidence === "string" && draft.run_evidence) {
        var facts = draft.run_evidence
          .split("\n")
          .filter(function (line) {
            return line.indexOf("- ") === 0;
          })
          .join(" | ");
        lines.push("已注入真实运行证据：" + facts);
      }
      var template = (draft && draft.template) || {};
      lines.push(
        (draft && draft.ok ? "校验通过" : "校验未通过") +
          "：" +
          String(template.name || template.template_id || "") +
          (draft && draft.repaired ? "（已按校验错误自动修复 " + String(draft.attempts || 2) + " 轮内）" : "")
      );
      if (template.description) lines.push(String(template.description));
      if (helper.previewLines) lines = lines.concat(helper.previewLines(template));
      if (helper.describeTrajectory && draft && draft.trajectory) {
        var trajectory = helper.describeTrajectory(draft.trajectory);
        if (trajectory.length) lines = lines.concat(["收敛轨迹："].concat(trajectory));
      }
      if (helper.describeDraftHealth) {
        var healthLines = helper.describeDraftHealth(draft);
        if (healthLines.length) lines.push("健康：" + healthLines.join(" · "));
      }
      (draft && Array.isArray(draft.errors) ? draft.errors : []).forEach(function (error) {
        lines.push("[错误] " + String(error));
      });
      return lines.join("\n");
    }

    function setError(root, message) {
      var box = root.querySelector(".owo-wf-error");
      if (!box) return;
      box.hidden = !message;
      box.textContent = message || "";
    }
    function setMeta(root, message) {
      var meta = root.querySelector(".owo-wf-meta");
      if (meta) meta.textContent = message || "";
    }
    /// 建队成功后把"看待办/看对话"变成一次点击（而不是让用户自己找面板）。
    function setTeamJump(root, teamId) {
      var meta = root.querySelector(".owo-wf-meta");
      if (!meta || !teamId) return;
      var existing = meta.querySelector(".owo-wf-open-team");
      if (existing) existing.remove();
      var button = document.createElement("button");
      button.type = "button";
      button.className = "owo-wf-open-team";
      button.textContent = "查看团队对话 →";
      button.title = "打开团队协作面板，展开成员对话与工具调用";
      button.addEventListener("click", function () {
        var helper = H;
        if (helper && typeof helper.openTeam === "function") {
          helper.openTeam(teamId);
        } else if (helper && typeof helper.openPanel === "function") {
          helper.openPanel("workswarm");
        }
      });
      meta.appendChild(document.createTextNode(" "));
      meta.appendChild(button);
    }
    function editorTemplate(root) {
      var helper = editor();
      if (!helper) throw new Error("工作流编辑器模块未加载");
      var parsed = helper.parseTemplate(root.querySelector(".owo-wf-editor").value);
      if (!parsed.ok) throw new Error(parsed.error);
      return parsed.template;
    }
    /// 逐角色健康行（HTML；风险用 title 展开，避免表格过宽）。
    function renderRoleRows(template) {
      var helper = editor();
      if (!helper || !helper.roleHealthRows) return "";
      var rows = helper.roleHealthRows(template);
      if (!rows.length) return "";
      return (
        '<div class="owo-wf-roles"><div class="hint">角色健康（工具/写范围/风险）</div><ul>' +
        rows
          .map(function (row) {
            return (
              "<li><strong>" +
              esc(row.role) +
              "</strong> <code>" +
              esc(row.assignee) +
              "</code> 工具 " +
              row.tools.length +
              (row.writePaths.length ? " · 写 " + esc(row.writePaths.join(", ")) : "") +
              (row.risks.length
                ? ' · <span class="owo-wf-risk" title="' + esc(row.risks.join("；")) + '">' + esc(row.risks.join("；")) + "</span>"
                : "") +
              "</li>"
            );
          })
          .join("") +
        "</ul></div>"
      );
    }

    /// 保存前检查清单（前端提示，不替代服务端校验）：缺 verify / 缺写范围 / 范围重叠 /
    /// 高风险工具 / 没有角色 / 没有写角色。warn 项给一键修复入口。
    function renderChecklist(template) {
      var helper = editor();
      var items = helper && helper.preSaveChecklist ? helper.preSaveChecklist(template) : [];
      if (!items.length) return "";
      var labels = { block: "必须修", warn: "建议修", info: "要知道" };
      var body = items
        .map(function (item) {
          var lvl = labels[item.level] || "提示";
          var fix = "";
          if (item.fix === "verify") {
            fix = ' <button type="button" class="owo-wf-fix-verify">补 non_empty 验证</button>';
          } else if (item.fix === "write_scope") {
            fix = ' <button type="button" class="owo-wf-fix-scope">补建议写目录</button>';
          }
          return (
            '<li class="owo-wf-check owo-wf-check-' +
            esc(item.level) +
            '"><b>' +
            esc(lvl) +
            "</b> " +
            esc(item.message) +
            fix +
            "</li>"
          );
        })
        .join("");
      return '<ul class="owo-wf-checklist">' + body + "</ul>";
    }

    function showPreview(root, text, template) {
      var preview = root.querySelector(".owo-wf-preview");
      if (preview) preview.textContent = text || "";
      if (template) {
        paintGraph(root, template);
        var info = root.querySelector(".owo-wf-graph-info");
        if (info) info.innerHTML = renderChecklist(template) + renderRoleRows(template);
      }
    }

    function refresh(root) {
      root = root || rootEl;
      if (!root) return Promise.resolve();
      var generation = lifecycleGeneration;
      setMeta(root, "加载中…");
      return H.get("/workflows/user")
        .then(function (listed) {
          if (generation !== lifecycleGeneration) return;
          var list = root.querySelector(".owo-wf-list");
          if (list) list.innerHTML = renderWorkflowRows(listed);
          setMeta(root, "共 " + String((listed && listed.count) || 0) + " 条（校验未通过 " + String((listed && listed.invalid) || 0) + " 条）");
          bindRowActions(root, listed);
        })
        .catch(function (error) {
          if (generation !== lifecycleGeneration) return;
          setMeta(root, "加载失败");
          setError(root, "读取工作流失败：" + H.friendlyError(error));
        });
    }

    function bindRowActions(root, listed) {
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-edit")).forEach(function (button) {
        button.addEventListener("click", function () {
          var id = button.closest(".owo-wf-item").getAttribute("data-id");
          var item = ((listed && listed.templates) || []).find(function (entry) {
            return entry.template && entry.template.template_id === id;
          });
          if (!item) return setError(root, "此工作流已不可用，请刷新列表后重试。");
          var area = root.querySelector(".owo-wf-editor");
          var text = JSON.stringify(item.template, null, 2);
          var generation = lifecycleGeneration;
          var confirmation = area.value.trim() && area.value.trim() !== text.trim()
            ? H.confirm({ title: "打开其他工作流？", message: "编辑器中已有内容。打开这条工作流会替换当前编辑内容；已保存的配置不会改变。", confirmText: "打开工作流" })
            : Promise.resolve(true);
          Promise.resolve(confirmation).then(function (confirmed) {
            if (!confirmed || generation !== lifecycleGeneration || rootEl !== root) return;
            area.value = text;
            var section = area.closest("details");
            if (section) section.open = true;
            setError(root, "");
            showPreview(root, "正在编辑：" + String(item.template.name || id) + "。修改后请重新校验并保存。", item.template);
            area.scrollIntoView({ block: "center" });
            area.focus({ preventScroll: true });
          });
        });
      });
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-export")).forEach(function (button) {
        button.addEventListener("click", function () {
          var id = button.closest(".owo-wf-item").getAttribute("data-id");
          var request = editor().buildExportRequest(id);
          if (!request.ok) {
            setError(root, request.error);
            return;
          }
          setMeta(root, "正在导出 " + id + " …");
          H.post(request.path, request.body)
            .then(function (exported) {
              var box = root.querySelector(".owo-wf-review-out");
              if (box) {
                box.textContent =
                  "社区包（复制后发给别人导入）：\n" +
                  String(exported.package_b64 || "") +
                  "\n\n预审：\n" +
                  renderReviewText({ review: exported.review });
              }
              setMeta(root, "已导出 " + id + "（含 sha256 与预审发现）");
            })
            .catch(function (error) {
              setError(root, "导出失败：" + H.friendlyError(error));
            });
        });
      });
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-delete")).forEach(function (button) {
        button.addEventListener("click", function () {
          var id = button.closest(".owo-wf-item").getAttribute("data-id");
          H.delete("/workflows/user/" + encodeURIComponent(id))
            .then(function () {
              setMeta(root, "已删除 " + id);
              return refresh(root);
            })
            .catch(function (error) {
              setError(root, "删除失败：" + H.friendlyError(error));
            });
        });
      });
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-run")).forEach(function (button) {
        button.addEventListener("click", function () {
          var id = button.closest(".owo-wf-item").getAttribute("data-id");
          var objectiveInput = root.querySelector(".owo-wf-objective");
          createTeamFromRow(root, id, objectiveInput ? objectiveInput.value : "");
        });
      });
      // 「按证据改进」：把该工作流的真实运行证据注入 AI 草稿（生成候选，不直接改配置）。
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-improve")).forEach(function (button) {
        button.addEventListener("click", function () {
          var id = button.closest(".owo-wf-item").getAttribute("data-id");
          var descInput = root.querySelector(".owo-wf-desc");
          var typed = descInput ? String(descInput.value || "").trim() : "";
          var description = typed || ("按真实运行证据改进工作流 " + id + "，让它稳定通过");
          if (descInput) descInput.value = description;
          var request = editor().buildDraftRequest(
            description,
            root.querySelector(".owo-wf-constraints") ? root.querySelector(".owo-wf-constraints").value : "",
            "",
            root.querySelector(".owo-wf-rounds") ? root.querySelector(".owo-wf-rounds").value : 1,
            id
          );
          if (!request.ok) {
            setError(root, request.error);
            return;
          }
          setError(root, "");
          var out = root.querySelector(".owo-wf-draft-out");
          if (out) out.textContent = "按运行证据生成改进候选…";
          H.post(request.path, request.body)
            .then(function (draft) {
              if (out) out.textContent = renderDraftText(draft);
              var draftId = (draft && draft.draft_id) || (draft && draft.draft && draft.draft.draft_id) || "";
              root.setAttribute("data-draft-id", draftId);
              var adopt = root.querySelector(".owo-wf-adopt");
              var discard = root.querySelector(".owo-wf-discard");
              if (adopt) adopt.disabled = !draftId;
              if (discard) discard.disabled = !draftId;
            })
            .catch(function (error) {
              if (out) out.textContent = "";
              setError(root, "生成改进候选失败：" + H.friendlyError(error));
            });
        });
      });
      // 「再跑」：沿用最近一次运行的目标（没有则回落到输入框）。
      Array.prototype.slice.call(root.querySelectorAll(".owo-wf-item .owo-wf-rerun")).forEach(function (button) {
        button.addEventListener("click", function () {
          rerunFromLast(root, button.closest(".owo-wf-item").getAttribute("data-id"));
        });
      });
    }

    /// 读取最近一次运行 → 用同一目标建队（列表行与统计行共用）。
    function rerunFromLast(root, id) {
      if (!id) return;
      setError(root, "");
      setMeta(root, "读取 " + id + " 的最近一次运行…");
      H.get("/workflows/user/" + encodeURIComponent(id) + "/last-run")
        .then(function (payload) {
          var last = editor().describeLastRun(payload);
          if (!last.hasRun) {
            setMeta(root, id + " 还没有运行记录；先在目标框写目标再点「建队」");
            return;
          }
          var objectiveInput = root.querySelector(".owo-wf-objective");
          var typed = objectiveInput ? String(objectiveInput.value || "").trim() : "";
          if (objectiveInput && !typed) objectiveInput.value = last.objective;
          var objective = typed || last.objective;
          setMeta(root, "沿用上次目标（" + last.statusLabel + "：" + (last.objective || "?") + "）");
          createTeamFromRow(root, id, objective);
        })
        .catch(function (error) {
          setError(root, "读取最近一次运行失败：" + H.friendlyError(error));
        });
    }

    /// 建队（「建队」与「再跑」共用：能力面 block 拒绝、warn 需一次确认、带 acknowledge_risk）。
    function attachWorkspaceContext(root, body) {
      var snapshot = H.snapshot ? H.snapshot() : {};
      if (snapshot.workspace) {
        var allow = root.querySelector(".owo-wf-allow-writes");
        body.workspace = { root: snapshot.workspace, read_only: !(allow && allow.checked) };
        var parent = (snapshot.sessions || []).find(function (s) { return s.id === snapshot.sessionId; });
        if (parent && parent.workspace === snapshot.workspace) body.parent_session_id = parent.id;
      }
      return body;
    }

    /// 建队前读取"允许按声明范围写入"复选框（缺省勾选：自定义工作流没有交互审批通道）。
    function scopedWritesAllowed(root) {
      var box = root.querySelector(".owo-wf-allow-writes");
      return !box || box.checked;
    }

    function createTeamFromRow(root, id, objective) {
      var request = editor().buildTeamCreateRequest(
        { template_id: id, mode: "team" },
        objective,
        false,
        scopedWritesAllowed(root)
      );
      if (!request.ok) {
        setError(root, request.error);
        return;
      }
      var row = root.querySelector('.owo-wf-item[data-id="' + id + '"]');
      var findings = [];
      try {
        findings = JSON.parse((row && row.getAttribute("data-findings")) || "[]");
      } catch (error) {
        findings = [];
      }
      var confirmText = editor().buildTeamCreateConfirm({ findings: findings, template: { template_id: id } });
      if (confirmText) {
        if (confirmText.indexOf("不能建队") >= 0) {
          setError(root, confirmText);
          return;
        }
        if (!window.confirm(confirmText)) {
          setMeta(root, "已取消建队（未发送请求）");
          return;
        }
      }
      // 用户已确认（或本来没有风险提示）：告诉服务端允许在社区来源下建队。
      if (request.body) request.body.acknowledge_risk = true;
      setError(root, "");
      setMeta(root, "正在用 " + id + " 创建团队…");
      H.post(request.path, attachWorkspaceContext(root, request.body))
        .then(function (created) {
          var teamId = (created && (created.team_id || (created.team && created.team.team_id))) || "";
          setMeta(root, "已创建团队 " + (teamId || "(未知 id)") + "：可就地查看成员对话");
          setTeamJump(root, teamId);
        })
        .catch(function (error) {
          setError(root, "创建团队失败：" + H.friendlyError(error));
        });
    }

    function mount(root, helpers) {
      dispose();
      H = {
        snapshot: helpers && helpers.snapshot,
        confirm: (helpers && helpers.confirm) || function (options) { return Promise.resolve(win.confirm(options.message)); },
        get: (helpers && helpers.get) || defaultGet,
        post: (helpers && helpers.post) || defaultPost,
        delete: (helpers && helpers.delete) || defaultDelete,
        esc: (helpers && helpers.esc) || defaultEsc,
        friendlyError: (helpers && helpers.friendlyError) || defaultFriendlyError,
      };
      if (!root) return;
      rootEl = root;
      root.innerHTML = nav();
      refreshCatalog(root);
      root.querySelector(".owo-wf-catalog-refresh").addEventListener("click", function () {
        refreshCatalog(root);
      });
      root.querySelector(".owo-wf-refresh").addEventListener("click", function () {
        refresh(root);
      });
      root.querySelector(".owo-wf-stats").addEventListener("click", function () {
        refreshStats(root);
      });
      root.querySelector(".owo-wf-stats-sort").addEventListener("change", function () {
        refreshStats(root);
      });
      root.querySelector(".owo-wf-check").addEventListener("click", function () {
        try {
          var template = editorTemplate(root);
          var problems = editor().quickCheck(template);
          showPreview(root, problems.length ? problems.join("\n") : "本地检查通过（完整校验以服务端为准）", template);
        } catch (error) {
          showPreview(root, String(error.message || error));
        }
      });
      // 一键补 verify：直接改编辑器文本并重新渲染（用户可继续手改）。
      root.addEventListener("click", function (event) {
        var button = event.target && event.target.closest
          ? event.target.closest(".owo-wf-fix-verify")
          : null;
        if (!button) return;
        var helper = editor();
        var area = root.querySelector(".owo-wf-editor");
        if (!helper || !area) return;
        var parsed = helper.parseTemplate(area.value);
        if (!parsed.ok) {
          setError(root, parsed.error);
          return;
        }
        var isScope = button.classList.contains("owo-wf-fix-scope");
        var fixed = isScope
          ? helper.withSuggestedWritePaths(parsed.template)
          : helper.withDefaultVerify(parsed.template);
        area.value = JSON.stringify(fixed, null, 2);
        var preview = root.querySelector(".owo-wf-preview");
        showPreview(root, preview ? preview.textContent : "", fixed);
        setMeta(
          root,
          isScope
            ? "已给缺写范围的写角色补上 outputs/<角色> 目录（可自行修改，记得点保存）"
            : "已给缺失角色补上 verify: non_empty（记得点保存）"
        );
      });

      root.querySelector(".owo-wf-validate").addEventListener("click", function () {
        var template;
        try {
          template = editorTemplate(root);
        } catch (error) {
          showPreview(root, String(error.message || error));
          return;
        }
        H.post("/workflows/user/validate", template)
          .then(function (result) {
            showPreview(root, renderReviewText(result), template);
          })
          .catch(function (error) {
            showPreview(root, "校验请求失败：" + H.friendlyError(error));
          });
      });
      root.querySelector(".owo-wf-save").addEventListener("click", function () {
        var template;
        try {
          template = editorTemplate(root);
        } catch (error) {
          showPreview(root, String(error.message || error));
          return;
        }
        H.post("/workflows/user", template)
          .then(function () {
            showPreview(root, "已保存：" + String(template.template_id || ""), template);
            return refresh(root);
          })
          .catch(function (error) {
            showPreview(root, "保存失败：" + H.friendlyError(error));
          });
      });
      root.querySelector(".owo-wf-draft").addEventListener("click", function () {
        var request = editor().buildDraftRequest(
          root.querySelector(".owo-wf-desc").value,
          root.querySelector(".owo-wf-constraints").value,
          root.getAttribute("data-refine-of") || "",
          root.querySelector(".owo-wf-rounds") ? root.querySelector(".owo-wf-rounds").value : 1
        );
        if (!request.ok) {
          setError(root, request.error);
          return;
        }
        setError(root, "");
        var button = root.querySelector(".owo-wf-draft");
        var out = root.querySelector(".owo-wf-draft-out");
        button.disabled = true;
        button.textContent = "生成中…";
        out.textContent = "正在拆解角色、依赖、工具与交付契约…（模型返回前保持等待）";
        H.post(request.path, request.body)
          .then(function (draft) {
            out.textContent = renderDraftText(draft);
            var draftId = (draft && draft.draft_id) || (draft && draft.draft && draft.draft.draft_id) || "";
            root.setAttribute("data-draft-id", draftId);
            root.querySelector(".owo-wf-adopt").disabled = !draftId;
            root.querySelector(".owo-wf-discard").disabled = !draftId;
            if (draft && draft.template) {
              var preview = root.querySelector(".owo-wf-preview");
              showPreview(root, preview ? preview.textContent : "", draft.template);
              var info = root.querySelector(".owo-wf-graph-info");
              if (info && !draft.ok) {
                info.insertAdjacentHTML(
                  "afterbegin",
                  '<p class="hint owo-wf-risk">候选仍有校验问题：可直接拖动节点看结构，修正后再采纳。</p>'
                );
              }
            }
          })
          .catch(function (error) {
            out.textContent = "生成失败：" + H.friendlyError(error);
          })
          .then(function () {
            button.disabled = false;
            button.textContent = "生成工作流";
          });
      });
      root.querySelector(".owo-wf-adopt").addEventListener("click", function () {
        var draftId = root.getAttribute("data-draft-id");
        if (!draftId) return;
        H.post("/workflows/draft/" + encodeURIComponent(draftId) + "/adopt", {})
          .then(function (adopted) {
            var plan = adopted && adopted.verify ? adopted.verify : null;
            if (!plan) {
              setMeta(root, "已采纳草稿 " + draftId + "（这版没有运行基线，跑一次后再用「按证据改进」对比）");
              return refresh(root);
            }
            // 有基线：给出可对比的验证入口（沿用同一个目标再跑一次）。
            setMeta(
              root,
              "已采纳 " +
                (adopted.template && adopted.template.template_id) +
                "；基线：" +
                plan.previous_status +
                "（目标：" +
                plan.objective +
                "）"
            );
            var out = root.querySelector(".owo-wf-draft-out");
            if (out && window.confirm("采纳成功。现在用同一个目标再跑一次做前后对比？")) {
              setMeta(root, "验证运行（沿用基线目标）：" + plan.objective);
              H.post("/teams", attachWorkspaceContext(root, {
                objective: plan.objective,
                mode: "team",
                template_id: adopted.template ? adopted.template.template_id : plan.adopted_template_id,
                acknowledge_risk: true,
              }))
                .then(function (created) {
                  var teamId =
                    (created && (created.team_id || (created.team && created.team.team_id))) || "";
                  setMeta(
                    root,
                    "验证团队 " +
                      teamId +
                      " 已创建（对比基线 " +
                      plan.previous_status +
                      "）：可就地查看成员对话"
                  );
                  setTeamJump(root, teamId);
                  return refresh(root);
                })
                .catch(function (error) {
                  setError(root, "验证运行创建失败：" + H.friendlyError(error));
                });
              return undefined;
            }
            return refresh(root);
          })
          .catch(function (error) {
            setError(root, "采纳失败：" + H.friendlyError(error));
          });
      });
      root.querySelector(".owo-wf-discard").addEventListener("click", function () {
        var draftId = root.getAttribute("data-draft-id");
        if (!draftId) return;
        H.delete("/workflows/draft/" + encodeURIComponent(draftId))
          .then(function () {
            root.querySelector(".owo-wf-draft-out").textContent = "已丢弃草稿 " + draftId;
            root.querySelector(".owo-wf-adopt").disabled = true;
            root.querySelector(".owo-wf-discard").disabled = true;
          })
          .catch(function (error) {
            setError(root, "丢弃失败：" + H.friendlyError(error));
          });
      });
      root.querySelector(".owo-wf-review").addEventListener("click", function () {
        var request = editor().buildImportRequest(root.querySelector(".owo-wf-package").value, true);
        if (!request.ok) {
          setError(root, request.error);
          return;
        }
        H.post(request.path, request.body)
          .then(function (result) {
            var diffLines = editor() && editor().describeLocalDiff ? editor().describeLocalDiff(result) : [];
            root.querySelector(".owo-wf-review-out").textContent =
              renderReviewText(result) + (diffLines.length ? "\n\n" + diffLines.join("\n") : "");
          })
          .catch(function (error) {
            root.querySelector(".owo-wf-review-out").textContent = "评审失败：" + H.friendlyError(error);
          });
      });
      root.querySelector(".owo-wf-import").addEventListener("click", function () {
        var request = editor().buildImportRequest(root.querySelector(".owo-wf-package").value, false);
        if (!request.ok) {
          setError(root, request.error);
          return;
        }
        H.post(request.path, request.body)
          .then(function (imported) {
            var importedDiff = editor() && editor().describeLocalDiff ? editor().describeLocalDiff(imported) : [];
            root.querySelector(".owo-wf-review-out").textContent =
              "已导入（" + String((imported && imported.template && imported.template.origin) || "?") + "）：\n" +
              renderReviewText(imported) +
              (importedDiff.length ? "\n\n" + importedDiff.join("\n") : "");
            return refresh(root);
          })
          .catch(function (error) {
            root.querySelector(".owo-wf-review-out").textContent = "导入被拒绝：" + H.friendlyError(error);
          });
      });
      refresh(root);
    }

    /// 从对话/Work 建议直达：打开面板后把一句话填进生成框并聚焦。
    function focusBrief(text) {
      if (!rootEl) return false;
      var input = rootEl.querySelector(".owo-wf-desc");
      if (!input) return false;
      var value = String(text == null ? "" : text).trim();
      if (value) input.value = value;
      input.focus();
      if (typeof input.scrollIntoView === "function") input.scrollIntoView({ block: "center", behavior: "smooth" });
      return true;
    }

    function dispose() {
      lifecycleGeneration += 1;
      rootEl = null;
    }

    return {
      id: ID,
      mount: mount,
      dispose: dispose,
      refresh: refresh,
      focusBrief: focusBrief,
      renderWorkflowRows: renderWorkflowRows,
      renderReviewText: renderReviewText,
      renderDraftText: renderDraftText,
      renderChecklist: renderChecklist,
      renderGraphSvg: renderGraphSvg,
      renderToolCatalog: renderToolCatalog,
      renderRoleRows: renderRoleRows,
      renderStatsRows: renderStatsRows,
      refreshStats: refreshStats,
      _test: {
        renderWorkflowRows: renderWorkflowRows,
        renderReviewText: renderReviewText,
        renderDraftText: renderDraftText,
        renderChecklist: renderChecklist,
        renderGraphSvg: renderGraphSvg,
        renderToolCatalog: renderToolCatalog,
        renderRoleRows: renderRoleRows,
        renderStatsRows: renderStatsRows,
      },
    };
  })();
})();
