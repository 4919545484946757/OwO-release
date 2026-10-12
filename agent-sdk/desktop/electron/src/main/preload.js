// 预加载：把主进程能力以最小面暴露给渲染层（contextIsolation 保持开启）。
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("owo", {
  pendingFeed: () => ipcRenderer.invoke('pet:feed:pending'),
  onOpenFeed: handler => {const fn=(_event,ref)=>handler(ref);ipcRenderer.on('pet:feed:open-run',fn);return ()=>ipcRenderer.removeListener('pet:feed:open-run',fn);},
  getCoreState: () => ipcRenderer.invoke("core:get"),
  downloadArtifact: id => ipcRenderer.invoke("artifact:download", id),
  teamFiles: { list: input => ipcRenderer.invoke("team:files:list", input), act: input => ipcRenderer.invoke("team:files:act", input) },
  setZoom: factor => ipcRenderer.invoke("view:zoom", factor),
  customModels: { list: () => ipcRenderer.invoke("models:list"), upsert: input => ipcRenderer.invoke("models:upsert", input), remove: id => ipcRenderer.invoke("models:remove", { id }), migrate: input => ipcRenderer.invoke("models:migrate", input), probe: input => ipcRenderer.invoke("models:probe", input) },
  restartCore: () => ipcRenderer.invoke("core:restart"),
  readConfig: () => ipcRenderer.invoke("config:read"),
  writeConfig: (config) => ipcRenderer.invoke("config:write", config),
  applyConfig: (config) => ipcRenderer.invoke("config:apply", config),
  revealConfig: () => ipcRenderer.invoke("config:reveal"),
  getWorkspace: () => ipcRenderer.invoke("workspace:get"),
  chooseWorkspace: () => ipcRenderer.invoke("workspace:choose"),
  openExternal: (url) => ipcRenderer.invoke("app:openExternal", url),
  onCoreState: (handler) => {
    const listener = (_event, state) => handler(state);
    ipcRenderer.on("core:state", listener);
    return () => ipcRenderer.removeListener("core:state", listener);
  },
});

// 桌宠页与壳之间唯一的三个动作。桌宠由核心静态托管（`GET /pet`），页面本身不带
// Node 权限，所以移动/隐藏/拉起工作台必须经主进程。缺了这层桥，桌宠只能"看得见
// 状态、做不了动作"，也就失去了「替用户省一步」的价值。
contextBridge.exposeInMainWorld("petShell", {
  readFeedText: () => ipcRenderer.invoke('pet:feed:read'),
  confirmTask: input => ipcRenderer.invoke('pet:task:confirm', input),
  saveFeed: ref => ipcRenderer.invoke('pet:feed:save', ref),
  presentFeed: state => ipcRenderer.invoke('pet:feed:activity', state),
  loadFeed: () => ipcRenderer.invoke('pet:feed:load'),
  openFeed: ref => ipcRenderer.invoke('pet:feed:open', ref),
  onFeed: handler => {const fn=(_event,text)=>handler(text);ipcRenderer.on('pet:feed:text',fn);return ()=>ipcRenderer.removeListener('pet:feed:text',fn);},
  onFeedRun: handler => {const fn=(_event,ref)=>handler(ref);ipcRenderer.on("pet:feed:run",fn);return ()=>ipcRenderer.removeListener("pet:feed:run",fn);},
  openApprovals: () => ipcRenderer.invoke("pet:feed:approvals"),
  forwardFeed: text => ipcRenderer.invoke("pet:feed:forward",text),
  moveBy: (dx, dy) => ipcRenderer.invoke("pet:actor:move", dx, dy),
  setVisible: (visible) => ipcRenderer.invoke("pet:actor:visible", visible),
  showWorkbench: () => ipcRenderer.invoke("pet:workbench"),
  // 真实可见性 + 归位：桌宠页用心跳对账（见 main.js pet:query 注释）。
  query: () => ipcRenderer.invoke("pet:actor:query"),
  resetPosition: () => ipcRenderer.invoke("pet:actor:reset"),
  // 偏好存壳侧文件，不存 localStorage（origin 绑随机端口，见 main.js petPrefPath）。
  getPref: () => ipcRenderer.invoke("pet:pref:get"),
  setPref: (patch) => ipcRenderer.invoke("pet:pref:set", patch),
});

// ADR-003：Tauri 兼容桥。
//
// desktop/web 对壳的调用面是 `window.__TAURI_INTERNALS__.invoke(command, args)`
// （部分代码也认 `window.__TAURI__.core.invoke`），共 14 个命令。壳由 Tauri 换成
// Electron 后这个全局对象消失，会让「模型配置保存」「原生目录选择器」「诊断页
// 重启核心/打开日志」全部静默降级成"非桌面环境"，而界面看起来毫无异常。
//
// 这里把同一形状暴露出来、后端接到主进程的 shell:invoke，命令实现见
// src/main/shell-commands.js —— desktop/web 因此一行都不用改，UI 与功能都与旧壳一致。
const tauriInvoke = (command, args) => ipcRenderer.invoke("shell:invoke", command, args);
contextBridge.exposeInMainWorld("__TAURI_INTERNALS__", { invoke: tauriInvoke });
contextBridge.exposeInMainWorld("__TAURI__", { core: { invoke: tauriInvoke } });
