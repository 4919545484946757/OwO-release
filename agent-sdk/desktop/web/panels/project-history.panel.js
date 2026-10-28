// ============================================================================
// 项目与运行历史面板 —— desktop/web/panels/project-history.panel.js
//
// 八期第四路：把历史 TeamRun 变成可检索、可复用的工作档案。全部数据来自
// **既有路由**，不新增服务端接口：
//   GET  /teams                        {teams:[TeamRun & {active,interrupted}]}
//                                      （TeamRun 序列化：team_id/mode/status/
//                                       template_id?/project_space_id?/created_at/
//                                       updated_at/budget/strategy_decision?）
//   GET  /teams/{id}                   {team,tasks,interrupted,audit_tail}
//                                      （复跑预填：objective 未来 additive 优先，
//                                       回退 audit_tail「目标 …」事件启发式提取）
//   GET  /projects/{pid}/artifacts     {artifacts:[{review_state,...}]}
//   GET  /projects/{pid}/workspace     {workspace:{root,read_only,...}|null}
//                                      （六期冻结；未绑定 404 → 复跑不带 workspace）
//   复用目标仅预填新建任务页；用户确认角色、模型、工作区后才创建任务。
//
// 容错原则：所有字段缺省 []/""/null；详情/交付物/工作区请求失败各自降级为空并记
// 错误行，不阻塞列表渲染；workspace 404 视为「未绑定」静默处理。
//
// Node 兼容：globalThis 回退 + module.exports 导出 _test 纯逻辑挂钩（浏览器零差异）。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels["project-history"] = (function () {
    var ID = "project-history";
    var MAX_ROWS = 50;       // 过滤后列表渲染上限
    var MAX_DETAIL = 12;     // 团队详情（任务数/objective）并发上限
    var MAX_PROJECTS = 8;    // 项目交付物并发上限

    // ---------- helpers（优先 app.js 注入，缺失时自建回退，与 action-center 同款） ----------
    var H = {};
    var rootEl = null;
    function defaultGet(path) {
      return window.OwoApi.get(path);
    }

    function defaultEsc(s) {
      return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
        return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
      });
    }

    function defaultFriendlyError(e) {
      var msg = String((e && e.message) || e || "");
      if (/^404/.test(msg)) return "资源不存在（404）。";
      if (/^400/.test(msg)) return "请求被拒绝（400）：" + msg.slice(5, 160);
      if (/^401|^403/.test(msg)) return "无权访问（401/403）。请检查凭据与权限范围。";
      if (/^409/.test(msg)) return "状态冲突（409）：当前状态不允许该操作。" + msg.slice(4, 160);
      if (/Failed to fetch|NetworkError/i.test(msg)) return "网络错误：无法连接服务端，请确认服务已启动。";
      return msg || "未知错误";
    }

    function el(sel) {
      if (!rootEl) return null;
      return rootEl.querySelector(String(sel || ""));
    }

    function esc(s) {
      return (H.esc || defaultEsc)(s);
    }

    function friendly(e) {
      return (H.friendlyError && H.friendlyError(e)) || defaultFriendlyError(e);
    }

    function isNotFound(e) {
      return !!(e && Number(e.status) === 404) || /\b404\b/.test(String((e && e.message) || e || ""));
    }

    // ---------- 状态 ----------
    var state = {
      loading: false,
      loadedOnce: false,
      teams: [],           // GET /teams 原始列表
      details: {},         // team_id -> 详情 payload（失败不写入）
      detailErrors: {},    // team_id -> 错误
      deliverables: {},    // pid -> { approved, pending, error }（404 → 空且 error=""）
      workspaces: {},      // pid -> 绑定 payload（仅明确 404 才记为 null）
      workspaceErrors: {}, // pid -> 读取失败；未知状态不能等同于未绑定
      detailRequests: {},
      deliverableRequests: {},
      workspaceRequests: {},
      filterEnrichTimer: null,
      filters: { status: "", template: "", project: "", q: "" },
      rowLimit: MAX_ROWS,
      rerunBusy: {},       // team_id -> true
      rerunResults: {},    // team_id -> { ok, text, new_team_id? }
      errors: [],
      lastLoadedAt: "",
      hostScope: "",
    };

    var enrichmentGeneration = 0;
    var loadGeneration = 0;
    var lifecycleGeneration = 0;

    function invalidateEnrichment() {
      enrichmentGeneration += 1;
      state.details = {};
      state.detailErrors = {};
      state.deliverables = {};
      state.workspaces = {};
      state.workspaceErrors = {};
      state.detailRequests = {};
      state.deliverableRequests = {};
      state.workspaceRequests = {};
    }

    function owns(obj, key) {
      return Object.prototype.hasOwnProperty.call(obj || {}, key);
    }

    // ---------- 纯逻辑层（Node 测试挂钩覆盖） ----------
    // 团队状态归一：列表是 snake_case（serde），创建响应/SSE 是 Debug 形式——统一小写
    // 归一化（与 action-center/workswarm 同款）。
    function normStatus(s) {
      var x = String(s == null ? "" : s).toLowerCase();
      if (x === "awaitinghuman") return "awaiting_human";
      return x;
    }

    var STATUS_OPTIONS = [
      { value: "", label: "全部状态" },
      { value: "running", label: "运行中" },
      { value: "awaiting_human", label: "等待人工" },
      { value: "created", label: "已创建" },
      { value: "succeeded", label: "已完成" },
      { value: "failed", label: "失败" },
      { value: "cancelled", label: "已取消" },
    ];

    var STATUS_CN = {
      created: "已创建",
      running: "运行中",
      awaiting_human: "等待人工",
      succeeded: "已完成",
      failed: "失败",
      cancelled: "已取消",
    };

    // 耗时：created_at/updated_at（ISO rfc3339）→ "1m 24s" 人类可读；缺失/不可解析 → ""。
    function durationText(createdAt, updatedAt) {
      var a = Date.parse(String(createdAt == null ? "" : createdAt));
      var b = Date.parse(String(updatedAt == null ? "" : updatedAt));
      if (!isFinite(a) || !isFinite(b) || b < a) return "";
      var s = Math.round((b - a) / 1000);
      if (s < 60) return s + "s";
      var m = Math.floor(s / 60);
      var rest = s % 60;
      if (m < 60) return m + "m " + rest + "s";
      var h = Math.floor(m / 60);
      return h + "h " + (m % 60) + "m";
    }

    // 计划调用预算：strategy_decision.budget_calls_total（八期一路 additive 容错）。
    function plannedCalls(team) {
      var sd = team && team.strategy_decision;
      var n = sd && sd.budget_calls_total;
      return typeof n === "number" && n > 0 ? n : null;
    }

    // 任务计数（详情侧）：tasks 视图按状态归类；详情缺失 → null。
    function taskCounts(detail) {
      var tasks = (detail && detail.tasks) || [];
      if (!tasks.length) return null;
      var done = 0;
      var failed = 0;
      tasks.forEach(function (t) {
        var st = normStatus(t && t.status);
        if (st === "succeeded") done += 1;
        if (st === "failed" || st === "aborted") failed += 1;
      });
      return { total: tasks.length, done: done, failed: failed };
    }

    // 过滤选项：模板（template_id 缺失 = 动态组队）、项目（project_space_id 缺失按
    // "proj-"+team_id 派生，与 action-center 同款）。返回 [{value,label}]，value=""
    // 表示不过滤。
    function optionList(teams, keyOf, labelOf) {
      var seen = Object.create(null);
      var out = [];
      (teams || []).forEach(function (t) {
        if (!t) return;
        var v = keyOf(t);
        if (v == null || seen[v]) return;
        seen[v] = true;
        out.push({ value: v, label: labelOf ? labelOf(t, v) : v });
      });
      return out;
    }

    function projectKeyOf(team) {
      if (!team) return "";
      return team.project_space_id || "proj-" + team.team_id;
    }

    function templateOptions(teams) {
      var opts = optionList(teams, function (t) {
        return t.template_id ? String(t.template_id) : "__owo_dynamic__";
      }, function (t, v) {
        return v === "__owo_dynamic__" ? "动态组队（无模板）" : v;
      });
      opts.sort(function (a, b) {
        if (a.value === "__owo_dynamic__") return -1;
        if (b.value === "__owo_dynamic__") return 1;
        return a.label < b.label ? -1 : (a.label > b.label ? 1 : 0);
      });
      return opts;
    }

    function projectOptions(teams) {
      var opts = optionList(teams, projectKeyOf, function (t, v) { return v; });
      opts.sort(function (a, b) { return a.value.localeCompare(b.value); });
      return opts;
    }

    // 过滤判定：status 归一化相等（空 = 全部）、template 精确（"" = 动态组队桶）、
    // project 精确、q 大小写不敏感匹配 team_id。
    function matchesFilters(team, f) {
      if (!team || !team.team_id) return false;
      f = f || {};
      if (f.status && normStatus(team.status) !== f.status) return false;
      if (f.template === "__owo_dynamic__") { if (team.template_id) return false; }
      else if (f.template && String(team.template_id || "") !== f.template) return false;
      if (f.project && projectKeyOf(team) !== f.project) return false;
      if (f.q) {
        var q = String(f.q).trim().toLocaleLowerCase();
        var known = objectiveFromDetail(state.details[String(team.team_id)] || {}) || team.objective || "";
        if ([team.team_id, projectKeyOf(team), team.template_id, known, (team.members || []).map(function (m) { return m.role || ""; }).join(" ")].join(" ").toLocaleLowerCase().indexOf(q) < 0) return false;
      }
      return true;
    }

    // 可见列表：过滤 + created_at 降序（历史视角最新在前）+ 截断。
    function visibleTeams(teams, f, cap) {
      var list = (teams || []).filter(function (t) { return matchesFilters(t, f); });
      list.sort(function (a, b) {
        return String(b.created_at || "").localeCompare(String(a.created_at || ""));
      });
      return list.slice(0, cap == null ? MAX_ROWS : cap);
    }

    // 复跑目标预填：detail.objective（未来 additive，优先）→ audit_tail 中最早一条
    // 「目标 …」事件 → null（用户手工输入）。audit_tail 为最新在前，取最后一条匹配。
    function objectiveFromDetail(detail) {
      if (!detail) return null;
      var direct = detail.objective || (detail.team && detail.team.objective);
      if (typeof direct === "string" && direct.trim()) return direct.trim();
      var tail = detail.audit_tail || [];
      var found = null;
      var completed = null;
      for (var i = 0; i < tail.length; i++) {
        var d = String((tail[i] && tail[i].detail) || "");
        if (/^目标\s+/.test(d)) found = d.replace(/^目标\s+/, "").trim();
        if (tail[i] && tail[i].event === "team.succeeded" && /^目标达成[：:]/.test(d)) completed = d.replace(/^目标达成[：:]\s*/, "").trim();
      }
      return found || completed || null;
    }

    function localWhen(value) {
      var d = new Date(value);
      return isFinite(d.getTime()) ? d.toLocaleString() : "时间未知";
    }

    // ---------- 渲染（纯 HTML 构建器） ----------
    function statusBadge(status) {
      var st = normStatus(status);
      var cn = STATUS_CN[st] || (st || "未知");
      return '<span class="ph-st ph-st-' + esc(st || "unknown") + '">' + esc(cn) + "</span>";
    }

    function rowHtml(team, extras) {
      extras = extras || {};
      var tid = String(team.team_id || "");
      var st = state.rerunResults[tid];
      var busy = state.rerunBusy[tid];
      var calls = plannedCalls(team);
      var dur = durationText(team.created_at, team.updated_at);
      var tc = taskCounts(extras.detail);
      var dlv = extras.deliverables;
      var dlvText = dlv
        ? "批准 " + (dlv.approved || 0) + " · 待评审 " + (dlv.pending || 0)
        : "";
      return (
        '<div class="ph-row" data-ph-row="' + esc(tid) + '">' +
        '<div class="ph-row-head">' +
        '<h3 class="ph-title">' + esc(extras.objective || team.objective || (team.members || []).map(function (m) { return m.role || ""; }).filter(Boolean).join("、") || tid) + "</h3>" +
        statusBadge(team.status) +
        '<span class="ph-mode">' + esc(String(team.mode || "").toLowerCase() || "team") + "</span>" +
        (team.template_id
          ? '<span class="ph-tpl">' + esc(String(team.template_id)) + "</span>"
          : '<span class="ph-tpl ph-tpl-none">动态组队</span>') +
        "</div>" +
        '<div class="ph-source hint">' + esc(tid) + " · " + esc(projectKeyOf(team)) + "</div>" +
        '<div class="ph-row-meta hint">' +
        "<span>创建 " + esc(localWhen(team.created_at)) + "</span>" +
        (dur ? "<span>运行跨度 " + esc(dur) + "</span>" : "") +
        (calls != null ? "<span>预算 " + calls + " 次调用</span>" : "") +
        (tc ? "<span>任务 " + tc.done + "/" + tc.total + (tc.failed ? "（失败 " + tc.failed + "）" : "") + "</span>" : "") +
        (dlvText ? "<span>交付物：" + esc(dlvText) + "</span>" : "") +
        "</div>" +
        '<div class="ph-row-actions">' +
        '<button type="button" class="owo-ac-mini" data-ph-open="' + esc(tid) + '">打开详情</button>' +
        '<button type="button" class="owo-ac-mini ph-rerun" data-ph-rerun="' + esc(tid) + '"' +
        (busy ? " disabled" : "") + ">复用目标</button>" +
        "</div>" +
        '<div class="ph-result" data-ph-result="' + esc(tid) + '">' +
        (st ? esc(st.text) : "") +
        "</div>" +
        "</div>"
      );
    }

    function listHtml(teams, st) {
      st = st || state;
      var rows = visibleTeams(teams, st.filters, st.rowLimit || MAX_ROWS);
      if (!rows.length) {
        return '<div class="ph-empty">没有匹配的历史运行（调整过滤条件或刷新）。</div>';
      }
      return rows
        .map(function (t) {
          var tid = String(t.team_id || "");
          return rowHtml(t, {
            detail: st.details[tid],
            deliverables: (function () {
              var d = st.deliverables[projectKeyOf(t)];
              return d && !d.error ? d : null;
            })(),
            workspace: st.workspaces[projectKeyOf(t)],
            workspaceError: st.workspaceErrors[projectKeyOf(t)],
            objective: (function () {
              var d = st.details[tid];
              return d ? objectiveFromDetail(d) : null;
            })(),
          });
        })
        .join("") + (visibleTeams(teams, st.filters, Number.MAX_SAFE_INTEGER).length > rows.length
          ? '<button type="button" class="owo-ac-mini ph-more" data-ph-more>显示更多任务</button>' : "");
    }

    function filtersHtml(st) {
      st = st || state;
      var f = st.filters;
      function opt(list, cur) {
        return list.map(function (o) {
          return '<option value="' + esc(o.value) + '"' + (o.value === cur ? " selected" : "") + ">" +
            esc(o.label) + "</option>";
        }).join("");
      }
      return (
        '<div class="ph-filters">' +
        '<input id="ph-f-q" type="search" aria-label="搜索任务、项目或模板" placeholder="搜索任务、项目或模板" value="' + esc(f.q) + '" />' +
        '<select id="ph-f-status" aria-label="按状态过滤">' + opt(STATUS_OPTIONS, f.status) + "</select>" +
        '<select id="ph-f-template" aria-label="按模板过滤">' +
        '<option value="">全部模板</option>' + opt(templateOptions(st.teams), f.template) + "</select>" +
        '<select id="ph-f-project" aria-label="按项目过滤">' +
        '<option value="">全部项目</option>' + opt(projectOptions(st.teams), f.project) + "</select>" +
        "</div>"
      );
    }

    function shellHtml() {
      return (
        '<div class="ph">' +
        '<div class="ph-head">' +
        '<span class="hint">查找历史任务，查看结果，复用任务目标。</span>' +
        '<button type="button" id="ph-refresh" class="owo-ac-mini">刷新</button>' +
        '<span id="ph-meta" class="hint" aria-live="polite"></span>' +
        "</div>" +
        '<div id="ph-errors" aria-live="polite"></div>' +
        '<div id="ph-filters-box"></div>' +
        '<div id="ph-list"><div class="ph-empty">正在加载历史运行…</div></div>' +
        '<p class="ph-reuse-note hint">复用目标后，在新建任务中确认角色、模型与工作区。</p>' +
        "</div>"
      );
    }

    // ---------- 行为层 ----------
    function paintMeta(text) {
      var m = el("#ph-meta");
      if (m) m.textContent = text;
    }

    function paintErrors() {
      var box = el("#ph-errors");
      if (!box) return;
      var lines = state.errors.slice();
      Object.keys(state.detailErrors || {}).forEach(function (tid) {
        lines.push("团队 " + tid + " 详情加载失败：" + state.detailErrors[tid]);
      });
      Object.keys(state.deliverables || {}).forEach(function (pid) {
        var d = state.deliverables[pid];
        if (d && d.error) lines.push("项目 " + pid + " 交付物加载失败：" + d.error);
      });
      Object.keys(state.workspaceErrors || {}).forEach(function (pid) {
        lines.push("项目 " + pid + " 工作区读取失败：" + state.workspaceErrors[pid]);
      });
      box.innerHTML = lines.map(function (x) { return '<div class="owo-ac-err">' + esc(x) + "</div>"; }).join("");
    }

    function renderFilters() {
      var box = el("#ph-filters-box");
      if (box) box.innerHTML = filtersHtml(state);
    }

    function render() {
      if (!rootEl) return;
      var box = el("#ph-list");
      if (box) box.innerHTML = listHtml(state.teams, state);
      paintErrors();
      var refresh = el("#ph-refresh");
      if (refresh) { refresh.disabled = state.loading; refresh.textContent = state.loading ? "刷新中…" : "刷新"; }
      var total = (state.teams || []).length;
      var shown = visibleTeams(state.teams, state.filters, state.rowLimit || MAX_ROWS).length;
      paintMeta(
        state.loading
          ? "加载中…"
          : "共 " + total + " 次运行 · 显示 " + shown + " 条" +
            (state.lastLoadedAt ? " · 更新于 " + state.lastLoadedAt : "")
      );
    }

    function paintRerun(tid) {
      var box = el("#ph-list");
      if (!box) return;
      var res = state.rerunResults[tid];
      var div = box.querySelector('[data-ph-result="' + cssEsc(tid) + '"]');
      if (div) {
        div.textContent = res ? res.text : "";
        div.className = "ph-result" + (res && res.ok ? " ok" : "") + (res && !res.ok ? " bad" : "");
      }
      var btn = box.querySelector('[data-ph-rerun="' + cssEsc(tid) + '"]');
      if (btn) {
        btn.disabled = !!state.rerunBusy[tid];
        btn.textContent = state.rerunBusy[tid] ? "读取目标…" : "复用目标";
      }
    }

    function cssEsc(s) {
      return String(s == null ? "" : s).replace(/"/g, '\\"');
    }

    // 详情 + 交付物 + 工作区（可见行的补充信息；按 key 缓存并合并并发请求）。
    function requestDetail(teamId, generation) {
      if (owns(state.details, teamId) || owns(state.detailErrors, teamId)) return Promise.resolve();
      if (state.detailRequests[teamId]) return state.detailRequests[teamId];
      var request = Promise.resolve()
        .then(function () { return H.get("/teams/" + encodeURIComponent(teamId)); })
        .then(
          function (d) {
            if (generation !== enrichmentGeneration) return;
            state.details[teamId] = d;
            delete state.detailErrors[teamId];
          },
          function (e) {
            if (generation === enrichmentGeneration) state.detailErrors[teamId] = friendly(e);
          }
        )
        .then(function () {
          if (generation === enrichmentGeneration && state.detailRequests[teamId] === request) {
            delete state.detailRequests[teamId];
          }
        });
      state.detailRequests[teamId] = request;
      return request;
    }

    function requestDeliverables(pid, generation) {
      if (owns(state.deliverables, pid)) return Promise.resolve();
      if (state.deliverableRequests[pid]) return state.deliverableRequests[pid];
      var request = Promise.resolve()
        .then(function () {
          return H.get("/projects/" + encodeURIComponent(pid) + "/artifacts");
        })
        .then(
          function (d) {
            if (generation !== enrichmentGeneration) return;
            if (!d || !Array.isArray(d.artifacts)) {
              state.deliverables[pid] = { error: "产物列表格式无效，暂无法统计评审状态" };
              return;
            }
            var counts = { approved: 0, pending: 0 };
            d.artifacts.forEach(function (artifact) {
              var review = String(artifact && artifact.review_state || "").toLowerCase().replace(/[_ -]/g, "");
              if (review === "approved") counts.approved++;
              if (review === "pendingreview") counts.pending++;
            });
            state.deliverables[pid] = counts;
          },
          function (e) {
            if (generation !== enrichmentGeneration) return;
            var is404 = /^404/.test(String((e && e.message) || e || ""));
            state.deliverables[pid] = { approved: 0, pending: 0, error: is404 ? "" : friendly(e) };
          }
        )
        .then(function () {
          if (generation === enrichmentGeneration && state.deliverableRequests[pid] === request) {
            delete state.deliverableRequests[pid];
          }
        });
      state.deliverableRequests[pid] = request;
      return request;
    }

    function requestWorkspace(pid, generation) {
      if (owns(state.workspaces, pid) || owns(state.workspaceErrors, pid)) return Promise.resolve();
      if (state.workspaceRequests[pid]) return state.workspaceRequests[pid];
      var request = Promise.resolve()
        .then(function () {
          return H.get("/projects/" + encodeURIComponent(pid) + "/workspace");
        })
        .then(
          function (w) {
            if (generation !== enrichmentGeneration) return;
            if (w && owns(w, "workspace")) {
              if (w.workspace === null) { state.workspaces[pid] = null; return; }
              w = w.workspace;
            }
            if (w && typeof w.root === "string" && w.root.trim()) {
              state.workspaces[pid] = w;
            } else {
              state.workspaceErrors[pid] = "服务端返回的工作区信息缺少有效 root";
            }
          },
          function (e) {
            if (generation !== enrichmentGeneration) return;
            if (isNotFound(e)) {
              // Only an explicit 404 means that this project has no workspace binding.
              state.workspaces[pid] = null;
            } else {
              state.workspaceErrors[pid] = friendly(e);
            }
          }
        )
        .then(function () {
          if (generation === enrichmentGeneration && state.workspaceRequests[pid] === request) {
            delete state.workspaceRequests[pid];
          }
        });
      state.workspaceRequests[pid] = request;
      return request;
    }

    function enrichVisible(rows) {
      var generation = enrichmentGeneration;
      var detailTargets = rows.slice(0, MAX_DETAIL);
      var projects = (function () {
        var seen = Object.create(null);
        var out = [];
        for (var i = 0; i < rows.length && out.length < MAX_PROJECTS; i++) {
          var pid = projectKeyOf(rows[i]);
          if (pid && !seen[pid]) {
            seen[pid] = true;
            out.push(pid);
          }
        }
        return out;
      })();

      var jobs = detailTargets.map(function (t) {
        return requestDetail(t.team_id, generation);
      });
      projects.forEach(function (pid) {
        jobs.push(requestDeliverables(pid, generation));
        jobs.push(requestWorkspace(pid, generation));
      });
      return Promise.all(jobs);
    }

    function load() {
      if (state.loading) return Promise.resolve();
      var requestGeneration = ++loadGeneration;
      if (state.filterEnrichTimer) {
        clearTimeout(state.filterEnrichTimer);
        state.filterEnrichTimer = null;
      }
      state.loading = true;
      state.errors = [];
      render();
      return Promise.resolve()
        .then(function () { return H.get("/teams"); })
        .then(function (d) {
          if (requestGeneration !== loadGeneration) return;
          invalidateEnrichment();
          state.teams = (d && d.teams) || [];
          state.loading = false;
          state.loadedOnce = true;
          state.lastLoadedAt = new Date().toLocaleTimeString();
          renderFilters();
          render();
          var rows = visibleTeams(state.teams, state.filters, state.rowLimit || MAX_ROWS);
          return enrichVisible(rows).then(function () {
            if (requestGeneration !== loadGeneration) return;
            state.lastLoadedAt = new Date().toLocaleTimeString();
            render();
          });
        })
        .catch(function (e) {
          if (requestGeneration !== loadGeneration) return;
          state.errors.push("团队列表加载失败：" + friendly(e));
          state.loading = false;
          state.loadedOnce = true;
          render();
        });
    }

    // Only prefill the goal: runtime role/model/budget settings cannot be reconstructed
    // from TeamRun. The launcher remains the explicit, reviewable creation boundary.
    function startRerun(teamId) {
      var tid = String(teamId || "");
      if (!tid || state.rerunBusy[tid]) return Promise.resolve();
      var generation = lifecycleGeneration, get = H.get, openPanel = H.openPanel;
      state.rerunBusy[tid] = true;
      delete state.rerunResults[tid];
      paintRerun(tid);
      return Promise.resolve(state.details[tid] || null)
        .then(function (detail) {
          if (generation !== lifecycleGeneration) return null;
          return detail || get("/teams/" + encodeURIComponent(tid));
        })
        .then(function (detail) {
          if (generation !== lifecycleGeneration) return;
          var objective = objectiveFromDetail(detail);
          if (!objective) throw new Error("无法自动提取原目标，请打开详情确认任务内容。");
          var launcher = win.OwoPanels && win.OwoPanels["project-launcher"];
          if (!openPanel || !launcher || typeof launcher.setObjective !== "function") {
            throw new Error("新建任务页暂不可用，请刷新页面后重试。");
          }
          openPanel("project-launcher");
          launcher.setObjective(objective);
        })
        .catch(function (e) {
          if (generation === lifecycleGeneration) state.rerunResults[tid] = { ok: false, text: friendly(e) };
        })
        .then(function () {
          delete state.rerunBusy[tid];
          if (generation === lifecycleGeneration) paintRerun(tid);
        });
    }

    function gotoWorkswarm(teamId) {
      var id = String(teamId || "");
      if (!id) return;
      if (H.openTeam) { H.openTeam(id); return; }
      try {
        if (win.OwoPanels && win.OwoPanels.workswarm && win.OwoPanels.workswarm.open) {
          // 侧导航切到 workswarm 面板（app.js 挂载机制），再深链详情。
          var nav = document.querySelector('#panelNav button[data-panel="workswarm"]');
          if (nav) nav.click();
          win.OwoPanels.workswarm.open(id);
          return;
        }
      } catch (_e) { /* 深链失败静默：详情可手工打开 */ }
    }

    function onListClick(ev) {
      var t = ev.target;
      if (!t || !t.closest) return;
      var openBtn = t.closest("[data-ph-open]");
      if (openBtn) {
        gotoWorkswarm(openBtn.getAttribute("data-ph-open") || "");
        return;
      }
      var rerunBtn = t.closest("[data-ph-rerun]");
      if (rerunBtn) {
        startRerun(rerunBtn.getAttribute("data-ph-rerun") || "");
        return;
      }
      var more = t.closest("[data-ph-more]");
      if (more) { state.rowLimit = (state.rowLimit || MAX_ROWS) + MAX_ROWS; render(); }
    }

    function onFilterChange(ev) {
      var t = ev.target;
      if (!t || !t.id) return;
      if (t.id === "ph-f-status") state.filters.status = t.value;
      else if (t.id === "ph-f-template") state.filters.template = t.value;
      else if (t.id === "ph-f-project") state.filters.project = t.value;
      else if (t.id === "ph-f-q") state.filters.q = t.value;
      else return;
      state.rowLimit = MAX_ROWS;
      render();
      if (state.filterEnrichTimer) {
        clearTimeout(state.filterEnrichTimer);
        state.filterEnrichTimer = null;
      }
      if (t.id === "ph-f-q") {
        // 搜索命中本地已加载列表；等待短暂停顿再补充行详情，避免每个按键都打 API。
        state.filterEnrichTimer = setTimeout(function () {
          state.filterEnrichTimer = null;
          enrichVisible(visibleTeams(state.teams, state.filters, state.rowLimit || MAX_ROWS)).then(render);
        }, 180);
        return;
      }
      enrichVisible(visibleTeams(state.teams, state.filters, state.rowLimit || MAX_ROWS)).then(render);
    }

    function dispose() {
      lifecycleGeneration++;
      loadGeneration += 1;
      state.loading = false;
      if (state.filterEnrichTimer) clearTimeout(state.filterEnrichTimer);
      state.filterEnrichTimer = null;
      invalidateEnrichment();
      rootEl = null;
    }

    function mount(root, helpers) {
      lifecycleGeneration++;
      rootEl = root;
      H = helpers || {};
      H.baseUrl = H.baseUrl || (win.OwoPanels && win.OwoPanels.baseUrl) || "";
      if (state.hostScope !== H.baseUrl) {
        state.teams = []; state.errors = []; state.rerunResults = {}; state.loadedOnce = false;
        invalidateEnrichment();
      }
      state.hostScope = H.baseUrl;
      if (!H.get) H.get = defaultGet;
      if (!H.esc) H.esc = defaultEsc;
      root.innerHTML = shellHtml();
      var box = el("#ph-list");
      if (box) box.addEventListener("click", onListClick);
      var fbox = el("#ph-filters-box");
      if (fbox) {
        fbox.addEventListener("change", function (ev) { if (ev.target.id !== "ph-f-q") onFilterChange(ev); });
        fbox.addEventListener("input", function (ev) { if (ev.target.id === "ph-f-q") onFilterChange(ev); });
      }
      var refresh = el("#ph-refresh");
      if (refresh) refresh.onclick = function () { load(); };
      load();
    }

    // ---------- 测试挂钩 ----------
    var TEST_API = {
      state: state,
      normStatus: normStatus,
      durationText: durationText,
      plannedCalls: plannedCalls,
      taskCounts: taskCounts,
      templateOptions: templateOptions,
      projectOptions: projectOptions,
      projectKeyOf: projectKeyOf,
      matchesFilters: matchesFilters,
      visibleTeams: visibleTeams,
      objectiveFromDetail: objectiveFromDetail,
      localWhen: localWhen,
      dispose: dispose,
      onListClick: onListClick,
      setHelpers: function (helpers) { H = helpers; },
      rowHtml: rowHtml,
      listHtml: listHtml,
      filtersHtml: filtersHtml,
      shellHtml: shellHtml,
      render: render,
      load: load,
      enrichVisible: enrichVisible,
      onFilterChange: onFilterChange,
      invalidateEnrichment: invalidateEnrichment,
      startRerun: startRerun,
      gotoWorkswarm: gotoWorkswarm,
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
    };

    return {
      id: ID,
      title: "任务记录",
      mount: mount,
      dispose: dispose,
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效、零运行时差异。
if (typeof module !== "undefined" && module.exports) {
  var __phWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __phWin.OwoPanels["project-history"];
}
