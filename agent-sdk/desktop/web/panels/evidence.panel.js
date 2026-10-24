(function (root) {
  "use strict";
  root.OwoPanels = root.OwoPanels || {};
  let host, H, gen = 0, handlers;
  const state = { mode: "project", projects: [], sessions: [], selected: "", manifest: [], artifactId: "", detail: null, content: null, session: null, diffs: [], error: "", notice: "", busy: false, previewBusy: false, previewError: "", historyError: "", filter: "", tab: "preview" };
  const keys = new Map(), drafts = new Map(), pending = new Set();
  const tabs = [["preview", "正文"], ["evidence", "版本与证据"], ["review", "人工评审"]];
  const normalize = value => String(value || "").toLowerCase().replace(/[_\s-]/g, "");
  function reviewText(value) {
    const labels = { draft: "草稿", pendingreview: "待评审", approved: "已批准", rejected: "已拒绝", superseded: "已被新版本取代" };
    return Object.prototype.hasOwnProperty.call(labels, normalize(value)) ? labels[normalize(value)] : (value ? "未知状态（" + value + "）" : "评审状态未返回");
  }
  function validationText(value) { return !value || typeof value.valid !== "boolean" ? "格式校验未返回" : value.valid ? "格式校验通过" : "格式校验失败：" + (value.reason || "原因未返回"); }
  function draftKey(meta) { return meta ? JSON.stringify([meta.artifact_id, meta.version]) : ""; }
  function draftFor(meta) {
    const key = draftKey(meta);
    if (!drafts.has(key)) drafts.set(key, { reviewer: "human", comment: "" });
    return drafts.get(key);
  }
  function escape(value) { return H.esc(value == null ? "" : String(value)); }
  function icon(name) {
    const paths = { refresh: '<path d="M20 7v5h-5M4 17v-5h5"/><path d="M6.1 7a7 7 0 0 1 11.6-1L20 9M4 15l2.3 3A7 7 0 0 0 17.9 17"/>', search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/>', file: '<path d="M14 3H6a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8z"/><path d="M14 3v5h5"/>', download: '<path d="M12 3v12m-4-4 4 4 4-4M4 16v5h16v-5"/>' };
    return '<svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">' + paths[name] + "</svg>";
  }
  function artifactRows() {
    const q = state.filter.trim().toLocaleLowerCase();
    const list = state.manifest.filter(a => [a.file_name, a.kind, a.artifact_id, reviewText(a.review_state)].join(" ").toLocaleLowerCase().includes(q));
    if (!state.selected) return '<p class="evidence-list-empty">选择项目后查看产物。</p>';
    if (!state.manifest.length) return '<p class="evidence-list-empty">' + (state.busy ? "正在读取产物…" : "此项目尚无产物。") + "</p>";
    if (!list.length) return '<p class="evidence-list-empty" role="status">没有匹配的产物。</p>';
    return list.map(a => '<button type="button" class="evidence-artifact-row" data-artifact="' + escape(a.artifact_id) + '" aria-current="' + (state.artifactId === a.artifact_id) + '">' + icon("file") + '<span><strong>' + escape(displayName(a)) + '</strong><small>v' + escape(a.version) + " · " + escape(reviewText(a.review_state)) + "</small></span></button>").join("");
  }
  function displayName(meta) { return meta.kind && /^artifact\./i.test(meta.file_name || "") ? meta.kind : meta.file_name || meta.kind || meta.artifact_id; }
  function previewHtml(meta) {
    if (state.previewBusy) return '<p role="status" class="hint">正在读取正文…</p>';
    if (state.previewError) return '<div class="panel-notice" role="alert">' + escape(state.previewError) + '<button data-artifact-preview>重试正文预览</button></div>';
    if (state.content == null) return '<div class="panel-empty-state"><strong>正文尚未读取</strong><span>可预览不超过 256 KiB 的正文，也可下载原文件。</span><button data-artifact-preview>读取正文</button></div>';
    if (!state.content) return '<p class="hint">此版本正文为空。</p>';
    return '<div class="evidence-preview">' + (normalize(meta.format) === "markdown" ? H.renderMarkdown(state.content.replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1（图片未自动加载）")) : '<pre>' + escape(state.content) + "</pre>") + "</div>";
  }
  function historyHtml(history) {
    if (state.historyError) return '<div class="panel-notice" role="alert">' + escape(state.historyError) + '<button data-evidence-history>重试评审记录</button></div>';
    const reviews = Array.isArray(history?.reviews) ? history.reviews : [];
    const labels = { approve: "批准", requestchanges: "要求修改", reject: "拒绝" };
    return '<h4>版本与评审记录</h4>' + (history?.superseded_by ? '<p>后续版本：<code>' + escape(history.superseded_by) + "</code></p>" : "") +
      (reviews.length ? '<ol class="evidence-review-history">' + reviews.map(r => '<li><div><strong>' + escape(labels[normalize(r.decision)] || r.decision || "评审") + '</strong><span>' + escape(r.reviewer) + '</span><time datetime="' + escape(r.created_at) + '">' + escape(r.created_at ? new Date(r.created_at).toLocaleString("zh-CN", { hour12: false }) : "时间未返回") + '</time></div><p>' + escape(r.comment || "未填写评审说明") + "</p></li>").join("") + "</ol>" : '<p class="hint">此版本尚无人工评审记录。</p>') +
      '<details class="evidence-raw-history"><summary>原始版本记录</summary><pre>' + escape(JSON.stringify(history || {}, null, 2).slice(0, 131072)) + "</pre></details>";
  }
  function metadataHtml(meta, history) {
    return '<dl class="evidence-facts">' + [
      ["产物标识", meta.artifact_id], ["文件名", meta.file_name || "未返回"], ["版本", "v" + meta.version], ["任务 / 尝试", [meta.task_id, meta.attempt_id].filter(Boolean).join(" / ") || "旧产物未标注"],
      ["SHA-256", meta.sha256 || "未返回"], ["字节数", meta.size_bytes ?? "未知"], ["人工评审", reviewText(meta.review_state)], ["格式校验", validationText(meta.validation)],
    ].map(([key, value]) => "<dt>" + escape(key) + "</dt><dd>" + escape(value) + "</dd>").join("") + '</dl><h4>证据引用</h4>' +
      ((meta.evidence_refs || []).length ? '<ul>' + meta.evidence_refs.map(ref => "<li><code>" + escape(typeof ref === "string" ? ref : JSON.stringify(ref)) + "</code></li>").join("") + "</ul>" : "<p class=\"hint\">没有证据引用；格式正确不等同于功能检查通过。</p>") +
      '<h4>未解决问题</h4><p>' + escape((meta.open_issues || []).join("；") || "宿主未返回未解决问题") + '</p><button data-evidence-export>导出证据摘要</button>' + historyHtml(history);
  }
  function reviewHtml(meta, history) {
    const draft = draftFor(meta), waiting = pending.has(draftKey(meta)), disabled = state.busy || waiting;
    const superseded = normalize(meta.review_state) === "superseded" || Boolean(history?.superseded_by);
    return '<form class="evidence-review" data-evidence-review-form><h4>评审当前版本</h4><p>当前为 v' + escape(meta.version) + " · " + escape(reviewText(meta.review_state)) + '。提交后将追加人工评审记录，实际授权由宿主检查。</p><label>评审者标识<input data-reviewer value="' + escape(draft.reviewer) + '" aria-label="人工评审者标识"' + (waiting ? " disabled" : "") + '></label><label>评审说明<textarea data-review-note rows="5" aria-label="评审说明"' + (waiting ? " disabled" : "") + ">" + escape(draft.comment) + '</textarea></label><div class="evidence-actions">' +
      [["approve", "批准此版本"], ["request_changes", "要求修改"], ["reject", "拒绝"]].map(([action, label]) => '<button type="button" data-review="' + action + '" data-core-action' + (disabled || action === "approve" && superseded ? " disabled" : "") + (action === "approve" && superseded ? ' title="此版本已被取代，请核对最新版本"' : "") + ">" + (waiting ? "提交中…" : label) + "</button>").join("") + '</div><p>评审说明会保留到提交成功；切换版本不会混用草稿。</p></form>';
  }
  function detailHtml() {
    const meta = state.detail?.metadata, history = state.detail?.history;
    if (!meta) return '<div class="panel-empty-state evidence-detail-empty"><strong>' + (state.busy ? "正在读取产物…" : "选择产物查看正文与证据") + '</strong><span>可核对版本、来源、格式校验与人工评审记录。</span></div>';
    return '<article class="evidence-detail" data-evidence-identity="' + escape(draftKey(meta)) + '" aria-label="产物详情"><header class="evidence-detail-head"><div><h3>' + escape(displayName(meta)) + '</h3><p>v' + escape(meta.version) + " · " + escape(reviewText(meta.review_state)) + '</p></div><button data-artifact-download>' + icon("download") + '下载原文件</button></header><div class="evidence-detail-tabs" role="tablist" aria-label="产物详情">' +
      tabs.map(([id, label]) => '<button type="button" role="tab" id="evidence-tab-' + id + '" data-evidence-tab="' + id + '" aria-controls="evidence-pane-' + id + '" aria-selected="' + (state.tab === id) + '" tabindex="' + (state.tab === id ? "0" : "-1") + '">' + label + "</button>").join("") + "</div>" +
      tabs.map(([id]) => '<section role="tabpanel" id="evidence-pane-' + id + '" aria-labelledby="evidence-tab-' + id + '"' + (state.tab !== id ? " hidden" : "") + ">" +
        (id === "preview" ? previewHtml(meta) : id === "evidence" ? metadataHtml(meta, history) : reviewHtml(meta, history)) + "</section>").join("") +
      '<footer class="evidence-validation">' + escape(validationText(meta.validation)) + '<span>格式校验不代表任务已通过功能验收。</span></footer></article>';
  }
  function view() {
    const options = state.mode === "project" ? state.projects : state.sessions.map(s => ({ id: s.id, title: s.title || s.id }));
    const selector = '<div class="evidence-toolbar"><div class="evidence-source" role="group" aria-label="数据来源">' +
      [["project", "团队交付"], ["session", "会话变更"]].map(([mode, label]) => '<button type="button" data-evidence-mode="' + mode + '" aria-pressed="' + (state.mode === mode) + '">' + label + "</button>").join("") +
      '</div><label>' + (state.mode === "project" ? "项目" : "会话") + '<select data-evidence-select aria-label="' + (state.mode === "project" ? "项目" : "会话") + '"><option value="">选择…</option>' + options.map(p => '<option value="' + escape(p.id) + '"' + (p.id === state.selected ? " selected" : "") + ">" + escape(p.title) + "</option>").join("") + '</select></label><button class="evidence-refresh" data-evidence-refresh aria-label="刷新证据" title="刷新证据">' + icon("refresh") + "</button></div>";
    let body;
    if (state.mode === "session") {
      const facts = state.session ? root.OwoSessionFacts.fromDetail(state.session) : null;
      const record = root.OwoCompletionRecord.summarize(facts?.lastCompletionRecord);
      body = '<div class="evidence-session"><h3>宿主完成记录</h3><p>' + escape(record?.compact || "没有可用的宿主完成记录，不能据此认定任务通过验收") + '</p><p>实际模型：' + escape(facts?.lastExecutionModel || "未知") + "</p>" +
        (record ? '<dl class="evidence-facts">' + record.details.map(([key, value]) => "<dt>" + escape(key) + "</dt><dd>" + escape(value) + "</dd>").join("") + "</dl>" : "") +
        '<h3>文件变更</h3>' + (state.diffs.length ? state.diffs.map(diff => '<details class="evidence-diff"><summary>' + escape(diff.path) + '</summary><div class="diff-columns"><div><strong>改动前</strong><pre>' + escape(String(diff.before ?? "").slice(0, 131072)) + '</pre></div><div><strong>改动后</strong><pre>' + escape(String(diff.after ?? "").slice(0, 131072)) + "</pre></div></div></details>").join("") : '<div class="panel-empty-state"><strong>未记录文件变更</strong><span>这不代表任务已经验收，请同时查看宿主完成记录。</span></div>') +
        '<div class="evidence-actions"><button data-evidence-audit' + (!state.selected ? " disabled" : "") + '>查看会话与执行记录</button><button data-evidence-export>导出证据摘要</button></div></div>';
    } else body = '<div class="evidence-columns"><aside class="evidence-artifacts" aria-label="产物版本列表"><label class="evidence-search">' + icon("search") + '<input type="search" data-evidence-search placeholder="搜索产物" aria-label="搜索产物" value="' + escape(state.filter) + '"></label><div data-evidence-list>' + artifactRows() + "</div></aside>" + detailHtml() + "</div>";
    return '<section data-panel="evidence" data-layout="custom" class="evidence-page">' + selector +
      (state.error ? '<div class="panel-notice" role="alert">' + escape(state.error) + "</div>" : "") +
      (state.notice ? '<p class="evidence-notice" role="status">' + escape(state.notice) + "</p>" : "") +
      '<p class="evidence-loading" role="status"' + (!state.busy ? " hidden" : "") + ">正在读取宿主数据…</p>" + body + "</section>";
  }
  function render() {
    if (!host) return;
    const focused = root.document?.activeElement, ownsFocus = host.contains?.(focused);
    const tabId = ownsFocus ? focused?.getAttribute?.("data-evidence-tab") : null;
    const field = ownsFocus ? ["data-reviewer", "data-review-note", "data-evidence-search"].find(name => focused.hasAttribute?.(name)) : null;
    const identity = focused?.closest?.("[data-evidence-identity]")?.getAttribute("data-evidence-identity");
    const selection = field ? [focused.selectionStart, focused.selectionEnd] : null;
    host.innerHTML = view(); host.setAttribute?.("aria-busy", String(state.busy));
    root.OwoCoreActionAvailability?.update(root.OwoCoreActionAvailability.isAvailable(), host);
    if (tabId) host.querySelector?.('[data-evidence-tab="' + tabId + '"]')?.focus();
    if (field && (field === "data-evidence-search" || identity === draftKey(state.detail?.metadata))) {
      const next = host.querySelector?.("[" + field + "]");
      if (next && !next.disabled) { next.focus({preventScroll: true}); if (Number.isInteger(selection[0])) next.setSelectionRange?.(...selection); }
    }
  }
  async function load() {
    const g = ++gen, previous = state.artifactId;
    state.busy = true; state.error = ""; state.notice = ""; render();
    try {
      if (!state.selected) { state.manifest = []; state.detail = null; state.session = null; state.diffs = []; }
      else if (state.mode === "project") {
        const data = await H.get("/projects/" + encodeURIComponent(state.selected) + "/artifacts");
        if (g !== gen || !host) return;
        state.manifest = (data.artifacts || []).filter(a => a && a.artifact_id);
        const next = state.manifest.some(a => a.artifact_id === previous) ? previous : state.manifest[0]?.artifact_id;
        if (next) return openArtifact(next);
        state.artifactId = ""; state.detail = null; state.content = null;
      } else {
        const [detail, diff] = await Promise.all([H.get("/session/" + encodeURIComponent(state.selected)), H.get("/session/" + encodeURIComponent(state.selected) + "/diff")]);
        if (g !== gen || !host) return; state.session = detail; state.diffs = Array.isArray(diff) ? diff : diff.diffs || diff.entries || [];
      }
    } catch (error) { if (g === gen && host) state.error = "读取失败：" + H.friendlyError(error); }
    if (g === gen && host) { state.busy = false; render(); }
  }
  async function openArtifact(id) {
    const changed = state.artifactId !== id, g = ++gen;
    state.artifactId = id; state.busy = true; state.previewBusy = false; state.content = null; state.detail = null; state.error = ""; state.historyError = ""; state.previewError = "";
    if (changed) { state.tab = "preview"; state.notice = ""; }
    render();
    const [meta, history] = await Promise.allSettled([H.get("/artifacts/" + encodeURIComponent(id) + "/metadata"), H.get("/artifacts/" + encodeURIComponent(id) + "/history")]);
    if (g !== gen || !host) return;
    if (meta.status === "fulfilled") {
      state.detail = { metadata: meta.value, history: history.status === "fulfilled" ? history.value : null };
      if (history.status === "rejected") state.historyError = "评审记录读取失败：" + H.friendlyError(history.reason);
    } else state.error = "产物详情不可用：" + H.friendlyError(meta.reason);
    state.busy = false; render();
    if (state.detail) return preview();
  }
  async function preview() {
    const meta = state.detail?.metadata, g = gen;
    if (!meta || state.previewBusy) return;
    if (!Number.isSafeInteger(meta.size_bytes) || meta.size_bytes < 0 || meta.size_bytes > 262144) { state.previewError = "正文大小未确认或超过 256 KiB，请下载原文件。"; render(); return; }
    state.previewBusy = true; state.previewError = ""; render();
    try {
      const data = await H.get("/artifacts/" + encodeURIComponent(meta.artifact_id) + "/content");
      if (g === gen && host) state.content = String(data.content ?? "").slice(0, 262144);
    } catch (error) { if (g === gen && host) state.previewError = "正文读取失败：" + H.friendlyError(error); }
    if (g === gen && host) { state.previewBusy = false; render(); }
  }
  function exportEvidence() {
    const value = { project_or_session: state.selected, source: "shared_daemon", completion: state.session?.turn_history || null, artifacts: state.detail || state.manifest, generated_at: new Date().toISOString() };
    const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob), link = document.createElement("a"); link.href = url; link.download = "evidence-summary.json"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  async function review(decision) {
    if (!["approve", "request_changes", "reject"].includes(decision) || state.busy) return;
    const metadata = state.detail?.metadata, key = draftKey(metadata);
    if (pending.has(key)) return;
    if (!metadata?.team_id || !metadata.version) { state.error = "缺少团队或版本身份，无法提交评审"; render(); return; }
    const draft = draftFor(metadata), draftReviewer = draft.reviewer, reviewer = draftReviewer.trim(), comment = draft.comment;
    if (!reviewer) { state.error = "请填写人工评审者标识"; render(); return; }
    const sourceGen = gen, sourceHost = host;
    pending.add(key); render();
    try {
      if (!await H.confirm("确认提交对版本 " + metadata.version + " 的评审？实际授权以服务端判定为准。")) return;
      if (sourceGen !== gen || sourceHost !== host || state.detail?.metadata !== metadata) return;
      const intent = JSON.stringify([metadata.artifact_id, metadata.version, decision, reviewer, comment]);
      if (!keys.has(intent)) keys.set(intent, crypto.randomUUID());
      const response = await H.post("/artifacts/" + encodeURIComponent(metadata.artifact_id) + "/review", {
        team_id: metadata.team_id, decision, reviewer, comment, expected_version: metadata.version, idempotency_key: keys.get(intent),
      });
      // Clear only the submitted draft, never an edited draft or another version.
      if (drafts.get(key)?.reviewer === draftReviewer && drafts.get(key)?.comment === comment) drafts.delete(key);
      if (sourceGen === gen && sourceHost === host) {
        state.notice = response?.replayed ? "评审已提交（幂等重放，无重复副作用）。" : "评审已提交。";
        const updated = response?.artifact;
        if (updated) state.manifest = state.manifest.map(a => a.artifact_id === updated.artifact_id ? updated : a);
        await openArtifact(metadata.artifact_id);
      }
    } catch (error) {
      if (sourceGen === gen && sourceHost === host) state.error = "评审未提交成功，草稿已保留：" + H.friendlyError(error);
    } finally {
      pending.delete(key);
      if (sourceHost === host) render();
    }
  }
  function selectTab(id, focus) {
    if (!tabs.some(([key]) => key === id)) return;
    state.tab = id;
    for (const [key] of tabs) {
      const button = host.querySelector?.('[data-evidence-tab="' + key + '"]'), panel = host.querySelector?.("#evidence-pane-" + key);
      button?.setAttribute("aria-selected", String(key === id)); button?.setAttribute("tabindex", key === id ? "0" : "-1");
      if (panel) panel.hidden = key !== id;
      if (focus && key === id) button?.focus();
    }
  }
  function changeSource(mode, selected) {
    gen++; state.mode = mode; state.selected = selected; state.artifactId = ""; state.detail = null; state.content = null; state.manifest = []; state.session = null; state.diffs = []; state.filter = ""; state.previewBusy = false;
    return load();
  }
  async function click(event) {
    const button = event.target.closest("button"); if (!button || button.disabled) return;
    if (button.dataset.artifact) return openArtifact(button.dataset.artifact);
    if (button.dataset.evidenceMode) {
      if (button.dataset.evidenceMode === state.mode) return;
      return changeSource(button.dataset.evidenceMode, "");
    }
    if (button.dataset.evidenceTab) return selectTab(button.dataset.evidenceTab, false);
    if (button.hasAttribute("data-evidence-refresh")) return load();
    if (button.hasAttribute("data-evidence-history")) return openArtifact(state.artifactId);
    if (button.hasAttribute("data-evidence-audit")) return H.openSession(state.selected);
    if (button.hasAttribute("data-evidence-export")) return exportEvidence();
    if (button.dataset.review) return review(button.dataset.review);
    const metadata = state.detail?.metadata, g = gen;
    if (button.hasAttribute("data-artifact-download") && metadata) {
      try { await H.downloadArtifact(metadata.artifact_id, metadata.file_name); if (g === gen && host) H.notify?.("已发起下载，请查看保存窗口或下载目录"); }
      catch (error) { if (g === gen && host) { state.error = "下载未开始：" + H.friendlyError(error); render(); } } return;
    }
    if (button.hasAttribute("data-artifact-preview")) return preview();
  }
  function dispose() {
    gen++;
    if (host && handlers) for (const [name, handler] of Object.entries(handlers)) host.removeEventListener?.(name, handler);
    host = null; handlers = null; state.busy = false; state.previewBusy = false;
  }
  root.OwoPanels.evidence = { id: "evidence", title: "产物与证据", async mount(rootEl, helpers) {
    dispose(); host = rootEl; H = helpers; state.sessions = H.snapshot?.().sessions || []; state.error = ""; state.notice = "";
    handlers = {
      change: event => { if (event.target.hasAttribute("data-evidence-select")) return changeSource(state.mode, event.target.value); },
      input: event => {
        if (event.target.hasAttribute("data-evidence-search")) {
          state.filter = event.target.value;
          const list = host.querySelector?.("[data-evidence-list]"); if (list) list.innerHTML = artifactRows();
        }
        const meta = state.detail?.metadata;
        if (meta && event.target.hasAttribute("data-reviewer")) draftFor(meta).reviewer = event.target.value;
        if (meta && event.target.hasAttribute("data-review-note")) draftFor(meta).comment = event.target.value;
      },
      click,
      submit: event => { if (event.target.hasAttribute("data-evidence-review-form")) event.preventDefault(); },
      keydown: event => {
        const button = event.target.closest?.("[data-evidence-tab]");
        if (!button || event.altKey || event.ctrlKey || event.metaKey || event.isComposing) return;
        const index = tabs.findIndex(([id]) => id === button.dataset.evidenceTab);
        let next;
        if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
        else if (event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = tabs.length - 1;
        else return;
        event.preventDefault(); selectTab(tabs[next][0], true);
      },
    };
    for (const [name, handler] of Object.entries(handlers)) host.addEventListener(name, handler);
    const g = ++gen; state.busy = true; render();
    try {
      const data = await H.get("/teams"); if (g !== gen || !host) return;
      const projects = new Map();
      for (const team of data.teams || []) if (team.project_space_id) projects.set(team.project_space_id, { id: team.project_space_id, title: team.objective || team.project_space_id });
      state.projects = [...projects.values()];
      if (!state.projects.length && state.sessions.length && state.mode === "project") { state.mode = "session"; state.selected = ""; }
      const options = state.mode === "project" ? state.projects : state.sessions;
      if (!options.some(item => item.id === state.selected)) state.selected = options[0]?.id || "";
      await load();
    } catch (error) { if (g === gen && host) { state.error = "项目目录不可用：" + H.friendlyError(error); state.busy = false; render(); } }
  }, openSession(id) { return changeSource("session", id); }, refresh: load, dispose,
  _test: { validationText, reviewText, view, state, load, openArtifact, preview, review, draftFor, draftKey, drafts, pending, changeSource, selectTab, artifactRows }
  };
})(typeof window !== "undefined" ? window : globalThis);
