/* ✨ 增强提示词（2026-11-14 重构：真实模型增强 + 澄清问答 + 编辑保护）。
 *
 * 主路径：核心服务 `POST /prompt/enhance`——任务类型识别、动态改写、结构化
 * 结果（ready / needs_clarification），与 Agent 共用同一模型网关和凭据链。
 * 兼容路径：核心是尚未包含该路由的旧版本（404）时，回落壳层 IPC 直连
 * （同样是真实模型调用）；两条路径都失败才报错。
 * 已删除：本地固定模板追加降级——失败就如实提示并允许重试，绝不用
 * "固定 Markdown 后缀"冒充增强结果。
 *
 * 交互（保留一键风格）：
 *   1. 写好草稿 → 点一下 ✨ → 模型理解并改写 → 直接替换输入框内容；
 *   2. 再点一下 → 还原原文；可继续编辑或再次增强；
 *   3. 模型认为缺少关键信息时，在输入框下方展开轻量问答面板，
 *      答案随原文一起重新增强；原始输入不动，绝不自动发送；
 *   4. 增强期间用户改了草稿 → 结果只作为预览展示，由用户决定是否采用。
 */
(function (global) {
  "use strict";
  const doc = global.document;
  if (!doc) return;

  function setTip(trigger, text) {
    if (!trigger) return;
    const value = String(text || "");
    if (global.OwoTooltip && typeof global.OwoTooltip.set === "function") {
      global.OwoTooltip.set(trigger, value);
    } else {
      trigger.setAttribute("data-tip", value);
      trigger.setAttribute("data-tooltip", value);
    }
  }

  /* ---------------- ✨ 增强提示词 ---------------- */
  const records = new Map();

  function record(target) {
    let entry = records.get(target);
    if (entry) return entry;
    entry = {
      original: null,
      enhanced: "",
      applied: false,
      busy: false,
      applying: false,
      // 澄清问答累计（跨轮保留，重新增强时随原文一起提交）。
      clarified: [],
    };
    records.set(target, entry);
    // 用户手动改动后，旧的「草稿 / 增强版」配对失效，按钮回到可增强状态。
    target.addEventListener("input", () => {
      if (entry.applying || entry.busy) return;
      if (!entry.applied && !entry.enhanced) return;
      entry.original = null;
      entry.enhanced = "";
      entry.applied = false;
      const trigger = triggerFor(target);
      if (trigger) {
        trigger.dataset.enhance = "idle";
        setTip(trigger, "增强提示词");
      }
    });
    return entry;
  }

  function triggerFor(target) {
    const id = target.id;
    if (!id) return null;
    return doc.querySelector('[data-prompt-polish="' + id + '"]');
  }

  function fill(target, value, entry) {
    entry.applying = true;
    target.value = value;
    // 输入框自适应高度、字数统计、发送按钮可用性都挂在 input 上。
    target.dispatchEvent(new Event("input", { bubbles: true }));
    entry.applying = false;
  }

  function stateOf(trigger, state, tip) {
    if (!trigger) return;
    trigger.dataset.enhance = state;
    setTip(trigger, tip);
  }

  /* ---------------- 请求构造（纯逻辑，供单测） ---------------- */

  // 真实模型 id：getComposerModel 返回自定义模型/预设模型的 id 本体，
  // 而不是 UI 胶囊上的显示文案（旧实现传显示名导致保管库匹配失败）。
  function currentModelId() {
    try {
      if (typeof global.getComposerModel === "function") return String(global.getComposerModel() || "").trim();
    } catch (_) { /* app.js 未加载时无模型上下文 */ }
    return "";
  }

  // 设置页「提示词增强模型」的解析结果：follow 用当前对话的模型，
  // cloud / local 用指定模型。解析不出来（模型被删、服务商未就绪）时按 follow
  // 处理——绝不带着一个不存在的模型去发请求。
  function enhanceTarget() {
    try {
      const api = global.OwoPromptEnhanceSettings;
      if (api && typeof api.resolve === "function") {
        const target = api.resolve();
        if (target && typeof target === "object") return target;
      }
    } catch (_) { /* 设置模块未加载时按跟随当前模型处理 */ }
    return { mode: "follow", model: "", useConnection: false };
  }

  function buildRequestBody(text, clarifications) {
    const body = { text: String(text || "").trim() };
    const target = enhanceTarget();
    const model = target.model || currentModelId();
    if (model) body.model = model;
    // 自定义模型（桌面保管库）：与回合相同的连接对象（密钥只经 env 引用）。
    // cloud 模式只覆盖模型 id，不能把自定义连接一起带过去。
    if (target.mode !== "cloud") {
      try {
        const routing = global.ModelRouting || (typeof ModelRouting !== "undefined" ? ModelRouting : null);
        if (routing && typeof global.loadCustomModels === "function") {
          const keys = typeof global.customModelKeys === "function" ? global.customModelKeys() : {};
          const connection = routing.buildCustomModelConnection(model, global.loadCustomModels(), keys || {});
          if (connection) body.model_connection = connection;
        }
      } catch (_) { /* 连接非法时让核心用默认 provider，错误会体现在响应里 */ }
    }
    // 上下文：只带调用方已知事实；未声明的一律不编造。
    const context = {};
    try {
      let workspace = "";
      if (typeof global.state === "object" && global.state) workspace = String(global.state.sessionWorkspace || "").trim();
      if (!workspace) {
        const box = doc.getElementById("workspace");
        if (box) workspace = String(box.value || "").trim();
      }
      if (workspace) {
        context.workspace = workspace;
        const parts = workspace.split(/[\\/]/).filter(Boolean);
        if (parts.length) context.project = parts[parts.length - 1];
      }
      if (typeof global.currentSessionMode === "function") {
        const mode = String(global.currentSessionMode() || "").trim();
        if (mode) context.session_mode = mode;
      }
    } catch (_) { /* 上下文获取失败不影响增强本身 */ }
    if (Object.keys(context).length) body.context = context;
    if (Array.isArray(clarifications) && clarifications.length) body.clarifications = clarifications;
    return body;
  }

  // 旧核心（404）时的壳层 IPC 结果 → 统一结构化形状（只有 ready 一种状态）。
  function legacyResultShape(ipc) {
    if (!ipc || !ipc.ok) return null;
    const text = String((ipc && ipc.text) || "").trim();
    if (!text) return null;
    return { status: "ready", enhanced_prompt: text, clarifying_questions: [], change_summary: null, task_type: null };
  }

  function errorMessageOf(error) {
    const raw = String((error && error.message) || error || "").trim();
    if (!raw) return "未知错误";
    // ApiError 形如 "502: prompt_enhance/provider_failed: ..."，去掉状态码前缀。
    return raw.replace(/^\d{3}:\s*/, "").slice(0, 160);
  }

  async function callEnhance(text, clarifications) {
    const body = buildRequestBody(text, clarifications);
    if (global.OwoApi && typeof global.OwoApi.request === "function") {
      try {
        return await global.OwoApi.request("/prompt/enhance", { method: "POST", json: body });
      } catch (error) {
        // 404 = 当前核心还没有 /prompt/enhance（版本偏旧）；网络级失败（无
        // status）= 核心暂时不可达。两种情况都走壳层 IPC 兼容路径——核心
        // 挂了不代表模型路径没了，但绝不降级到本地模板拼接。
        if (error && error.status && Number(error.status) !== 404) throw error;
      }
    }
    // 壳层 IPC：同样是真实模型调用；结果统一成结构化形状。
    if (global.owo && typeof global.owo.enhancePrompt === "function") {
      const ipc = await global.owo.enhancePrompt({ text: body.text, model: body.model || "" });
      const mapped = legacyResultShape(ipc);
      if (mapped) return mapped;
      throw Error(errorMessageOf(ipc && ipc.error) || "prompt/unavailable");
    }
    throw Error("prompt/service_unavailable");
  }

  /* ---------------- 澄清 / 预览面板（轻量内联，非弹窗） ---------------- */

  function panelFor(target) {
    const id = target.id;
    if (!id) return null;
    let panel = doc.querySelector('[data-enhance-panel="' + id + '"]');
    if (!panel) {
      panel = doc.createElement("div");
      panel.className = "prompt-enhance-panel";
      panel.setAttribute("data-enhance-panel", id);
      panel.setAttribute("role", "group");
      panel.setAttribute("aria-label", "提示词增强");
      target.insertAdjacentElement("afterend", panel);
    }
    return panel;
  }

  function closePanel(target) {
    const id = target.id;
    if (!id) return;
    const panel = doc.querySelector('[data-enhance-panel="' + id + '"]');
    if (panel && panel.parentNode) panel.parentNode.removeChild(panel);
  }

  // 澄清问答：原文不动，用户作答后「继续增强」带答案重跑。
  function renderClarifyPanel(target, questions, entry, trigger) {
    const panel = panelFor(target);
    if (!panel) return;
    panel.textContent = "";
    const title = doc.createElement("div");
    title.className = "pe-title";
    title.textContent = "AI 认为还差几个关键信息，回答后继续增强";
    panel.appendChild(title);
    const inputs = [];
    questions.slice(0, 3).forEach((question, index) => {
      const row = doc.createElement("div");
      row.className = "pe-question";
      const label = doc.createElement("label");
      label.textContent = (index + 1) + ". " + question;
      const area = doc.createElement("textarea");
      area.rows = 2;
      area.className = "pe-answer";
      area.setAttribute("aria-label", "问题 " + (index + 1) + " 的回答");
      row.appendChild(label);
      row.appendChild(area);
      panel.appendChild(row);
      inputs.push({ question, area });
    });
    const actions = doc.createElement("div");
    actions.className = "pe-actions";
    const go = doc.createElement("button");
    go.type = "button";
    go.className = "pe-primary";
    go.textContent = "继续增强";
    const cancel = doc.createElement("button");
    cancel.type = "button";
    cancel.textContent = "取消";
    actions.appendChild(go);
    actions.appendChild(cancel);
    panel.appendChild(actions);
    const note = doc.createElement("div");
    note.className = "pe-note";
    note.textContent = "原始输入保持不变；答案只用于本次增强，不会自动发送。";
    panel.appendChild(note);
    go.addEventListener("click", () => {
      const answered = inputs
        .map(({ question, area }) => ({ question, answer: String(area.value || "").trim() }))
        .filter((item) => item.answer);
      entry.clarified = entry.clarified.concat(answered);
      closePanel(target);
      runEnhance(target, trigger, entry);
    });
    cancel.addEventListener("click", () => {
      entry.clarified = [];
      closePanel(target);
      stateOf(trigger, "idle", "增强提示词");
    });
    const first = panel.querySelector(".pe-answer");
    if (first) first.focus({ preventScroll: true });
  }

  // 预览：增强期间用户改了草稿 → 不覆盖，展示结果让用户决定。
  function renderPreviewPanel(target, result, entry, trigger) {
    const panel = panelFor(target);
    if (!panel) return;
    panel.textContent = "";
    const title = doc.createElement("div");
    title.className = "pe-title";
    title.textContent = "增强完成 — 期间你修改了草稿，结果未自动替换";
    panel.appendChild(title);
    const preview = doc.createElement("pre");
    preview.className = "pe-preview";
    preview.textContent = String(result.enhanced_prompt || "");
    panel.appendChild(preview);
    const actions = doc.createElement("div");
    actions.className = "pe-actions";
    const apply = doc.createElement("button");
    apply.type = "button";
    apply.className = "pe-primary";
    apply.textContent = "替换为增强结果";
    const discard = doc.createElement("button");
    discard.type = "button";
    discard.textContent = "放弃";
    actions.appendChild(apply);
    actions.appendChild(discard);
    panel.appendChild(actions);
    apply.addEventListener("click", () => {
      entry.original = target.value;
      entry.enhanced = String(result.enhanced_prompt || "");
      entry.applied = true;
      fill(target, entry.enhanced, entry);
      closePanel(target);
      stateOf(trigger, "on", "还原原文（再点一次）");
      target.focus({ preventScroll: true });
    });
    discard.addEventListener("click", () => {
      closePanel(target);
      stateOf(trigger, "idle", "增强提示词");
    });
  }

  /* ---------------- 主流程 ---------------- */

  async function runEnhance(target, trigger, entry) {
    if (entry.busy) return;
    const text = String(target.value || "").trim();
    if (!text) {
      setTip(trigger, "先写点内容再增强");
      return;
    }
    closePanel(target);
    // 双向开关之一：当前显示的是增强版 → 还原成用户草稿。
    if (entry.applied && entry.original !== null) {
      fill(target, entry.original, entry);
      entry.applied = false;
      stateOf(trigger, "idle", "增强提示词");
      target.focus({ preventScroll: true });
      return;
    }
    // 草稿没变过，直接切回上次增强结果，不重复消耗一次模型请求。
    if (entry.enhanced && entry.original === target.value) {
      fill(target, entry.enhanced, entry);
      entry.applied = true;
      stateOf(trigger, "on", "还原原文（再点一次）");
      target.focus({ preventScroll: true });
      return;
    }

    entry.busy = true;
    stateOf(trigger, "busy", "正在增强…");
    const baseline = target.value;
    let result;
    try {
      result = await callEnhance(text, entry.clarified);
    } catch (error) {
      entry.busy = false;
      entry.clarified = [];
      stateOf(trigger, "idle", "增强失败：" + errorMessageOf(error) + "（点此重试）");
      return;
    }
    entry.busy = false;
    const status = String((result && result.status) || "").trim();
    if (status === "needs_clarification") {
      const questions = Array.isArray(result.clarifying_questions)
        ? result.clarifying_questions.map((item) => String(item || "").trim()).filter(Boolean).slice(0, 3)
        : [];
      if (!questions.length) {
        stateOf(trigger, "idle", "增强失败：返回了空的澄清问题（点此重试）");
        return;
      }
      stateOf(trigger, "clarify", "需要补充信息");
      renderClarifyPanel(target, questions, entry, trigger);
      return;
    }
    const enhanced = String((result && result.enhanced_prompt) || "").trim();
    if (status !== "ready" || !enhanced) {
      entry.clarified = [];
      stateOf(trigger, "idle", "增强失败：模型没有返回有效结果（点此重试）");
      return;
    }
    entry.clarified = [];
    if (target.value !== baseline) {
      // 增强期间草稿被改动：结果只做预览，不覆盖用户的新输入。
      stateOf(trigger, "preview", "增强完成（预览）");
      renderPreviewPanel(target, { enhanced_prompt: enhanced }, entry, trigger);
      return;
    }
    entry.original = baseline;
    entry.enhanced = enhanced;
    entry.applied = true;
    fill(target, enhanced, entry);
    const summary = String((result && result.change_summary) || "").trim();
    stateOf(trigger, "on", "还原原文（再点一次）" + (summary ? "｜" + summary.slice(0, 80) : ""));
    try { target.setSelectionRange(enhanced.length, enhanced.length); } catch (_) { /* 忽略 */ }
    target.focus({ preventScroll: true });
  }

  function enhance(target, trigger) {
    runEnhance(target, trigger, record(target));
  }

  function bind() {
    doc.addEventListener("click", (event) => {
      const trigger = event.target.closest && event.target.closest("[data-prompt-polish]");
      if (!trigger) return;
      event.preventDefault();
      const target = doc.getElementById(trigger.getAttribute("data-prompt-polish"));
      if (target) enhance(target, trigger);
    });
    doc.addEventListener("keydown", (event) => {
      if (!(event.ctrlKey || event.metaKey) || !event.shiftKey) return;
      if (String(event.key).toLowerCase() !== "l") return;
      const target = doc.getElementById("prompt");
      const trigger = doc.getElementById("promptPolishBtn");
      if (!target || !trigger) return;
      event.preventDefault();
      event.stopPropagation();
      enhance(target, trigger);
    }, true);
  }

  if (doc.readyState === "loading") {
    doc.addEventListener("DOMContentLoaded", bind, { once: true });
  } else {
    bind();
  }

  global.OwoPromptEnhance = {
    enhance,
    setTip,
    _test: { records, stateOf, currentModelId, enhanceTarget, buildRequestBody, legacyResultShape, errorMessageOf },
  };
})(typeof window !== "undefined" ? window : globalThis);
