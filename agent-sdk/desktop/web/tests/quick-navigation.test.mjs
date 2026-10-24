import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import fs from "node:fs";
const scope = {};
vm.runInNewContext(fs.readFileSync(new URL("../core/quick-navigation.js", import.meta.url), "utf8"), scope);
function node(id) {
  return { id, value: "", events: {}, attrs: {}, clicked: 0, scrolls: 0,
    classList: { toggle() {} },
    addEventListener(name, fn) { this.events[name] = fn; },
    setAttribute(name, value) { this.attrs[name] = value; },
    removeAttribute(name) { delete this.attrs[name]; },
    scrollIntoView() { this.scrolls++; },
    click() { this.clicked++; }
  };
}
function setup() {
  const input = node("input"), buttons = [node("notes"), node("workflow"), node("permissions")], status = {};
  scope.OwoQuickNavigation.bind(input, buttons, status, query => button => button.id.includes(query));
  const key = (key, options={}) => input.events.keydown({key, preventDefault(){}, ...options});
  return { input, buttons, status, key };
}
test("功能面板按上下键循环选择，Enter 打开当前项", () => {
  const { input, buttons, key } = setup();
  assert.equal(input.attrs["aria-activedescendant"], "notes");
  key("ArrowDown"); key("Enter");
  assert.equal(buttons[1].clicked, 1);
  key("ArrowUp"); key("ArrowUp");
  assert.equal(input.attrs["aria-activedescendant"], "permissions");
  assert.equal(buttons[2].scrolls, 1);
});
test("搜索后重置选中项，无匹配时不会执行隐藏功能", () => {
  const { input, buttons, status, key } = setup();
  input.value = "permission"; input.events.input();
  assert.equal(input.attrs["aria-activedescendant"], "permissions");
  key("Enter"); assert.equal(buttons[2].clicked, 1);
  input.value = "missing"; input.events.input();
  key("ArrowDown"); key("Enter");
  assert.equal(input.attrs["aria-activedescendant"], undefined);
  assert.equal(buttons[2].clicked, 1);
  assert.match(status.textContent, /没有找到/);
});
test("输入法候选确认不打开功能页", () => {
  const { buttons, key } = setup();
  key("Enter", {isComposing:true}); key("Enter", {keyCode:229});
  assert.equal(buttons[0].clicked, 0);
  key("Enter"); assert.equal(buttons[0].clicked, 1);
});
