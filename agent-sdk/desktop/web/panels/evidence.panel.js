(function (root) {
  "use strict";
  root.OwoPanels = root.OwoPanels || {};
  let host, H, gen = 0;
  let state = { mode: "project", projects: [], sessions: [], selected: "", manifest: [], detail: null, content: null, session: null, diffs: [], error: "", busy: false };
  const keys = new Map();
  function validationText(value) { return !value ? "格式校验未返回" : value.valid === true ? "格式校验通过" : "格式校验失败：" + (value.reason || "原因未返回"); }
  function view() {
    const esc = H.esc, options = state.mode === "project" ? state.projects : state.sessions.map(s => ({ id: s.id, title: s.title || s.id }));
    const selector = '<label>数据来源<select data-evidence-mode><option value="project"' + (state.mode === "project" ? " selected" : "") + '>团队交付</option><option value="session"' + (state.mode === "session" ? " selected" : "") + '>会话变更与验收</option></select></label><label>项目或会话<select data-evidence-select><option value="">选择…</option>' + options.map(p => '<option value="' + esc(p.id) + '"' + (p.id === state.selected ? " selected" : "") + '>' + esc(p.title) + '</option>').join("") + '</select></label><button data-evidence-refresh>刷新</button>';
    let body;
    if (state.mode === "session") {
      const facts = state.session ? root.OwoSessionFacts.fromDetail(state.session) : null;
      const record = root.OwoCompletionRecord.summarize(facts?.lastCompletionRecord);
      body = '<div class="evidence-session"><h3>宿主完成记录</h3><p>' + esc(record?.compact || "没有可用的宿主完成记录，不能据此认定任务通过验收") + '</p><p>实际模型：' + esc(facts?.lastExecutionModel || "未知") + '</p>' +
        (record ? '<dl>' + record.details.map(([key, value]) => '<dt>' + esc(key) + '</dt><dd>' + esc(value) + '</dd>').join("") + '</dl>' : "") +
        '<h3>文件变更</h3>' + (state.diffs.length ? state.diffs.map(diff => '<details class="evidence-diff"><summary>' + esc(diff.path) + '</summary><div class="diff-columns"><div><strong>改动前</strong><pre>' + esc(String(diff.before ?? "").slice(0, 131072)) + '</pre></div><div><strong>改动后</strong><pre>' + esc(String(diff.after ?? "").slice(0, 131072)) + '</pre></div></div></details>').join("") : '<div class="panel-empty-state"><strong>未记录文件变更</strong><span>这不代表任务已经验收，请同时查看宿主完成记录。</span></div>') +
        '<div class="evidence-actions"><button data-evidence-audit>查看会话与执行记录</button><button data-evidence-export>导出证据摘要</button></div></div>';
    } else {
      const meta = state.detail?.metadata, history = state.detail?.history;
      const list = state.manifest.map(a => '<button class="evidence-artifact-row" data-artifact="' + esc(a.artifact_id) + '" aria-current="' + (state.detail?.metadata?.artifact_id === a.artifact_id) + '"><strong>' + esc(a.file_name || a.kind || a.artifact_id) + '</strong><span>v' + esc(a.version) + ' · ' + (a.approved ? "已人工批准" : "待评审") + '</span><small>' + esc(validationText(a.validation)) + '</small></button>').join("");
      const detail = meta ? '<div class="evidence-detail"><h3>' + esc(meta.file_name || meta.kind || meta.artifact_id) + '</h3><dl>' + [
        ["版本", meta.version], ["任务 / 尝试", [meta.task_id, meta.attempt_id].filter(Boolean).join(" / ") || "旧产物未标注"],
        ["SHA-256", meta.sha256 || "未返回"], ["字节数", meta.size_bytes ?? "未知"], ["人工评审", meta.review_state || "未返回"],
        ["格式校验", validationText(meta.validation)],
      ].map(([key, value]) => '<dt>' + esc(key) + '</dt><dd>' + esc(value) + '</dd>').join("") + '</dl><h4>证据引用</h4>' +
        ((meta.evidence_refs || []).length ? '<ul>' + meta.evidence_refs.map(ref => '<li><code>' + esc(typeof ref === "string" ? ref : JSON.stringify(ref)) + '</code></li>').join("") + '</ul>' : '<p>没有证据引用；格式正确不等同于功能检查通过。</p>') +
        '<h4>未解决问题</h4><p>' + esc((meta.open_issues || []).join("；") || "宿主未返回未解决问题") + '</p><div class="evidence-actions"><button data-artifact-preview>查看正文</button><button data-artifact-download>下载原文件</button><button data-evidence-export>导出证据摘要</button></div>' +
        (state.content != null ? '<div class="evidence-preview">' + (meta.format === "markdown" ? H.renderMarkdown(state.content.replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1（图片未自动加载）")) : '<pre>' + esc(state.content) + '</pre>') + '</div>' : "") +
        '<details><summary>版本与评审记录</summary><pre>' + esc(JSON.stringify(history || {}, null, 2).slice(0, 131072)) + '</pre></details>' +
        '<div class="evidence-review"><h4>人工评审</h4><label>评审者标识<input data-reviewer value="human" aria-label="人工评审者标识"></label><label>评审说明<textarea data-review-note rows="2"></textarea></label><div class="evidence-actions"><button data-review="approve" data-core-action>批准此版本</button><button data-review="request_changes" data-core-action>要求修改</button><button data-review="reject" data-core-action>拒绝</button></div><p>提交评审针对当前版本；版本变化后请重新核对，最终授权由宿主检查。</p></div></div>' : '<div class="panel-empty-state"><strong>选择交付物查看证据</strong><span>可核对版本、校验、来源和人工评审记录。</span></div>';
      body = '<div class="evidence-columns"><div class="evidence-artifacts">' + (list || '<div class="panel-empty-state"><strong>尚无交付产物</strong><span>可以切换到会话查看文件变更，或返回任务定义交付目标。</span><button data-evidence-new>新建任务</button></div>') + '</div>' + detail + '</div>';
    }
    return '<section data-panel="evidence" data-layout="custom" class="evidence-page"><div class="evidence-toolbar">' + selector + '</div>' +
      (state.error ? '<div class="panel-notice" role="alert">' + esc(state.error) + '</div>' : "") +
      (state.busy ? '<p role="status">正在读取宿主数据…</p>' : "") + body + '</section>';
  }
  function render() { if (host) { host.innerHTML = view(); host.setAttribute?.("aria-busy", String(state.busy)); root.OwoCoreActionAvailability?.update(root.OwoCoreActionAvailability.isAvailable(), host); } }
  async function load() {
    const g = ++gen; state.busy = true; state.error = ""; state.detail = null; state.content = null; state.session = null; state.diffs = []; render();
    try {
      if (!state.selected) { state.manifest = []; state.session = null; state.diffs = []; }
      else if (state.mode === "project") {
        const data = await H.get("/projects/" + encodeURIComponent(state.selected) + "/delivery-manifest");
        if (g !== gen || !host) return; state.manifest = data.manifest || [];
      } else {
        const [detail, diff] = await Promise.all([H.get("/session/" + encodeURIComponent(state.selected)), H.get("/session/" + encodeURIComponent(state.selected) + "/diff")]);
        if (g !== gen || !host) return; state.session = detail; state.diffs = Array.isArray(diff) ? diff : diff.diffs || diff.entries || [];
      }
    } catch (error) { if (g === gen && host) state.error = "读取失败：" + H.friendlyError(error); }
    if (g === gen && host) { state.busy = false; render(); }
  }
  async function openArtifact(id) {
    const g = ++gen; state.busy = true; state.content = null; state.detail = null; state.error = ""; render();
    try {
      const [metadata, history] = await Promise.all([H.get("/artifacts/" + encodeURIComponent(id) + "/metadata"), H.get("/artifacts/" + encodeURIComponent(id) + "/history")]);
      if (g !== gen || !host) return; state.detail = { metadata, history }; state.error = "";
    } catch (error) { if (g === gen && host) state.error = "产物详情不可用：" + H.friendlyError(error); }
    if (g === gen && host) { state.busy = false; render(); }
  }
  function exportEvidence() {
    const value = { project_or_session: state.selected, source: "shared_daemon", completion: state.session?.turn_history || null, artifacts: state.detail || state.manifest, generated_at: new Date().toISOString() };
    const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob), link = document.createElement("a"); link.href = url; link.download = "evidence-summary.json"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async function review(decision) {
    if (state.busy) return;
    const metadata = state.detail?.metadata;
    if (!metadata?.team_id || !metadata.version) { state.error = "缺少团队或版本身份，无法提交评审"; render(); return; }
    const reviewer = host.querySelector("[data-reviewer]").value.trim(), comment = host.querySelector("[data-review-note]").value;
    if (!reviewer) { state.error = "请填写人工评审者标识"; render(); return; }
    const sourceGen = gen;
    if (!await H.confirm("确认提交对版本 " + metadata.version + " 的评审？实际授权以服务端判定为准。")) return;
    if (sourceGen !== gen || !host || state.busy || state.detail?.metadata !== metadata) return;
    const g = gen, intent = JSON.stringify([metadata.artifact_id, metadata.version, decision, reviewer, comment]);
    if (!keys.has(intent)) keys.set(intent, crypto.randomUUID());
    state.busy = true; render();
    try {
      await H.post("/artifacts/" + encodeURIComponent(metadata.artifact_id) + "/review", {
        team_id: metadata.team_id, decision, reviewer, comment, expected_version: metadata.version, idempotency_key: keys.get(intent),
      });
      if (g === gen && host) await openArtifact(metadata.artifact_id);
    } catch (error) { if (g === gen && host) { state.error = "评审未提交成功，请刷新版本或检查权限：" + H.friendlyError(error); state.busy = false; render(); } }
  }
  root.OwoPanels.evidence = { id: "evidence", title: "产物与证据", async mount(rootEl, helpers) {
    host = rootEl; H = helpers; state.sessions = H.snapshot?.().sessions || [];
    host.addEventListener("change", event => {
      if (event.target.hasAttribute("data-evidence-mode")) { state.mode = event.target.value; state.selected = ""; load(); }
      else if (event.target.hasAttribute("data-evidence-select")) { state.selected = event.target.value; load(); }
    });
    host.addEventListener("click", async event => {
      const button = event.target.closest("button"); if (!button) return;
      if (button.dataset.artifact) return openArtifact(button.dataset.artifact);
      if (button.hasAttribute("data-evidence-refresh")) return load();
      if (button.hasAttribute("data-evidence-new")) return H.openPanel("project-launcher");
      if (button.hasAttribute("data-evidence-audit")) return H.openSession(state.selected);
      if (button.hasAttribute("data-evidence-export")) return exportEvidence();
      if (button.dataset.review) return review(button.dataset.review);
      const metadata = state.detail?.metadata;
      if (button.hasAttribute("data-artifact-download") && metadata) {
        try { await H.downloadArtifact(metadata.artifact_id); H.notify?.("已发起下载，请查看保存窗口或下载目录"); }
        catch (error) { state.error = "下载未开始：" + H.friendlyError(error); render(); } return;
      }
      if (button.hasAttribute("data-artifact-preview") && metadata) {
        if (!Number.isSafeInteger(metadata.size_bytes) || metadata.size_bytes < 0 || metadata.size_bytes > 262144) { state.error = "正文大小未确认或超过预览限制，请下载原文件"; render(); return; }
        const g = gen;
        try { const data = await H.get("/artifacts/" + encodeURIComponent(metadata.artifact_id) + "/content"); if (g === gen && host) { state.content = String(data.content || "").slice(0, 262144); render(); } }
        catch (error) { if (g === gen && host) { state.error = H.friendlyError(error); render(); } }
      }
    });
    const g = ++gen; render();
    try {
      const data = await H.get("/teams"); if (g !== gen || !host) return;
      const projects = new Map();
      for (const team of data.teams || []) if (team.project_space_id) projects.set(team.project_space_id, { id: team.project_space_id, title: team.objective || team.project_space_id });
      state.projects = [...projects.values()];
      if (!state.projects.length && state.sessions.length) state.mode = "session";
      state.selected = state.selected || (state.mode === "project" ? state.projects[0]?.id : state.sessions[0]?.id) || ""; await load();
    } catch (error) { if (g === gen && host) { state.error = "项目目录不可用：" + H.friendlyError(error); render(); } }
  }, openSession(id) { state.mode = "session"; state.selected = id; return load(); }, refresh: load, dispose() { gen++; host = null; },
  _test: { validationText, view, state, load, openArtifact }
  };
})(typeof window !== "undefined" ? window : globalThis);
