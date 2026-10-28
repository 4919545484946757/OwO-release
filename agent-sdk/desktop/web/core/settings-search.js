// Settings discovery indexes labels and help copy only, never input values or secrets.
(function (root) {
  "use strict";
  function normalize(value) {
    return String(value || "").normalize("NFKC").toLocaleLowerCase().replace(/\s+/g, " ").trim();
  }
  function search(entries, query) {
    const words = normalize(query).split(" ").filter(Boolean);
    return words.length ? entries.filter(entry => words.every(word => normalize(entry.label + " " + entry.description + " " + entry.category).includes(word))) : [];
  }
  function collect(doc) {
    const entries = [];
    for (const tab of doc.querySelectorAll(".settings-tab")) {
      const tabId = tab.dataset.settingsTab;
      const title = tab.querySelector(".settings-page-title strong")?.textContent.trim() || tabId;
      const description = tab.querySelector(".settings-page-title span")?.textContent.trim() || "";
      entries.push({ tab: tabId, category: title, label: title, description, target: tab });
      for (const row of tab.querySelectorAll(".settings-row, .settings-block, .shortcut-table tbody tr")) {
        const label = row.querySelector(".settings-row-label strong, .settings-block-head strong, td")?.textContent.trim();
        if (!label) continue;
        const help = row.querySelector(".settings-row-label span, td + td")?.textContent.trim() || "";
        entries.push({ tab: tabId, category: title, label, description: help, target: row });
      }
    }
    return entries;
  }
  function init(doc, selectTab) {
    const input = doc.getElementById("settingsSearch"), results = doc.getElementById("settingsSearchResults");
    const list = doc.getElementById("settingsSearchList"), status = doc.getElementById("settingsSearchStatus");
    if (!input || !results || !list || !status) return null;
    const entries = collect(doc);
    let selected = doc.querySelector(".settings-tab:not([hidden])");
    let highlighted = null;
    function removeHighlight() {
      highlighted?.classList.remove("settings-search-target");
      highlighted = null;
    }
    function clear() {
      input.value = "";
      results.hidden = true;
      list.replaceChildren();
      if (selected) selected.hidden = false;
      removeHighlight();
    }
    function render() {
      const query = input.value.trim();
      if (!query) { clear(); return; }
      if (results.hidden) selected = doc.querySelector(".settings-tab:not([hidden])");
      for (const tab of doc.querySelectorAll(".settings-tab")) tab.hidden = true;
      removeHighlight();
      results.hidden = false;
      list.replaceChildren();
      const matches = search(entries, query);
      status.textContent = matches.length ? "找到 " + matches.length + " 项设置" : "没有找到相关设置，试试“模型”“主题”或“快捷键”。";
      for (const entry of matches) {
        const button = doc.createElement("button");
        button.type = "button";
        button.className = "settings-search-result";
        const category = doc.createElement("small"), label = doc.createElement("strong"), description = doc.createElement("span");
        category.textContent = entry.category;
        label.textContent = entry.label;
        description.textContent = entry.description;
        button.append(category, label, description);
        button.addEventListener("click", () => {
          clear();
          selectTab(entry.tab);
          selected = doc.querySelector('.settings-tab[data-settings-tab="' + entry.tab + '"]');
          highlighted = entry.target;
          highlighted.classList.add("settings-search-target");
          highlighted.scrollIntoView({ block: "center", behavior: "auto" });
          const control = highlighted.querySelector("input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled])");
          if (control) control.focus({ preventScroll: true });
          else { highlighted.setAttribute("tabindex", "-1"); highlighted.focus({ preventScroll: true }); }
        });
        list.appendChild(button);
      }
    }
    input.addEventListener("input", render);
    input.addEventListener("keydown", event => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === "Escape") { event.preventDefault(); clear(); }
      if (event.key === "ArrowDown") { event.preventDefault(); list.querySelector("button")?.focus(); }
    });
    return Object.freeze({ clear });
  }
  root.OwoSettingsSearch = Object.freeze({ init, collect, search, normalize });
})(typeof window !== "undefined" ? window : globalThis);
