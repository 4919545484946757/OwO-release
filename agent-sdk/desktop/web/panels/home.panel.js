(function (root) {
  "use strict";
  root.OwoPanels = root.OwoPanels || {};
  let host, H, generation = 0;
  let data = { sessions: [], teams: [], inbox: null, errors: [], loading: false, teamsAvailable: false };
  const labels = { running: "运行中", waiting_approval: "等待审批", awaiting_human: "等待人工", succeeded: "运行结束", failed: "运行失败", cancelled: "已取消", interrupted: "已中断" };
  function projectSessions(sessions, workspace) {
    const canonical = value => {
      let path = String(value || "").split(String.fromCharCode(92)).join("/").toLowerCase();
      while (path.endsWith("/")) path = path.slice(0, -1);
      return path;
    };
    return sessions.filter(item => !item.archived && (!workspace || canonical(item.workspace) === canonical(workspace)));
  }
  function view() {
    const snap = H.snapshot?.() || {}, settings = snap.settings || {}, runtime = settings.runtime || {};
    const pending = data.inbox ? data.inbox.filter(item => !["resolved", "cancelled"].includes(item.status)).length : null;
    const sessionRows = data.sessions.slice(0, 6).map(item => {
      const f = item.facts, status = f ? (f.outcome === "running" ? "运行中" : root.OwoCompletionRecord.summarize(f.lastCompletionRecord)?.status || ({ cancelled: "已取消", failed: "失败", interrupted: "终态未确认", completed: "回答完成 · 未验证" })[f.outcome] || "尚无完成记录") : "正在读取宿主记录";
      return '<button class="home-task-row" data-home-session="' + H.esc(item.id) + '"><strong>' + H.esc(item.title || "未命名会话") + '</strong><span>' + H.esc(status) + '</span><small>' + H.esc(f?.lastExecutionModel ? "实际模型 " + f.lastExecutionModel : "实际模型未知") + '</small></button>';
    }).join("");
    const teamRows = data.teams.slice(0, 6).map(team => '<button class="home-task-row" data-home-team="' + H.esc(team.team_id) + '"><strong>' + H.esc(team.objective || team.team_id) + '</strong><span>' + H.esc(team.interrupted ? "已中断 · 等待恢复" : labels[team.status] || team.status || "未知") + '</span><small>团队运行 · 交付验收请查看证据页</small></button>').join("");
    return '<section data-panel="home" data-layout="custom" class="project-home"><div class="home-intro"><h3>' + H.esc(root.OwoWorkspaceDisplay?.alias(snap.workspace) || "项目工作台") + '</h3><p>从目标到交付，查看每一步的状态和依据。</p></div><div class="home-summary-grid">' +
      '<button class="home-summary-card" data-home-panel="action-center"><span>待我处理</span><strong>' + (pending == null ? "未知" : pending) + '</strong><small>人工结果、评审与失败恢复 →</small></button>' +
      '<button class="home-summary-card" data-home-panel="project-history"><span>团队运行</span><strong>' + (data.teamsAvailable ? data.teams.length : "未知") + '</strong><small>本地全部项目的团队运行 →</small></button>' +
      '<button class="home-summary-card" data-home-panel="evidence"><span>产物与证据</span><strong>查看交付</strong><small>版本、变更与验证记录 →</small></button></div>' +
      '<div class="home-environment"><strong>环境状态</strong><span>核心：' + (root.OwoCoreActionAvailability?.isAvailable() ? "已授权连接" : "未连接") + '</span><span>模型：' + H.esc(runtime.model || settings.model || "未配置") + '</span><span>' + (runtime.ready === false ? "模型尚未就绪" : "配置状态不代表远端连通性已验证") + '</span></div>' +
      (data.errors.length ? '<div class="panel-notice" role="status">' + data.errors.map(H.esc).join("；") + '<button data-home-refresh>重试读取</button></div>' : "") +
      '<div class="home-columns"><div class="home-section"><div class="section-toolbar"><h3>最近会话</h3><button data-home-chat>开始对话</button></div>' + (sessionRows || '<div class="panel-empty-state"><strong>从一个明确目标开始</strong><span>在当前项目中定义任务，或开始一次对话。</span><button data-home-panel="project-launcher">新建任务</button></div>') + '</div>' +
      '<div class="home-section"><div class="section-toolbar"><h3>最近团队任务（本地全部项目）</h3><button data-home-refresh>刷新</button></div>' + (teamRows || '<div class="panel-empty-state"><strong>让协作从清晰分工开始</strong><span>当前尚无团队运行。任务预览会列出角色、范围和权限。</span><button data-home-panel="project-launcher">定义一个任务</button></div>') + '</div></div>' +
      '<div class="home-starters"><h3>从一个明确目标开始</h3><div class="home-starter-grid">' + ["整理项目资料并列出来源", "分析实验数据并保存结果", "修复小型代码问题并运行检查"].map(text => '<button data-home-start="' + H.esc(text) + '">' + H.esc(text) + '<small>预填任务草稿，开始前仍需选择范围</small></button>').join("") + '</div></div></section>';
  }
  function render() { if (host) { host.innerHTML = view(); host.setAttribute?.("aria-busy", String(data.loading)); } }
  async function load() {
    const g = ++generation; data.loading = true; data.errors = []; render();
    const snap = H.snapshot?.() || {};
    let sessions, sessionsError = "";
    try { sessions = snap.sessionsKnown ? snap.sessions.map(item => ({...item})) : await H.get("/sessions"); } catch { sessions = []; sessionsError = "会话读取失败"; }
    if (!host || g !== generation) return;
    data.sessions = projectSessions(sessions, snap.workspace); if (sessionsError) data.errors.push(sessionsError);
    const results = await Promise.allSettled([H.get("/teams"), H.get("/human/inbox")]);
    if (!host || g !== generation) return;
    data.teamsAvailable = results[0].status === "fulfilled";
    data.teams = results[0].status === "fulfilled" ? results[0].value.teams || [] : [];
    data.inbox = results[1].status === "fulfilled" ? results[1].value.items || [] : null;
    if (results[0].status !== "fulfilled") data.errors.push("团队状态不可用");
    if (results[1].status !== "fulfilled") data.errors.push("待办状态不可用，未按零项显示");
    data.loading = false; render();
    const jobs = data.sessions.slice(0, 6).map(item => async () => {
      try { const detail = await H.get("/session/" + encodeURIComponent(item.id)); if (g === generation && host) { item.facts = root.OwoSessionFacts.fromDetail(detail); render(); } } catch { if (g === generation && host) { item.facts = { outcome: "interrupted" }; render(); } }
    });
    for (let i = 0; i < jobs.length && g === generation; i += 2) await Promise.all(jobs.slice(i, i + 2).map(run => run()));
  }
  root.OwoPanels.home = { id: "home", title: "项目首页",
    mount(rootEl, helpers) {
      host = rootEl; H = helpers;
      host.addEventListener("click", async event => {
        const button = event.target.closest("button"); if (!button) return;
        if (button.hasAttribute("data-home-refresh")) return load();
        if (button.hasAttribute("data-home-chat")) return H.openChat();
        if (button.dataset.homeSession) return H.openSession(button.dataset.homeSession);
        if (button.dataset.homeTeam) return H.openTeam(button.dataset.homeTeam);
        if (button.dataset.homePanel) return H.openPanel(button.dataset.homePanel);
        if (button.dataset.homeStart) { H.openPanel("project-launcher"); root.OwoPanels["project-launcher"].setObjective?.(button.dataset.homeStart); }
      });
      return load();
    }, refresh: load, dispose() { generation++; host = null; },
    _test: { labels, view, projectSessions }
  };
})(typeof window !== "undefined" ? window : globalThis);
