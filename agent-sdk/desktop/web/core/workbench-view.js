// Single owner for the workbench's mutually exclusive chat, tools, and settings views.
(function (root) {
  "use strict";

  function resolveInitialRoute(hash, panelIds) {
    const id = String(hash || "").replace(/^#/, "");
    if (id === "settings") return { kind: "settings" };
    if (id && Array.isArray(panelIds) && panelIds.includes(id)) {
      return { kind: "panel", id };
    }
    return { kind: "chat" };
  }

  function replaceRouteHash(location, history, route, push) {
    if (!location || !history || typeof history.replaceState !== "function") {
      throw new TypeError("location and history.replaceState are required");
    }
    const id = String(route || "").replace(/^#/, "").trim();
    const hash = id ? "#" + id : "";
    if (location.hash === hash) return false;
    const write = push && typeof history.pushState === "function" ? "pushState" : "replaceState";
    history[write](null, "", String(location.pathname || "/") + String(location.search || "") + hash);
    return true;
  }

  const TOOL_GROUPS = ["workspace", "intelligence", "automation", "system"];

  function clearGroupFilters(body, groups = TOOL_GROUPS) {
    for (const group of groups) body.classList.remove(`tools-group-${group}`);
    for (const button of body.querySelectorAll("[data-codex-group]")) {
      button.classList.remove("active");
    }
    for (const button of body.querySelectorAll("[data-jump]")) {
      button.classList.toggle("active", button.dataset.jump === "all");
    }
  }

  function create(options) {
    const body = options.body;
    const toggleButton = options.toggleButton;
    const clearToolGroups = options.clearToolGroups || function () {};
    const scrollSettings = options.scrollSettings || function () {};
    const replaceRoute = options.replaceRoute || function () {};
    const leavePanel = options.leavePanel || function () {};

    function setToggleButton(expanded) {
      toggleButton.setAttribute("aria-expanded", String(expanded));
      toggleButton.textContent = expanded ? "收起工具与设置" : "显示工具与设置";
    }

    function showTools(visible, preserveRoute) {
      if (!visible) leavePanel();
      body.classList.remove("panel-open");
      body.classList.toggle("show-tools", visible);
      body.classList.toggle("tools-open", visible);
      body.classList.toggle("settings-open", false);
      setToggleButton(visible);
      if (!preserveRoute) replaceRoute(null);
    }

    function showSettings(visible) {
      leavePanel();
      body.classList.remove("panel-open");
      body.classList.toggle("settings-open", visible);
      if (visible) {
        body.classList.remove("tools-open");
        body.classList.add("show-tools");
        setToggleButton(true);
        scrollSettings();
        replaceRoute("settings");
        return;
      }
      body.classList.remove("tools-open", "show-tools");
      clearToolGroups();
      setToggleButton(false);
      replaceRoute(null);
    }

    function setRoute(route) {
      replaceRoute(route);
    }

    function toggleTools() {
      if (body.classList.contains("show-tools")) showSettings(false);
      else showTools(true);
    }

    return Object.freeze({ showTools, showSettings, toggleTools, setRoute });
  }

  root.OwoWorkbenchView = Object.freeze({ create, resolveInitialRoute, replaceRouteHash, clearGroupFilters });
})(window);
