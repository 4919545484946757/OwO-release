(function (root) {
  "use strict";
  const workbench = [
    { label: "工作台", primary: true, items: [
      ["home", "项目概览", "查看最近任务和待处理事项"],
      ["project-launcher", "新建任务", "为项目目录设定目标并开始协作"],
      ["action-center", "待我处理", "处理待确认事项、评审和失败任务"],
      ["project-history", "团队任务记录", "查找团队运行记录并复用任务目标"],
      ["evidence", "产物与证据", "查看交付文件、版本与验证记录"],
    ] },
    { label: "资料与技能", items: [
      ["goal", "目标与计划", "维护目标和执行计划"],
      ["notes", "项目笔记", "整理 Markdown 文档与项目资料"],
      ["team", "共享技能", "评审、导入与导出技能包"],
    ] },
    { label: "工具与权限", items: [
      ["agent-tasks", "运行详情", "查看任务状态、检查点与验证记录"],
      ["plugin-market", "插件市场", "管理能力扩展"],
      ["permissions", "权限中心", "查看实际生效范围与待审批操作"],
      ["command", "工具命令", "按需使用高级操作入口"],
    ] },
    { label: "诊断与实验", items: [
      ["observability", "运行指标", "查看资源、用量与服务指标"],
      ["diagnostics-ledger", "诊断记录", "检查核心连接与调用记录"],
      ["capabilities", "能力与成熟度", "查看已登记能力和使用边界"],
      ["memory", "记忆检索", "查询时间线与关联信息"],
      ["eval", "模型评测", "检查模型能力与护栏"],
      ["product-eval", "单人 / 团队对比", "区分参考回放与真实模型评测"],
      ["fleet", "协作设备", "查看已连接的协作设备"],
    ] },
  ].map(({ label, primary, items }) => ({
    label,
    primary,
    entries: items.map(([id, title, description]) => ({ id, title, description, group: label })),
  }));
  const standalone = [
    ["owobot", "OwOBot", "项目群聊、任务与成员接力"],
    ["automations", "定时任务", "创建提醒与定期执行任务"],
    ["custom-workflows", "工作流", "查看、校验、运行和编辑项目工作流"],
    ["about", "帮助与关于", "查看产品说明和使用指南"],
  ].map(([id, title, description]) => ({ id, title, description, group: "独立入口" }));
  const workbenchEntries = workbench.flatMap(group => group.entries);
  const entries = [...workbenchEntries, ...standalone];

  function sections(available) {
    return workbench.map(group => ({
      ...group,
      entries: group.entries.filter(entry => !available || available.includes(entry.id)),
    })).filter(group => group.entries.length);
  }
  function find(id) { return entries.find(item => item.id === id) || null; }
  function isWorkbenchPanel(id) { return workbenchEntries.some(item => item.id === id); }
  function search(query, available) {
    const q = String(query || "").trim().toLowerCase();
    return workbenchEntries.filter(item => (!available || available.includes(item.id)) &&
      (!q || [item.id, item.title, item.description, item.group].join(" ").toLowerCase().includes(q)));
  }
  root.OwoWorkbenchNavigation = Object.freeze({ entries, sections, find, search, isWorkbenchPanel });
  if (typeof module !== "undefined" && module.exports) module.exports = root.OwoWorkbenchNavigation;
})(typeof window !== "undefined" ? window : globalThis);
