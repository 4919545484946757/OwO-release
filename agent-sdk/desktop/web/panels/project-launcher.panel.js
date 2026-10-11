// ============================================================================
// Project Launcher（新建项目任务）面板 —— desktop/web/panels/project-launcher.panel.js
//
// 六期第四路：把工作区、模板、自适应组队和交付物串成普通用户可直接使用的
// 启动流程（七步）：① 目标 项目目录 ③ 只读/写入范围 执行方式
// ⑤ 已安装模板 ⑥ 预览角色/预算/权限 开始任务并进入 TeamRun。
//
// 依赖路由（六期冻结契约，字段 UI 全容错缺省）：
//   GET  /teams/templates/catalog            {catalog:[{template_id,version,title,
//                                            category,description,roles[],edges[],
//                                            budget_calls_total,artifact_kinds[],
//                                            installed,builtin}]}
//   POST /teams/templates/catalog/{id}/install  → {template_id,version,installed,replayed}
//   POST /teams                              {objective, template_id?, strategy?,
//                                            workspace?{root,read_only,write_allowed_paths,tree_depth}}
//                                            → 202 {team_id, strategy_decision, ...}
//
// 模板目录只展示候选：未安装模板在选择器中禁用（提示需先安装）；安装按钮
// 幂等（replayed 如实提示）。创建成功后经 window.OwoPanels.workswarm.open()
// 直达团队详情（不重复挂载面板）。
//
// 七期第四路：步骤⑥预览升级为"实际执行权限"——每角色附由工作区设置 + 角色
// 画像推导的权限行（只读/可写路径/命令/浏览器；真实生效以团队详情
// worker_profiles 为准），不再只展示模板描述文字。
//
// Node 兼容：globalThis 回退 + module.exports 导出 _test 纯逻辑挂钩（浏览器
// 零差异），供 tests/project-launcher.panel.test.mjs 断言。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels["project-launcher"] = (function () {
    var ID = "project-launcher";

    // ---------- helpers（优先 app.js 注入，缺失时自建回退，与 workswarm 面板同款） ----------
    var H = {};
    var rootEl = null;
    var lifecycleGeneration = 0;
    var treeGeneration = 0;
    var catalogGeneration = 0;

    function isCurrent(generation) {
      return generation === lifecycleGeneration && !!rootEl;
    }

    function defaultGet(path) {
      return window.OwoApi.get(path);
    }

    function defaultPost(path, body) {
      return window.OwoApi.post(path, body || {});
    }

    function defaultEsc(s) {
      return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
        return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
      });
    }

    function defaultFriendlyError(e) {
      var msg = String((e && e.message) || e || "");
      if (/^404/.test(msg)) return "资源不存在（404）。请确认目录/模板已就绪后重试。";
      if (/^400/.test(msg)) return "请求被拒绝（400）：" + msg.slice(5, 160);
      if (/^401|^403/.test(msg)) return "无权访问（401/403）。请检查凭据与权限范围。";
      if (/Failed to fetch|NetworkError/i.test(msg)) return "网络错误：无法连接服务端，请确认服务已启动。";
      return msg || "未知错误";
    }

    function el(sel) {
      if (!rootEl) return null;
      var s = String(sel || "");
      return rootEl.querySelector(s.charAt(0) === "#" ? s : "#" + s);
    }

    function stateBox(kind, text) {
      if (kind === "failed") {
        return '<div class="owo-pl-failed owo-pl-load-error" role="alert"><span>' +
          esc(text) +
          '</span><button type="button" class="owo-ws-mini" data-pl-catalog-retry>重试</button></div>';
      }
      var cls = kind === "empty" ? "hint" : "hint";
      var role = kind === "loading" ? ' role="status"' : "";
      return '<div class="' + cls + '"' + role + ">" + esc(text) + "</div>";
    }

    var esc = defaultEsc;

    // ---------- 状态 ----------
    var state = {
      objective: "",
      root: "",
      readOnly: true,
      writePathsRaw: "",
      treeDepth: 2,
      // §8.1 目录树多选选择器（写入路径）：预绑定树预览 + 勾选行 + 搜索过滤。
      treePickerOpen: false,
      treeBusy: false,
      treeError: "",
      treeLoadFailed: false,
      treeEntries: null, // 最近一次 /workspace/tree 预览的 entries
      treeRoot: "", // 预览对应的规范化 root 回显
      treeDepthUsed: 0, // 预览实际使用的深度
      treeQuery: "", // 选择器内搜索子串
      pickedRows: [], // 勾选行 {path, type:"dir"|"file"}
      strategy: "auto",
      catalog: [],
      catalogLoaded: false,
      catalogError: "",
      selectedTemplate: "",
      installBusy: {},
      creating: false,
      wizardStep: 1,
      result: null,
      error: "",
    };

    // ---------- 纯函数（_test 导出，Node 可测） ----------

    /// 解析写入范围输入：逗号/换行分隔 → 去空白/去重/去空项。
    function parseWritePaths(raw) {
      var seen = {};
      var out = [];
      String(raw == null ? "" : raw).split(/[\n,]+/).forEach(function (p) {
        var t = String(p).trim();
        if (t && !seen[t]) {
          seen[t] = true;
          out.push(t);
        }
      });
      return out;
    }

    /// 客户端基础校验：越界片段（..）、以 / 或盘符转义开头的越权尝试直接拒绝。
    function pathIsSafe(p) {
      if (!p) return false;
      if (p === ".." || p.indexOf("..\\") >= 0 || p.indexOf("../") >= 0) return false;
      if (/^[a-zA-Z]:/.test(p)) return false; // 允许 root 本身带盘符；允许路径必须相对
      return true;
    }

    // ---------- §8.1 目录树多选（纯函数，Node 可测） ----------

    /// 树条目过滤：按子串匹配路径（不区分大小写）；query 空时原样返回。
    function filterTreeEntries(entries, query) {
      var q = String(query == null ? "" : query).trim().toLowerCase();
      var list = Array.isArray(entries) ? entries : [];
      if (!q) return list;
      return list.filter(function (en) {
        return String((en && en.path) || "").toLowerCase().indexOf(q) >= 0;
      });
    }

    /// 勾选切换（纯函数）：存在同路径行则移除，否则追加 {path, type}。
    function toggleTreePick(rows, path, type) {
      var p = String(path || "");
      var out = [];
      var found = false;
      (Array.isArray(rows) ? rows : []).forEach(function (r) {
        if (r && String(r.path) === p) {
          found = true;
          return;
        }
        out.push(r);
      });
      if (!found && p) out.push({ path: p, type: type === "dir" ? "dir" : "file" });
      return out;
    }

    /// 勾选行合入 writePathsRaw：剔除 prevRows（上一轮勾选）保留手输，再并入当前勾选。
    /// pathIsSafe 为额外防线（服务端树本身不含 `..`/符号链接）。
    function mergePickedIntoRaw(raw, rows, prevRows) {
      var prevSet = {};
      (Array.isArray(prevRows) ? prevRows : []).forEach(function (r) {
        if (r && r.path) prevSet[String(r.path)] = true;
      });
      var manual = parseWritePaths(raw).filter(function (p) {
        return !prevSet[p];
      });
      var seen = {};
      manual.forEach(function (p) {
        seen[p] = true;
      });
      var out = manual.slice();
      (Array.isArray(rows) ? rows : []).forEach(function (r) {
        var p = String((r && r.path) || "");
        if (p && !seen[p] && pathIsSafe(p)) {
          seen[p] = true;
          out.push(p);
        }
      });
      return out.join(", ");
    }

    /// 选择器 HTML（依赖 esc；state 为只读入参）。
    function treePickerHtml(s) {
      var entries = filterTreeEntries(s.treeEntries, s.treeQuery);
      var head =
        '<div class="owo-pl-tree-head">' +
        '<input id="pl-tree-search" size="36" placeholder="搜索路径（子串）" value="' + esc(s.treeQuery) + '">' +
        '<button type="button" class="owo-ws-mini" data-pl-tree-reload>按当前深度重新读取</button>' +
        '<span class="hint">' +
        (s.treeRoot ? "root（规范化）：" + esc(s.treeRoot) + " · " : "") +
        "深度 " + esc(String(s.treeDepthUsed || s.treeDepth)) +
        (Array.isArray(s.treeEntries) ? " · 共 " + s.treeEntries.length + " 项" : "") +
        "</span></div>";
      var picked;
      if (s.pickedRows.length) {
        picked =
          '<div class="owo-pl-picked">' +
          '<span class="hint">已选（' + s.pickedRows.length + "）：</span>" +
          s.pickedRows.map(function (r) {
            return '<span class="owo-pl-pickrow">' + esc(r.path) +
              ' <button type="button" class="owo-ws-mini" data-pl-unpick="' + esc(r.path) + '" title="移除">×</button></span>';
          }).join(" ") + "</div>";
      } else {
        picked = '<div class="hint">尚未勾选：勾选目录（含其子树）或文件加入允许写入范围。</div>';
      }
      var list;
      if (!Array.isArray(s.treeEntries)) {
        list = '<div class="hint">尚未读取目录树。</div>';
      } else if (!entries.length) {
        list = '<div class="hint">无匹配条目。</div>';
      } else {
        list =
          '<div class="owo-pl-tree-list">' +
          entries.map(function (en) {
            var p = String((en && en.path) || "");
            if (!p) return "";
            var isDir = en.type === "dir";
            var checked = (s.pickedRows || []).some(function (r) {
              return String(r.path) === p;
            });
            var level = Math.max(0, p.split("/").length - 1);
            var name = p.split("/").pop() || p;
            return '<label class="owo-pl-tree-row" style="padding-left:' + level * 14 + 'px">' +
              '<input type="checkbox" class="pl-tree-check" data-pl-tree-path="' + esc(p) +
              '" data-pl-tree-type="' + (isDir ? "dir" : "file") + '"' + (checked ? " checked" : "") + "> " +
              '<span title="' + esc(p) + '">' + esc(name) + (isDir ? "/" : "") + "</span></label>";
          }).join("") + "</div>";
      }
      return head + picked + list;
    }

    /// 汇总校验错误（空数组 = 通过）。
    function validateState(s) {
      var errs = [];
      if (!s || !String(s.objective || "").trim()) errs.push("请填写任务目标（步骤 ①）");
      var root = String((s && s.root) || "").trim();
      if (!root) errs.push("请绑定项目目录（步骤 ②）");
      var paths = parseWritePaths(s && s.writePathsRaw);
      for (var i = 0; i < paths.length; i++) {
        if (!pathIsSafe(paths[i])) {
          errs.push("写入范围含不安全路径：" + paths[i] + "（禁止 .. 与绝对路径转义）");
        }
      }
      var depth = Number(s && s.treeDepth);
      if (!isFinite(depth) || depth < 1 || depth > 8) errs.push("目录树深度须在 1–8 之间");
      return errs;
    }

    /// 组装 POST /teams 请求体（冻结契约；workspace 根为空时不带 workspace 字段）。
    function buildCreateBody(s) {
      var body = {
        objective: String((s && s.objective) || "").trim(),
        strategy: (s && s.strategy) || "auto",
      };
      var tpl = String((s && s.selectedTemplate) || "");
      if (tpl) body.template_id = tpl;
      var root = String((s && s.root) || "").trim();
      if (root) {
        var ws = { root: root, read_only: !(s && s.readOnly === false) };
        var paths = parseWritePaths(s && s.writePathsRaw);
        if (paths.length) ws.write_allowed_paths = paths;
        var depth = Number(s && s.treeDepth);
        if (isFinite(depth)) ws.tree_depth = Math.max(1, Math.min(8, Math.round(depth)));
        body.workspace = ws;
      }
      if (body.strategy === "single") body.mode = "single";
      if (body.strategy === "team") body.mode = "team";
      return body;
    }

    /// 从模板目录条目生成预览（角色/依赖/预算/产物类型）。
    function previewFromTemplate(entry) {
      var n = normTemplate(entry);
      if (!n) return null;
      return {
        source: "template",
        title: n.title,
        version: n.version,
        roleCount: n.roles.length,
        roles: n.roles,
        edgeCount: n.edgeCount,
        budget: n.budgetTotal,
        artifactKinds: n.kinds,
      };
    }

    /// 目录条目双形状归一：冻结契约顶层形状（template_id/version/title/…）
    /// 与三路实现形状（template.template_id/template.name/budget_calls_per_role）
    /// 均安全读取；非对象/无 id → null。
    function normTemplate(t) {
      if (!t || typeof t !== "object") return null;
      var inner = t.template && typeof t.template === "object" ? t.template : null;
      var id = String(t.template_id || (inner && inner.template_id) || "");
      if (!id) return null;
      var rawRoles = Array.isArray(t.roles) ? t.roles : Array.isArray(inner && inner.roles) ? inner.roles : [];
      var roles = rawRoles.map(function (r) {
        return {
          role: String((r && r.role) || ""),
          duty: String((r && (r.duty || r.handoff_contract)) || ""),
        };
      }).filter(function (r) {
        return r.role;
      });
      var edges = Array.isArray(t.edges) ? t.edges.slice() : [];
      rawRoles.forEach(function (r) {
        var deps = r && Array.isArray(r.depends_on) ? r.depends_on : [];
        deps.forEach(function (d) {
          edges.push({ from: String(d), to: String(r.role || "") });
        });
      });
      var perRole = Array.isArray(t.budget_calls_per_role) ? t.budget_calls_per_role : [];
      var budgetTotal = t.budget_calls_total != null
        ? Number(t.budget_calls_total)
        : perRole.length
          ? perRole.reduce(function (acc, b) {
              return acc + (b && b.budget_calls != null ? Number(b.budget_calls) || 0 : 0);
            }, 0)
          : null;
      return {
        id: id,
        title: String(t.title || (inner && inner.name) || id),
        version: t.version != null ? String(t.version) : null,
        category: String(t.category || ""),
        roles: roles,
        edgeCount: edges.length,
        budgetTotal: isFinite(budgetTotal) ? budgetTotal : null,
        kinds: (Array.isArray(t.artifact_kinds) ? t.artifact_kinds : []).map(String),
        installed: !!t.installed,
      };
    }

    /// 无模板时按组队模式生成预览（auto 交由服务端策略判定，不伪造角色数）。
    function previewFromStrategy(strategy) {
      var s = String(strategy || "auto");
      if (s === "single") {
        return {
          source: "strategy",
          roleCount: 1,
          roles: [{ role: "producer", duty: "单 Agent 独立完成并自检" }],
          budget: null,
          note: "强制单 Agent：零评审零裁决，调用量最小",
        };
      }
      if (s === "team") {
        return {
          source: "strategy",
          roleCount: null,
          roles: [
            { role: "producer", duty: "产出主交付物" },
            { role: "critic", duty: "只读评审（只提交评审结论，不覆盖交付物）" },
          ],
          budget: null,
          note: "强制多 Agent：完整评审流水线",
        };
      }
      return {
        source: "strategy",
        roleCount: null,
        roles: [],
        budget: null,
        note: "自动组队：系统按任务画像判定单/多 Agent（简单任务默认单 Agent），判定理由可在团队详情查看",
      };
    }

    /// 权限预览文案。
    function permissionsSummary(s) {
      var readOnly = !(s && s.readOnly === false);
      var paths = parseWritePaths(s && s.writePathsRaw);
      var depth = Number(s && s.treeDepth);
      var parts = [];
      parts.push(readOnly ? "只读（默认，安全）" : "允许写入（受允许路径约束）");
      if (!readOnly && paths.length) parts.push("允许路径：" + paths.join("、"));
      if (!readOnly && !paths.length) parts.push("未列允许路径：写入将被拒绝");
      if (isFinite(depth)) parts.push("目录树深度 " + depth);
      return parts.join(" · ");
    }

    // ---------- 七期：角色实际执行权限预览（纯函数） ----------
    // 与二路 WorkerProfile 语义对齐的客户端推导：
    //   - 工作区只读（默认）→ 全部角色 read_only；
    //   - 评审类角色（critic/reviewer）恒只读（评审只提交结论，不覆盖交付物）；
    //   - 可写工作区按允许路径下发；路径留空 = 拒绝一切写入；
    //   - 浏览器能力按角色画像（research/search/browser 命名）推导，与读写正交。
    // 真实生效以团队详情 worker_profiles 为准（运行中可见）。
    var REVIEWER_ROLE_RE = /critic|review|judge|arbitrat/i;
    var BROWSER_ROLE_RE = /research|search|browser/i;

    function rolePermissions(role, s) {
      var name = String(role == null ? "" : role).trim();
      var workspaceReadOnly = !(s && s.readOnly === false);
      var isReviewer = REVIEWER_ROLE_RE.test(name);
      var readOnly = workspaceReadOnly || isReviewer;
      var paths = readOnly ? [] : parseWritePaths(s && s.writePathsRaw);
      return {
        role: name,
        read_only: readOnly,
        can_run_command: !readOnly,
        can_use_browser: BROWSER_ROLE_RE.test(name),
        write_allowed_paths: paths,
        // read_only=工作区只读或评审角色；scoped=按允许路径下发；denied=可写工作区但未声明路径
        write_mode: readOnly ? "read_only" : paths.length ? "scoped" : "denied",
      };
    }

    /// 角色权限行文案。
    function rolePermText(perm) {
      if (!perm || !perm.role) return "";
      var head = perm.read_only
        ? "实际执行权限：只读"
        : perm.write_mode === "scoped"
          ? "实际执行权限：可写 " + perm.write_allowed_paths.join("、")
          : "实际执行权限：写入被拒绝（未声明允许路径）";
      var extra = [];
      if (perm.can_run_command) extra.push("可执行命令");
      if (perm.can_use_browser) extra.push("可浏览网页");
      return head + (extra.length ? " · " + extra.join(" · ") : "");
    }

    /// 组装预览视图模型。
    function buildPreview(s, catalog) {
      var tpl = String((s && s.selectedTemplate) || "");
      if (tpl && Array.isArray(catalog)) {
        for (var i = 0; i < catalog.length; i++) {
          var n = normTemplate(catalog[i]);
          if (n && n.id === tpl) {
            var p = previewFromTemplate(catalog[i]);
            if (p) return p;
          }
        }
      }
      return previewFromStrategy(s && s.strategy);
    }

    /// 预览 HTML（角色列表 + 预算 + 权限；无横向溢出的两列布局）。
    function previewHtml(p, s) {
      if (!p) return stateBox("empty", "填写目标与目录后此处显示角色/预算/权限预览。");
      var html = '<div class="owo-pl-preview">';
      html += "<div><b>来源</b> " + esc(p.source === "template" ? "模板 " + (p.title || "") + (p.version ? " v" + p.version : "") : "组队模式") + "</div>";
      html += "<div><b>角色</b> " + (p.roleCount != null ? esc(String(p.roleCount)) + " 个" : "由策略判定");
      if (p.roles && p.roles.length) {
        // 七期：每个角色附"实际执行权限"行（由工作区设置 + 角色画像推导；
        // 真实生效以团队详情 worker_profiles 为准）。
        html += '<div class="owo-pl-roles">' + p.roles.map(function (r) {
          return (
            '<span class="owo-pl-role"><b>' + esc(r.role) + "</b>" +
            (r.duty ? '<span class="hint">' + esc(r.duty) + "</span>" : "") +
            '<span class="hint">' + esc(rolePermText(rolePermissions(r.role, s))) + "</span></span>"
          );
        }).join("") + "</div>";
      }
      html += "</div>";
      if (p.budget != null) html += "<div><b>调用预算</b> " + esc(String(p.budget)) + " 次</div>";
      if (p.artifactKinds && p.artifactKinds.length) html += "<div><b>产物类型</b> " + esc(p.artifactKinds.join("、")) + "</div>";
      if (p.note) html += '<div class="hint">' + esc(p.note) + "</div>";
      html += "<div><b>权限</b> " + esc(permissionsSummary(s)) + "</div>";
      html += "</div>";
      return html;
    }

    /// 模板选择器选项（未安装禁用 + 提示）。
    function templateOptionsHtml(catalog, selected) {
      var opts = '<option value="">不使用模板（按组队模式自动编队）</option>';
      (Array.isArray(catalog) ? catalog : []).forEach(function (raw) {
        var t = normTemplate(raw);
        if (!t) return;
        var label = esc(t.title + (t.version ? " v" + t.version : "") + (t.installed ? "" : "（未安装）"));
        var sel = selected && t.id === selected ? " selected" : "";
        opts += '<option value="' + esc(t.id) + '"' + (t.installed ? "" : " disabled") + sel + ">" + label + "</option>";
      });
      return opts;
    }

    /// 目录列表 HTML（候选区：展示 + 安装按钮；未安装可一键安装）。
    function catalogHtml(catalog, installBusy) {
      if (!Array.isArray(catalog) || !catalog.length) {
        return stateBox("empty", "模板目录为空（服务端未返回内置模板）。");
      }
      var rows = catalog.map(function (raw) {
        var t = normTemplate(raw);
        if (!t) return "";
        var busy = installBusy && installBusy[t.id];
        var badge = t.installed
          ? '<span class="owo-pl-badge ok">已安装</span>'
          : '<span class="owo-pl-badge">候选</span>';
        var btn = t.installed
          ? '<button class="owo-ws-mini" data-pl-select="' + esc(t.id) + '">选用</button>'
          : '<button class="owo-ws-mini primary" data-core-action data-core-busy="' + (busy ? "true" : "false") + '" data-pl-install="' + esc(t.id) + '"' + (busy ? " disabled" : "") + ">安装</button>";
        var roles = t.roles.map(function (r) {
          return r.role;
        }).join("→");
        return (
          '<div class="owo-pl-cat" data-pl-cat="' + esc(t.id) + '">' +
          "<div><b>" + esc(t.title) + "</b> " + badge +
          '<span class="hint"> ' + esc(t.id) + (t.version ? " · v" + esc(t.version) : "") +
          (t.category ? " · " + esc(t.category) : "") + "</span></div>" +
          (roles ? '<div class="hint">角色：' + esc(roles) + (t.budgetTotal != null ? " · 预算 " + esc(String(t.budgetTotal)) + " 次" : "") + "</div>" : "") +
          '<div class="owo-ws-inline">' + btn + "</div>" +
          "</div>"
        );
      }).filter(Boolean);
      return '<div class="owo-pl-cats">' + rows.join("") + "</div>";
    }

    /// 创建结果 HTML（team_id + 直达详情按钮）。
    function decisionHtml(decision) {
      if (!decision || typeof decision !== "object") return '<div class="hint">组队理由尚未返回，请在团队详情查看。</div>';
      const reasons = Array.isArray(decision.reasons) ? decision.reasons : [];
      return '<div class="owo-pl-decision"><strong>核心选择：' + esc(decision.mode || decision.strategy || "未返回") + '</strong>' +
        (reasons.length ? '<ul>' + reasons.map(reason => '<li>' + esc(String(reason)) + '</li>').join("") + '</ul>' : '<span>核心未返回推荐理由。</span>') + '</div>';
    }
    function resultHtml(r) {
      if (!r || !r.team_id) return "";
      return (
        '<div class="owo-pl-result ok">' +
        "<div>团队已创建：<b>" + esc(r.team_id) + "</b>" +
        (r.project_space_id ? '<span class="hint">（' + esc(r.project_space_id) + "）</span>" : "") +
        "</div>" +
        decisionHtml(r.strategy_decision) +
        '<button class="primary" id="pl-goto-team">查看执行进度与交付物 →</button>' +
        "</div>"
      );
    }

    // ---------- 交互 ----------

    function syncStateFromDom() {
      // 元素不存在（Node 测试/未挂载）时保留当前 state，不清空。
      var e;
      if ((e = el("#pl-objective"))) state.objective = e.value || "";
      if ((e = el("#pl-root")) && (/[\\/]|^[A-Za-z]:/.test(e.value.trim()) || !state.root)) state.root = e.value || "";
      if ((e = el("#pl-readonly"))) state.readOnly = !!e.checked;
      if ((e = el("#pl-writepaths"))) state.writePathsRaw = e.value || "";
      if ((e = el("#pl-depth"))) {
        var d = Number(e.value);
        if (isFinite(d)) state.treeDepth = d;
      }
      if ((e = el("#pl-strategy"))) state.strategy = e.value || "auto";
      if ((e = el("#pl-template"))) state.selectedTemplate = e.value || "";
    }

    function repaintPreview() {
      syncStateFromDom();
      var box = el("#pl-preview");
      if (box) box.innerHTML = previewHtml(buildPreview(state, state.catalog), state);
      var errs = el("#pl-errors");
      if (errs) errs.innerHTML = "";
    }

    // ---------- §8.1 目录树多选（交互） ----------

    /// 拉取预绑定目录树并展开选择器（root 取 ② 输入；depth 取当前深度）。
    function loadTree() {
      syncStateFromDom();
      var lifecycle = lifecycleGeneration;
      var request = ++treeGeneration;
      var root = String(state.root || "").trim();
      var box = el("#pl-treebox");
      if (!root) {
        state.treeLoadFailed = false;
        state.treeError = "请先在 ② 填写项目目录（需真实存在的绝对路径）";
        if (box) box.innerHTML = '<div class="owo-pl-failed" role="alert">' + esc(state.treeError) + "</div>";
        return Promise.resolve();
      }
      state.treeBusy = true;
      state.treeError = "";
      state.treeLoadFailed = false;
      if (box) box.innerHTML = '<div class="hint" role="status">正在读取目录树…（不跟随符号链接）</div>';
      return Promise.resolve()
        .then(function () {
          return H.get("/workspace/tree?root=" + encodeURIComponent(root) + "&depth=" + encodeURIComponent(String(Number(state.treeDepth) || 2)));
        })
        .then(function (d) {
          if (!isCurrent(lifecycle) || request !== treeGeneration || String(state.root || "").trim() !== root) return;
          state.treeBusy = false;
          state.treeLoadFailed = false;
          state.treeEntries = (d && Array.isArray(d.entries)) ? d.entries : [];
          state.treeRoot = String((d && d.root) || root);
          state.treeDepthUsed = Number((d && d.depth) || state.treeDepth) || 2;
          state.treePickerOpen = true;
          repaintTreePicker();
        })
        .catch(function (e) {
          if (!isCurrent(lifecycle) || request !== treeGeneration || String(state.root || "").trim() !== root) return;
          state.treeBusy = false;
          state.treeError = friendly(e);
          state.treeLoadFailed = true;
          if (box) box.innerHTML =
            '<div class="owo-pl-failed owo-pl-load-error" role="alert"><span>目录树读取失败：' +
            esc(state.treeError) +
            '</span><button type="button" class="owo-ws-mini" data-pl-tree-reload>重试</button></div>';
        });
    }

    /// 仅重绘选择器容器（不整面板重渲，保持滚动/输入位置）。
    function repaintTreePicker() {
      var box = el("#pl-treebox");
      if (box) box.innerHTML = state.treePickerOpen ? treePickerHtml(state) : "";
    }

    /// 勾选/取消勾选一个路径：更新 pickedRows 并同步 writePathsRaw（手输保留）。
    function treeCheckChange(path, type, checked) {
      var prev = state.pickedRows.slice();
      if (checked && !pathIsSafe(String(path || ""))) {
        var box = el("#pl-treebox");
        if (box) {
          var bad = box.querySelector('[data-pl-tree-path="' + String(path).replace(/"/g, '\\"') + '"]');
          if (bad) bad.checked = false;
        }
        state.treeError = "已拒绝不安全路径：" + path;
        repaintTreePicker();
        return;
      }
      state.pickedRows = toggleTreePick(state.pickedRows, path, type);
      state.writePathsRaw = mergePickedIntoRaw(state.writePathsRaw, state.pickedRows, prev);
      var wp = el("#pl-writepaths");
      if (wp) wp.value = state.writePathsRaw;
      repaintTreePicker();
      repaintPreview();
    }

    function unpickPath(path) {
      var prev = state.pickedRows.slice();
      state.pickedRows = toggleTreePick(state.pickedRows, path, "");
      state.writePathsRaw = mergePickedIntoRaw(state.writePathsRaw, state.pickedRows, prev);
      var wp = el("#pl-writepaths");
      if (wp) wp.value = state.writePathsRaw;
      repaintTreePicker();
      repaintPreview();
    }

    function loadCatalog() {
      var lifecycle = lifecycleGeneration;
      var request = ++catalogGeneration;
      var box = el("#pl-catalog");
      state.catalogError = "";
      state.catalogLoaded = false;
      if (box) box.innerHTML = stateBox("loading", "正在加载内置模板目录…");
      return Promise.resolve()
        .then(function () { return H.get("/teams/templates/catalog"); })
        .then(function (d) {
        if (!isCurrent(lifecycle) || request !== catalogGeneration) return;
        state.catalog = (d && d.catalog) || [];
        state.catalogLoaded = true;
        state.catalogError = "";
        if (box) box.innerHTML = catalogHtml(state.catalog, state.installBusy);
        var sel = el("#pl-template");
        if (sel) sel.innerHTML = templateOptionsHtml(state.catalog, state.selectedTemplate);
        }).catch(function (e) {
        if (!isCurrent(lifecycle) || request !== catalogGeneration) return;
        state.catalog = [];
        state.catalogLoaded = true;
        state.catalogError = friendly(e);
        if (box) box.innerHTML = stateBox("failed", "模板目录加载失败：" + state.catalogError + "（目录不可用时仍可按组队模式手动创建）");
      });
    }

    function friendly(e) {
      return (H.friendlyError && H.friendlyError(e)) || defaultFriendlyError(e);
    }

    function installTemplate(id) {
      if (!id || state.installBusy[id]) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      state.installBusy[id] = true;
      var box = el("#pl-catalog");
      if (box) {
        var btn = box.querySelector('[data-pl-install="' + id.replace(/"/g, '\\"') + '"]');
        if (btn) {
          btn.disabled = true;
          if (btn.setAttribute) {
            btn.setAttribute("data-core-busy", "true");
            btn.setAttribute("aria-busy", "true");
          }
          btn.textContent = "安装中…";
        }
      }
      return Promise.resolve()
        .then(function () { return H.post("/teams/templates/catalog/" + encodeURIComponent(id) + "/install", {}); })
        .then(function (d) {
          if (!isCurrent(lifecycle)) return;
          var already = !!(d && (d.replayed || d.already_installed));
          var installed = !!(d && (d.installed || d.already_installed));
          if (!installed) throw new Error("安装未生效（服务端返回 installed=false）");
          state.selectedTemplate = id;
          return loadCatalog().then(function () {
            if (!isCurrent(lifecycle)) return;
            var sel = el("#pl-template");
            if (sel) sel.value = id;
            var msg = el("#pl-errors");
            if (msg) msg.innerHTML = '<div class="hint">' + (already ? "该模板已安装过（幂等重放，未覆盖）。" : "安装成功。") + "已自动选用。</div>";
            repaintPreview();
          });
        })
        .catch(function (e) {
          if (!isCurrent(lifecycle)) return;
          var msg = el("#pl-errors");
          if (msg) msg.innerHTML = '<div class="owo-pl-failed">安装失败：' + esc(friendly(e)) + "</div>";
        })
        .then(function () {
          delete state.installBusy[id];
          var currentBox = el("#pl-catalog");
          if (currentBox) {
            currentBox.innerHTML = state.catalogError
              ? stateBox("failed", "模板目录加载失败：" + state.catalogError + "（目录不可用时仍可按组队模式手动创建）")
              : catalogHtml(state.catalog, state.installBusy);
          }
        });
    }

    function doCreate() {
      if (state.creating) return;
      syncStateFromDom();
      var errs = validateState(state);
      var errBox = el("#pl-errors");
      if (errs.length) {
        if (errBox) errBox.innerHTML = errs.map(function (x) { return '<div class="owo-pl-failed">· ' + esc(x) + "</div>"; }).join("");
        return;
      }
      var lifecycle = lifecycleGeneration;
      state.creating = true;
      var btn = el("#pl-create");
      if (btn) {
        btn.disabled = true;
        if (btn.setAttribute) {
          btn.setAttribute("data-core-busy", "true");
          btn.setAttribute("aria-busy", "true");
        }
        btn.textContent = "创建中…";
      }
      var body = buildCreateBody(state);
      Promise.resolve()
        .then(function () { return H.post("/teams", body); })
        .then(function (d) {
          state.result = d || {};
          state.error = "";
          if (isCurrent(lifecycle)) {
            var res = el("#pl-result");
            if (res) res.innerHTML = resultHtml(state.result);
            var go = el("#pl-goto-team");
            if (go)
              go.onclick = function () {
                gotoTeam(String(state.result.team_id));
              };
            // 创建动作已由当前挂载页发起且成功：直接进入执行详情；结果卡仍保留
            // 显式入口，方便用户稍后返回后再次打开。
            // Keep the host decision visible before opening execution details.
          }
        }).catch(function (e) {
          state.error = friendly(e);
          if (isCurrent(lifecycle)) {
            var currentErrors = el("#pl-errors");
            if (currentErrors) currentErrors.innerHTML = '<div class="owo-pl-failed">创建失败：' + esc(state.error) + "</div>";
          }
        }).then(function () {
          state.creating = false;
          // 创建锁属于共享面板状态；释放时可解锁当前挂载的按钮，但不能把旧请求结果写入新页面。
          var currentButton = el("#pl-create");
          if (currentButton) {
            if (currentButton.removeAttribute) {
              currentButton.removeAttribute("data-core-busy");
              currentButton.removeAttribute("aria-busy");
            }
            currentButton.textContent = "创建团队并开始执行";
            if (win.OwoCoreActionAvailability) {
              win.OwoCoreActionAvailability.update(win.OwoCoreActionAvailability.isAvailable());
            } else {
              currentButton.disabled = false;
            }
          }
        });
    }

    function gotoTeam(teamId) {
      // 经导航按钮切到 WorkSwarm 面板（app.js 挂载），再用公开 open() 直达详情。
      var btn = document.querySelector('#panelNav button[data-panel="workswarm"]');
      if (btn) btn.click();
      var ws = win.OwoPanels && win.OwoPanels.workswarm;
      if (ws && typeof ws.open === "function" && teamId) {
        // open 内部有异步加载；给挂载一帧时间。
        setTimeout(function () {
          try {
            ws.open(teamId);
          } catch (e) {
            /* 面板打开失败不阻塞 launcher */
          }
        }, 60);
      }
    }

    // ---------- 渲染 ----------

    /// 纯函数视图 HTML（render 只负责赋值与绑定；Node 测试直接断言此函数）。
    function viewHtml() {
      return (
        '<div class="owo-pl">' +
        '<h2>新建任务</h2>' +
        '<p class="hint">目标 → 范围与方式 → 预览并开始。默认只读，执行仍受权限与审批约束。</p>' +
        '<nav class="owo-pl-wizard" aria-label="新任务步骤"><button type="button" data-pl-wizard="1">1 · 任务目标</button><button type="button" data-pl-wizard="2">2 · 范围与方式</button><button type="button" data-pl-wizard="3">3 · 预览并开始</button></nav><p id="pl-wizard-error" role="alert" hidden></p>' +
        '<div class="owo-pl-step"><label class="owo-pl-label">任务目标</label>' +
        '<textarea id="pl-objective" rows="3" placeholder="例如：修复登录超时问题并补充回归测试（结构化/文档/研究任务均可）">' + esc(state.objective) + "</textarea></div>" +
        '<div class="owo-pl-step"><label class="owo-pl-label">项目目录 <span class="hint">Agent 将以该目录为工作区</span></label>' +
        '<input id="pl-root" size="60" placeholder="例如 T:\\我的项目\\demo（真实存在的目录）" value="' + esc(win.OwoWorkspaceDisplay ? win.OwoWorkspaceDisplay.alias(state.root) : state.root) + '"><span class="hint">日常显示项目别名，完整位置仅在详情中查看</span></div>' +
        '<div class="owo-pl-step"><label class="owo-pl-label">读写范围</label>' +
        '<div class="owo-ws-inline"><label><input type="checkbox" id="pl-readonly"' + (state.readOnly ? " checked" : "") + "> 只读（默认；取消勾选进入受控写入）</label></div>" +
        '<div id="pl-writebox"' + (state.readOnly ? ' hidden' : "") + ">" +
        '<label class="owo-pl-label">允许写入路径（相对 root，逗号或换行分隔；留空 = 拒绝一切写入）</label>' +
        '<textarea id="pl-writepaths" rows="2" placeholder="例如：src/，tests/">' + esc(state.writePathsRaw) + "</textarea>" +
        '<div class="owo-ws-inline"><button type="button" class="owo-ws-mini" data-pl-tree-load>从目录树勾选允许路径（推荐）</button>' +
        '<span class="hint">读取 ② 目录的目录树（不跟随符号链接），勾选目录/文件即并入上方允许路径</span></div>' +
        '<div id="pl-treebox" class="owo-pl-treebox">' +
          (state.treeLoadFailed
            ? '<div class="owo-pl-failed owo-pl-load-error" role="alert"><span>目录树读取失败：' + esc(state.treeError) +
              '</span><button type="button" class="owo-ws-mini" data-pl-tree-reload>重试</button></div>'
            : state.treePickerOpen
              ? treePickerHtml(state)
              : "") +
          "</div>" +
        "</div>" +
        '<div class="owo-ws-inline" id="pl-depth-group" role="group" aria-label="目录树深度（扫描范围）">' +
        '<button type="button" class="owo-ws-mini' + (Number(state.treeDepth) <= 2 ? " primary" : "") + '" data-pl-depth="2">快速 · 2 层</button>' +
        '<button type="button" class="owo-ws-mini' + (Number(state.treeDepth) > 2 && Number(state.treeDepth) <= 4 ? " primary" : "") + '" data-pl-depth="4">标准 · 4 层</button>' +
        '<button type="button" class="owo-ws-mini' + (Number(state.treeDepth) > 4 ? " primary" : "") + '" data-pl-depth="8">深入 · 8 层</button>' +
        '<details class="owo-pl-advanced"><summary>自定义深度（1–8）</summary><input id="pl-depth" type="number" min="1" max="8" value="' + esc(String(state.treeDepth)) + '" size="3"></details>' +
        "</div></div>" +
        '<div class="owo-pl-step"><label class="owo-pl-label">执行方式</label>' +
        '<select id="pl-strategy">' +
        '<option value="auto"' + (state.strategy === "auto" ? " selected" : "") + ">自动组队（推荐：简单任务自动单 Agent）</option>" +
        '<option value="single"' + (state.strategy === "single" ? " selected" : "") + ">单 Agent</option>" +
        '<option value="team"' + (state.strategy === "team" ? " selected" : "") + ">多 Agent</option>" +
        "</select></div>" +
        '<div class="owo-pl-step"><label class="owo-pl-label">任务模板 <span class="hint">仅已安装模板可选；候选需先安装（幂等）</span></label>' +
        '<select id="pl-template">' + templateOptionsHtml(state.catalog, state.selectedTemplate) + "</select>" +
        '<div id="pl-catalog" class="owo-pl-cats">' +
        (state.catalogError
          ? stateBox("failed", "模板目录加载失败：" + state.catalogError + "（目录不可用时仍可按组队模式手动创建）")
          : state.catalogLoaded
            ? catalogHtml(state.catalog, state.installBusy)
            : stateBox("loading", "正在加载内置模板目录…")) +
        "</div></div>" +
        '<div class="owo-pl-step"><label class="owo-pl-label">执行预览 · 角色 / 预算 / 权限</label>' +
        '<div id="pl-preview">' + previewHtml(buildPreview(state, state.catalog), state) + "</div></div>" +
        '<div class="owo-pl-step"><label class="owo-pl-label">开始任务</label>' +
        '<div class="owo-ws-inline"><button id="pl-create" data-core-action data-core-busy="' + (state.creating ? "true" : "false") + '" class="primary"' + (state.creating ? ' disabled aria-busy="true"' : "") + ">创建团队并开始执行</button></div>" +
        '<div id="pl-errors" aria-live="polite"></div>' +
        '<div id="pl-result">' + resultHtml(state.result) + '</div></div><div class="owo-pl-wizard-actions"><button type="button" id="pl-prev">← 上一步</button><button type="button" id="pl-next" class="primary">下一步 →</button></div>' +
        "</div>"
      );
    }

    function wizardStage(index) { return index === 0 ? 1 : index < 5 ? 2 : 3; }
    function syncWizard() {
      if (!rootEl) return;
      Array.from(rootEl.querySelectorAll(".owo-pl-step")).forEach((section, index) => { section.hidden = wizardStage(index) !== state.wizardStep; });
      rootEl.querySelectorAll("[data-pl-wizard]").forEach(button => {
        if (Number(button.getAttribute("data-pl-wizard")) === state.wizardStep) button.setAttribute("aria-current", "step");
        else button.removeAttribute("aria-current");
      });
      const back = el("#pl-prev"), next = el("#pl-next");
      if (back) back.hidden = state.wizardStep === 1;
      if (next) next.hidden = state.wizardStep === 3;
    }
    function moveWizard(next) {
      syncStateFromDom();
      let message = "";
      if (next > state.wizardStep && !String(state.objective || "").trim()) message = "先描述任务目标，再选择执行范围。";
      if (next === 3 && !String(state.root || "").trim()) message = "请选择项目目录，再预览任务。";
      const error = el("#pl-wizard-error");
      if (error) { error.textContent = message; error.hidden = !message; }
      if (message) return;
      state.wizardStep = Math.max(1, Math.min(3, next));
      syncWizard(); rootEl.scrollTop = 0;
    }

    function render() {
      if (!rootEl) return;
      rootEl.innerHTML = viewHtml();

      syncWizard();
      rootEl.querySelectorAll("[data-pl-wizard]").forEach(button => button.addEventListener("click", () => moveWizard(Number(button.getAttribute("data-pl-wizard")))));
      const next = el("#pl-next"), back = el("#pl-prev");
      if (next) next.onclick = () => moveWizard(state.wizardStep + 1);
      if (back) back.onclick = () => moveWizard(state.wizardStep - 1);
      // —— 绑定 ——
      var obj = el("#pl-objective");
      if (obj) obj.addEventListener("input", repaintPreview);
      var rootI = el("#pl-root");
      if (rootI) rootI.addEventListener("input", function () {
        treeGeneration++;
        state.treeBusy = false;
        if (/[\\/]|^[A-Za-z]:/.test(rootI.value.trim())) rootI.dataset.workspaceRoot = rootI.value.trim();
        repaintPreview();
      });
      var ro = el("#pl-readonly");
      if (ro)
        ro.addEventListener("change", function () {
          var box = el("#pl-writebox");
          if (box) box.hidden = !ro.checked ? false : true;
          repaintPreview();
        });
      var wp = el("#pl-writepaths");
      if (wp) wp.addEventListener("input", repaintPreview);
      // §8.1 目录树多选：写路径区委托（点击载入/移除行；勾选变更；搜索输入）。
      var wb = el("#pl-writebox");
      if (wb) {
        wb.addEventListener("click", function (ev) {
          var t = ev.target;
          if (t && t.hasAttribute && t.hasAttribute("data-pl-tree-load")) {
            loadTree();
            return;
          }
          if (t && t.hasAttribute && t.hasAttribute("data-pl-tree-reload")) {
            loadTree();
            return;
          }
          var up = t && t.getAttribute && t.getAttribute("data-pl-unpick");
          if (up) unpickPath(up);
        });
        wb.addEventListener("change", function (ev) {
          var t = ev.target;
          if (t && t.classList && t.classList.contains("pl-tree-check")) {
            treeCheckChange(t.getAttribute("data-pl-tree-path"), t.getAttribute("data-pl-tree-type"), !!t.checked);
          }
        });
        wb.addEventListener("input", function (ev) {
          var t = ev.target;
          if (t && t.id === "pl-tree-search") {
            state.treeQuery = t.value || "";
            repaintTreePicker();
            var again = el("#pl-tree-search");
            if (again) again.focus();
          }
        });
      }
      var dep = el("#pl-depth");
      if (dep) dep.addEventListener("change", repaintPreview);
      var depthGroup = el("#pl-depth-group");
      if (depthGroup) depthGroup.addEventListener("click", function (ev) {
        var t = ev.target;
        var v = t && t.getAttribute && t.getAttribute("data-pl-depth");
        if (!v) return;
        var dep2 = el("#pl-depth");
        if (dep2) dep2.value = v;
        var chips = depthGroup.querySelectorAll("[data-pl-depth]");
        for (var i = 0; i < chips.length; i++) {
          if (chips[i].classList) chips[i].classList.toggle("primary", chips[i] === t);
        }
        repaintPreview();
      });
      var st = el("#pl-strategy");
      if (st) st.addEventListener("change", repaintPreview);
      var sel = el("#pl-template");
      if (sel) sel.addEventListener("change", repaintPreview);
      var create = el("#pl-create");
      if (create) create.onclick = doCreate;

      // 目录区委托：安装 / 选用
      var cat = el("#pl-catalog");
      if (cat)
        cat.addEventListener("click", function (ev) {
          var t = ev.target;
          var inst = t && t.getAttribute && t.getAttribute("data-pl-install");
          if (inst) {
            installTemplate(inst);
            return;
          }
          if (t && t.hasAttribute && t.hasAttribute("data-pl-catalog-retry")) {
            loadCatalog();
            return;
          }
          var pick = t && t.getAttribute && t.getAttribute("data-pl-select");
          if (pick) {
            state.selectedTemplate = pick;
            var sel2 = el("#pl-template");
            if (sel2) sel2.value = pick;
            repaintPreview();
          }
        });

      // 已有结果：直达按钮重绑
      var go = el("#pl-goto-team");
      if (go && state.result && state.result.team_id)
        go.onclick = function () {
          gotoTeam(String(state.result.team_id));
        };
    }

    function mount(root, helpers) {
      dispose();
      lifecycleGeneration++;
      rootEl = root;
      H = helpers || {};
      if (!state.root && typeof localStorage !== "undefined") state.root = localStorage.getItem("owo.workspace") || "";
      if (!H.get) H.get = defaultGet;
      if (!H.post) H.post = defaultPost;
      if (!H.esc) H.esc = defaultEsc;
      esc = H.esc;
      render();
      loadCatalog();
    }

    function dispose() {
      lifecycleGeneration++;
      treeGeneration++;
      catalogGeneration++;
      rootEl = null;
      state.treeBusy = false;
    }

    // ---------- 测试挂钩 ----------
    var TEST_API = {
      state: state,
      parseWritePaths: parseWritePaths,
      pathIsSafe: pathIsSafe,
      filterTreeEntries: filterTreeEntries,
      toggleTreePick: toggleTreePick,
      mergePickedIntoRaw: mergePickedIntoRaw,
      treePickerHtml: treePickerHtml,
      loadTree: loadTree,
      loadCatalog: loadCatalog,
      repaintTreePicker: repaintTreePicker,
      treeCheckChange: treeCheckChange,
      unpickPath: unpickPath,
      validateState: validateState,
      buildCreateBody: buildCreateBody,
      previewFromTemplate: previewFromTemplate,
      normTemplate: normTemplate,
      previewFromStrategy: previewFromStrategy,
      permissionsSummary: permissionsSummary,
      rolePermissions: rolePermissions,
      rolePermText: rolePermText,
      buildPreview: buildPreview,
      previewHtml: previewHtml,
      templateOptionsHtml: templateOptionsHtml,
      catalogHtml: catalogHtml,
      resultHtml: resultHtml,
      wizardStage: wizardStage,
      viewHtml: viewHtml,
      gotoTeam: gotoTeam,
      doCreate: doCreate,
      installTemplate: installTemplate,
      syncStateFromDom: syncStateFromDom,
      repaintPreview: repaintPreview,
      getTransport: function () {
        return { get: H.get, post: H.post };
      },
      setTransport: function (t) {
        if (t && t.get) H.get = t.get;
        if (t && t.post) H.post = t.post;
      },
      setRoot: function (r) {
        rootEl = r;
      },
      render: render,
      dispose: dispose,
    };

    return {
      id: ID,
      setObjective: function (text) { state.objective = String(text || ""); state.wizardStep = 1; render(); },
      title: "新建项目任务",
      mount: mount,
      dispose: dispose,
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效、零运行时差异。
if (typeof module !== "undefined" && module.exports) {
  var __plWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __plWin.OwoPanels["project-launcher"];
}
