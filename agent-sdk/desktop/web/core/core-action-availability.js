(function (global) {
  "use strict";

  var originalDisabled = new WeakMap();
  var originalTitles = new WeakMap();
  var available = false;
  var unavailableTitle = "连接并授权本地核心后可使用";

  function update(nextAvailable, root) {
    available = Boolean(nextAvailable);
    var scope = root || global.document;
    if (!scope || typeof scope.querySelectorAll !== "function") return;
    var controls = matching(scope, "[data-core-action]");
    ensurePanelHints(controls);
    for (var i = 0; i < controls.length; i += 1) {
      var control = controls[i];
      if (!originalDisabled.has(control)) {
        var startsBusy = control.getAttribute("data-core-busy") === "true" ||
          control.getAttribute("aria-busy") === "true";
        originalDisabled.set(control, startsBusy ? false : Boolean(control.disabled));
        originalTitles.set(control, control.getAttribute("title"));
      }
      var busy = control.getAttribute("data-core-busy") === "true" ||
        control.getAttribute("aria-busy") === "true";
      var disabled = !available || originalDisabled.get(control) || busy;
      control.disabled = disabled;
      control.setAttribute("aria-disabled", String(disabled));
      if (!available) control.setAttribute("title", unavailableTitle);
      else {
        var title = originalTitles.get(control);
        if (title == null) control.removeAttribute("title");
        else control.setAttribute("title", title);
      }
    }
    var hints = matching(scope, "[data-core-action-hint]");
    for (var j = 0; j < hints.length; j += 1) hints[j].hidden = available;
  }

  function ensurePanelHints(controls) {
    var document = global.document;
    if (!document || typeof document.createElement !== "function") return;
    for (var i = 0; i < controls.length; i += 1) {
      var control = controls[i];
      var panel = typeof control.closest === "function"
        ? control.closest("[data-panel]")
        : null;
      if (!panel || typeof panel.querySelector !== "function" ||
          typeof panel.insertBefore !== "function" ||
          panel.querySelector("[data-core-action-hint]")) continue;
      var hint = document.createElement("p");
      hint.className = "core-availability-hint";
      hint.hidden = available;
      hint.setAttribute("data-core-action-hint", "");
      hint.textContent = "连接并授权本地核心后可使用此页面的写入与管理操作。";
      panel.insertBefore(hint, panel.firstChild || null);
    }
  }

  function matching(scope, selector) {
    var found = [];
    if (scope && typeof scope.matches === "function" && scope.matches(selector)) found.push(scope);
    if (scope && typeof scope.querySelectorAll === "function") {
      var descendants = scope.querySelectorAll(selector);
      for (var i = 0; i < descendants.length; i += 1) found.push(descendants[i]);
    }
    return found;
  }

  global.OwoCoreActionAvailability = {
    update: update,
    isAvailable: function () { return available; },
    mark: function (control) {
      if (!control || typeof control.setAttribute !== "function") return;
      control.setAttribute("data-core-action", "true");
      update(available, control);
    },
    setBusy: function (control, busy) {
      if (!control || typeof control.setAttribute !== "function") return;
      control.setAttribute("data-core-action", "true");
      control.setAttribute("data-core-busy", busy ? "true" : "false");
      control.setAttribute("aria-busy", busy ? "true" : "false");
      update(available, control);
    },
  };
  update(false);

  // Panels render action controls after initial page setup. Gate marked controls as soon
  // as they enter the document so late-mounted features inherit the current core state.
  if (typeof global.MutationObserver === "function" && global.document && global.document.documentElement) {
    var observer = new global.MutationObserver(function (records) {
      for (var i = 0; i < records.length; i += 1) {
        var nodes = records[i].addedNodes || [];
        for (var j = 0; j < nodes.length; j += 1) update(available, nodes[j]);
      }
    });
    observer.observe(global.document.documentElement, { childList: true, subtree: true });
  }
})(window);
