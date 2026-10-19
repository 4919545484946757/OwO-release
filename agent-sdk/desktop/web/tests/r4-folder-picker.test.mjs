// R4-4 §4.4「文件夹必须走 Tauri 原生目录选择器」契约测试。
//
// §4.4 对这一项写得很死：控件 = Tauri 原生目录选择器，禁止做法 = 要求手输完整路径。
// 之所以要单独立测，是因为"看起来能用"的写法（把路径留在文本框里让用户敲）
// 在纯浏览器 dev 模式下也能跑通测试——但它绕过了壳侧的目录校验、持久化和
// 受控重启，真机上见过用户选到 `C:\Users\<user>` 整盘根目录。
// 因此断言分三层：
//   ① 行为层：VM 沙箱 + 假 Tauri IPC，选定/取消/失败三种终态都要有唯一的下游动作，
//      且取消必须是合法终态（不得当错误弹提示）；
//   ② 降级层：没有原生选择器（纯浏览器）时必须**显式禁用并说明原因**，
//      不得静默假装成功——同时保留开发用文本框，但不伪装成桌面路径；
//   ③ 接线层：index.html 侧栏与引导页两处入口都挂原生选择器，手输提示只在
//      非桌面分支存在；样式与脚本引入顺序正确。
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const pickerSource = readFileSync(join(WEB, "core", "folder-picker.js"), "utf8");
const indexHtml = readFileSync(join(WEB, "index.html"), "utf8");
const appSource = readFileSync(join(WEB, "app.js"), "utf8");
const appDomainSource = readFileSync(join(WEB, "app-domain.js"), "utf8");
const setupSource = readFileSync(join(WEB, "views", "setup-guide.view.js"), "utf8");
const cssSource = readFileSync(join(WEB, "style.css"), "utf8");

function makeElement(tag) {
  return {
    tagName: String(tag).toUpperCase(),
    value: "",
    title: "",
    placeholder: "",
    textContent: "",
    readOnly: false,
    disabled: false,
    type: "",
    dataset: {},
    attrs: {},
    listeners: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    getAttribute(name) {
      return this.attrs[name] === undefined ? null : this.attrs[name];
    },
    addEventListener(type, handler) {
      (this.listeners[type] = this.listeners[type] || []).push(handler);
    },
    click() {
      (this.listeners.click || []).forEach((fn) => fn({ type: "click" }));
    },
  };
}

function loadPicker(options) {
  const opts = options || {};
  const calls = [];
  const sandbox = {
    document: { createElement: makeElement },
    OwoWorkspaceDisplay: { alias: (root) => String(root).split(/[\\/]/).filter(Boolean).pop() },
  };
  if (opts.native) {
    sandbox.__TAURI_INTERNALS__ = {
      invoke: (command, payload) => {
        calls.push({ command, payload });
        return Promise.resolve(opts.result === undefined ? { ok: true, workspace: "T:\\proj\\demo", state: "starting", generation: 4 } : opts.result);
      },
    };
  }
  vm.createContext(sandbox);
  vm.runInContext(pickerSource, sandbox, { filename: "folder-picker.js" });
  sandbox.__calls = calls;
  return sandbox;
}

/** 让 attach 内部的 promise 链（含 .finally）跑完。 */
async function flush(times) {
  for (let i = 0; i < (times || 6); i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

test("桌面壳内选定目录：输入框转只读、按钮走原生 IPC、回调拿到壳侧权威路径", async () => {
  const sandbox = loadPicker({ native: true });
  const input = makeElement("input");
  const button = makeElement("button");
  input.value = "旧项目";
  const events = [];
  const attached = sandbox.OwoFolderPicker.attach(input, button, {
    onPicked: (workspace) => events.push(["picked", workspace]),
    onCanceled: () => events.push(["canceled"]),
    onError: (message) => events.push(["error", message]),
  });
  assert.equal(attached.native, true, "壳内必须识别为原生可用");
  assert.equal(input.readOnly, true, "§4.4：桌面版不得要求手输完整路径 → 输入框只读");
  assert.equal(input.attrs["aria-readonly"], "true", "只读必须同时告知辅助技术");
  assert.equal(button.textContent, "选择目录…");
  assert.equal(button.disabled, false);

  button.click();
  await flush();
  assert.equal(sandbox.__calls.length, 1, "点击必须且只调用一次壳侧命令");
  assert.equal(sandbox.__calls[0].command, "choose_project_directory");
  assert.deepEqual(events, [["picked", "T:\\proj\\demo"]]);
  assert.equal(input.value, "demo", "回填用别名显示，不回填完整盘符路径");
  assert.equal(input.dataset.path, "T:\\proj\\demo", "真实路径留在 dataset/title 供复制与诊断");
  assert.match(input.title, /当前项目：T:\\proj\\demo/);
});

test("工作区设置保留壳侧权威绝对路径，不用目录别名覆盖新会话路径", async () => {
  const sandbox = loadPicker({ native: true });
  const input = makeElement("input");
  const button = makeElement("button");
  sandbox.OwoFolderPicker.attach(input, button, { displayAlias: false });
  button.click();
  await flush();
  assert.equal(input.value, "T:\\proj\\demo");
  assert.equal(input.dataset.path, "T:\\proj\\demo");
});

test("取消是合法终态：不报错、不改值、按钮恢复可用", async () => {
  const sandbox = loadPicker({ native: true, result: { ok: false, canceled: true } });
  const input = makeElement("input");
  const button = makeElement("button");
  input.value = "旧项目";
  const events = [];
  sandbox.OwoFolderPicker.attach(input, button, {
    onPicked: () => events.push("picked"),
    onCanceled: () => events.push("canceled"),
    onError: () => events.push("error"),
  });
  button.click();
  await flush();
  assert.deepEqual(events, ["canceled"], "取消只能走 onCanceled，不得混成错误");
  assert.equal(input.value, "旧项目", "取消后工作区显示保持原值");
  assert.equal(button.disabled, false, "取消后必须能再次选择");
});

test("壳侧校验失败：错误文案原样上抛，不伪装成已设置", async () => {
  const sandbox = loadPicker({ native: true, result: { ok: false, error: "目录不存在或不可访问" } });
  const input = makeElement("input");
  const button = makeElement("button");
  input.value = "旧项目";
  const events = [];
  sandbox.OwoFolderPicker.attach(input, button, {
    onPicked: () => events.push("picked"),
    onError: (message) => events.push(["error", message]),
  });
  button.click();
  await flush();
  assert.deepEqual(events, [["error", "目录不存在或不可访问"]]);
  assert.equal(input.value, "旧项目");
});

test("IPC 抛异常（旧壳未注册命令）也必须落到 onError，不得留悬挂按钮", async () => {
  const sandbox = loadPicker({ native: true });
  sandbox.__TAURI_INTERNALS__.invoke = () => Promise.reject(new Error("No such command: choose_project_directory"));
  const input = makeElement("input");
  const button = makeElement("button");
  const events = [];
  sandbox.OwoFolderPicker.attach(input, button, { onError: (message) => events.push(message) });
  button.click();
  await flush();
  assert.equal(events.length, 1, "reject 必须被兜住并呈现");
  assert.match(events[0], /choose_project_directory/);
  assert.equal(button.disabled, false, "异常后按钮要恢复，否则用户被卡死");
});

test("纯浏览器降级：显式禁用原生入口并写明原因，不伪装成功", async () => {
  const sandbox = loadPicker({ native: false });
  const input = makeElement("input");
  const button = makeElement("button");
  const events = [];
  const attached = sandbox.OwoFolderPicker.attach(input, button, {
    onPicked: () => events.push("picked"),
    onError: (message) => events.push(["error", message]),
  });
  assert.equal(attached.native, false);
  assert.equal(button.disabled, true, "无原生选择器时按钮必须禁用，而不是点了没反应");
  assert.equal(button.dataset.owoFolderPicker, "browser");
  assert.match(button.title, /浏览器开发模式/, "降级原因必须写在可发现的位子上");
  assert.equal(input.readOnly, false, "浏览器 dev 模式保留手输（仅开发用）");
  assert.equal(events.length, 0);
  const result = await sandbox.OwoFolderPicker.pick();
  assert.equal(result.ok, false);
  assert.equal(result.unavailable, true, "非壳环境调用 pick 必须给出「不可用」结论");
});

test("normalize：代际/attempt 数字归一，缺失归 null（不得用 0 冒充未上报）", () => {
  const sandbox = loadPicker({ native: true });
  const ledger = sandbox.OwoFolderPicker;
  const ok = ledger.normalize({ ok: true, workspace: "T:\\a\\b", generation: "7", state: "starting" });
  assert.equal(ok.generation, 7, "壳侧数字可能是字符串形态，要归一");
  assert.equal(ledger.normalize({ ok: true, workspace: "T:\\a" }).generation, null);
  assert.equal(ledger.normalize({ ok: false, canceled: true }).canceled, true);
  assert.equal(ledger.normalize(null).ok, false, "空响应不得当成成功");
  assert.equal(ledger.normalize({ ok: true }).error, "设置工作区失败", "壳说成功却没回路径 → 判失败，不得凭空认为已设置");
});

test("接线：工具页的工作区选择与引导页均走原生选择器，所选路径进入新会话", () => {
  assert.match(indexHtml, /<div class="tool-actions">\s*<input id="workspace"[^>]*>[\s\S]*?<button type="button" id="workspaceBrowseBtn"/,
    "工作区输入与选择按钮必须同组");
  assert.match(indexHtml, /<script src="core\/folder-picker\.js"><\/script>/, "共享选择器必须被引入");
  assert.match(appSource, /async function pickDirectory\(\)[\s\S]*OwoFolderPicker\.pick\(window\)/,
    "工作区菜单和快捷键必须调用共享原生选择器");
  assert.match(appSource, /OwoFolderPicker\.attach\([\s\S]*\$\("workspace"\),\s*\$\("workspaceBrowseBtn"\)/,
    "工作区页必须将输入与按钮接入共享选择器");
  assert.match(appSource, /displayAlias:\s*false/, "工作区路径输入需保留真实路径供会话使用");
  assert.doesNotMatch(appSource, /api\("\/fs\/pick-directory"/,
    "工作区选择不得再绕过 Electron IPC 走旧 HTTP 入口");
  assert.match(indexHtml, /id="workspacePickerHint"/, "选择器需给浏览器预览说明明确状态");
  assert.match(appSource, /\$\("composerProjectBtn"\)\.addEventListener\("click",[\s\S]*openWorkspaceMenu/,
    "对话区项目入口打开统一工作区菜单");
  assert.match(appSource, /data-project-action="switch"[\s\S]*pickDirectory\(\)/,
    "工作区菜单继续提供原生目录选择入口");
  const projectMenu = /function openWorkspaceMenu\(trigger\) \{([\s\S]*?)\n\}/.exec(appSource)?.[1];
  assert.ok(projectMenu);
  assert.match(projectMenu, /OwoFolderPicker\?\.isNativeAvailable\(window\)/,
    "浏览器预览必须识别缺少本地目录能力的状态");
  assert.match(projectMenu, /data-project-action="create"[^\n]*actionDisabled/);
  assert.match(projectMenu, /data-project-action="switch"[^\n]*actionDisabled/);
  assert.match(projectMenu, /浏览器预览不能访问本地目录/);
  assert.match(appDomainSource, /async function newSession\(workspaceOverride\)[\s\S]*workspace,/, "新会话必须使用选定的工作区路径");
  assert.match(appSource, /\$\("workspace"\)\.addEventListener\("change", \(\) => \{[\s\S]*localStorage\.setItem\("owo\.workspace"/,
    "所选路径需持久为下次会话的默认值");

  assert.match(setupSource, /OwoFolderPicker\.isNativeAvailable\(global\)/, "首启引导必须判定原生选择器");
  assert.match(setupSource, /OwoFolderPicker\.attach\(pathInput, browseButton/, "首启选择走共享原生选择器");
  assert.match(setupSource, /pathHint\.textContent = "由原生目录选择器设定，不需要手输完整路径。"/);
  const nativeBranch = setupSource.indexOf("if (nativePicker)");
  const elseBranch = setupSource.indexOf("picker.addEventListener");
  assert.ok(nativeBranch > 0 && elseBranch > nativeBranch, "网页目录输入只作为浏览器开发回退");
  assert.ok(cssSource.includes(".tool-actions"), "工具页表单行样式必须存在");
});
test("脱敏红线：路径不进 title 之外的展示口径，别名列只显示目录名", () => {
  const sandbox = loadPicker({ native: true });
  assert.equal(sandbox.OwoFolderPicker.aliasFor("T:\\创新创业\\OwO-master\\agent-sdk"), "agent-sdk");
  // 无 OwoWorkspaceDisplay 时不得抛异常（壳早期或单测环境）。
  const bare = loadPicker({ native: true });
  delete bare.OwoWorkspaceDisplay;
  assert.equal(bare.OwoFolderPicker.aliasFor("T:\\x\\y"), "T:\\x\\y", "别名器缺失时原样返回，由 title 兜底");
});
