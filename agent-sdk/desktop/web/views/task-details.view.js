/* User-facing task detail. Reads the existing UI facts; owns no sessions or HTTP. */
(function (root) {
  "use strict";
  const phaseLabels = { thinking: "正在理解任务", reasoning: "正在整理思路", tool: "正在执行操作", answering: "正在整理结果", waiting: "等待你确认", asking: "需要补充信息", stopping: "正在停止" };
  function presentation(input = {}) {
    const task = (input.statusFacts || []).find(item => item.key === "task") || {};
    const backend = (input.statusFacts || []).find(item => item.key === "backend") || {};
    const pending = Number(input.pendingApprovals || 0);
    const needsInput = Boolean(input.pendingQuestion);
    const running = Boolean(input.running);
    const status = needsInput ? "需要补充信息" : pending ? "等待你确认" : running ? phaseLabels[input.phase] || "正在处理任务" : task.text === "空闲" || !task.text ? "等待任务" : task.text;
    const hint = needsInput ? "请先回答对话中的问题，再继续执行。" : pending ? "请检查操作范围与影响，再决定是否允许。" : running ? "进展来自实际执行记录；你可以随时停止。" : status === "等待任务" ? "写下目标，或先规划任务的范围与执行方式。" : /未验证|候选/.test(status) ? "回答已经结束。请检查结果和验证依据后再确认交付。" : /失败|阻断|未确认/.test(status) ? "任务尚未顺利结束，请查看对话中的说明与恢复入口。" : /验收通过/.test(status) ? "已有宿主验收记录，来源可在成果页查看。" : "详细结果和执行记录保留在对话中。";
    const title = input.title && !/^会话\s+[a-f0-9-]+$/i.test(input.title) ? input.title : input.request ? input.request.split("\n")[0] : "新的任务";
    return {
      title: String(title).slice(0, 100), status, hint,
      request: input.request || "还没有填写任务目标。",
      tone: needsInput || pending ? "warn" : task.tone || "muted",
      canReview: needsInput || pending > 0,
      connectionLabel: input.authFailed || backend.tone === "bad" ? "连接异常" : backend.tone === "warn" ? "检查连接" : "连接与设置",
      connectionTone: input.authFailed ? "bad" : backend.tone || "muted",
      steps: Array.isArray(input.steps) ? input.steps.slice(-5) : [],
      diffState: input.diffState || "idle",
    };
  }
  function setText(doc, id, value) {
    const el = doc.getElementById(id);
    if (el && el.textContent !== value) el.textContent = value;
  }
  function render(doc, input) {
    const p = presentation(input);
    setText(doc, "chatTaskTitle", p.title);
    setText(doc, "chatProjectAlias", input.workspace || "个人工作台");
    setText(doc, "taskStatusText", p.status);
    setText(doc, "taskDetailStatus", p.status);
    setText(doc, "taskDetailHint", p.hint);
    setText(doc, "taskDetailRequest", p.request);
    setText(doc, "connectionLabel", p.connectionLabel);
    const pill = doc.getElementById("taskStatusPill");
    if (pill) pill.dataset.tone = p.tone;
    // 运行态只来自宿主事实：running / 待确认 / 待回答；供样式表达"Agent 正在哪里工作"。
    const agentState = p.canReview ? "waiting" : input.running ? "running" : "idle";
    if (pill) pill.dataset.agent = agentState;
    if (doc.body && doc.body.dataset) doc.body.dataset.agent = agentState;
    const connection = doc.getElementById("connectionDetails");
    if (connection) connection.dataset.tone = p.connectionTone;
    const hasTask = Boolean(input.request || p.status !== "等待任务" || input.hasFileChanges || p.steps.length);
    const panel = doc.getElementById("right");
    if (panel) panel.dataset.mode = hasTask ? "task" : "idle";
    const visibility = { taskRequestSection: !input.request, taskActivitySection: !input.running && !p.steps.length, taskFilesSection: !hasTask, taskExportSection: !hasTask, taskDetailsStart: hasTask };
    for (const [id, hidden] of Object.entries(visibility)) {
      const section = doc.getElementById(id);
      if (section) section.hidden = hidden;
    }
    const review = doc.getElementById("taskDetailReview");
    if (review) { review.hidden = !p.canReview; review.textContent = input.pendingQuestion ? "回答问题" : "查看待确认操作"; }
    const steps = doc.getElementById("taskDetailSteps");
    if (steps) {
      const serialized = JSON.stringify(p.steps);
      if (steps.dataset.rendered !== serialized) {
        steps.dataset.rendered = serialized;
        steps.replaceChildren();
        for (const step of p.steps) {
          const li = doc.createElement("li");
          const label = doc.createElement("span"); label.textContent = step.label;
          const state = doc.createElement("small"); state.textContent = step.state;
          li.append(label, state); steps.appendChild(li);
        }
      }
      steps.hidden = !p.steps.length;
    }
    setText(doc, "taskDetailStepsEmpty", input.running ? "执行记录会随任务更新。" : "有操作发生时，这里会显示最近的执行步骤。");
    const empty = doc.getElementById("taskDetailStepsEmpty");
    if (empty) empty.hidden = p.steps.length > 0;
    return p;
  }
  function describeActions(actions) {
    const labels = { ui_change: "界面发生变化", focus: "切换了当前应用", click: "点击了界面", key: "使用了键盘", scroll: "滚动了页面", type: "输入了文字" };
    const unique = [...new Set((Array.isArray(actions) ? actions : []).map(action => labels[action] || "其他界面活动"))];
    return unique.length ? unique.slice(0, 4).join("、") : "暂未记录近期活动";
  }
  const api = { presentation, render, describeActions };
  root.OwoTaskDetails = Object.freeze(api);
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
