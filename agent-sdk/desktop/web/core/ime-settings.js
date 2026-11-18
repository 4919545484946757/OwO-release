(function (global) {
  "use strict";
  const defaults = () => ({ filesystem: "workspace_read", command: "deny", network: "deny", persistence: "once", scopes: [] });
  function readConfig(doc) {
    const value = id => doc.getElementById(id).value;
    const custom = value("imeMode") === "custom";
    return {
      mode: value("imeMode"), workspace: doc.getElementById("imeWorkspace").dataset.path || null,
      permission_spec: custom ? {
        filesystem: value("imeFilesystem"), command: value("imeCommand"), network: value("imeNetwork"),
        persistence: "once", scopes: [],
      } : null,
      allowed_tools: doc.getElementById("imeAllTools").checked ? null :
        Array.from(doc.querySelectorAll("#imeToolList input:checked")).map(el => el.value),
    };
  }
  function create({ api, document: doc = global.document, onSaved = () => {} }) {
    const $ = id => doc.getElementById(id);
    // The compact desktop shell can omit the full IME settings card.  Keep the
    // module optional so its missing form does not abort app.js before the rest
    // of the workbench can install its click handlers.
    if (!$('imeAccessForm')) {
      return { refresh: async () => {}, connection: async () => {} };
    }
    let dirty = false, loading = false, connectionTimer = null;
    function message(text, error = false) {
      $("imeResult").textContent = text; $("imeResult").setAttribute("role", error ? "alert" : "status");
    }
    function changed() { dirty = true; message("有未保存的更改"); sync(); }
    function sync() {
      $("imeCustom").hidden = $("imeMode").value !== "custom";
      $("imeToolList").hidden = $("imeAllTools").checked;
    }
    async function connection() {
      if (!global.owo?.imeConnection) {
        $("imeConnectionStatus").textContent = "请在 Agent Desktop 中连接输入法";
        $("imeConnect").disabled = true; $("imeDisconnect").disabled = true; return;
      }
      try {
        const info = await global.owo.imeConnection.status();
        $("imeConnectionStatus").textContent = info.message;
        $("imeConnect").disabled = ["connecting", "connected"].includes(info.state);
        $("imeDisconnect").disabled = !["connecting", "connected"].includes(info.state);
      } catch (error) { $("imeConnectionStatus").textContent = String(error.message || error); }
    }
    async function refresh(force = false) {
      if (loading || (dirty && !force)) return;
      loading = true;
      try {
        const result = await api("/settings/ime"), config = result.config, spec = config.permission_spec || defaults();
        $("imeMode").value = config.mode;
        $("imeWorkspace").dataset.path = config.workspace || "";
        $("imeWorkspace").textContent = config.workspace || result.default_workspace;
        const profiles = { read_only: "只读", workspace: "工作区权限", auto_review: "自动审查", full_access: "完全访问", unrestricted: "不限制范围", custom: "自定义" };
        $("imeProfile").textContent = "当前 Agent 权限：" + (profiles[result.agent_profile] || result.agent_profile);
        $("imeFilesystem").value = spec.filesystem; $("imeCommand").value = spec.command;
        $("imeNetwork").value = spec.network;
        $("imeAllTools").checked = config.allowed_tools === null;
        const chosen = new Set(config.allowed_tools || []);
        const list = $("imeToolList"); list.replaceChildren();
        const tools = [...result.tools];
        for (const name of chosen) if (!tools.some(t => t.name === name)) tools.push({ name, description: "当前不可用（保留已选配置）" });
        for (const tool of tools) {
          const label = doc.createElement("label"), input = doc.createElement("input"), text = doc.createElement("span");
          input.type = "checkbox"; input.value = tool.name; input.checked = chosen.has(tool.name);
          text.textContent = tool.name + " · " + tool.description;
          label.append(input, text); list.append(label);
        }
        dirty = false; sync(); message(""); await connection();
      } catch (error) { message("加载失败：" + (error.message || error), true); }
      finally { loading = false; }
    }
    $("imeAccessForm").addEventListener("change", changed);
    $("imeResetWorkspace").addEventListener("click", () => {
      $("imeWorkspace").dataset.path = ""; $("imeWorkspace").textContent = "使用 Agent 当前工作目录";
      changed();
    });
    $("imePickWorkspace").addEventListener("click", async () => {
      try {
        const result = await api("/fs/pick-directory", { method: "POST", body: JSON.stringify({ initial: $("imeWorkspace").dataset.path || null }) });
        if (result.path) { $("imeWorkspace").dataset.path = result.path; $("imeWorkspace").textContent = result.path; changed(); }
      } catch (error) { message("选择目录失败：" + (error.message || error), true); }
    });
    $("imeReload").addEventListener("click", () => refresh(true));
    $("imeAccessForm").addEventListener("submit", async event => {
      event.preventDefault(); $("imeSave").disabled = true;
      try {
        const config = readConfig(doc);
        const result = await api("/settings/ime", { method: "POST", body: JSON.stringify(config) });
        dirty = false; onSaved(config);
        message(result.interrupted_sessions ? "已保存，正在执行的输入法任务已中止；新任务使用此设置。" : "已保存，对下一次输入法任务生效。");
      } catch (error) { message("保存失败：" + (error.message || error), true); }
      finally { $("imeSave").disabled = false; }
    });
    $("imeConnect").addEventListener("click", async () => {
      $("imeConnect").disabled = true;
      try { await global.owo.imeConnection.start(); }
      catch (error) { message("连接失败：" + (error.message || error), true); }
      await connection();
      clearInterval(connectionTimer);
      connectionTimer = setInterval(connection, 2000);
    });
    $("imeDisconnect").addEventListener("click", async () => {
      try { await global.owo.imeConnection.stop(); await connection(); }
      catch (error) { message("断开失败：" + (error.message || error), true); }
    });
    global.addEventListener?.("beforeunload", () => clearInterval(connectionTimer));
    return { refresh, connection };
  }

  // The unified ingress has its own explicit desktop switch. Legacy /settings/ime
  // controls do not apply to queued Runtime tasks and are not shown here.
  function createTaskAccess({ document: doc = global.document } = {}) {
    const $ = id => doc.getElementById(id);
    if (!$("imeTasksEnabled")) return { refresh: async () => {}, connection: async () => {} };
    let saving = false, timer = null;
    const access = () => global.owo?.imeTaskAccess;
    function showAccess(info) {
      if (!saving) $("imeTasksEnabled").checked = info.requestedEnabled === true;
      $("imeTasksStatus").textContent = info.restartRequired
        ? "已保存，需在桌面设置中手动重启核心后生效。"
        : info.activeEnabled === true
          ? "当前核心已按启用设置启动；任务提交后在工作台确认和处理。"
          : info.activeEnabled === false ? "当前核心已关闭输入法任务接收。"
            : "设置已保存；请启动由此桌面管理的核心。";
    }
    async function connection() {
      const bridge = global.owo?.imeConnection;
      if (!bridge) {
        $("imeConnectionStatus").textContent = "请在 Agent Desktop 中连接输入法";
        $("imeConnect").disabled = true; $("imeDisconnect").disabled = true;
        return;
      }
      try {
        const info = await bridge.status();
        $("imeConnectionStatus").textContent = info.message;
        $("imeConnect").disabled = ["connecting", "connected"].includes(info.state);
        $("imeDisconnect").disabled = !["connecting", "connected"].includes(info.state);
      } catch { $("imeConnectionStatus").textContent = "无法读取输入法连接状态，请稍后重试。"; }
    }
    async function refresh() {
      const host = access();
      $("imeTasksEnabled").disabled = !host || saving;
      if (!host) $("imeTasksStatus").textContent = "请在 Agent Desktop 中管理此开关。";
      else {
        try { showAccess(await host.status()); }
        catch { $("imeTasksEnabled").disabled = true; $("imeTasksStatus").textContent = "无法读取任务开关，请稍后重试。"; }
      }
      await connection();
    }
    $("imeTasksEnabled").addEventListener("change", async () => {
      if (!access() || saving) return;
      const value = $("imeTasksEnabled").checked;
      saving = true; $("imeTasksEnabled").disabled = true;
      try {
        const result = await access().setEnabled(value);
        saving = false; showAccess(result);
      } catch {
        saving = false;
        try { showAccess(await access().status()); } catch {}
        $("imeTasksStatus").textContent = "保存失败，请重新加载设置后重试。";
      } finally { saving = false; $("imeTasksEnabled").disabled = !access(); }
    });
    $("imeConnect").addEventListener("click", async () => {
      $("imeConnect").disabled = true;
      try { await global.owo.imeConnection.start(); }
      catch { $("imeConnectionStatus").textContent = "连接失败，请检查输入法后重试。"; return; }
      await connection();
      clearInterval(timer); timer = setInterval(connection, 2000);
    });
    $("imeDisconnect").addEventListener("click", async () => {
      try { await global.owo.imeConnection.stop(); await connection(); }
      catch { $("imeConnectionStatus").textContent = "断开失败，请稍后重试。"; }
    });
    global.addEventListener?.("beforeunload", () => clearInterval(timer));
    return { refresh, connection };
  }

  global.OwoImeSettings = { create, readConfig, createTaskAccess };
  if (typeof module !== "undefined") module.exports = { readConfig };
})(typeof window !== "undefined" ? window : globalThis);
