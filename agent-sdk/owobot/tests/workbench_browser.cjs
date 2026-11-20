const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { app, BrowserWindow, nativeTheme } = require("electron");

const [, , directory, origin] = process.argv;
const deliveryThread = process.env.OWOBOT_SMOKE_DELIVERY_THREAD;
const launcherWorkspace = process.env.OWOBOT_SMOKE_LAUNCHER_WORKSPACE;
app.setPath("userData", path.join(directory, "chromium"));

async function exercise(window) {
  const forOrigin = new URL(origin).origin;
  const contents = window.webContents;
  // This smoke case uses an isolated Daemon and no network model or external service.
  contents.session.webRequest.onBeforeRequest(({ url }, respond) => {
    const remote = url.startsWith("http:") || url.startsWith("https:");
    respond({ cancel: remote && new URL(url).origin !== forOrigin });
  });
  await window.loadURL(origin + "/");
  await contents.executeJavaScript("new Promise(resolve => { const timer = setInterval(() => { if (shellHydrated) { clearInterval(timer); resolve(); } }, 50); })");
  if (launcherWorkspace) {
    const thread = await contents.executeJavaScript(
      "(" + checkLauncher.toString() + ")(" + JSON.stringify(launcherWorkspace) + ")",
    );
    fs.writeFileSync(path.join(directory, "launcher-thread.txt"), thread, "utf8");
    return;
  }
  const railPosition = () => contents.executeJavaScript(`[...document.querySelectorAll(
    "#sidebar .workbench-shortcuts > button, #codexSettingsEntry, #themeToggleSide, #codexUserCard"
  )].map(element => {
    const { x, y, width, height } = element.getBoundingClientRect();
    return [x, y, width, height];
  })`);
  const beforeOwobot = await railPosition();
  await contents.executeJavaScript('document.querySelector(\'[data-workbench-panel="owobot"]\').click()');
  if (!deliveryThread) {
    assert.deepEqual(await railPosition(), beforeOwobot, "Opening OwOBot moved the shared navigation");
  }
  if (deliveryThread) {
    const deliveryAction = (approve) => contents.executeJavaScript(
      "(" + applyDelivery.toString() + ")(" + JSON.stringify(deliveryThread) + "," + approve + ")",
    );
    await deliveryAction(false);
    await new Promise((resolve) => setTimeout(resolve, 300));
    fs.writeFileSync(path.join(directory, "owobot-review.png"), (await contents.capturePage()).toPNG());
    await deliveryAction(true);
    await new Promise((resolve) => setTimeout(resolve, 300));
    fs.writeFileSync(path.join(directory, "owobot-delivery.png"), (await contents.capturePage()).toPNG());
    return;
  }
  const themed = (mode) => contents.executeJavaScript("(" + setAppearance.toString() + ")(" + JSON.stringify(mode) + ")");
  await contents.executeJavaScript("(" + typeDraft.toString() + ")()");
  assert.deepEqual(await themed("light"), ["light", "rgb(255, 255, 255)", "flex", true]);
  const profileAction = (step) => contents.executeJavaScript(
    "(" + exerciseProfile.toString() + ")(" + JSON.stringify(step) + ")",
  );
  await profileAction("choose");
  contents.invalidate();
  await new Promise((resolve) => setTimeout(resolve, 200));
  fs.writeFileSync(path.join(directory, "owobot-bot-editor.png"), (await contents.capturePage()).toPNG());
  await profileAction("save");
  console.log("PASS desktop profile creation, appearance change and mention preserved the draft");
  await contents.executeJavaScript("(" + typeDraft.toString() + ")()");
  await captureAppearance(contents, "light");
  async function checkShell(narrow) {
    const layout = await contents.executeJavaScript(`(() => {
      const back = document.querySelector("#owobotReturn");
      const frame = document.querySelector(".owobot-panel-frame");
      const menu = document.querySelector("#menubar").getBoundingClientRect();
      const chat = frame.getBoundingClientRect();
      const doc = frame.contentDocument;
      return {
        narrow: innerWidth <= 860,
        back: !!back?.checkVisibility() && back.getBoundingClientRect().width > 0,
        hasTitleBar: !!document.querySelector("#owobotModeBar"),
        gap: Math.round(chat.top - menu.bottom),
        fits: doc.documentElement.scrollWidth <= frame.clientWidth + 1,
      };
    })()`);
    assert.deepEqual(layout, { narrow, back: true, hasTitleBar: false, gap: 0, fits: true });
  }
  await checkShell(false);
  const fullSize = window.getContentSize();
  window.setContentSize(580, 720);
  await contents.executeJavaScript("new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))");
  await checkShell(true);
  fs.writeFileSync(path.join(directory, "owobot-narrow.png"), (await contents.capturePage()).toPNG());
  window.setContentSize(...fullSize);

  assert.deepEqual(await themed("dark"), ["dark", "rgb(23, 24, 25)", "flex", true]);
  await captureAppearance(contents, "dark");
  nativeTheme.themeSource = "light";
  assert.deepEqual(await themed("system"), ["light", "rgb(255, 255, 255)", "flex", true]);
  nativeTheme.themeSource = "dark";
  assert.deepEqual(await themed(null), ["dark", "rgb(23, 24, 25)", "flex", true]);
  await contents.executeJavaScript("(" + sendDraft.toString() + ")()");
  assert.ok(await contents.executeJavaScript(`(() => {
    document.querySelector("#owobotReturn").click();
    return !document.body.classList.contains("owobot-mode") && !document.querySelector(".owobot-panel-frame");
  })()`), "The menu bar back button must restore the workbench");
  assert.deepEqual(await railPosition(), beforeOwobot, "Returning from OwOBot moved the shared navigation");

  for (const selector of ['[data-workbench-panel="home"]', "#codexSettingsEntry", "#sidebarHome"]) {
    await contents.executeJavaScript(`document.querySelector(${JSON.stringify(selector)}).click()`);
    assert.deepEqual(await railPosition(), beforeOwobot, `Navigation moved after opening ${selector}`);
    await contents.executeJavaScript('document.querySelector(\'[data-workbench-panel="owobot"]\').click()');
    assert.deepEqual(await railPosition(), beforeOwobot, `Navigation moved entering OwOBot from ${selector}`);
  }
  console.log("PASS shared navigation stayed in place across OwOBot, home, workbench, and settings");
  await contents.executeJavaScript("(" + exerciseWorkbench.toString() + ")()");
  await new Promise((resolve) => setTimeout(resolve, 200));
  fs.writeFileSync(path.join(directory, "workbench-overview.png"), (await contents.capturePage()).toPNG());
  console.log("PASS the workbench uses a project overview and OwOBot opens from the outer navigation");
}

async function exerciseWorkbench() {
  const open = (id) => document.querySelector('#panelNav [data-panel="' + id + '"]').click();
  document.querySelector('.workbench-shortcuts [data-workbench-panel="home"]').click();
  const nav = document.querySelector("#panelNav");
  for (const id of ["owobot", "automations", "custom-workflows"]) {
    if (nav.querySelector('[data-panel="' + id + '"]')) throw new Error("Duplicate workbench navigation: " + id);
    if (!document.querySelector('.workbench-shortcuts [data-workbench-panel="' + id + '"]')?.checkVisibility()) {
      throw new Error("Missing standalone navigation: " + id);
    }
  }
  const search = document.querySelector("#panelSearch");
  search.value = "OwOBot";
  search.dispatchEvent(new Event("input", { bubbles: true }));
  if ([...nav.querySelectorAll("button[data-panel]")].some(button => !button.hidden)) {
    throw new Error("OwOBot appeared in the workbench search");
  }
  search.value = "";
  search.dispatchEvent(new Event("input", { bubbles: true }));
  open("project-history");
  if (!document.querySelector("#ph-list") || document.querySelector("#ph-owobot")) {
    throw new Error("Team history must open without a duplicate OwOBot task list");
  }
  open("home");
  const overview = document.querySelector(".project-home");
  if (!overview?.querySelector(".home-inbox") || !overview.querySelector(".home-columns")) {
    throw new Error("Project overview did not show recent activity and pending items");
  }
}

async function exerciseProfile(step) {
  const { document: doc, frame, area, draft } = window.owobotSmoke;
  const byLabel = (label) => doc.querySelector('button[aria-label="' + label + '"]');
  const until = async (predicate, description) => {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      const result = predicate();
      if (result) return result;
      await new Promise((resolve) => setTimeout(resolve, 60));
    }
    throw new Error(description);
  };
  const name = "资料整理助手";
  const dialog = () => doc.querySelector(".owobot-profile-dialog[open]");
  if (step === "choose") {
    if (doc.querySelector(".owobot-delivery")) throw new Error("Empty chat reserved an unnecessary delivery column");
    byLabel("新建 Bot").click();
    const modal = await until(dialog, "Bot profile editor did not open");
    const input = modal.querySelector("input:not([type=radio])");
    Object.getOwnPropertyDescriptor(frame.contentWindow.HTMLInputElement.prototype, "value").set.call(input, name);
    input.dispatchEvent(new frame.contentWindow.Event("input", { bubbles: true }));
    modal.querySelector('input[value="grove"]').click();
    await until(() => modal.querySelector(".owobot-profile-preview [data-style='grove']"), "Appearance preview did not follow the choice");
    return;
  }
  dialog().querySelector('button[type="submit"]').click();
  const member = await until(() => byLabel("提及 " + name), "The created Bot did not appear in the project");
  if (!member.querySelector('[data-style="grove"]')) throw new Error("The saved Bot did not have its chosen appearance");
  byLabel("更换 " + name + " 的外观").click();
  const editor = await until(dialog, "Appearance editor did not open");
  editor.querySelector('input[value="tide"]').click();
  editor.querySelector('button[type="submit"]').click();
  await until(() => byLabel("提及 " + name)?.querySelector('[data-style="tide"]'), "Appearance change did not reach the roster");
  byLabel("提及 " + name).click();
  await until(() => area.value.includes("@" + name) && area.value.includes(draft), "Mention did not preserve the conversation draft");
}

async function checkLauncher(workspace) {
  const until = async (test, description) => {
    const deadline = Date.now() + 23_000;
    while (Date.now() < deadline) {
      const result = test();
      if (result) return result;
      await new Promise((resolve) => setTimeout(resolve, 80));
    }
    throw new Error(description + ": " + document.querySelector("#panelRoot")?.innerText.slice(0, 500));
  };
  const openPanel = (name) => {
    const button = document.querySelector('#panelNav [data-panel="' + name + '"]')
      || document.querySelector('[data-workbench-panel="' + name + '"]');
    if (!button) throw new Error("Missing desktop navigation for " + name);
    button.click();
  };
  // Folder switching clears the session when leaving its workspace.
  showHomeView();
  const input = document.querySelector("#workspace");
  input.value = workspace;
  input.dispatchEvent(new Event("change"));
  openPanel("project-launcher");
  if (document.querySelector("#pl-root")?.value !== workspace) throw new Error("Launcher used a different project");
  const objective = "查看第二个项目的报名资料。";
  document.querySelector("#pl-objective").value = objective;
  const create = await until(() => {
    const button = document.querySelector("#pl-create");
    return button && !button.disabled && button;
  }, "Launcher was unavailable");
  create.click();
  const frame = await until(() => document.querySelector(".owobot-panel-frame"), "Launcher did not open OwOBot");
  const thread = new URL(frame.src).searchParams.get("thread");
  if (!thread) throw new Error("Launcher did not open the created task");
  await until(() => frame.contentDocument?.querySelector(".glass-scroller")?.textContent.includes(objective),
    "The original objective was not sent to the project");
  const group = new URL(frame.src).searchParams.get("group");
  const createdConversation = await until(() => frame.contentDocument?.querySelector('.owobot-conversation[aria-current="page"]'),
    "The created task was not selected in OwOBot");
  frame.contentDocument.querySelector(".owobot-new-task").click();
  await until(() => {
    const selected = frame.contentDocument?.querySelector('.owobot-conversation[aria-current="page"]');
    return selected && selected !== createdConversation && !selected.textContent.includes(objective);
  }, "OwOBot did not switch to a different task");
  openPanel("project-history");
  if (document.querySelector("#ph-owobot")) throw new Error("Team history included a duplicate OwOBot list");
  openPanel("owobot");
  const reopened = await until(() => {
    const candidate = document.querySelector(".owobot-panel-frame");
    return candidate && new URL(candidate.src).searchParams.get("group") === group ? candidate : null;
  }, "The standalone OwOBot entry did not open the same project");
  const conversation = await until(() => [...(reopened.contentDocument?.querySelectorAll(".owobot-conversation") || [])]
    .find(button => button.textContent.includes(objective)), "OwOBot did not list the created task");
  conversation.click();
  await until(() => reopened.contentDocument?.querySelector(".glass-scroller")?.textContent.includes(objective),
    "OwOBot did not reopen the correct conversation from its list");
  return thread;
}

async function applyDelivery(thread, approve) {
  const until = async (pick, context) => {
    const end = Date.now() + 15_000;
    while (Date.now() < end) {
      const answer = pick();
      if (answer) return answer;
      await new Promise((resolve) => setTimeout(resolve, 80));
    }
    throw new Error(context);
  };
  const panel = () => {
    const doc = document.querySelector(".owobot-panel-frame")?.contentDocument;
    if (doc?.querySelector(".owobot-connection")) return null;
    return doc?.querySelector(".owobot-delivery");
  };
  const loaded = await until(() => {
    const candidate = panel();
    return candidate?.querySelector(".owobot-task-id")?.textContent === "owobot-" + thread ? candidate : null;
  }, "OwOBot did not reopen the reviewed task");
  const files = [...loaded.querySelectorAll(".owobot-delivery-file")];
  if (files.length !== 2 || !loaded.querySelector(".owobot-review")?.textContent.includes("共 12 人")) {
    throw new Error("The review and both delivery previews must be visible before approving");
  }
  for (const file of files) file.open = true;
  const button = loaded.querySelector(".owobot-delivery-actions button:first-child");
  if (button?.textContent.trim() !== "\u6279\u51c6\u5e76\u5e94\u7528") {
    throw new Error("The reviewed task did not offer the approval action");
  }
  if (!approve) return;
  button.click();
  await until(() => panel()?.querySelector(".owobot-delivery-status")?.textContent === "\u5df2\u5e94\u7528",
    "The desktop did not confirm that delivery was applied");
}

async function captureAppearance(contents, theme) {
  const position = await contents.executeJavaScript(`(() => {
    const frame = document.querySelector(".owobot-panel-frame");
    const bounds = frame.getBoundingClientRect();
    if (!frame.checkVisibility() || bounds.height < 360) return null;
    return {
      x: (bounds.left + bounds.width * 0.88) / innerWidth,
      y: (bounds.top + bounds.height * 0.54) / innerHeight,
    };
  })()`);
  assert.ok(position, "The collaboration panel must be visible for capture");
  for (let attempt = 0; attempt < 15; attempt++) {
    contents.invalidate();
    await new Promise((resolve) => setTimeout(resolve, 100));
    const shot = await contents.capturePage();
    if (shot.isEmpty()) continue;
    const { width, height } = shot.getSize();
    const offset = (Math.floor(height * position.y) * width + Math.floor(width * position.x)) * 4;
    const [blue, green, red] = shot.toBitmap().subarray(offset, offset + 3);
    const expected = theme === "dark" ? Math.max(red, green, blue) < 65 : Math.min(red, green, blue) > 235;
    if (expected) {
      fs.writeFileSync(path.join(directory, "owobot-" + theme + ".png"), shot.toPNG());
      return;
    }
  }
  throw new Error("Captured OwOBot panel did not paint the " + theme + " appearance");
}

async function typeDraft() {
  const deadline = Date.now() + 25_000;
  while (Date.now() < deadline) {
    const frame = document.querySelector(".owobot-panel-frame");
    const doc = frame?.contentDocument;
    const area = doc?.querySelector("textarea");
    if (area && !area.disabled && !doc.querySelector(".owobot-connection")) {
      const draft = "请查看项目目录，并列出可用的报名资料。";
      Object.getOwnPropertyDescriptor(frame.contentWindow.HTMLTextAreaElement.prototype, "value").set.call(area, draft);
      area.dispatchEvent(new frame.contentWindow.Event("input", { bubbles: true }));
      window.owobotSmoke = { document: doc, frame, area, draft };
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("OwOBot composer did not load from the desktop workbench: " +
    document.querySelector("#panelRoot")?.innerText?.slice(0, 200) + " / " +
    document.querySelector(".owobot-panel-frame")?.contentDocument?.body?.innerText?.slice(0, 200));
}

async function setAppearance(mode) {
  const pref = document.querySelector("#prefTheme");
  if (mode) {
    pref.value = mode;
    pref.dispatchEvent(new Event("change"));
  }
  const { document: doc, frame, area, draft } = window.owobotSmoke;
  const hostTheme = () => document.body.classList.contains("dark-theme") ? "dark" : "light";
  const scheme = mode === "system" || mode === null
    ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : mode;
  const end = Date.now() + 3_000;
  while (Date.now() < end && (doc.documentElement.dataset.owoTheme !== scheme || hostTheme() !== scheme)) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  const theme = doc.documentElement.dataset.owoTheme;
  const style = frame.contentWindow.getComputedStyle.bind(frame.contentWindow);
  const canvas = style(doc.querySelector(".owobot-workbench")).backgroundColor;
  const chat = style(doc.querySelector("main")).display;
  return [theme, canvas, chat, doc === frame.contentDocument && area.isConnected && area.value === draft];
}

async function sendDraft() {
  const { document: doc, frame, area, draft } = window.owobotSmoke;
  doc.querySelector(".owobot-new-task").click();
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const newArea = doc.querySelector("textarea");
    if (newArea && newArea !== area && !newArea.disabled) {
      Object.getOwnPropertyDescriptor(frame.contentWindow.HTMLTextAreaElement.prototype, "value")
        .set.call(newArea, draft);
      newArea.dispatchEvent(new frame.contentWindow.Event("input", { bubbles: true }));
      newArea.focus();
      newArea.dispatchEvent(new frame.contentWindow.KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      while (Date.now() < deadline) {
        if (doc.querySelector(".glass-scroller")?.textContent.includes(draft)) return;
        await new Promise((resolve) => setTimeout(resolve, 80));
      }
      throw new Error("Sent draft did not appear in the task");
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error("New group task was not selected");
}

app.whenReady().then(async () => {
  const window = new BrowserWindow({
    show: false, width: 1360, height: 940,
    webPreferences: { contextIsolation: true, backgroundThrottling: false, offscreen: true },
  });
  try {
    await exercise(window);
    console.log(launcherWorkspace
      ? "PASS project launcher first send and reopening from the standalone OwOBot conversation list in a second project"
      : deliveryThread ? "PASS desktop displayed both previews and the review, then applied with explicit user approval"
      : "PASS desktop light, dark and system appearance; composer draft and new task through the real OwOBot page");
    app.exit(0);
  } catch (error) {
    console.error(error);
    app.exit(1);
  }
});
