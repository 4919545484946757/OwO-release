const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { app, BrowserWindow, nativeTheme } = require("electron");

const [, , directory, origin] = process.argv;
const deliveryThread = process.env.OWOBOT_SMOKE_DELIVERY_THREAD;
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
  await contents.executeJavaScript('document.querySelector(\'[data-workbench-panel="owobot"]\').click()');
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
  await captureAppearance(contents, "light");
  assert.deepEqual(await themed("dark"), ["dark", "rgb(23, 24, 25)", "flex", true]);
  await captureAppearance(contents, "dark");
  nativeTheme.themeSource = "light";
  assert.deepEqual(await themed("system"), ["light", "rgb(255, 255, 255)", "flex", true]);
  nativeTheme.themeSource = "dark";
  assert.deepEqual(await themed(null), ["dark", "rgb(23, 24, 25)", "flex", true]);
  await contents.executeJavaScript("(" + sendDraft.toString() + ")()");
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
  const panel = () => document.querySelector(".owobot-panel-frame")?.contentDocument?.querySelector(".owobot-delivery");
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
    if (area && !area.disabled) {
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
    console.log(deliveryThread
      ? "PASS desktop displayed both previews and the review, then applied with explicit user approval"
      : "PASS desktop light, dark and system appearance; composer draft and new task through the real OwOBot page");
    app.exit(0);
  } catch (error) {
    console.error(error);
    app.exit(1);
  }
});
