// Keyboard selection for the feature palette; DOM nodes are owned by the modal lifecycle.
(function (root) {
  "use strict";
  function bind(input, buttons, status, match) {
    let visible = [], selected = -1;
    function select(index, scroll) {
      selected = visible.length ? (index + visible.length) % visible.length : -1;
      for (const button of buttons) {
        const active = button === visible[selected];
        button.classList.toggle("is-selected", active);
        button.setAttribute("aria-selected", String(active));
      }
      const active = visible[selected];
      if (active) {
        input.setAttribute("aria-activedescendant", active.id);
        if (scroll) active.scrollIntoView({ block: "nearest" });
      } else input.removeAttribute("aria-activedescendant");
    }
    function refresh() {
      const matches = match(input.value);
      visible = buttons.filter(button => {
        button.hidden = !matches(button);
        return !button.hidden;
      });
      select(0, false);
      status.textContent = visible.length ? visible.length + " 个可用功能" : "没有找到相关功能，试试“工作流”“权限”或“笔记”。";
    }
    input.addEventListener("input", refresh);
    input.addEventListener("keydown", event => {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        select(selected + (event.key === "ArrowDown" ? 1 : -1), true);
      } else if (event.key === "Enter") {
        event.preventDefault();
        visible[selected]?.click();
      }
    });
    for (const button of buttons) button.addEventListener("pointermove", () => {
      const index = visible.indexOf(button);
      if (index >= 0 && index !== selected) select(index, false);
    });
    refresh();
  }
  root.OwoQuickNavigation = Object.freeze({ bind });
})(typeof window !== "undefined" ? window : globalThis);
