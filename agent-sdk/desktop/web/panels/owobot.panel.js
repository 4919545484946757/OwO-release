(function (root) {
  "use strict";

  async function mount(host, helpers) {
    const status = document.createElement("p");
    status.className = "owobot-panel-status";
    status.setAttribute("role", "status");
    status.textContent = "正在启动 OwOBot…";
    host.replaceChildren(status);

    try {
      const opened = await helpers.post("/owobot/open", {});
      if (!host.isConnected) return;
      if (opened?.url !== "/owobot/" || typeof opened.groupId !== "string" || !opened.groupId) {
        throw new Error("Invalid OwOBot workbench response");
      }
      const url = new URL(opened.url, location.origin);
      url.searchParams.set("group", opened.groupId);
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
      retry.addEventListener("click", () => mount(host, helpers), { once: true });
      host.appendChild(retry);
    }
  }

  root.OwoPanels = root.OwoPanels || {};
  root.OwoPanels.owobot = { title: "OwOBot", mount };
})(window);
