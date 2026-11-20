(function (root) {
  "use strict";

  let requestedTask = null;

  // 外壳的浅色/深色偏好映射到 OwOBot 的皮肤：daylight / midnight。
  // 两者同源，直接改 iframe 文档的 data-skin 即时生效，同时写回 localStorage
  // 让下次加载保持一致。
  const SKIN_BY_THEME = { dark: "midnight", light: "daylight" };
  let activeFrame = null;

  function shellSkin() {
    const dark = typeof document !== "undefined" && document.body?.classList?.contains("dark-theme");
    return dark ? SKIN_BY_THEME.dark : SKIN_BY_THEME.light;
  }

  function paintSkin(frame) {
    if (!frame) return;
    try {
      localStorage.setItem("omb-skin", shellSkin());
    } catch (_) {
      // 存储被禁用时只影响记忆，不影响本次显示。
    }
    try {
      const doc = frame.contentDocument;
      if (doc?.documentElement) doc.documentElement.dataset.skin = shellSkin();
    } catch (_) {
      // 跨源时无法直接改文档，下次加载会从 localStorage 读到同样的皮肤。
    }
  }

  if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
    document.addEventListener("owo:theme-change", () => {
      if (activeFrame?.isConnected) paintSkin(activeFrame);
    });
  }

  function desktopConnection(helpers) {
    const connection = helpers.selectedModelConnection();
    if (!connection) return null;
    if (connection.api_key) {
      throw new Error("请在 OwO 桌面端重新保存模型凭据，再打开 OwOBot。");
    }
    // 旧版本把自定义模型的地址留在浏览器存储里，但没有同步安全凭据引用。
    // 把这样的半连接发给核心只会得到 400。回退到桌面核心已经验证过的默认连接。
    if (!connection.api_key_env) return null;
    return connection;
  }

  // 核心用 400 拒绝这次模型连接（凭据引用失效、模型与连接不匹配等）。
  // 这类失败不该让整个协作工作台打不开：退回核心默认模型，面板照常可用，
  // 具体原因由面板提示，用户改好设置后重试即可换回原模型。
  function modelConnectionRefused(error) {
    const message = String((error && error.message) || "");
    return /模型连接不可用|模型与连接不匹配|模型名称无效|模型连接需使用桌面端凭据引用/.test(message);
  }

  async function openProject(helpers, workspace) {
    const request = {
      workspace: workspace || undefined,
      model: helpers.selectedModel(),
      connection: desktopConnection(helpers),
    };
    let opened;
    try {
      opened = await helpers.post("/owobot/open", request);
    } catch (error) {
      if (!modelConnectionRefused(error)) throw error;
      opened = await helpers.post("/owobot/open", { workspace: request.workspace });
      if (opened && typeof opened === "object") opened = { ...opened, modelFallback: true };
    }
    if (opened?.url !== "/owobot/" || typeof opened.groupId !== "string" || !opened.groupId) {
      throw new Error("无法读取 OwOBot 项目会话。");
    }
    return opened;
  }

  async function postToProject(group, path, body) {
    const url = new URL("/owobot/api" + path, location.origin);
    url.searchParams.set("owoProject", group);
    const response = await window.OwoApi.stream(url.pathname + url.search, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "无法完成 OwOBot 操作。");
    return result;
  }

  async function createTask(helpers, workspace, objective, requestId) {
    const opened = await openProject(helpers, workspace);
    const group = opened.groupId;
    const path = "/groups/" + encodeURIComponent(group);
    const { task } = await postToProject(group, path + "/tasks", { title: objective, requestId });
    if (task?.threadId !== requestId) throw new Error("无法核对 OwOBot 任务编号。");
    await postToProject(group, path + "/messages", { text: objective, threadId: requestId, sendId: requestId });
    return { thread: requestId, workspace };
  }

  // 回退提示：只说明"这次用的是核心默认模型、怎么恢复"，不遮挡工作台内容。
  function fallbackNote() {
    const note = document.createElement("div");
    note.className = "owobot-panel-note";
    note.setAttribute("role", "status");
    const text = document.createElement("span");
    text.textContent = "当前自定义模型的凭据在本地核心里不可用，已改用核心默认模型。要恢复原模型，请在「设置 → 额外模型连接」重新保存该模型的密钥，再点重试。";
    const close = document.createElement("button");
    close.type = "button";
    close.className = "owobot-panel-note-close";
    close.setAttribute("aria-label", "关闭提示");
    close.textContent = "×";
    close.addEventListener("click", () => note.remove(), { once: true });
    note.append(text, close);
    return note;
  }

  async function mount(host, helpers) {
    const task = requestedTask;
    requestedTask = null;
    return showProject(host, helpers, task);
  }

  async function showProject(host, helpers, task) {
    const state = document.createElement("div");
    state.className = "owobot-panel-state";
    state.innerHTML =
      '<div class="owobot-panel-loading" role="status" aria-label="正在启动 OwOBot">' +
      '<div class="panel-skeleton owobot-skeleton-title"></div>' +
      '<div class="panel-skeleton owobot-skeleton-row"></div>' +
      '<div class="panel-skeleton owobot-skeleton-row"></div>' +
      '<div class="panel-skeleton owobot-skeleton-row"></div>' +
      '<p class="owobot-panel-caption">正在启动 OwOBot…</p>' +
      '</div>';
    host.replaceChildren(state);

    try {
      const workspace = task?.workspace || helpers.snapshot?.().workspace;
      const opened = await openProject(helpers, workspace);
      if (!host.isConnected) return;
      const url = new URL(opened.url, location.origin);
      url.searchParams.set("group", opened.groupId);
      if (task?.thread) url.searchParams.set("thread", task.thread);
      const frame = document.createElement("iframe");
      frame.className = "owobot-panel-frame";
      frame.title = "OwOBot 协作工作台";
      frame.allow = "camera 'none'; microphone 'none'; display-capture 'none'; geolocation 'none'";
      frame.addEventListener("load", () => paintSkin(frame));
      paintSkin(frame);            // 首次加载前先写入皮肤，避免闪一下深色
      frame.src = url.href;
      activeFrame = frame;
      const note = opened.modelFallback ? fallbackNote() : null;
      host.replaceChildren(...(note ? [note, frame] : [frame]));
    } catch (error) {
      if (!host.isConnected) return;
      const message = error?.message || "无法启动 OwOBot";
      const box = document.createElement("div");
      box.className = "owobot-panel-error";
      box.setAttribute("role", "alert");
      const icon = document.createElement("span");
      icon.className = "owobot-panel-error-icon";
      icon.setAttribute("aria-hidden", "true");
      icon.innerHTML = '<svg viewBox="0 0 20 20" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="10" cy="10" r="7.5"/><path d="M10 6v4.5M10 13.5v.01"/></svg>';
      const text = document.createElement("div");
      text.className = "owobot-panel-error-text";
      const title = document.createElement("strong");
      title.textContent = "OwOBot 启动失败";
      const detail = document.createElement("p");
      detail.textContent = message;
      const hint = document.createElement("p");
      hint.className = "owobot-panel-error-hint";
      hint.textContent = modelConnectionRefused(error)
        ? "通常是自定义模型的凭据已失效：请在「设置 → 额外模型连接」重新保存该模型的密钥，或在模型选择里换一个核心默认模型。"
        : "请确认本地核心已连接并完成模型授权，然后重试。";
      text.append(title, detail, hint);
      const retry = document.createElement("button");
      retry.type = "button";
      retry.textContent = "重试";
      retry.className = "secondary";
      retry.addEventListener("click", () => showProject(host, helpers, task), { once: true });
      box.append(icon, text, retry);
      host.replaceChildren(box);
    }
  }

  root.OwoPanels = root.OwoPanels || {};
  root.OwoPanels.owobot = {
    title: "OwOBot",
    mount,
    createTask,
    open(task) { requestedTask = task; },
  };
})(window);
