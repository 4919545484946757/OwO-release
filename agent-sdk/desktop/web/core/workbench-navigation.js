(function (root) {
  "use strict";
  const groups = [
    { label: "项目与协作", items: [
      ["home", "项目首页", "最近任务、待办、环境与交付入口"],
      ["evidence", "产物与证据", "打开产物、查看版本与验证来源"],
      ["project-history", "项目与任务", "查看运行历史、交付物与下一步"],
      ["project-launcher", "新建任务", "定义目标、范围并预览执行方式"],
      ["workswarm", "团队协作", "跟踪角色、执行进度与交付结果"],
      ["action-center", "待我处理", "处理人工结果、评审与失败步骤"],
      ["goal", "目标与计划", "维护目标和执行计划"],
    ] },
    { label: "知识与技能", items: [
      ["notes", "项目笔记", "整理 Markdown 文档与项目资料"],
      ["memory", "记忆检索", "查询时间线与关联信息 · 实验能力"],
      ["team", "共享技能", "评审、导入与导出技能包"],
    ] },
    { label: "定时与流程", items: [
      ["automations", "定时任务", "创建提醒与定期执行任务"],
      ["workflow", "工作流", "查看、校验与运行已有流程"],
      ["custom-workflows", "自定义工作流", "用角色/依赖/工具/交付契约描述任意工作流 · AI 生成与社区共享"],
    ] },
    { label: "工具与权限", items: [
      ["plugin-market", "插件市场", "管理能力扩展"],
      ["permissions", "权限中心", "查看实际生效范围与待审批操作"],
      ["command", "工具命令", "按需使用高级操作入口"],
    ] },
    { label: "诊断与实验", items: [
      ["capabilities", "能力与成熟度", "查看已登记能力和使用边界"],
      ["eval", "模型评测", "检查模型能力与护栏"],
      ["product-eval", "单人 / 团队对比", "区分参考回放与真实模型评测"],
      ["observability", "运行指标", "查看资源、用量与服务指标"],
      ["diagnostics-ledger", "诊断记录", "检查核心连接与调用记录"],
      ["fleet", "协作设备", "查看设备控制面 · 实验能力"],
      ["about", "帮助与关于", "查看产品说明和使用指南"],
    ] },
  ];
  const entries = groups.flatMap(group => group.items.map(([id, title, description]) =>
    ({ id, title, description, group: group.label })));
  const primaryIds = ["home", "project-launcher", "project-history", "workswarm", "evidence", "action-center"];
  function sections(available) {
    const allowed = entry => !available || available.includes(entry.id);
    const primary = primaryIds.map(id => entries.find(entry => entry.id === id)).filter(entry => entry && allowed(entry));
    const secondary = [
      { label: "资料与记忆", ids: ["goal", "notes", "memory", "team"] },
      { label: "自动化", ids: ["automations", "workflow", "custom-workflows"] },
      { label: "工具与权限", ids: ["plugin-market", "permissions", "command"] },
      { label: "诊断与实验", ids: ["capabilities", "eval", "product-eval", "observability", "diagnostics-ledger", "fleet", "about"] },
    ].map(group => ({
      label: group.label,
      entries: group.ids.map(id => entries.find(entry => entry.id === id)).filter(entry => entry && allowed(entry)),
    })).filter(group => group.entries.length);
    return [{ label: "工作台", primary: true, entries: primary }, ...secondary];
  }
  function find(id) { return entries.find(item => item.id === id) || null; }
  function search(query, available) {
    const q = String(query || "").trim().toLowerCase();
    return entries.filter(item => (!available || available.includes(item.id)) &&
      (!q || [item.id, item.title, item.description, item.group].join(" ").toLowerCase().includes(q)));
  }
  root.OwoWorkbenchNavigation = Object.freeze({ groups, entries, primaryIds, sections, find, search });
  if (typeof module !== "undefined" && module.exports) module.exports = root.OwoWorkbenchNavigation;
})(typeof window !== "undefined" ? window : globalThis);
