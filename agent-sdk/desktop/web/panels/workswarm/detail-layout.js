// Team detail navigation owns layout only. Original controls keep their nodes and API handlers.
(function (root) {
  "use strict";
  const sections = [
    ["conversation", "对话"], ["changes", "文件变更"], ["artifacts", "产物"], ["activity", "执行记录"]
  ];
  function bindTabs(tabs, panels, selected, onSelect) {
    function select(name, focus) {
      const index = tabs.findIndex(tab => tab.dataset.teamTab === name);
      if (index < 0) return false;
      tabs.forEach((tab, i) => {
        tab.setAttribute("aria-selected", String(i === index));
        tab.tabIndex = i === index ? 0 : -1;
        panels[i].hidden = i !== index;
      });
      if (focus) tabs[index].focus();
      if (onSelect) onSelect(name);
      return true;
    }
    tabs.forEach((tab, index) => {
      tab.addEventListener("click", () => select(tab.dataset.teamTab, false));
      tab.addEventListener("keydown", event => {
        if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
        let next;
        if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
        if (event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
        if (event.key === "Home") next = 0;
        if (event.key === "End") next = tabs.length - 1;
        if (next === undefined) return;
        event.preventDefault();
        select(tabs[next].dataset.teamTab, true);
      });
    });
    if (!select(selected, false)) select(sections[0][0], false);
    return { select };
  }
  function title(team, audit) {
    const start = (audit || []).find(entry => entry.event === "goal.start" && /^目标\s+/.test(entry.detail || ""));
    return start ? String(start.detail).replace(/^目标\s+/, "") : "团队运行";
  }
  const draftFields = [".owo-ws-review-reviewer", ".owo-ws-review-comment", ".owo-ws-rework-instruction"];
  const disclosures = [".owo-ws-review", ".owo-ws-rework", ".owo-ws-art-preview"];
  function captureArtifactDrafts(box) {
    if (!box || !box.querySelectorAll) return [];
    return Array.from(box.querySelectorAll("[data-art-row]")).map(row => ({
      id: row.getAttribute("data-art-row"),
      fields: draftFields.map(selector => { const node = row.querySelector(selector); return node ? node.value : null; }),
      open: disclosures.map(selector => !!row.querySelector(selector)?.open)
    }));
  }
  function restoreArtifactDrafts(box, drafts) {
    if (!box || !box.querySelectorAll) return;
    const rows = Array.from(box.querySelectorAll("[data-art-row]"));
    (drafts || []).forEach(draft => {
      const row = rows.find(node => node.getAttribute("data-art-row") === draft.id);
      if (!row) return;
      draftFields.forEach((selector, i) => {
        const node = row.querySelector(selector);
        if (node && draft.fields[i] !== null) node.value = draft.fields[i];
      });
      disclosures.forEach((selector, i) => {
        const node = row.querySelector(selector);
        if (node) node.open = draft.open[i];
      });
    });
  }
  function mount({ main, header, document: doc, selected, onSelect }) {
    if (!main || !header || !doc) return;
    const original = Array.from(main.children);
    const tablist = doc.createElement("div");
    tablist.className = "team-detail-tabs";
    tablist.setAttribute("role", "tablist");
    tablist.setAttribute("aria-label", "团队运行详情");
    const tabs = [], panels = {};
    sections.forEach(([name, label]) => {
      const tab = doc.createElement("button");
      tab.type = "button";
      tab.id = "team-tab-" + name;
      tab.dataset.teamTab = name;
      tab.textContent = label;
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-controls", "team-pane-" + name);
      tablist.appendChild(tab);
      tabs.push(tab);
      const pane = doc.createElement("div");
      pane.id = "team-pane-" + name;
      pane.className = "team-detail-pane";
      pane.setAttribute("role", "tabpanel");
      pane.setAttribute("aria-labelledby", tab.id);
      pane.tabIndex = 0;
      panels[name] = pane;
    });
    function group(label) {
      const details = doc.createElement("details");
      details.className = "team-more";
      details.dataset.teamSection = label;
      const summary = doc.createElement("summary");
      summary.textContent = label;
      details.appendChild(summary);
      return details;
    }
    const execution = group("执行过程");
    const manual = group("调整任务与人工协作");
    original.forEach(section => {
      if (section.id === "ws-d-conversation") panels.conversation.appendChild(section);
      else if (section.id === "ws-d-delivery" || section.querySelector("#ws-d-artifacts")) panels.artifacts.appendChild(section);
      else if (section.querySelector("#ws-d-changes")) panels.changes.prepend(section);
      else if (section.querySelector("#ws-d-workspace")) panels.changes.appendChild(section);
      else if (section.querySelector("#ws-d-members, #ws-d-dag, #ws-d-progress")) execution.appendChild(section);
      else if (section.querySelector("#ws-act-continue, #ws-h-task, #ws-x-task")) manual.appendChild(section);
      else if (section.classList.contains("owo-ws-sec")) panels.activity.appendChild(section);
    });
    panels.conversation.appendChild(execution);
    panels.conversation.appendChild(manual);
    const metadata = doc.createElement("div");
    metadata.className = "team-run-meta";
    ["ws-d-id", "ws-d-meta", "ws-d-diag"].forEach(id => {
      const node = header.querySelector("#" + id);
      if (node) metadata.appendChild(node);
    });
    panels.activity.appendChild(metadata);
    [["ws-act-continue", "继续任务"], ["ws-act-cancel", "停止任务"]].forEach(([id, label]) => {
      const button = main.querySelector("#" + id) || manual.querySelector("#" + id);
      if (button) { button.textContent = label; header.appendChild(button); }
    });
    main.appendChild(tablist);
    sections.forEach(([name]) => main.appendChild(panels[name]));
    return bindTabs(tabs, sections.map(([name]) => panels[name]), selected, onSelect);
  }
  const api = { mount, bindTabs, title, captureArtifactDrafts, restoreArtifactDrafts };
  root.OwoTeamDetailLayout = api;
  if (typeof module !== "undefined") module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
