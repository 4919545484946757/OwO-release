(function (root) {
  "use strict";

  let requestedTask = null;

  async function openProject(helpers, workspace, selectModel = true) {
    const request = { workspace: workspace || undefined };
    if (selectModel) {
      request.model = helpers.selectedModel();
      request.connection = helpers.selectedModelConnection();
      if (request.connection?.api_key) {
        throw new Error("请在 OwO 桌面端重新保存模型凭据，再打开 OwOBot。");
      }
    }
    const opened = await helpers.post("/owobot/open", request);
    if (opened?.url !== "/owobot/" || typeof opened.groupId !== "string" || !opened.groupId) {
      throw new Error("无法读取 OwOBot 项目会话。");
    }
    return opened;
  }

  async function projectRequest(group, path, body, signal) {
    const url = new URL("/owobot/api" + path, location.origin);
    url.searchParams.set("owoProject", group);
    const response = await fetch(url, body === undefined
      ? { signal }
      : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), signal });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "无法完成 OwOBot 操作。");
    return result;
  }

  async function createTask(helpers, workspace, objective, requestId) {
    const opened = await openProject(helpers, workspace);
    const group = opened.groupId;
    const path = "/groups/" + encodeURIComponent(group);
    const { task } = await projectRequest(group, path + "/tasks", { title: objective, requestId });
    if (task?.threadId !== requestId) throw new Error("无法核对 OwOBot 任务编号。");
    await projectRequest(group, path + "/messages", { text: objective, threadId: requestId, sendId: requestId });
    return { thread: requestId, workspace };
  }

  async function history(helpers, workspace, signal) {
    const { groupId } = await openProject(helpers, workspace, false);
    signal.throwIfAborted();
    const { groups } = await projectRequest(groupId, "/bots?messages=0", undefined, signal);
    const group = groups.find(candidate => candidate.id === groupId);
    return (group?.tasks || []).filter(task => !task.archivedAt)
      .sort((a, b) => (b.lastThreadOrderAt ?? b.createdAt) - (a.lastThreadOrderAt ?? a.createdAt));
  }

  async function mount(host, helpers) {
    const task = requestedTask;
    requestedTask = null;
    return showProject(host, helpers, task);
  }

  async function showProject(host, helpers, task) {
    const status = document.createElement("p");
    status.className = "owobot-panel-status";
    status.setAttribute("role", "status");
    status.textContent = "正在启动 OwOBot…";
    host.replaceChildren(status);

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
      frame.src = url.href;
      host.replaceChildren(frame);
    } catch (error) {
      if (!host.isConnected) return;
      status.textContent = error?.message || "无法启动 OwOBot";
      const retry = document.createElement("button");
      retry.type = "button";
      retry.textContent = "重试";
      retry.className = "secondary";
      retry.addEventListener("click", () => showProject(host, helpers, task), { once: true });
      host.appendChild(retry);
    }
  }

  root.OwoPanels = root.OwoPanels || {};
  root.OwoPanels.owobot = {
    title: "OwOBot",
    mount,
    createTask,
    history,
    open(task) { requestedTask = task; },
  };
})(window);
