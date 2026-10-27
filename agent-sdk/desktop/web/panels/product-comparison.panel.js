/* Product comparison lifecycle. The existing eval panel remains an engineering suite. */
(function (global) {
  "use strict";
  const ACTIVE = new Set(["queued", "running"]);
  const STATES = new Set(["queued", "running", "completed", "failed", "cancelled", "interrupted"]);
  const LABELS = { queued: "排队中", running: "执行中", completed: "已结束", failed: "运行失败", cancelled: "已取消", interrupted: "服务重启后中断" };
  const esc = value => String(value ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  function normalizeConfig(config) {
    const repetitions = Number(config.repetitions);
    if (!Number.isInteger(repetitions) || repetitions < 1 || repetitions > 20) throw Error("重复次数必须是 1 到 20 的整数");
    if (!["reference", "live"].includes(config.execution)) throw Error("请选择参考回放或真实模型");
    if (config.category && !["code", "research", "document"].includes(config.category)) throw Error("任务分类无效");
    return { suite: "v1", execution: config.execution, modes: ["single", "workswarm"], repetitions,
      category: config.category || null, only: String(config.only || "").trim() || null };
  }
  function summarize(report) {
    const result = { single: null, team: null, paired: 0, missing: 0, duplicates: 0,
      invalid: 0, speedRatio: null, tokenRatio: null, toolRatio: null, comparable: false, reason: "尚未取得报告" };
    if (!report || !Array.isArray(report.runs)) return result;
    const groups = { single: [], multi: [] }, pairs = new Map();
    for (const run of report.runs) {
      const mode = run?.key?.agent_mode;
      if (!["single", "multi"].includes(mode) || typeof run.key?.case_id !== "string" || !run.key.case_id.trim() || !Number.isInteger(run.key.repetition) || run.key.repetition < 0 || !["passed", "failed", "error", "timeout", "cancelled"].includes(run.status)) { result.invalid++; continue; }
      groups[mode].push(run);
      const key = JSON.stringify([run.key.case_id, run.key.repetition]);
      const pair = pairs.get(key) || {};
      if (pair[mode]) result.duplicates++;
      pair[mode] = run; pairs.set(key, pair);
    }
    const aggregate = runs => {
      if (!runs.length) return null;
      const knownTokens = runs.every(run => Number.isFinite(run.total_tokens) && run.total_tokens >= 0);
      const knownWall = runs.every(run => Number.isFinite(run.wall_ms) && run.wall_ms >= 0);
      const knownChecks = runs.every(run => Number.isInteger(run.checker_total) && run.checker_total > 0 && Number.isInteger(run.checker_passed) && run.checker_passed >= 0 && run.checker_passed <= run.checker_total);
      const knownCalls = runs.every(run => Number.isInteger(run.model_calls) && run.model_calls >= 0);
      const observedTools = runs.map(run => Number.isInteger(run.tool_calls) && run.tool_calls >= 0 ? run.tool_calls :
        Array.isArray(run.tool_log) && run.tool_log.length > 0 ? run.tool_log.length : null);
      const knownTools = observedTools.every(value => value !== null);
      return { attempted: runs.length, passed: runs.filter(run => run.status === "passed").length,
        meanWallMs: knownWall ? runs.reduce((sum, run) => sum + run.wall_ms, 0) / runs.length : null,
        totalTokens: knownTokens ? runs.reduce((sum, run) => sum + run.total_tokens, 0) : null,
        modelCalls: knownCalls ? runs.reduce((sum, run) => sum + run.model_calls, 0) : null,
        toolCalls: knownTools ? observedTools.reduce((sum, value) => sum + value, 0) : null,
        checkerPassed: knownChecks ? runs.reduce((sum, run) => sum + run.checker_passed, 0) : null,
        checkerTotal: knownChecks ? runs.reduce((sum, run) => sum + run.checker_total, 0) : null };
    };
    result.single = aggregate(groups.single); result.team = aggregate(groups.multi);
    const models = new Set(report.runs.map(run => run?.model).filter(Boolean));
    for (const pair of pairs.values()) {
      if (pair.single && pair.multi) result.paired++;
      else result.missing++;
    }
    const fingerprints = ["suite_hash", "run_contract_sha256", "evaluator_binary_sha256", "provider_endpoint_sha256"];
    if (report.execution !== "live") result.reason = "参考回放仅检查评测链路，不能证明 Team 的真实能力或速度";
    else if (result.invalid || result.missing || result.duplicates || !result.paired || (report.pending != null && !Array.isArray(report.pending) || (report.pending || []).length)) result.reason = "配对样本不完整或存在重复，保留失败与缺席记录";
    else if (!fingerprints.every(key => typeof report[key] === "string" && report[key].trim())) result.reason = "缺少任务、执行器或端点身份，当前仅展示诊断指标";
    else if (!report.model || models.size !== 1 || !models.has(report.model) || report.runs.some(run => !run.model)) result.reason = "两侧模型身份缺失或不一致";
    else {
      result.comparable = true;
      result.reason = "同一报告内样本完整配对；指标用于比较，综合优势仍需预先约定门槛和重复实验";
      if (result.single.meanWallMs > 0 && result.team.meanWallMs != null) result.speedRatio = result.team.meanWallMs / result.single.meanWallMs;
      if (result.single.totalTokens > 0 && result.team.totalTokens != null) result.tokenRatio = result.team.totalTokens / result.single.totalTokens;
      if (result.single.toolCalls > 0 && result.team.toolCalls != null) result.toolRatio = result.team.toolCalls / result.single.toolCalls;
    }
    return result;
  }
  function createController(api, options = {}) {
    const schedule = options.setTimeout || global.setTimeout, clear = options.clearTimeout || global.clearTimeout;
    const emit = options.onChange || (() => {}), visible = options.visible || (() => !global.document?.hidden);
    const state = { runs: [], runId: null, detail: null, error: "", submitting: false, cancelling: false, loading: false };
    let active = true, sequence = 0, listSequence = 0, timer = null, readAbort = null, failures = 0;
    const publish = () => { if (active) emit(state); };
    function setDetail(detail) {
      state.detail = detail;
      state.runs = state.runs.map(run => run.run_id === detail.run_id
        ? { ...run, status: detail.status, progress: detail.progress } : run);
    }
    const path = id => "/product-eval/runs/" + encodeURIComponent(id);
    const stop = () => { if (timer !== null) clear(timer); timer = null; };
    function queuePoll() {
      stop();
      if (!active || !state.runId || (state.detail && !ACTIVE.has(state.detail.status))) return;
      timer = schedule(async () => {
        timer = null;
        if (!active) return;
        if (visible()) await readSelected();
        else queuePoll();
      }, Math.min(15000, 1500 * (1 + failures)));
    }
    async function readSelected() {
      if (!active || !state.runId || state.cancelling) return;
      const id = state.runId, version = sequence;
      readAbort?.abort(); const controller = new AbortController(); readAbort = controller;
      state.loading = true; publish();
      try {
        const detail = await api.get(path(id), { signal: controller.signal });
        if (!active || version !== sequence || controller.signal.aborted) return;
        if (!detail || detail.run_id !== id || !STATES.has(detail.status)) throw Error("服务返回的评测运行身份或状态无效");
        setDetail(detail); state.error = ""; failures = 0;
      } catch (error) {
        if (!active || version !== sequence || controller.signal.aborted) return;
        state.error = error.message || String(error); failures++;
      } finally {
        if (active && version === sequence && !controller.signal.aborted) {
          state.loading = false; publish(); queuePoll();
        }
      }
    }
    async function refreshList() {
      const version = ++listSequence;
      try {
        const response = await api.get("/product-eval/runs");
        if (active && version === listSequence) { if (!Array.isArray(response?.runs) || response.runs.some(run => !run || typeof run.run_id !== "string" || !run.run_id || !STATES.has(run.status))) throw Error("服务返回的历史运行列表无效"); state.runs = response.runs; publish(); }
      } catch (error) {
        if (active && version === listSequence) { state.error = error.message || String(error); publish(); }
      }
    }
    const controller = {
      state,
      refreshList,
      async select(id) {
        if (!active) return;
        stop(); readAbort?.abort(); sequence++;
        state.runId = id; state.detail = null; state.error = ""; state.cancelling = false; failures = 0;
        await readSelected();
      },
      async start(config) {
        if (!active || state.submitting) return;
        const body = normalizeConfig(config), version = sequence;
        state.submitting = true; state.error = ""; publish();
        try {
          const created = await api.post("/product-eval/runs", body);
          if (!created?.run_id || !STATES.has(created.status)) throw Error("服务没有返回有效运行身份");
          if (active && version === sequence) await controller.select(created.run_id);
          if (active) await refreshList();
          return created;
        } catch (error) {
          if (active) { state.error = error.message || String(error); publish(); }
          throw error;
        } finally { state.submitting = false; publish(); }
      },
      async cancel() {
        if (!active || state.cancelling || !ACTIVE.has(state.detail?.status)) return;
        stop(); readAbort?.abort(); sequence++;
        const id = state.runId, version = sequence;
        state.loading = false;
        state.cancelling = true; publish();
        try {
          const result = await api.post(path(id) + "/cancel", {});
          if (active && version === sequence) {
            if (result?.run_id !== id || !STATES.has(result.status)) throw Error("取消响应身份或状态无效");
            stop(); setDetail({ ...state.detail, status: result.status }); publish(); queuePoll();
          }
        } catch (error) {
          if (active && version === sequence) { state.error = error.message || String(error); publish(); }
          throw error;
        } finally { if (active && version === sequence) { state.cancelling = false; publish(); queuePoll(); } }
      },
      refresh: readSelected,
      dispose() { active = false; sequence++; listSequence++; stop(); readAbort?.abort(); },
    };
    return controller;
  }
  function reportHtml(detail, sampleLimit = 50) {
    if (!detail) return '<p class="sub">选择历史运行或创建一次对比。</p>';
    const summary = summarize(detail.report);
    const metric = (value, suffix = "") => (!Number.isFinite(value) || value < 0) ? "未知" : esc(Number(value).toFixed(suffix === " 秒" ? 2 : 0)) + suffix;
    const row = (name, key, transform = value => value) => '<tr><th>' + name + '</th>' +
      ["single", "team"].map(mode => '<td>' + metric(summary[mode] ? transform(summary[mode][key]) : null, key === "meanWallMs" ? " 秒" : "") + '</td>').join("") + '</tr>';
    let html = '<div class="owo-pe-summary"><h3>' + esc(LABELS[detail.status] || detail.status) + '</h3><p>' + esc(detail.execution === "live" ? "真实模型" : detail.execution === "reference" ? "参考回放 · 不代表真实模型能力或速度" : "执行方式未知") + ' · 模型：' + esc(detail.model || detail.report?.model || "尚未取得") + '</p><p class="sub">运行 ID：' + esc(detail.run_id || '未知') + '</p>';
    if (detail.progress) html += '<p>已记录样本：' + esc(detail.progress.done ?? "?") + ' / ' + esc(detail.progress.total ?? "?") + '</p>';
    if (detail.status === "cancelled") html += '<p class="sub">取消已提交；正在执行的样本可能仍在收尾，已有记录保留。</p>';
    if (detail.status === "interrupted") html += '<p class="sub">服务重启后不会自动重跑模型，可查看保留的报告或创建新运行。</p>';
    if (detail.error) html += '<p class="owo-pe-error">' + esc(detail.error) + '</p>';
    html += '<p role="status">' + esc(summary.reason) + '</p></div><div class="owo-pe-table"><table><thead><tr><th>指标</th><th>Single</th><th>Team</th></tr></thead><tbody>' +
      row("已尝试样本", "attempted") + row("通过样本", "passed") + row("平均耗时", "meanWallMs", value => value == null ? null : value / 1000) +
      row("模型调用", "modelCalls") + row("工具调用", "toolCalls") + row("Token 总量", "totalTokens") + row("通过检查数", "checkerPassed") + row("检查总数", "checkerTotal") +
      '</tbody></table></div><p>完整配对 ' + summary.paired + ' · 缺席 ' + summary.missing + ' · 重复 ' + summary.duplicates + '</p>';
    if (summary.speedRatio != null) html += '<p>Team / Single 耗时：' + summary.speedRatio.toFixed(2) + ' 倍</p>';
    if (summary.tokenRatio != null) html += '<p>Team / Single Token：' + summary.tokenRatio.toFixed(2) + ' 倍</p>';
    if (summary.toolRatio != null) html += '<p>Team / Single 工具调用：' + summary.toolRatio.toFixed(2) + ' 倍</p>';
    const comparison = detail.report?.comparison;
    if (comparison) {
      const rules = ['alignment_guardrails', 'quality_guardrails', 'resource_guardrails', 'rules'].flatMap(key => Array.isArray(comparison[key]) ? comparison[key] : []);
      const verified = detail.status === 'completed' && summary.comparable && comparison.enabled === true && comparison.sample_sufficient === true && rules.length > 0 && rules.every(rule => rule && rule.satisfied === true);
      const verdict = verified ? '建议启用 Team' : '暂不自动启用 Team';
      html += '<details class="owo-pe-verdict"><summary>报告的启用判定与检查规则</summary><h4>共享启用判定：' + esc(verdict) + '</h4><p>' +
        esc(comparison.sample_sufficient === true ? '报告声明样本量达到判定门槛' : comparison.sample_sufficient === false ? '样本量不足，结论仅作观察' : '报告未提供样本充分性') +
        '</p><p class="sub">报告启用标志：' + (comparison.enabled === true ? '开启' : comparison.enabled === false ? '关闭' : '未知') +
        (comparison.enabled === true && !verified ? '（未满足本页的比较前提）' : '') + '</p><ul>' + rules.map(rule => '<li>' +
        (rule?.satisfied === true ? '通过' : rule?.satisfied === false ? '未通过' : '未知') + ' · ' + esc(rule?.name || '未命名规则') + '：' + esc(rule?.detail || '无说明') + '</li>').join('') + '</ul></details>';
    }
    const runs = Array.isArray(detail.report?.runs) ? detail.report.runs : [];
    if (runs.length) html += '<details class="owo-pe-samples"><summary>样本、检查与交付记录</summary><p>显示 ' + Math.min(sampleLimit, runs.length) + ' / ' + runs.length + ' 条；汇总仍覆盖全部记录。</p>' + runs.slice(0, sampleLimit).map(value => {
      const run = value || {}, list = key => Array.isArray(run[key]) ? run[key].join('；') || '无记录' : '无记录';
      return '<article class="owo-pe-sample"><strong>' + esc(run.key?.case_id || '身份未知') + ' · ' +
      esc(run.key?.agent_mode === 'multi' ? 'Team' : run.key?.agent_mode === 'single' ? 'Single' : '模式未知') + ' · 第 ' + esc(Number.isInteger(run.key?.repetition) ? run.key.repetition + 1 : '?') + ' 轮</strong><p>' +
      esc(run.status || '未知') + ' · ' + metric(run.wall_ms == null ? null : run.wall_ms / 1000, ' 秒') +
      ' · 模型调用 ' + metric(run.model_calls) + ' · 工具调用 ' + metric(run.tool_calls == null ? (Array.isArray(run.tool_log) && run.tool_log.length ? run.tool_log.length : null) : run.tool_calls) + '</p>' +
      (run.error ? '<p class="owo-pe-error">' + esc(run.error) + '</p>' : '') + '<p>失败检查：' + esc(list('failed_steps')) + '</p><p>交付引用：' + esc(list('artifact_refs')) + '</p></article>';
    }).join('') + (runs.length > sampleLimit ? '<button type="button" class="owo-pe-more">再显示 50 条</button>' : '') + '</details>';
    return html;
  }
  let current = null;
  const scopes = new Map(), tabs = ['create', 'history', 'report'];
  const panel = {
    id: 'product-eval', title: '单人 / 团队对比',
    nav() {
      return '<section data-panel="product-eval" data-layout="custom" class="owo-pe-panel owo-insight">' +
        '<div class="owo-insight-toolbar"><div class="owo-insight-tabs" role="tablist" aria-label="单人 / 团队对比">' + tabs.map((tab, i) =>
          '<button type="button" role="tab" id="pe-tab-' + tab + '" data-pe-tab="' + tab + '" aria-controls="pe-pane-' + tab + '" aria-selected="' + (i === 0) + '" tabindex="' + (i === 0 ? 0 : -1) + '">' + ['创建对比', '历史运行', '对比报告'][i] + '</button>').join('') +
        '</div><button type="button" class="owo-pe-list-refresh"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.6-1.5L20 9M4 15l2.3 3.5A7 7 0 0 0 17.9 17"/></svg>刷新历史</button></div>' +
        '<p class="owo-pe-error" role="alert"></p><p class="owo-pe-notice sub" role="status" aria-live="polite"></p>' +
        '<section role="tabpanel" id="pe-pane-create" aria-labelledby="pe-tab-create"><h3>创建对比</h3><p class="sub">两种模式使用相同任务、重复次数与检查器；真实模型会消耗额度。</p>' +
        '<form class="owo-pe-config"><label for="pe-execution">执行方式</label><select id="pe-execution" name="execution"><option value="reference">参考回放（不调用模型）</option><option value="live">真实模型</option></select>' +
        '<label for="pe-repetitions">每任务重复次数</label><input id="pe-repetitions" name="repetitions" type="number" min="1" max="20" value="1" required>' +
        '<label for="pe-category">分类</label><select id="pe-category" name="category"><option value="">全部</option><option value="code">代码</option><option value="research">研究</option><option value="document">文档</option></select>' +
        '<label for="pe-only">任务名称筛选</label><input id="pe-only" name="only" placeholder="留空则运行所选分类">' +
        '<div class="owo-insight-actions"><button class="owo-pe-start primary" type="submit" data-core-action>创建对比</button></div></form>' +
        '<section class="owo-insight-section"><div class="owo-pe-create-state owo-insight-empty">尚未创建对比。</div></section></section>' +
        '<section role="tabpanel" id="pe-pane-history" aria-labelledby="pe-tab-history" hidden><h3>历史运行</h3><div class="owo-pe-history" role="status" aria-live="polite">正在读取历史…</div></section>' +
        '<section role="tabpanel" id="pe-pane-report" aria-labelledby="pe-tab-report" hidden><div class="owo-pe-report-head"><h3>对比报告</h3><div class="owo-insight-actions"><button type="button" class="owo-pe-detail-refresh">刷新所选运行</button><button type="button" class="owo-pe-cancel" data-core-action disabled>取消运行</button></div></div>' +
        '<div class="owo-pe-report"></div></section></section>';
    },
    mount(root, helpers = {}) {
      panel.dispose(); root.innerHTML = panel.nav();
      const section = root.querySelector('.owo-pe-panel'), form = section.querySelector('form');
      const scope = (helpers.baseUrl || global.location?.origin || '') + '#' + (helpers.coreInstanceId || global.OwoApi?.coreInstanceId || '');
      if (!scopes.has(scope)) scopes.set(scope, { draft:{execution:'reference',repetitions:'1',category:'',only:''}, tab:'create', selected:null, starting:null, cancelling:new Set(), notice:'' });
      const owner = scopes.get(scope);
      let sampleLimit = 50, renderedRun = null;
      const $ = selector => section.querySelector(selector);
      const alive = () => current === controller && section.isConnected;
      function selectTab(tab) {
        if (!alive() || !tabs.includes(tab)) return;
        owner.tab = tab;
        tabs.forEach(key => { const button = $('[data-pe-tab="' + key + '"]'); button.setAttribute('aria-selected', String(tab === key)); button.setAttribute('tabindex', tab === key ? '0' : '-1'); $('#pe-pane-' + key).hidden = tab !== key; });
      }
      function busy(button, value, label) {
        button.disabled = value; button.setAttribute('aria-busy', String(value)); button.textContent = label;
        global.OwoCoreActionAvailability?.setBusy(button, value);
      }
      function paint(state) {
        if (!alive()) return;
        const changedRun = renderedRun !== state.runId;
        if (changedRun) { renderedRun = state.runId; sampleLimit = 50; }
        busy($('.owo-pe-start'), !!owner.starting, owner.starting ? '创建中…' : '创建对比');
        const cancelling = owner.cancelling.has(state.runId);
        busy($('.owo-pe-cancel'), cancelling || !ACTIVE.has(state.detail?.status), cancelling ? '取消中…' : '取消运行');
        $('.owo-pe-detail-refresh').disabled = !state.runId || cancelling;
        $('.owo-pe-error').textContent = state.error;
        $('.owo-pe-notice').textContent = owner.notice;
        $('.owo-pe-notice').hidden = !owner.notice; $('.owo-pe-error').hidden = !state.error;
        $('.owo-pe-create-state').textContent = owner.starting ? '正在创建对比…' : owner.selected ? '最近选择的运行：' + owner.selected : '尚未创建对比。';
        $('.owo-pe-history').innerHTML = state.runs.map(run => '<button type="button" class="owo-pe-history-row" aria-pressed="' + (run.run_id === state.runId) + '" data-run-id="' + esc(run.run_id) + '"><span>' + esc(LABELS[run.status] || '未知') + '</span><span><strong>' + esc(run.run_id) + '</strong><span class="sub">' + esc(run.execution === 'live' ? '真实模型' : run.execution === 'reference' ? '参考回放' : '执行方式未知') + ' · ' + esc(run.created_at || '创建时间未知') + '</span></span></button>').join('') || '<div class="owo-insight-empty">暂无历史运行</div>';
        const report = $('.owo-pe-report');
        const openStates = ['.owo-pe-verdict', '.owo-pe-samples'].map(selector => !changedRun && !!report.querySelector(selector)?.open);
        report.innerHTML = state.loading && !state.detail ? '<div class="owo-insight-empty">正在读取运行…</div>' : reportHtml(state.detail, sampleLimit);
        ['.owo-pe-verdict', '.owo-pe-samples'].forEach((selector, i) => { const details = report.querySelector(selector); if (details && openStates[i]) details.open = true; });
      }
      const api = { get: (path, options) => helpers.get ? helpers.get(path, options) : global.OwoApi.get(path, options), post: (path, body) => helpers.post ? helpers.post(path, body) : global.OwoApi.post(path, body) };
      const controller = createController(api, { visible:() => section.isConnected && !global.document.hidden, onChange:paint });
      controller.owner = owner; controller.repaint = () => paint(controller.state); current = controller;
      function updateMounted() { if (current?.owner === owner) current.repaint(); }
      Object.keys(owner.draft).forEach(key => { $('[name="' + key + '"]').value = owner.draft[key]; });
      function saveDraft() { Object.keys(owner.draft).forEach(key => { owner.draft[key] = $('[name="' + key + '"]').value; }); }
      form.addEventListener('input', () => { if (alive()) saveDraft(); });
      form.addEventListener('change', () => { if (alive()) saveDraft(); });
      form.addEventListener('submit', async event => {
        event.preventDefault(); if (!alive() || owner.starting) return;
        saveDraft(); const token = {}; owner.starting = token; owner.notice = ''; updateMounted();
        try {
          const created = await controller.start(owner.draft);
          if (created) {
            owner.notice = '已创建运行：' + created.run_id;
            if (alive() && controller.state.runId === created.run_id) { owner.selected = created.run_id; selectTab('report'); }
            else if (current?.owner === owner) current.refreshList();
          }
        } catch (error) { if (!alive()) owner.notice = '创建失败：' + (error.message || error); }
        finally { if (owner.starting === token) owner.starting = null; updateMounted(); }
      });
      $('.owo-pe-cancel').addEventListener('click', async () => {
        const id = controller.state.runId;
        if (!alive() || owner.cancelling.has(id) || !ACTIVE.has(controller.state.detail?.status)) return;
        owner.cancelling.add(id); updateMounted();
        try { await controller.cancel(); }
        catch (error) { if (!alive()) owner.notice = '取消失败：' + (error.message || error); }
        finally { owner.cancelling.delete(id); if (!alive() && current?.owner === owner) { current.refreshList(); if (current.state.runId === id) current.refresh(); } updateMounted(); }
      });
      $('.owo-pe-list-refresh').addEventListener('click', () => { if (alive()) controller.refreshList(); });
      $('.owo-pe-detail-refresh').addEventListener('click', () => { if (alive() && !owner.cancelling.has(controller.state.runId)) controller.refresh(); });
      $('.owo-pe-history').addEventListener('click', event => { if (!alive()) return; const button = event.target.closest('[data-run-id]'); if (button) { owner.selected = button.dataset.runId; controller.select(owner.selected); selectTab('report'); } });
      $('.owo-pe-report').addEventListener('click', event => { if (!alive()) return; if (event.target.closest('.owo-pe-more')) { sampleLimit += 50; paint(controller.state); const details = $('.owo-pe-report .owo-pe-samples'); if (details) details.open = true; } });
      tabs.forEach((tab, i) => {
        $('[data-pe-tab="' + tab + '"]').addEventListener('click', () => selectTab(tab));
        $('[data-pe-tab="' + tab + '"]').addEventListener('keydown', event => { if (event.ctrlKey || event.metaKey || event.altKey) return; const next = event.key === 'ArrowRight' ? (i + 1) % 3 : event.key === 'ArrowLeft' ? (i + 2) % 3 : event.key === 'Home' ? 0 : event.key === 'End' ? 2 : -1; if (next < 0 || !alive()) return; event.preventDefault(); selectTab(tabs[next]); $('[data-pe-tab="' + tabs[next] + '"]').focus(); });
      });
      selectTab(owner.tab); paint(controller.state); controller.refreshList(); if (owner.selected) controller.select(owner.selected);
    },
    dispose() { current?.dispose(); current = null; },
    refresh() { return current?.refresh(); }
  };
  if (global.document) { global.OwoPanels = global.OwoPanels || {}; global.OwoPanels[panel.id] = panel; }
  const exported = { normalizeConfig, summarize, createController, reportHtml, panel };
  if (typeof module !== "undefined" && module.exports) module.exports = exported;
  else global.OwoProductComparison = Object.freeze(exported);
})(typeof window !== "undefined" ? window : globalThis);
