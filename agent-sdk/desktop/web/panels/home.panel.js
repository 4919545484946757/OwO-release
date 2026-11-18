(function (root) {
  "use strict";
  root.OwoPanels = root.OwoPanels || {};
  let host, H, generation = 0, sessionFilter = "";
  function emptyData() { return { sessions: [], teams: [], inbox: null, errors: [], loading: true, sessionsAvailable: false, teamsAvailable: false }; }
  let data = emptyData();
  const labels = { running: "运行中", waiting_approval: "等待审批", awaiting_human: "等待人工", succeeded: "运行结束", failed: "运行失败", cancelled: "已取消", interrupted: "已中断" };
  const icons = {
    chat: '<svg class="panel-empty-icon" viewBox="0 0 20 20" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 5.5A2.5 2.5 0 0 1 5.5 3h9A2.5 2.5 0 0 1 17 5.5v6a2.5 2.5 0 0 1-2.5 2.5H8l-3.5 3v-3H5.5A2.5 2.5 0 0 1 3 11.5z"/></svg>',
    team: '<svg class="panel-empty-icon" viewBox="0 0 20 20" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="7" cy="6.5" r="2.5"/><path d="M2.5 16v-1a4.5 4.5 0 0 1 9 0v1"/><circle cx="14.5" cy="7.5" r="2"/><path d="M13.5 11.6a3.5 3.5 0 0 1 4 3.4v1"/></svg>'
  };
  function projectSessions(sessions, workspace) {
    const canonical = value => {
      let path = String(value || "").split(String.fromCharCode(92)).join("/").toLowerCase();
      while (path.endsWith("/")) path = path.slice(0, -1);
      return path;
    };
    return sessions.filter(item => item && !item.archived && (!workspace || canonical(item.workspace) === canonical(workspace)));
  }
  function recent(items) {
    const stamp = item => {
      const value = Date.parse(item.updated_at || item.created_at || "");
      return Number.isFinite(value) ? value : 0;
    };
    return items.slice().sort((a, b) => stamp(b) - stamp(a));
  }
  function when(item) {
    const date = new Date(item.updated_at || item.created_at || "");
    return Number.isFinite(date.getTime()) ? date.toLocaleString() : "时间未知";
  }
  function objectiveFromDetail(detail) {
    const direct = detail?.objective || detail?.team?.objective;
    if (typeof direct === "string" && direct.trim()) return direct.trim();
    const tail = Array.isArray(detail?.audit_tail) ? detail.audit_tail : [];
    const start = tail.find(item => /^目标\s+/.test(String(item?.detail || "")));
    if (start) return String(start.detail).replace(/^目标\s+/, "").trim();
    const end = tail.find(item => item?.event === "team.succeeded" && /^目标达成[：:]/.test(String(item.detail || "")));
    return end ? String(end.detail).replace(/^目标达成[：:]\s*/, "").trim() : "";
  }
  function teamTitle(team) {
    return team.objective || (Array.isArray(team.members) ? team.members.map(member => member?.role).filter(Boolean).join("、") : "") || team.team_id;
  }
  function sessionStatus(item) {
    if (item.factsError) return "状态读取失败";
    const f = item.facts;
    if (!f) return "正在读取运行状态";
    if (f.outcome === "running") return "运行中";
    return root.OwoCompletionRecord?.summarize(f.lastCompletionRecord)?.status ||
      ({ cancelled: "已取消", failed: "运行失败", interrupted: "终态未确认", completed: "回答完成 · 未验证" })[f.outcome] || "尚无完成记录";
  }
  function skeletonRows(label) {
    return '<div class="home-skeleton" role="status" aria-label="' + label + '"><div class="panel-skeleton home-skeleton-row"></div><div class="panel-skeleton home-skeleton-row"></div><div class="panel-skeleton home-skeleton-row"></div></div>';
  }
  function placeholder(available, emptyTitle, text, icon) {
    if (!available) return data.loading ? skeletonRows("正在读取") : '<div class="home-list-state" role="status">暂时无法读取，请重试。</div>';
    return '<div class="panel-empty-state">' + (icon || "") + '<strong>' + H.esc(emptyTitle) + '</strong><span>' + H.esc(text) + "</span></div>";
  }
  function filterSessions(sessions, filter, facts) {
    return !filter || !root.OwoTaskWorkbench ? sessions : sessions.filter(item => root.OwoTaskWorkbench.matches(item, filter, facts));
  }
  function sessionFilterControl() {
    const options = [["", "全部会话"], ["all", "Agent 任务"], ["running", "正在运行"], ["waiting", "等待处理"], ["history", "历史任务"], ["chat", "普通对话"]];
    return '<select data-home-session-filter aria-label="筛选项目会话">' + options.map(([value, label]) => '<option value="' + value + '"' + (value === sessionFilter ? ' selected' : '') + '>' + label + '</option>').join("") + '</select>';
  }
  function groupHead(label, count, note, actions) {
    return '<div class="home-group-head"><div class="home-group-title"><span class="panel-eyebrow">' + H.esc(label) + '</span>' +
      (count == null ? "" : '<span class="home-group-count panel-num">' + count + '</span>') +
      (note ? '<small class="home-group-note">' + H.esc(note) + '</small>' : "") +
      '</div>' + (actions ? '<div class="home-group-actions">' + actions + '</div>' : "") + '</div>';
  }
  function view() {
    const snap = H.snapshot?.() || {}, settings = snap.settings || {}, runtime = settings.runtime || {};
    const pending = data.inbox == null ? null : data.inbox.filter(item => !["resolved", "cancelled"].includes(item.status)).length;
    const visibleSessions = filterSessions(data.sessions, sessionFilter, snap.taskFacts);
    const sessionRows = visibleSessions.slice(0, 6).map(item =>
      '<button class="home-task-row" data-home-session="' + H.esc(item.id) + '"><strong>' + H.esc(item.title || "未命名会话") + '</strong><span class="home-task-status">' + H.esc(sessionStatus(item)) + '</span><small>' + H.esc(item.facts?.lastExecutionModel ? "实际模型 " + item.facts.lastExecutionModel : "实际模型未知") + ' · ' + H.esc(when(item)) + "</small></button>").join("");
    const teamRows = data.teams.slice(0, 6).map(team =>
      '<button class="home-task-row" data-home-team="' + H.esc(team.team_id) + '"><strong>' + H.esc(teamTitle(team)) + '</strong><span class="home-task-status" data-status="' + H.esc(team.interrupted ? "interrupted" : team.status) + '">' + H.esc(team.interrupted ? "已中断 · 等待恢复" : labels[team.status] || team.status || "未知") + '</span><small>' + H.esc(team.team_id) + " · " + H.esc(when(team)) + (team.detailError ? " · 目标读取失败" : "") + "</small></button>").join("");
    const refresh = '<button data-home-refresh' + (data.loading ? ' disabled aria-busy="true" title="正在读取最新状态"' : ' title="重新读取会话与团队状态"') + ">" + (data.loading ? "刷新中…" : "刷新") + "</button>";
    const sessionCount = data.sessionsAvailable ? visibleSessions.length : null;
    const teamCount = data.teamsAvailable ? data.teams.length : null;
    return '<section data-panel="home" data-layout="custom" class="project-home"><div class="home-intro"><div><h3>' + H.esc(root.OwoWorkspaceDisplay?.alias(snap.workspace) || "项目工作台") + '</h3><p>从目标到交付，查看每一步的状态和依据。</p></div></div>' +
      '<div class="home-group home-group-overview">' + groupHead("概览", null, null, "") +
      '<div class="home-summary-grid">' +
      '<button class="home-summary-card" data-home-panel="action-center"><span>待我处理</span><strong>' + (pending == null ? (data.loading ? "读取中" : "不可用") : pending) + '</strong><small>人工结果、评审与失败恢复</small></button>' +
      '<button class="home-summary-card" data-home-panel="project-history"><span>任务记录</span><strong>' + (data.teamsAvailable ? data.teams.length : data.loading ? "读取中" : "不可用") + '</strong><small>本地全部项目的团队运行</small></button>' +
      '<button class="home-summary-card" data-home-panel="evidence"><span>产物与证据</span><strong>查看交付</strong><small>版本、变更与验证记录</small></button></div>' +
      '<div class="home-environment"><strong>环境状态</strong><span>核心：' + (root.OwoCoreActionAvailability?.isAvailable() ? "已连接" : "未连接") + '</span><span>模型：' + H.esc(runtime.model || settings.model || "未配置") + '</span><span>' + (runtime.ready === false ? "模型尚未就绪" : "连接状态以实际调用结果为准") + "</span></div></div>" +
      (data.errors.length ? '<div class="panel-notice" role="status">' + data.errors.map(H.esc).join("；") + '<button data-home-refresh' + (data.loading ? " disabled" : "") + ">重试读取</button></div>" : "") +
      '<div class="home-columns"><div class="home-group">' + groupHead("最近会话", sessionCount, snap.workspace ? "当前项目" : "本地全部项目", sessionFilterControl() + '<button class="primary" data-home-chat>开始对话</button>') +
      (sessionRows || placeholder(data.sessionsAvailable, sessionFilter ? "没有匹配的会话" : "从一次对话开始", sessionFilter ? "试试其他分类，或清除筛选条件。" : "当前项目还没有会话，从一次对话开始。", icons.chat)) + '</div>' +
      '<div class="home-group">' + groupHead("最近团队任务", teamCount, "本地全部项目 · 运行结束不代表交付已验收", refresh + '<button class="primary" data-home-team-new>发起团队任务</button>') +
      (teamRows || placeholder(data.teamsAvailable, "还没有团队任务", "定义目标，确认角色和执行范围后开始。", icons.team)) + '</div></div>' +
      '<div class="home-group home-starters">' + groupHead("从一个明确目标开始", null, "预填任务目标，再确认执行范围", "") +
      '<div class="home-starter-grid">' +
      ["整理项目资料并列出来源", "分析实验数据并保存结果", "修复小型代码问题并运行检查"].map(text => '<button data-home-start="' + H.esc(text) + '">' + H.esc(text) + "<small>预填目标后确认执行范围</small></button>").join("") + "</div></div></section>";
  }
  function render() {
    if (!host) return;
    const active = host.ownerDocument?.activeElement;
    const focus = active && host.contains?.(active) ? Array.from(active.attributes || []).find(attr => attr.name.startsWith("data-home-")) : null;
    const scroll = host.scrollTop;
    host.innerHTML = view();
    host.setAttribute?.("aria-busy", String(data.loading));
    if (focus) Array.from(host.querySelectorAll("button,select")).find(button => button.getAttribute(focus.name) === focus.value)?.focus({ preventScroll: true });
    host.scrollTop = scroll;
  }
  async function load() {
    const g = ++generation, helpers = H;
    const current = () => !!host && g === generation && helpers === H;
    data.loading = true; data.errors = []; render();
    const snap = helpers.snapshot?.() || {};
    const results = await Promise.allSettled([
      Promise.resolve().then(() => snap.sessionsKnown ? snap.sessions : helpers.get("/sessions")).then(value => {
        if (!Array.isArray(value)) throw Error("invalid sessions"); return projectSessions(value, snap.workspace);
      }),
      Promise.resolve().then(() => helpers.get("/teams")).then(value => {
        if (!Array.isArray(value?.teams) || value.teams.some(item => !item?.team_id)) throw Error("invalid teams"); return recent(value.teams);
      }),
      Promise.resolve().then(() => helpers.get("/human/inbox")).then(value => {
        if (!Array.isArray(value?.items) || value.items.some(item => !item || typeof item !== "object")) throw Error("invalid inbox"); return value.items;
      })
    ]);
    if (!current()) return;
    data.sessionsAvailable = results[0].status === "fulfilled";
    data.sessions = data.sessionsAvailable ? recent(results[0].value).map(item => ({ ...item })) : [];
    data.teamsAvailable = results[1].status === "fulfilled";
    data.teams = data.teamsAvailable ? results[1].value.map(item => ({ ...item })) : [];
    data.inbox = results[2].status === "fulfilled" ? results[2].value : null;
    if (!data.sessionsAvailable) data.errors.push("会话读取失败");
    if (!data.teamsAvailable) data.errors.push("团队状态不可用");
    if (data.inbox == null) data.errors.push("待办状态不可用");
    data.loading = false; render();
    const jobs = data.sessions.slice(0, 6).map(item => async () => {
      try {
        const detail = await helpers.get("/session/" + encodeURIComponent(item.id));
        if (current()) item.facts = root.OwoSessionFacts.fromDetail(detail);
      } catch { if (current()) item.factsError = true; }
    }).concat(data.teams.slice(0, 6).map(item => async () => {
      try {
        const detail = await helpers.get("/teams/" + encodeURIComponent(item.team_id));
        if (current()) item.objective = objectiveFromDetail(detail) || item.objective;
      } catch { if (current()) item.detailError = true; }
    }));
    for (let i = 0; i < jobs.length && current(); i += 2) {
      await Promise.all(jobs.slice(i, i + 2).map(run => run()));
      if (current()) render();
    }
  }
  root.OwoPanels.home = { id: "home", title: "项目首页",
    mount(rootEl, helpers) {
      generation++; host = rootEl; H = helpers; data = emptyData(); sessionFilter = "";
      const mountedHost = rootEl, mountedHelpers = helpers;
      host.onclick = event => {
        if (host !== mountedHost || H !== mountedHelpers) return;
        const button = event.target.closest("button"); if (!button || !host.contains(button)) return;
        if (button.hasAttribute("data-home-refresh")) return load();
        if (button.hasAttribute("data-home-chat")) return H.openChat();
        if (button.hasAttribute("data-home-team-new")) return H.openPanel("project-launcher");
        if (button.dataset.homeSession) return H.openSession(button.dataset.homeSession);
        if (button.dataset.homeTeam) return H.openTeam(button.dataset.homeTeam);
        if (button.dataset.homePanel) return H.openPanel(button.dataset.homePanel);
        if (button.dataset.homeStart) { H.openPanel("project-launcher"); root.OwoPanels["project-launcher"].setObjective?.(button.dataset.homeStart); }
      };
      host.onchange = event => {
        if (host !== mountedHost || H !== mountedHelpers || !event.target.matches?.("[data-home-session-filter]")) return;
        sessionFilter = event.target.value;
        render();
      };
      return load();
    }, refresh: load, dispose() { generation++; if (host) { host.onclick = null; host.onchange = null; } host = null; },
    _test: { labels, view, projectSessions, recent, sessionStatus, objectiveFromDetail, filterSessions }
  };
})(typeof window !== "undefined" ? window : globalThis);