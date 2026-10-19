// Agent 工作台域层：会话/审批/工具域数据/扩展面板与路由。
// 经典脚本在 app.js 前加载；这些函数在 boot 之后才调用，运行时复用 app.js
// 建立的 API、状态和渲染能力。此文件是上述域逻辑的唯一实现来源。

// 侧栏与会话附属面板使用递增代次，丢弃较早请求的迟到结果。
let sessionListRefreshGeneration = 0;
let sessionContextRefreshGeneration = 0;
let sessionDiffRefreshGeneration = 0;
const promptSessionStart = OwoSessionStart.createSessionStartGate();

// ---------- R8 存储与恢复（/storage/* + /server/status） ----------

async function refreshServerStatus() {
  try {
    const status = await api("/server/status");
    const gate = status.shutdown_gate || {};
    const storage = status.storage || {};
    const chips = [
      { label: "并发回合", value: `${gate.active_turns ?? 0} / ${gate.max_concurrent_turns ?? "?"}` },
      { label: "运行状态", value: gate.shutting_down ? "关闭中" : "运行中" },
      { label: "存储", value: storage.read_only ? "只读降级" : "正常" },
    ];
    if (storage.migration_warning) {
      chips.push({ label: "提示", value: storage.migration_warning, warn: true });
    }
    $("serverStatusPanel").innerHTML = chips
      .map(
        (chip) =>
          `<span class="status-chip${chip.warn ? " warn" : ""}"><span class="sub">${esc(chip.label)}</span><strong>${esc(chip.value)}</strong></span>`
      )
      .join("");
  } catch (error) {
    $("serverStatusPanel").innerHTML = `<span class="status-chip"><strong>${esc(friendlyError(error))}</strong></span>`;
  }
}

// 存储操作反馈：toast（即时）+ 账户页行内结果（留存细节）
function showStorageResult(text, kind = "ok") {
  const el = $("storageResult");
  el.textContent = text;
  el.hidden = false;
  el.classList.toggle("error", kind === "error");
}

async function storageBackup() {
  try {
    const result = await api("/storage/backup", { method: "POST", body: "{}" });
    const size = (result.size_bytes / 1024 / 1024).toFixed(2);
    showStorageResult(`备份完成（${size} MB），保存于：${result.saved_to}`);
    showToast(`备份完成（${size} MB）`, "ok");
    refreshServerStatus();
  } catch (error) {
    showStorageResult(friendlyError(error), "error");
    showToast(`备份失败：${error.message || error}`, "error");
  }
}

async function storageExport() {
  try {
    const data = await api("/storage/export", { method: "POST", body: "{}" });
    const counts = data.counts || {};
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `owo-export-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.json`;
    link.click();
    URL.revokeObjectURL(url);
    showStorageResult(
      `导出完成：${counts.sessions ?? 0} 会话 / ${counts.audit ?? 0} 审计 / ` +
        `${counts.notes ?? 0} 笔记 / ${counts.skills ?? 0} 技能 / ${counts.workflows ?? 0} 工作流（标准 JSON 已下载）`
    );
    showToast("导出完成，JSON 已开始下载", "ok");
  } catch (error) {
    showStorageResult(friendlyError(error), "error");
    showToast(`导出失败：${error.message || error}`, "error");
  }
}

async function storageRestore(file) {
  if (!file) return;
  let archive_b64;
  try {
    archive_b64 = await new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const raw = reader.result;
        const bytes = typeof raw === "string" ? atob(raw.split(",")[1] || "") : raw;
        resolve(bytes);
      };
      reader.onerror = () => reject(new Error("读取备份文件失败"));
      reader.readAsDataURL(file);
    });
  } catch (error) {
    showStorageResult(`读取备份文件失败：${error.message || error}`, "error");
    return;
  }
  confirmModal({
    title: "从备份恢复",
    message:
      "恢复会覆盖当前 settings / notes / skills / workflows（恢复前会自动再备份一份），index.db 需重启核心服务后生效。确认继续？",
    confirmText: "恢复",
    kind: "danger",
    onConfirm: async () => {
      try {
        const result = await api("/storage/restore", {
          method: "POST",
          body: JSON.stringify({ archive_b64 }),
        });
        showStorageResult(
          `恢复完成：${result.restored.length} 项已还原，${result.staged.length} 项暂存` +
            `${result.restart_required ? "（index.db 重启后生效）" : ""}；恢复前自动备份：${result.pre_backup}`
        );
        showToast("恢复完成", "ok");
        refreshServerStatus();
      } catch (error) {
        showStorageResult(friendlyError(error), "error");
        showToast(`恢复失败：${error.message || error}`, "error");
      }
    },
  });
}

async function storageClear() {
  openModal({
    title: "一键清空数据",
    body: `
      <p class="modal-message">将清空全部会话 / 审计 / 笔记 / 记忆 / 自动化（技能与工作流保留），<strong>不可恢复</strong>。</p>
      <label class="field-label">请输入 <code>CLEAR_ALL</code> 以确认
        <input id="clearToken" placeholder="CLEAR_ALL" autocomplete="off">
      </label>`,
    actions: [
      { label: "取消", kind: "ghost", onClick: ({ close }) => close() },
      {
        label: "确认清空",
        kind: "danger",
        onClick: async ({ close }) => {
          if ($("clearToken").value.trim() !== "CLEAR_ALL") {
            showToast("确认码不正确，已取消", "error");
            return;
          }
          close();
          try {
            const result = await api("/storage/clear", {
              method: "POST",
              body: JSON.stringify({ confirm: "CLEAR_ALL" }),
            });
            showStorageResult(
              `已清空：${(result.cleared || []).join("、")}；完整性校验：${result.integrity}`
            );
            showToast("数据已清空", "ok");
            refreshSessions(null);
            refreshAudit();
            refreshServerStatus();
          } catch (error) {
            showStorageResult(friendlyError(error), "error");
            showToast(`清空失败：${error.message || error}`, "error");
          }
        },
      },
    ],
  });
}

async function executePackage(pkg) {
  let variables = {};
  if (pkg.variables && pkg.variables.length) {
    const raw = await askText({
      title: "填写变量",
      label: `为技能包填写变量（JSON，如 {"value":"小李"}）：`,
      value: "{}",
      confirmText: "继续",
    });
    if (raw === null) return;
    try {
      variables = JSON.parse(raw || "{}");
    } catch (_) {
      addMessage("error", "变量 JSON 解析失败");
      return;
    }
  }
  const ok = await askConfirm({
    title: "确认执行",
    message: `确认执行技能包 ${pkg.name}？首次执行需要审批。`,
    confirmText: "执行",
  });
  if (!ok) return;
  let highRiskAck = false;
  if (pkg.sensitivity === "high") {
    const riskOk = await askConfirm({
      title: "高敏感操作",
      message: `⚠ ${pkg.name} 是高敏感技能包（可能操作支付/验证码等场景），再次确认执行？`,
      confirmText: "仍要执行",
      kind: "danger",
    });
    if (!riskOk) return;
    highRiskAck = true;
  }
  try {
    const report = await api("/learn/execute-package", {
      method: "POST",
      body: JSON.stringify({ name: pkg.name, variables, confirm: true, high_risk_ack: highRiskAck }),
    });
    if (report.ok) {
      addMessage("system", `技能包 ${pkg.name} 执行成功（${report.steps.length} 步）`);
    } else {
      addMessage("error", `技能包 ${pkg.name} 执行失败：${report.error || ""}`);
    }
    for (const step of report.steps) {
      addMessage("tool", `${step.status.toUpperCase()} ${step.node_id}（${step.action}）：${step.detail || "ok"}`, "执行步骤");
    }
  } catch (error) {
    addMessage("error", `执行失败：${error.message}`);
  }
}

async function refreshSuggestions() {
  try {
    const suggestions = await api("/proactive/suggestions");
    const list = $("suggestionList");
    list.innerHTML = "";
    for (const suggestion of suggestions) {
      const li = document.createElement("li");
      li.innerHTML = `<strong>${esc(suggestion.app_id)}</strong><span class="sub">${esc(suggestion.summary)}</span><span class="sub">${esc(suggestion.sequence.join(" → "))}</span>`;
      const actions = ["learn", "execute_once", "ignore", "mute_forever"];
      const labels = { learn: "保存为技能", execute_once: "执行一次", ignore: "忽略", mute_forever: "不再提示" };
      for (const action of actions) {
        const button = document.createElement("button");
        button.textContent = labels[action];
        button.addEventListener("click", async () => {
          try {
            const result = await api("/proactive/decide", {
              method: "POST",
              body: JSON.stringify({ suggestion_id: suggestion.id, action }),
            });
            await refreshSuggestions();
            if (action === "learn" && result && result.package) {
              addMessage(
                "system",
                `建议已沉淀为技能包 ${result.package.name}（变量：${(result.package.variables || []).join(",") || "无"}）`
              );
              await refreshPackages();
            }
          } catch (error) {
            addMessage("error", `建议处理失败：${error.message}`);
          }
        });
        li.appendChild(button);
      }
      list.appendChild(li);
    }
    if (!suggestions.length) list.innerHTML = '<li class="sub">暂无建议</li>';
  } catch (error) {
    $("suggestionList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

async function refreshAudit() {
  try {
    const params = new URLSearchParams({ limit: "50" });
    const eventFilter = $("auditEvent").value.trim();
    const toolFilter = $("auditTool").value.trim();
    const qFilter = $("auditQ").value.trim();
    if (eventFilter) params.set("event", eventFilter);
    if (toolFilter) params.set("tool", toolFilter);
    if (qFilter) params.set("q", qFilter);
    const entries = await api(`/audit?${params.toString()}`);
    const list = $("auditList");
    list.innerHTML = "";
    for (const entry of entries) {
      const li = document.createElement("li");
      const ok = entry.approved === undefined ? "" : entry.approved ? "✅" : "⛔";
      const tool = entry.tool ? ` [${esc(entry.tool)}]` : "";
      li.innerHTML = `<span class="sub">${esc((entry.ts || "").slice(0, 19).replace("T", " "))} ${esc(entry.event)} ${ok}${tool}</span><span class="sub">${esc(entry.detail || "")}</span>`;
      list.appendChild(li);
    }
    if (!entries.length) list.innerHTML = '<li class="sub">暂无审计记录</li>';
  } catch (error) {
    $("auditList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

// ---------- 会话 / 任务 ----------

function closeSessionMenus() {
  for (const menu of document.querySelectorAll(".owo-session-menu:not([hidden])")) {
    menu.hidden = true;
    menu.parentElement.querySelector(".owo-session-more")?.setAttribute("aria-expanded", "false");
  }
}

async function refreshSessions(selectId) {
  const generation = ++sessionListRefreshGeneration;
  const selectionVersion = state.selectionVersion;
  try {
    await refreshSessionsImpl(selectId, generation, selectionVersion);
  } catch (error) {
    if (generation !== sessionListRefreshGeneration) return;
    renderSessionListFailure(error, selectId);
  }
}

function sessionListFailureMessage(error) {
  const raw = String((error && error.message) || error || "");
  const status = Number((error && error.status) || ((raw.match(/(?:HTTP )?(\d{3})/) || [])[1]) || 0);
  if (/此浏览器未获得桌面授权|当前核心属于另一个桌面实例/.test(raw)) return raw;
  if (status === 404 || status === 405) {
    const hasDesktopBridge = typeof window !== "undefined" && Boolean(
      window.owo || window.__TAURI_INTERNALS__ ||
      (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === "function")
    );
    if (!hasDesktopBridge) {
      return "当前是浏览器预览，未通过 Electron 配对连接会话核心服务。请在 Electron 工作台中打开；如果你已在桌面端，请确认工作台服务已启动。";
    }
    return "核心服务未找到会话接口（HTTP " + status + "）。请重启 Electron 工作台；如果仍发生，请确认桌面端与核心服务版本一致。";
  }
  if (status === 401 || status === 403) return "本地授权暂不可用。请在 Electron 工作台重新连接后重试。";
  if (!status || /Failed to fetch|NetworkError|fetch failed|无法连接/i.test(raw)) {
    return "无法连接本地核心服务。请确认服务正在运行后重试。";
  }
  return "会话暂时无法读取。请重试；如果问题持续，请查看服务状态与诊断信息。";
}

function renderSessionListFailure(error, selectId) {
  const list = $("sessionList");
  if (!list) return;
  const row = document.createElement("li");
  row.className = "sub session-load-error";
  row.setAttribute("role", "alert");
  const message = document.createElement("span");
  message.textContent = `会话读取失败：${sessionListFailureMessage(error)}`;
  const details = document.createElement("details");
  details.className = "session-load-details";
  const summary = document.createElement("summary");
  summary.textContent = "技术详情";
  const raw = document.createElement("code");
  raw.textContent = String((error && error.message) || error || "未知错误");
  details.append(summary, raw);
  const retry = document.createElement("button");
  retry.type = "button";
  retry.className = "session-load-retry";
  retry.textContent = "重试";
  retry.setAttribute("aria-label", "重新加载会话列表");
  retry.addEventListener("click", async () => {
    if (retry.disabled) return;
    retry.disabled = true;
    retry.textContent = "正在重试…";
    await refreshSessions(selectId);
  });
  row.append(message, details, retry);
  list.replaceChildren(row);
}

async function refreshSessionsImpl(selectId, generation, selectionVersion) {
  const sessions = await api("/sessions");
  if (generation !== sessionListRefreshGeneration) return;
  state.sessionIndex = sessions;
  state.sessionsKnown = true;
  // 用户在请求期间切换了会话时，按实时状态标记选中项；列表刷新不拥有选中状态。
  const selectedSessionId = selectionVersion === state.selectionVersion
    ? (selectId || state.sessionId)
    : state.sessionId;
  const showArchived = $("showArchived").checked;
  const visible = sessions.filter((session) => showArchived || !session.archived);
  const visibleIds = new Set(visible.map((session) => session.id));
  const byParent = new Map();
  for (const session of visible) {
    const key = session.parent_id || "";
    if (!byParent.has(key)) byParent.set(key, []);
    byParent.get(key).push(session);
  }
  const isRoot = (session) => !session.parent_id || !visibleIds.has(session.parent_id);
  const list = $("sessionList");
  if (!list.dataset.sessionMenusBound) {
    list.dataset.sessionMenusBound = "true";
    document.addEventListener("click", (event) => {
      if (event.target.closest(".owo-session-menu, .owo-session-more")) return;
      closeSessionMenus();
    });
    list.addEventListener("keydown", (event) => {
      const menu = event.target.closest(".owo-session-menu");
      if (menu && ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        const items = [...menu.querySelectorAll('button[role="menuitem"]:not(:disabled)')];
        if (!items.length) return;
        const current = items.indexOf(document.activeElement);
        const next = event.key === "Home" ? 0
          : event.key === "End" ? items.length - 1
            : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
        event.preventDefault();
        items[next].focus();
        return;
      }
      if (event.key !== "Escape") return;
      const openMenu = list.querySelector(".owo-session-menu:not([hidden])");
      if (!openMenu) return;
      event.preventDefault();
      openMenu.hidden = true;
      const trigger = openMenu.parentElement.querySelector(".owo-session-more");
      trigger.setAttribute("aria-expanded", "false");
      trigger.focus();
    });
  }
  list.innerHTML = "";
  const renderSession = (session, depth, groupWorkspace) => {
    const li = document.createElement("li");
    li.dataset.groupWorkspace = groupWorkspace || "";
    // 运行中徽标：该会话仍有未完成回合（后台并行执行）。
    if (state.activeTurns.has(session.id)) {
      li.classList.add("running");
      li.title = (li.title ? `${li.title} ` : "") + "任务运行中";
    }
    if (session.id === selectedSessionId) li.classList.add("active");
    const badges = [];
    if (session.pinned) badges.push("📌");
    if (session.archived) badges.push("🗄");
    // Codex 风格：列表只显示标题 + 短时间，模型与完整时间收入 title 提示
    const raw = session.updated_at || session.created_at || "";
    const updated = raw ? raw.slice(5, 16).replace("T", " ") : "";
    li.title = `${session.model || "默认模型"} · ${raw.slice(0, 19).replace("T", " ")}`;
    li.innerHTML = `
      <div class="owo-session-row" style="margin-left:${depth * 14}px">
        <button type="button" class="owo-session-title" data-session-select aria-current="${session.id === selectedSessionId ? "page" : "false"}" title="打开会话：${esc(session.title || session.id.slice(0, 12))}">
          <strong>${esc(session.title || session.id.slice(0, 12))} ${badges.join(" ")}</strong>
          <span class="sub">${esc(updated)}</span>
        </button>
        <button type="button" class="owo-session-more" data-act="menu" aria-haspopup="menu" aria-expanded="false" aria-label="会话操作">⋯</button>
        <div class="owo-session-menu" role="menu" hidden>
          <button type="button" role="menuitem" data-act="rename">重命名</button>
          <button type="button" role="menuitem" data-act="pin">${session.pinned ? "取消置顶" : "置顶"}</button>
          <button type="button" role="menuitem" data-act="archive">${session.archived ? "取消归档" : "归档"}</button>
          <button type="button" role="menuitem" data-act="fork">分叉子会话</button>
          <button type="button" role="menuitem" data-act="rewind">回退</button>
          <button type="button" role="menuitem" data-act="redo">重做</button>
          <button type="button" role="menuitem" data-act="delete" class="danger">删除</button>
        </div>
      </div>`;
    const titleButton = li.querySelector("[data-session-select]");
    titleButton.addEventListener("click", async (event) => {
      event.stopPropagation();
      closeSessionMenus();
      await selectSession(session.id);
    });
    for (const button of li.querySelectorAll("button[data-act]")) {
      if (button.dataset.act === "menu") {
        button.addEventListener("click", (event) => {
          event.stopPropagation();
          const menu = li.querySelector(".owo-session-menu");
          const shouldOpen = menu.hidden;
          closeSessionMenus();
          if (shouldOpen) {
            menu.hidden = false;
            button.setAttribute("aria-expanded", "true");
            menu.querySelector("button")?.focus();
          }
        });
        button.addEventListener("keydown", (event) => {
          if (event.key !== "ArrowDown") return;
          event.preventDefault();
          if (button.getAttribute("aria-expanded") !== "true") button.click();
          else li.querySelector(".owo-session-menu button")?.focus();
        });
        continue;
      }
      button.addEventListener("click", async (event) => {
        event.stopPropagation();
        const act = button.dataset.act;
        li.querySelector(".owo-session-menu").hidden = true;
        const menuTrigger = li.querySelector(".owo-session-more");
        menuTrigger.setAttribute("aria-expanded", "false");
        menuTrigger.focus();
        try {
          if (act === "rename") {
            promptModal({
              title: "重命名会话",
              label: "输入新的会话标题：",
              value: session.title || "",
              placeholder: "例如：重构登录模块",
              confirmText: "保存",
              onConfirm: async (title) => {
                if (!title) return;
                await api(`/session/${session.id}/rename`, {
                  method: "POST",
                  body: JSON.stringify({ title }),
                });
                await refreshSessions(state.sessionId);
              },
            });
            return;
          } else if (act === "pin") {
            await api(`/session/${session.id}/pin`, {
              method: "POST",
              body: JSON.stringify({ pinned: !session.pinned }),
            });
          } else if (act === "archive") {
            await api(`/session/${session.id}/archive`, {
              method: "POST",
              body: JSON.stringify({ archived: !session.archived }),
            });
          } else if (act === "fork") {
            promptModal({
              title: "分叉子会话",
              label: "从第几条消息处分叉？留空表示最后一条。",
              placeholder: "0",
              confirmText: "分叉",
              onConfirm: async (raw) => {
                const parsed = parseInt(raw, 10);
                const message_index = Number.isFinite(parsed) ? parsed : 999999;
                const child = await api(`/session/${session.id}/fork`, {
                  method: "POST",
                  body: JSON.stringify({ message_index }),
                });
                addMessage("system", `已 fork 子会话 ${child.id}（可在列表中选择）`);
                await refreshSessions(state.sessionId);
              },
            });
            return;
          } else if (act === "delete") {
            // 只能归档不能删除会让长列表迟早爆掉（Codex 侧栏可删线程）。
            confirmModal({
              title: "删除会话",
              message: `删除后不可恢复：${session.title || session.id.slice(0, 12)}`,
              confirmText: "删除",
              kind: "danger",
              onConfirm: async () => {
                await api(`/session/${session.id}`, { method: "DELETE" });
                if (session.id === state.sessionId) {
                  state.sessionId = null;
                  localStorage.removeItem("owo.lastSession");
                  const view = $("messages").querySelector(
                    `.session-view[data-sid="${session.id}"]`
                  );
                  if (view) view.remove();
                  $("emptyState").classList.remove("hidden");
                }
                showToast("会话已删除", "ok");
                await refreshSessions(state.sessionId);
              },
            });
            return;
          } else if (act === "rewind") {
            await sessionUndo(session.id);
            return;
          } else if (act === "redo") {
            await sessionRedo(session.id);
            return;
          }
          await refreshSessions(state.sessionId);
        } catch (error) {
          addMessage("system", `操作失败：${friendlyError(error, { resource: true })}`);
        }
      });
    }
    list.appendChild(li);
    for (const child of byParent.get(session.id) || []) {
      renderSession(child, depth + 1, groupWorkspace);
    }
  };
  // Codex 风格：一级按项目（工作区）分桶，fork 父子树留在组内。
  // 会话记录本就带 workspace 字段，之前只在列表里裸铺，多项目下无法分辨。
  const groups = new Map();
  for (const session of visible) {
    if (!isRoot(session)) continue;
    const workspace = String(session.workspace || "").trim();
    const key = workspace.toLowerCase();
    if (!groups.has(key)) groups.set(key, { workspace, roots: [] });
    groups.get(key).roots.push(session);
  }
  for (const group of groups.values()) {
    // 项目分组可折叠：组头点击折叠/展开，展开状态按「工作区名」记忆在 localStorage，
    // 刷新后保持。多项目并行时默认全部展开（同屏可见），用户可自行收起。
    const groupId = "grp:" + (group.workspace || "").toLowerCase();
    const header = document.createElement("li");
    header.className = "codex-session-group";
    header.dataset.groupWorkspace = group.workspace || "";
    const collapsed = collapsedGroups.has(group.workspace || "");
    header.setAttribute("role", "button");
    header.setAttribute("tabindex", "0");
    header.setAttribute("aria-expanded", collapsed ? "false" : "true");
    header.title = (group.workspace || "该会话未记录工作区") + (collapsed ? "（点击展开）" : "（点击折叠）");

    const caret = document.createElement("span");
    caret.className = "codex-group-caret" + (collapsed ? " collapsed" : "");
    caret.setAttribute("aria-hidden", "true");
    const label = document.createElement("span");
    label.className = "codex-group-label";
    label.textContent = workspaceLabel(group.workspace);
    const count = document.createElement("span");
    count.className = "codex-group-count";
    count.textContent = String(group.roots.length);
    header.append(caret, label, count);

    const children = [];
    for (const session of group.roots) children.push(session);

    const toggle = () => {
      const nowCollapsed = !collapsedGroups.has(group.workspace || "");
      if (nowCollapsed) collapsedGroups.add(group.workspace || "");
      else collapsedGroups.delete(group.workspace || "");
      localStorage.setItem(
        "owo.collapsedGroups",
        JSON.stringify([...collapsedGroups])
      );
      // 只重渲染列表，保留当前会话与滚动位置。
      refreshSessions(state.sessionId);
    };
    header.addEventListener("click", toggle);
    header.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        toggle();
      }
    });
    list.appendChild(header);
    for (const session of children) renderSession(session, 0, group.workspace);
    if (collapsed) {
      // 折叠：隐藏本组标题之后、下一个组标题之前的全部条目（含 fork 子树）。
      let node = header.nextElementSibling;
      while (node && !node.classList.contains("codex-session-group")) {
        const next = node.nextElementSibling;
        node.classList.add("hidden");
        node = next;
      }
    }
  }
  if (!list.children.length) list.innerHTML = '<li class="sub">暂无会话</li>';
  filterSessionList();
}

/// 工作区显示名：取路径最后一段（Windows 反斜杠与 POSIX 斜杠都支持）。
function workspaceLabel(workspace) {
  const raw = String(workspace || "").trim().replace(/[\\/]+$/, "");
  if (!raw) return "未指定工作区";
  const parts = raw.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || raw;
}

// 会话列表搜索过滤（Codex 侧栏搜索框）
//
// 注意与折叠态的关系：搜索是"临时看全部"，因此**搜索期间忽略折叠**——
// 否则用户会抱怨"明明折叠了却又冒出来"。query 非空时按搜索结果判定可见性，
// query 清空时恢复折叠态（由 refreshSessions 重渲染时统一处理）。
function filterSessionList() {
  const input = $("sessionSearch");
  const query = (input.value || "").trim().toLowerCase();
  const items = [...$("sessionList").querySelectorAll("li")];
  for (const li of items) {
    if (li.classList.contains("codex-session-group")) continue;
    // 搜索期间：命中的显示，未命中的隐藏（忽略折叠标记）。
    // 非搜索期间：不动 hidden，折叠态由 refreshSessions 负责。
    if (query) li.classList.toggle("hidden", !li.textContent.toLowerCase().includes(query));
    else li.classList.toggle("hidden", collapsedGroups.has(li.dataset.groupWorkspace || ""));
  }
  // 组标题：搜索后没有可见子项的组一并隐藏，不留空标题。
  for (const li of items) {
    if (!li.classList.contains("codex-session-group")) continue;
    let node = li.nextElementSibling;
    let anyVisible = false;
    while (node && !node.classList.contains("codex-session-group")) {
      if (!node.classList.contains("hidden")) {
        anyVisible = true;
        break;
      }
      node = node.nextElementSibling;
    }
    li.classList.toggle("hidden", Boolean(query) && !anyVisible);
    li.setAttribute("aria-expanded", String(query ? anyVisible : !collapsedGroups.has(li.dataset.groupWorkspace || "")));
    li.querySelector(".codex-group-caret")?.classList.toggle("searching", Boolean(query));
  }
  // 搜索时把组标题的折叠箭头显示成"展开"态，避免误导（内容其实已强制展开）。
  if (query) {
    for (const li of items) {
      if (!li.classList.contains("codex-session-group")) continue;
      const caret = li.querySelector(".codex-group-caret");
      if (caret) caret.classList.add("searching");
      li.setAttribute("aria-expanded", "true");
    }
  }
}

/// 回退一个回合：把会话截断到最后一条 user 消息之前（连带其后的助手回复）。
/// 后端 rewind 需要显式 keep 值，这里从会话详情推导，避免让用户手填条数。
async function sessionUndo(sessionId) {
  const id = sessionId || state.sessionId;
  if (!id) {
    showToast("请先选择会话", "error");
    return;
  }
  const detail = await api(`/session/${id}`);
  const messages = detail.messages || [];
  let lastUser = -1;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i].role === "user") {
      lastUser = i;
      break;
    }
  }
  if (lastUser < 0) {
    showToast("没有可回退的回合", "error");
    return;
  }
  const result = await api(`/session/${id}/rewind`, {
    method: "POST",
    body: JSON.stringify({ keep: lastUser }),
  });
  showToast(`已回退 ${result.removed || 0} 条消息`, "ok");
  if (id === state.sessionId) await selectSession(id);
  else await refreshSessions(state.sessionId);
}

/// 重做：恢复最近一次回退掉的回合。
async function sessionRedo(sessionId) {
  const id = sessionId || state.sessionId;
  if (!id) {
    showToast("请先选择会话", "error");
    return;
  }
  const result = await api(`/session/${id}/redo`, { method: "POST" });
  showToast(`已恢复 ${result.restored || 0} 条消息`, "ok");
  if (id === state.sessionId) await selectSession(id);
  else await refreshSessions(state.sessionId);
}

function restoreSessionFacts(sessionId, detail) {
  const facts = window.OwoSessionFacts.fromDetail(detail);
  state.completedTurnFacts.set(sessionId, facts);
  if (detail.active_turn_id && !state.activeTurns.has(sessionId)) resumeHostTurn(sessionId, detail);
  if (state.sessionId === sessionId && currentTurn()) { startRunStatus(); updateComposerRunning(); }
  window.OwoStatusBar?.repaint();
  return facts;
}
function renderHistoricalCompletion(sessionId, facts) {
  if (!facts.historyAvailable || !facts.terminal || facts.outcome === "running") return;
  const node = document.createElement("section");
  node.className = "historical-completion";
  const summary = window.OwoCompletionRecord.summarize(facts.lastCompletionRecord);
  const usage = window.OwoCompletionRecord.usageFromTurnStats(facts.stats);
  node.textContent = [summary?.compact || "宿主终态：" + facts.outcome, "实际模型：" + (facts.lastExecutionModel || "未知"), usage?.text || "用量明细未返回"].join(" · ");
  if (["failed","cancelled","interrupted"].includes(facts.outcome)) node.appendChild(turnRecoveryActions(sessionId, state.lastSessionPrompts.get(sessionId) || "", facts.outcome));
  sessionView(sessionId).appendChild(node);
}
function resumeHostTurn(sessionId, detail) {
  const turnId = detail.active_turn_id;
  const temp = { tools: 0, failed: 0, modelCalls: 0, startedAt: Date.parse(detail.activity?.started_at || "") || Date.now(), files: new Set(), server: null };
  let text = "", bubble = null, terminal = null;
  const entry = { remote: true, turnId, startedAt: temp.startedAt, phase: detail.activity?.phase || "thinking", controller: new AbortController() };
  state.activeTurns.set(sessionId, entry);
  const observer = window.OwoTurnRecovery.observe({
    sessionId, turnId, getPage: (path, options) => window.OwoApi.get(path, options),
    visible: () => !uiHidden() && state.sessionId === sessionId,
    onPage(records) {
      const previousTarget = writeTargetSid, previousTurn = state.turn;
      writeTargetSid = sessionId; state.turn = temp;
      try {
        for (const record of records) {
          const value = record.payload;
          if (value.type === "token_delta") {
            text = (text + String(value.delta || "")).slice(-1048576);
            if (!bubble) bubble = addMessage("assistant", "");
            bubble.innerHTML = renderMarkdown(text); followScroll();
          } else if (value.type === "final") {
            text = value.text || text; if (!bubble) bubble = addMessage("assistant", "");
            bubble.innerHTML = renderMarkdown(text); bindCopyButtons(bubble);
          } else if (value.type === "tool_use") { entry.phase = "tool"; entry.tool = value.tool; pushToolUse(value); }
          else if (value.type === "tool_result") { pushToolResult(value); entry.phase = "thinking"; }
          else if (value.type === "progress") { entry.phase = "thinking"; if (value.message === "模型调用") temp.modelCalls++; }
          else if (value.type === "user_question") showQuestionCard(value);
          else if (value.type === "user_answered") markQuestionAnswered(value);
          else if (["turn_stats", "turn_failed"].includes(value.type)) terminal = value;
        }
      } finally { writeTargetSid = previousTarget; state.turn = previousTurn; }
      if (state.sessionId === sessionId) setRunPhase(entry.phase, entry.tool ? "正在执行：" + entry.tool : "正在恢复宿主输出");
    },
    async onEnd(page) {
      if (state.activeTurns.get(sessionId) !== entry) return;
      state.activeTurns.delete(sessionId);
      if (terminal) rememberTurnFacts(sessionId, terminal);
      else state.completedTurnFacts.set(sessionId, { outcome: "interrupted", turnId, lastExecutionModel: "", lastCompletionRecord: null });
      if (state.sessionId === sessionId) {
        stopRunStatus(); updateComposerRunning(); window.OwoStatusBar?.repaint();
        if (terminal?.type === "turn_failed") {
          const previous = writeTargetSid; writeTargetSid = sessionId;
          showTurnFailure(terminal.message || "宿主报告回合未完成"); writeTargetSid = previous;
        }
        await selectSession(sessionId, true);
      }
    },
    onError() { if (state.sessionId === sessionId) setRunPhase("waiting", "连接暂不可用，保留任务并等待重连"); }
  });
  entry.observer = observer;
}
function turnRecoveryActions(sessionId, prompt, reason) {
  const row = document.createElement("div"); row.className = "turn-recovery-actions";
  for (const [label, action] of [["查看变更与证据", "evidence"], ["修改输入", "edit"], ["重试此任务", "retry"]]) {
    const button = document.createElement("button"); button.type = "button"; button.textContent = label;
    button.addEventListener("click", async () => {
      if (action === "evidence") { openPanelById("evidence"); window.OwoPanels.evidence.openSession?.(sessionId); return; }
      if (!prompt) { showToast("未找到原始输入，请在会话中填写任务"); return; }
      if (state.sessionId !== sessionId) await selectSession(sessionId);
      if (currentTurn()) { showToast("此会话仍有任务在运行"); return; }
      if ($("prompt").value.trim() && !await askConfirm("当前输入已有草稿，是否替换为这个任务？")) return;
      $("prompt").value = prompt; $("prompt").focus();
      if (action === "retry" && await askConfirm("重新执行此任务？已有文件变更会保留，新操作仍由服务端策略与审批控制。")) await sendPrompt();
    });
    row.appendChild(button);
  }
  return row;
}

async function selectSession(id, forceHistory = false) {
  const selectionVersion = ++state.selectionVersion;
  state.sessionId = id;
  // 记住当前对话：下次进入自动恢复，不再要求手动切换。
  localStorage.setItem("owo.lastSession", id);
  state.attachments = [];
  renderAttachmentChips();
  // 切会话：清掉上一回合的流式句柄与未答提问，回复底部跟随。
  resetRunBlocks();
  state.pendingQuestion = null;
  state.autoScroll = true;
  updateScrollBottomBtn();
  // 每会话独立视图：切回时保留该会话的本地输出（含仍在运行回合的实时内容）。
  showSessionView(id);
  // 并行回合：状态条/发送钮跟随切回来的会话（该会话在跑 → 恢复运行中视图）。
  const runningTurn = currentTurn();
  if (runningTurn) {
    startRunStatus();
    state.runStartedAt = runningTurn.startedAt || Date.now();
  } else {
    stopRunStatus();
  }
  updateComposerRunning();
  const reusedView = Boolean(
    $("messages").querySelector(`.session-view[data-sid="${id}"]`)
  );
  try {
    const detail = await api(`/session/${id}`);
    if (selectionVersion !== state.selectionVersion) return;
    state.selectedModel = String((detail && detail.model_override) || "").trim();
    if (typeof refreshComposerModelChip === "function") refreshComposerModelChip();
    const previousInput = [...(detail.messages || [])].reverse().find(message => message.role === "user" && message.content);
    if (previousInput) state.lastSessionPrompts.set(id, String(previousInput.content));
    const restoredFacts = restoreSessionFacts(id, detail);
    if (reusedView && (!forceHistory || runningTurn)) {
      // 本地视图还在（可能正有运行中回合的实时输出）：不重拉历史覆盖，只刷新侧栏。
      syncSessionWelcome();
      await refreshSessions(id);
      if (selectionVersion !== state.selectionVersion) return;
      await refreshDiff(id);
      if (selectionVersion !== state.selectionVersion) return;
      await refreshSessionContext(id);
      return;
    }
    sessionView(id).innerHTML = "";
    let rendered = 0;
    // 历史回放：把 role=tool 的结果按 tool_call_id 配回 assistant 的 tool_calls，
    // 还原成与会话进行中一致的步骤 chip。旧实现把这些记录折叠成一句
    // 「已折叠未显示」，用户无从复核 agent 到底做过什么。
    const history = detail.messages || [];
    const toolResults = new Map();
    for (const message of history) {
      if (message.role !== "tool") continue;
      const content = String(message.content || "");
      const failed = content.startsWith("工具错误：");
      toolResults.set(String(message.tool_call_id || ""), {
        ok: !failed,
        preview: content,
        error: failed ? content.replace(/^工具错误：/, "") : "",
      });
    }
    // 仍检测「最后一条是用户消息但没回复」的失败回合，显式告诉用户。
    let lastRole = null;
    // 连续的工具回合共用同一段折叠（见 appendHistoryToolSteps 注释）。
    let historyToolRun = null;
    for (const message of history) {
      if (message.role === "system") {
        // 压缩摘要等系统记录：按系统提示显示，避免被当作助手回复。
        // 历史摘要动辄上千字，直接铺成正文会把真正的对话挤出屏幕——收成可展开的
        // 时间线事件（与回合内实时压缩用同一枚 chip）。
        const content = String(message.content || "");
        if (content.includes("历史摘要（已压缩）")) {
          addEventChip("compact", "上下文已压缩", content);
        } else {
          addMessage("system", content);
        }
        lastRole = "system";
        historyToolRun = null;
        continue;
      }
      if (message.role === "tool") {
        lastRole = "tool";
        continue;
      }
      if (message.role === "assistant") {
        const calls = Array.isArray(message.tool_calls) ? message.tool_calls : [];
        if (calls.length) {
          historyToolRun = appendHistoryToolSteps(calls, toolResults, historyToolRun);
          lastRole = "tool";
          rendered += 1;
        }
        if (!message.content) continue;
      } else if (!message.content) {
        continue;
      }
      const storedContent = String(message.content || "");
      if (message.role === "assistant" && storedContent.startsWith("回合未完成：") &&
          typeof addHistoricalTurnFailure === "function") {
        addHistoricalTurnFailure(storedContent);
      } else {
        addMessage(message.role === "user" ? "user" : "assistant", message.content);
      }
      lastRole = message.role === "user" ? "user" : "assistant";
      historyToolRun = null;
      rendered += 1;
    }
    renderHistoricalCompletion(id, restoredFacts);
    if (lastRole === "user") {
      addMessage("system", "上一条消息还没有回复（回合可能失败或被中断），可重新发送让助手继续");
    }
    syncSessionWelcome();
  } catch (error) {
    if (selectionVersion !== state.selectionVersion) return;
    sessionView(id).innerHTML = "";
    addMessage("error", `会话加载失败：${friendlyError(error, { resource: true })}`);
  }
  await refreshSessions(id);
  if (selectionVersion !== state.selectionVersion) return;
  await refreshDiff(id);
  if (selectionVersion !== state.selectionVersion) return;
  await refreshSessionContext(id);
}

// ---------- 会话上下文仪表（v0.5.7，对标 Codex 上下文状态显示） ----------

async function refreshSessionContext(sessionId) {
  const bar = $("contextBar");
  if (sessionId && sessionId !== state.sessionId) return;
  const generation = ++sessionContextRefreshGeneration;
  if (!sessionId) {
    bar.classList.add("hidden");
    return;
  }
  try {
    const ctx = await api(`/session/${sessionId}/context`);
    if (generation !== sessionContextRefreshGeneration || sessionId !== state.sessionId) return;
    const fill = $("contextFill");
    const ratio = ctx.token_budget > 0 ? ctx.estimated_tokens / ctx.token_budget : 0;
    fill.style.width = `${Math.min(100, Math.round(ratio * 100))}%`;
    fill.className = ratio > 1 ? "over" : ratio > 0.8 ? "warn" : "";
    const compactionBadge = ctx.last_compaction ? "已压缩" : "未压缩";
    $("contextLabel").textContent =
      `上下文 ${ctx.messages} 条 · 估算 ${ctx.estimated_tokens}/${ctx.token_budget} tokens`;
    bar.title =
      `规则注入：${ctx.rules_injected ? "是" : "否"} · 压缩：${compactionBadge}` +
      (ctx.compaction_enabled ? "" : "（压缩关闭）") +
      (ctx.last_compaction ? `\n最近压缩摘要：${ctx.last_compaction.slice(0, 300)}` : "");
    bar.classList.remove("hidden");
  } catch (error) {
    if (generation !== sessionContextRefreshGeneration || sessionId !== state.sessionId) return;
    $("contextLabel").textContent = friendlyError(error, { resource: true });
    bar.title = "会话上下文状态";
    bar.classList.remove("hidden");
  }
}

async function newSession() {
  const workspace = $("workspace").value.trim();
  if (!workspace) {
    const error = new Error("请先选择项目工作区，再新建会话");
    error.userFacing = true;
    throw error;
  }
  localStorage.setItem("owo.workspace", workspace);
  const session = await api("/session", {
    method: "POST",
    body: JSON.stringify(
      ModelRouting.buildCreateSessionRequest(workspace, state.pendingModelOverride)
    ),
  });
  state.pendingModelOverride = null;
  await selectSession(session.id);
  showToast("新对话已就绪");
  syncSessionWelcome();
}

// 进入时自动恢复上次的对话：优先 localStorage 记忆，其次最近更新的会话；
// 都没有则保持空状态，不强制用户手动切换。
async function restoreLastSession() {
  if (state.sessionId) return;
  try {
    const sessions = (await api("/sessions")) || [];
    const last = localStorage.getItem("owo.lastSession");
    const remembered =
      last && sessions.some((session) => session.id === last && !session.archived);
    if (last && !remembered) localStorage.removeItem("owo.lastSession");
    const fallback = sessions.find((session) => !session.archived);
    const target = remembered ? last : fallback && fallback.id;
    if (!target) return;
    await selectSession(target);
  } catch (_) {
    /* 恢复失败保持空状态，不打扰用户 */
  }
}

// ---------- 对话（SSE） ----------

function parseSseBlock(block) {
  return window.OwoTurnSse.parseBlock(block);
}

function rememberTurnFacts(sessionId, stats) {
  if (!state.completedTurnFacts) state.completedTurnFacts = new Map();
  const facts = window.OwoCompletionRecord?.fromTurnStats(stats) || {};
  const status = stats?.completion_status;
  state.completedTurnFacts.set(sessionId, { ...facts, outcome: status === "aborted" ? "cancelled" : status === "blocked" ? "failed" : "completed" });
  window.OwoStatusBar?.repaint();
}

async function sendPrompt() {
  const promptInput = $("prompt");
  const promptValue = promptInput.value;
  const prompt = promptValue.trim();
  if (!prompt) return;
  // 配置门与输入校验先于自动建会话，避免空提交或未配置服务时留下空线程。
  // The composer can collect a draft before a workspace is chosen. Sending the
  // first prompt should open the project picker directly instead of surfacing a
  // model/setup error first; keep the draft in the textarea until session creation.
  if (!state.sessionId && !$("workspace").value.trim()) {
    openWorkspaceMenu($("composerProjectBtn"));
    return;
  }
  let modelConnection;
  try {
    modelConnection = ModelRouting.buildCustomModelConnection(getComposerModel(), loadCustomModels(), customModelKeys());
    const capabilities = await api("/capabilities");
    if (capabilities?.constraints?.scoped_turn_cancellation !== true) throw new Error("当前核心不支持按回合停止，请更新核心；本次未发送");
    if (modelConnection) {
      if (modelConnection.api_key_env && capabilities?.constraints?.custom_model_env_credentials !== true) throw new Error("当前核心不支持安全凭据引用，请更新核心；本次未发送");
      if (!capabilities || !capabilities.constraints || capabilities.constraints.custom_model_connection !== true) {
        throw new Error("当前核心不支持自定义模型连接配置，请更新核心；本次未发送");
      }
    }
  } catch (error) {
    showToast(friendlyError(error), "error");
    return;
  }
  if (modelGateMissing()) {
    openModelGateSettings();
    return;
  }
  // 无会话时**自动新建**再发，而不是甩一句"请先新建或选择一个会话"。
  // 用户按下回车的意图是"把这句话说出去"，让他先点一次"新建对话"是多余的一步
  // （Codex 的做法：输入框永远可用，发送时兜底建线程）。
  if (!state.sessionId) {
    try {
      await promptSessionStart.run(() => newSession());
    } catch (error) {
      showToast(`创建会话失败：${friendlyError(error)}`, "error");
      return;
    }
    if (!state.sessionId) return; // newSession 内部可能因工作区为空而放弃
  }
  // 只有「本会话」已有回合才拒绝：其它会话在跑不影响这里（并行对话互不干扰）。
  if (currentTurn()) {
    showToast("本会话已有回合在运行，可先中断再发送", "error");
    return;
  }
  if (promptInput.value === promptValue) promptInput.value = "";
  updateComposerHint();
  // 主动发送：回到跟随底部（此前上滑过也要跟住新回合）。
  state.autoScroll = true;
  updateScrollBottomBtn();
  addMessage("user", prompt);
  if (!state.lastSessionPrompts) state.lastSessionPrompts = new Map();
  state.lastSessionPrompts.set(state.sessionId, prompt);
  const attachments = state.attachments.map((attachment) => attachment.id);
  if (attachments.length) {
    addMessage("system", `附带 ${attachments.length} 个附件`);
  }
  // 助手气泡按需创建：让思考块/工具分组先于回答出现，保持时间顺序。
  let streaming = null;
  const ensureStreaming = () => {
    if (!streaming) streaming = addMessage("assistant", "");
    return streaming;
  };
  let assistantText = "";
  let failureDisplayed = false;
  const turn = { tools: 0, failed: 0, modelCalls: 0, startedAt: Date.now(), server: null, card: null, files: new Set() };
  state.turn = turn;

  state.reading = true;
  state.lastTurnOutcome = "";
  state.abortController = new AbortController();
  updateComposerRunning();
  startRunStatus();
  // 流写入绑定发起回合的会话：切走后任务继续跑、输出留在原视图，切回即恢复。
  const turnSessionId = state.sessionId;
  writeTargetSid = turnSessionId;
  state.activeTurns.set(turnSessionId, {
    controller: state.abortController,
    startedAt: Date.now(),
    turnId: (typeof crypto !== "undefined" && crypto.randomUUID) ? crypto.randomUUID() : "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, c => { const v = Math.floor(Math.random() * 16); return (c === "x" ? v : (v & 3) | 8).toString(16); }),
    prompt, phase: "thinking",
    requestedModel: getComposerModel(),
  });
  try {
    const controller = state.abortController;
    const response = await window.OwoApi.stream("/session/" + encodeURIComponent(turnSessionId) + "/turn", {
      method: "POST", json: { prompt, attachments, turn_id: state.activeTurns.get(turnSessionId).turnId, ...(modelConnection ? { model_connection: modelConnection } : {}) }, signal: controller.signal,
    });
    const handlePayload = (event, payload) => {
      writeTargetSid = turnSessionId;
      const previousTurn = state.turn;
      state.turn = turn;
      try {
      switch (event) {
        case "reasoning_delta":
          setRunPhase("reasoning");
          pushReasoning(payload.delta || "");
          break;
        case "token_delta": {
          finishThinking();
          setRunPhase("answering");
          assistantText += payload.delta || "";
          const bubble = ensureStreaming();
          bubble.innerHTML = renderMarkdown(assistantText);
          bindCopyButtons(bubble);
          followScroll();
          break;
        }
        case "progress":
          // 「模型调用」每个回合都发，不再成行，只计入汇报卡的轮次统计。
          if (payload.message === "模型调用") {
            if (state.turn) state.turn.modelCalls += 1;
            // 桌宠气泡带轮次，让「在想…」有进度感。
            setRunPhase("thinking", state.turn ? `第 ${state.turn.modelCalls} 轮思考…` : undefined);
          } else if (payload.message) {
            addMessage("system", `[${payload.message}]`);
          }
          break;
        case "tool_use":
          setRunPhase("tool", `正在执行工具：${payload.tool || "工具"}`);
          pushToolUse(payload);
          break;
        case "tool_result":
          setRunPhase("thinking", "继续思考…");
          pushToolResult(payload);
          break;
        case "permission_request":
          setRunPhase("waiting");
          window.OwoNotificationSound?.play(loadLocalPrefs().sound);
          showApproval(payload);
          break;
        case "permission_resolved":
          // B1-5 审批终态（超时/中止/其他通道响应）：即时收尾，不等 5s 轮询。
          markApprovalResolved(payload);
          break;
        case "user_question":
          // ask_user：回合挂起等待回答，桌宠/状态条进入阻塞态。
          setRunPhase("asking");
          showQuestionCard(payload);
          break;
        case "user_answered":
          markQuestionAnswered(payload);
          break;
        case "turn_stats":
          turn.server = payload;
          rememberTurnFacts(turnSessionId, payload);
          finishThinking();
          settlePendingQuestion("ended");
          stopRunStatus();
          state.toolRun = null;
          renderTurnSummary(turnSessionId, turn);
          break;
        case "final": {
          finishThinking();
          assistantText = payload.text || assistantText;
          const bubble = ensureStreaming();
          bubble.innerHTML = renderMarkdown(assistantText);
          bindCopyButtons(bubble);
          setRunPhase("thinking", "等待宿主验收…");
          break;
        }
        case "turn_failed": {
          // 服务端失败终态：明确告诉用户失败原因与真实完成状态，不留在「执行中」。
          failureDisplayed = true;
          stopRunStatus();
          state.toolRun = null;
          state.lastTurnOutcome = payload.completion_status === "aborted" ? "cancelled" : "failed";
          turn.server = {
            completion_status: payload.completion_status,
            completion_record: payload.completion_record,
          };
          const completionLabel = ({
            response_complete: "",
            candidate: " · 代码变更待验收",
            accepted: " · 宿主验收通过",
            unverified: " · 结果未验证",
            blocked: " · 存在阻断问题",
            aborted: " · 已取消",
          })[payload.completion_status] || "";
          rememberTurnFacts(turnSessionId, turn.server);
          showTurnFailure((payload.message || "未知原因") + completionLabel);
          break;
        }
        case "compaction":
          addEventChip("compact", "上下文已自动压缩", payload.summary || "");
          break;
      }
      } finally { state.turn = previousTurn; }
    };

    const result = await window.OwoTurnSse.consumeResponse(response, {
      onEvent: handlePayload,
      signal: controller.signal,
      replay: (turnId, cursor, signal) => window.OwoApi.get(
        window.OwoTurnSse.replayPath(turnSessionId, turnId, cursor), { signal },
      ),
    });
    turn.completionStatus = result.completionStatus;
    const facts = state.completedTurnFacts?.get(turnSessionId);
    if (facts) facts.outcome = result.completionStatus === "aborted" ? "cancelled" : result.completionStatus === "blocked" ? "failed" : "completed";
    if (result.completionStatus !== "aborted") window.OwoNotificationSound?.play(loadLocalPrefs().sound);
    state.lastTurnOutcome = result.completionStatus === "aborted"
      ? "cancelled"
      : ["unverified", "blocked"].includes(result.completionStatus) ? "failed" : "completed";
    writeTargetSid = turnSessionId;
    hideApproval();
    state.attachments = [];
    renderAttachmentChips();
    await refreshSessions(state.sessionId);
    await refreshDiff(state.sessionId);
    await refreshSessionContext(state.sessionId);
  } catch (error) {
    writeTargetSid = turnSessionId;
    if (!assistantText && streaming) streaming.remove();
    if (error.name !== "AbortError" && error.completionStatus !== "aborted") {
      window.OwoNotificationSound?.play(loadLocalPrefs().sound);
      state.lastTurnOutcome = "failed";
      if (!state.completedTurnFacts) state.completedTurnFacts = new Map();
      state.completedTurnFacts.set(turnSessionId, { ...(state.completedTurnFacts.get(turnSessionId) || {}), outcome: "failed" });
      if (!failureDisplayed) showTurnFailure(error.message);
    } else if (error.completionStatus === "aborted") {
      state.lastTurnOutcome = "cancelled";
      if (!state.completedTurnFacts) state.completedTurnFacts = new Map();
      state.completedTurnFacts.set(turnSessionId, { ...(state.completedTurnFacts.get(turnSessionId) || {}), outcome: "cancelled" });
      addEventChip("stop", "宿主已停止此回合", "已产生的文件变更可能保留，请在证据页查看。");
    } else {
      if (!state.completedTurnFacts) state.completedTurnFacts = new Map();
      state.completedTurnFacts.set(turnSessionId, { outcome: "interrupted", lastExecutionModel: "", lastCompletionRecord: null });
      showTurnFailure("连接已中断；宿主是否结束尚未确认，请重新打开会话查看。");
    }
  } finally {
    if (turnSessionId) {
      state.activeTurns.delete(turnSessionId);
      if (writeTargetSid === turnSessionId) writeTargetSid = null;
    }
    // 视图级收尾只在本回合仍属于「当前视图会话」时执行：并行回合下，
    // 后台会话结束不能把前台会话的流式句柄/状态条/审批卡一起清掉。
    const isVisible = state.sessionId === turnSessionId;
    if (isVisible) {
      hideApproval();
      finishThinking();
      // 终态兜底：任何路径（中断/异常/断流）结束后，未答的提问卡都必须显式作废。
      settlePendingQuestion("aborted");
      stopRunStatus();
      resetRunBlocks();
      state.turn = null;
      state.abortController = null;
    }
    state.reading = state.activeTurns.size > 0;
    window.OwoStatusBar?.repaint();
    updateComposerRunning();
  }
}

function renderAttachmentChips() {
  const container = $("attachmentChips");
  container.innerHTML = "";
  for (const attachment of state.attachments) {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.textContent = `${attachment.name} ×`;
    chip.title = attachment.mime || "attachment";
    chip.addEventListener("click", () => {
      state.attachments = state.attachments.filter(
        (item) => item.id !== attachment.id
      );
      renderAttachmentChips();
    });
    container.appendChild(chip);
  }
}

async function uploadAttachments(files) {
  if (!state.sessionId) {
    addMessage("system", "请先新建或选择一个会话，再添加附件");
    return;
  }
  for (const file of files) {
    try {
      const dataUrl = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
      });
      const comma = String(dataUrl).indexOf(",");
      const dataB64 = comma >= 0 ? String(dataUrl).slice(comma + 1) : String(dataUrl);
      const uploaded = await api(`/session/${state.sessionId}/attachments`, {
        method: "POST",
        body: JSON.stringify({
          name: file.name,
          mime: file.type || "application/octet-stream",
          data_b64: dataB64,
        }),
      });
      state.attachments.push({ id: uploaded.id, name: uploaded.name, mime: uploaded.mime });
      renderAttachmentChips();
    } catch (error) {
      addMessage("system", `附件上传失败 ${file.name}：${error.message || error}`);
    }
  }
  $("attachmentInput").value = "";
}

// ---------- 审批条 ----------
// 待确认请求始终由用户决定，不按工具名称或浏览器偏好代为批准。
// 审批超时预算（与引擎侧一致）：超时即按拒绝处理，因此卡片必须自带倒计时，
// 否则用户一旦没注意到顶部审批条，回合会在 5 分钟后静默失败（实测因此丢了两次 PPT 请求）。
const APPROVAL_TIMEOUT_MS = 300000;
const APPROVAL_URGENT_MS = 60000;
// 原始标题（无待审批时恢复用）；取不到就留空，标题改写自动跳过。
const BASE_DOC_TITLE = typeof document !== "undefined" ? document.title || "" : "";

function showApproval(payload) {
  const requestId = payload.request_id;
  if (!requestId) return;
  state.pendingApprovals.set(requestId, {
    tool: payload.tool || "",
    reason: payload.reason || "",
    level: payload.level || "",
    explain: payload.explain || null,
    redactedArgs: payload.redacted_args || null,
    riskNote: payload.risk_note || "",
    // 流帧驱动时 writeTargetSid 即发起回合的会话（跨会话审批归属正确）。
    sessionId: writeTargetSid || state.sessionId,
    // 计时起点只在此处落一次：后续 5s 轮询同步不会重置已有的倒计时。
    requestedAt: Date.now(),
  });
  renderApprovals();
}

// 自动放行记录（FIFO）：审批与工具调用按同一顺序到达，同名工具一一对应。
function queueAutoAllowed(tool) {
  state.autoAllowed.push(String(tool || ""));
  if (state.autoAllowed.length > 64) state.autoAllowed.shift();
}

function takeAutoAllowed(tool) {
  const name = String(tool || "");
  const index = state.autoAllowed.indexOf(name);
  if (index < 0) return false;
  state.autoAllowed.splice(index, 1);
  return true;
}

// 渲染审批队列：当前会话与其它会话的待审批卡并存，各自独立允许/拒绝。
// 每张卡带剩余秒数：超时引擎按拒绝处理，必须让"还多久过期"可见。
function describeApproval(payload) {
  const item = payload && typeof payload === "object" ? payload : {};
  const explain = item.explain && typeof item.explain === "object" ? item.explain : {};
  const tool = String(item.tool || "");
  const action = String(explain.action || "");
  const target = String(explain.target || "");
  let actionLine = action;
  if (/^(write_file|edit_file|multi_edit|apply_patch)$/.test(tool)) actionLine = "将写入：" + (target || action || "目标文件");
  else if (tool === "read_file") actionLine = "将读取：" + (target || action || "目标文件");
  else if (tool === "run_command") actionLine = action ? "将执行命令：" + action.replace(/^执行命令：/, "") : "将执行命令";
  else if (action.startsWith("联网访问：")) actionLine = "将联网访问：" + (target || action.slice("联网访问：".length));
  let redactedArgs = "";
  const safeArgs = item.redacted_args || item.redactedArgs;
  if (safeArgs && typeof safeArgs === "object") {
    try { redactedArgs = JSON.stringify(safeArgs); } catch { redactedArgs = ""; }
  }
  return { action: actionLine, target: target && target !== action ? target : "", undoable: explain.undoable === true,
    risk: String(item.risk_note || item.riskNote || explain.risk || ""), level: String(item.level || ""), redactedArgs };
}

function renderApprovals() {
  const list = $("approvalList");
  const bar = $("approvalBar");
  if (!list || !bar) return;
  list.innerHTML = "";
  const items = [...state.pendingApprovals.entries()];
  bar.classList.toggle("hidden", items.length === 0);
  let anyUrgent = false;
  if (items.length) {
    // 标题行：先说清"这是要你点头"，再列具体动作——审批卡本身要能自解释。
    const head = document.createElement("div");
    head.className = "approval-head";
    const title = document.createElement("span");
    title.textContent = "需要你的确认";
    const badge = document.createElement("span");
    badge.className = "approval-head-badge";
    badge.textContent = `${items.length} 项待处理`;
    const tip = document.createElement("span");
    tip.className = "approval-head-tip";
    tip.textContent = "超时未响应将按拒绝处理";
    head.append(title, badge, tip);
    list.appendChild(head);
  }
  for (const [requestId, entry] of items) {
    const row = document.createElement("div");
    row.className = "approval-item";
    const content = document.createElement("div");
    content.className = "approval-item-content";
    const cross = entry.sessionId && entry.sessionId !== state.sessionId;
    const text = document.createElement("span");
    text.className = "approval-item-text";
    const prefix = cross
      ? `[会话 ${String(entry.sessionId).slice(0, 6)}…] `
      : "";
    if (prefix) text.appendChild(document.createTextNode(prefix));
    const name = document.createElement("span");
    name.className = "tool-name";
    name.textContent = entry.tool ? toolLabel(entry.tool) : "未知工具";
    text.appendChild(name);
    content.appendChild(text);
    const summary = describeApproval(entry);
    const addLine = (className, value) => {
      if (!value) return;
      const node = document.createElement("div");
      node.className = className;
      node.textContent = value;
      content.appendChild(node);
    };
    addLine("approval-explain", summary.action);
    addLine("approval-target", summary.target ? `影响目标：${summary.target}` : "");
    addLine("tool-reason", entry.reason ? `原因：${entry.reason}` : "");
    addLine("approval-args", summary.redactedArgs ? `脱敏参数：${summary.redactedArgs}` : "");
    addLine("approval-risk", summary.risk ? `风险：${summary.risk}` : "");
    addLine("approval-reversibility", summary.undoable ? "可通过会话快照撤销" : "无法自动撤销");
    const timer = document.createElement("span");
    timer.className = "approval-timer";
    const remain = approvalRemainingMs(entry);
    if (remain <= APPROVAL_URGENT_MS) {
      timer.classList.add("urgent");
      anyUrgent = true;
    }
    timer.textContent = approvalTimerText(remain);
    const allow = document.createElement("button");
    allow.className = "allow";
    allow.textContent = "允许";
    allow.dataset.rid = requestId;
    const deny = document.createElement("button");
    deny.className = "deny";
    deny.textContent = "拒绝";
    deny.dataset.rid = requestId;
    const actions = document.createElement("div");
    actions.className = "approval-actions";
    const scope = document.createElement("select");
    scope.className = "approval-scope";
    scope.dataset.rid = requestId;
    scope.setAttribute("aria-label", "授权范围");
    const scopes = [["once", "仅本次"]];
    // 当前权限契约把所有非 Read 请求视为破坏性操作；只有 Read 可记忆授权。
    if (summary.level === "read") {
      scopes.push(["task", "本任务"], ["one_hour", "一小时内"], ["workspace", "工作区长期"], ["always_readonly", "只读长期"]);
    }
    for (const [value, label] of scopes) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      scope.appendChild(option);
    }
    actions.append(scope, allow, deny);
    row.append(content, timer, actions);
    list.appendChild(row);
  }
  bar.classList.toggle("urgent", anyUrgent);
  // 用户可能已切到别的窗口：把待审批数量写进标题栏，任务栏也看得见。
  if (typeof document !== "undefined" && BASE_DOC_TITLE) {
    document.title = items.length
      ? `⚠ 待审批（${items.length}） · ${BASE_DOC_TITLE}`
      : BASE_DOC_TITLE;
  }
}

function approvalRemainingMs(entry) {
  const start = Number(entry && entry.requestedAt) || Date.now();
  return Math.max(0, APPROVAL_TIMEOUT_MS - (Date.now() - start));
}

function approvalTimerText(remainMs) {
  const seconds = Math.max(0, Math.ceil(remainMs / 1000));
  if (seconds <= 0) return "已超时";
  if (seconds >= 60) {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `剩余 ${m}:${String(s).padStart(2, "0")}`;
  }
  return `剩余 ${seconds}s`;
}

// 每秒刷新倒计时文本（不整体重建 DOM，避免打断点击焦点）。
function tickApprovalTimers() {
  if (state.pendingApprovals.size === 0) return;
  const rows = [...document.querySelectorAll("#approvalList .approval-item")];
  const entries = [...state.pendingApprovals.values()];
  let anyUrgent = false;
  rows.forEach((row, index) => {
    const entry = entries[index];
    if (!entry) return;
    const timer = row.querySelector(".approval-timer");
    if (!timer) return;
    const remain = approvalRemainingMs(entry);
    timer.textContent = approvalTimerText(remain);
    const urgent = remain <= APPROVAL_URGENT_MS;
    timer.classList.toggle("urgent", urgent);
    if (urgent) anyUrgent = true;
  });
  const bar = $("approvalBar");
  if (bar) bar.classList.toggle("urgent", anyUrgent);
}
setInterval(tickApprovalTimers, 1000);

// 跨会话待审批同步：轮询服务端 pending 列表，把错过 SSE 事件/其它会话的审批并入队列；
// 服务端已不存在的（已响应/超时/会话结束）同步移除，避免点了必 404 的僵尸卡。
async function syncPendingApprovals() {
  if (Date.now() < connectionUnavailableUntil) return;
  try {
    const data = await api("/approvals/pending");
    const remote = Array.isArray(data && data.pending) ? data.pending : [];
    let changed = false;
    for (const item of remote) {
      if (!item.request_id || state.pendingApprovals.has(item.request_id)) continue;
      state.pendingApprovals.set(item.request_id, {
        tool: item.tool || "",
        reason: item.args_summary || "",
        sessionId: item.session_id || "",
        // 轮询补入的卡片拿不到真实发起时刻，保守按"刚发起"计（宁可少显示剩余时间，
        // 也不让倒计时虚高到真实超时之后）。
        requestedAt: Date.now(),
      });
      changed = true;
    }
    for (const requestId of [...state.pendingApprovals.keys()]) {
      if (!remote.some((item) => item.request_id === requestId)) {
        state.pendingApprovals.delete(requestId);
        changed = true;
      }
    }
    if (changed) renderApprovals();
  } catch {
    /* 引擎未就绪时静默 */
  }
}
setInterval(syncPendingApprovals, 5000);

function hideApproval() {
  // 兼容旧调用（turn 终态）：清掉当前会话的待审批卡（该回合已结束，审批不再有意义）。
  clearSessionApprovals(state.sessionId);
}

/// B1-5：审批终态回执（引擎侧已解决）——即时从队列移除卡片，并在时间线留下
/// 终态说明。超时/中止场景此前要等 5s 轮询兜底且时间线无痕。
function markApprovalResolved(payload) {
  const requestId = payload && payload.request_id;
  if (!requestId) return;
  const entry = state.pendingApprovals.get(requestId);
  state.pendingApprovals.delete(requestId);
  renderApprovals();
  const source = (payload && payload.source) || "user";
  // 用户自己点的允许/拒绝已有系统消息/事件 chip，不重复。
  if (source === "user") return;
  const tool = entry && entry.tool ? toolLabel(entry.tool) : "工具调用";
  if (source === "timeout") {
    addEventChip("deny", `审批超时：${tool}`, "300 秒未响应，引擎已按拒绝处理。");
  } else if (source === "aborted") {
    addEventChip("deny", `审批作废：${tool}`, "回合已中止，本次审批无需再响应。");
  }
}

function clearSessionApprovals(sessionId) {
  if (!sessionId) return;
  let changed = false;
  for (const [requestId, entry] of state.pendingApprovals) {
    if (entry.sessionId === sessionId) {
      state.pendingApprovals.delete(requestId);
      changed = true;
    }
  }
  if (changed) renderApprovals();
}

async function respondApproval(requestId, allow, scope) {
  const entry = state.pendingApprovals.get(requestId);
  if (!entry) return;
  const tool = entry.tool || "";
  const sessionId = entry.sessionId || state.sessionId;
  state.pendingApprovals.delete(requestId);
  renderApprovals();
  // 审批提示写进卡片所属会话的视图（点卡时用户可能停在别的会话）。
  const prevTarget = writeTargetSid;
  writeTargetSid = sessionId;
  try {
    await api(`/session/${sessionId}/permission/${requestId}`, {
      method: "POST",
      body: JSON.stringify(allow ? { allow: true, scope: scope || "once" } : { allow: false }),
    });
    if (!allow) {
      // 拒绝是回合内的关键事件：写成时间线 chip，附工具与原因，便于事后复盘。
      // 允许则不写任何消息——审批卡消失 + 对应工具步骤转绿就是最直接的反馈，
      // 再补一条「已允许该操作」只会把对话流刷成权限日志。
      addEventChip(
        "deny",
        tool ? `已拒绝「${toolLabel(tool)}」` : "已拒绝该操作",
        tool ? `工具：${tool}` : "该工具调用已按你的选择拒绝执行。"
      );
    }
  } catch (error) {
    addMessage("error", `审批失败：${error.message}`);
  } finally {
    writeTargetSid = prevTarget;
  }
}

// ---------- 用户提问卡（ask_user：回合挂起，等你回答） ----------

function showQuestionCard(payload) {
  const card = document.createElement("div");
  card.className = "msg question";
  card.dataset.questionId = payload.question_id || "";
  state.pendingQuestion = payload.question_id || null;

  const title = document.createElement("div");
  title.className = "question-title";
  title.textContent = "助手需要你确认";
  const text = document.createElement("div");
  text.className = "question-text";
  text.textContent = payload.question || "";
  card.append(title, text);

  const options = Array.isArray(payload.options) ? payload.options.filter(Boolean) : [];
  if (options.length) {
    const row = document.createElement("div");
    row.className = "question-options";
    for (const option of options) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = option;
      button.addEventListener("click", () => submitQuestionAnswer(card, option));
      row.appendChild(button);
    }
    card.appendChild(row);
  }

  const form = document.createElement("form");
  form.className = "question-form";
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = options.length ? "或输入其他回答…" : "输入你的回答后回车…";
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.textContent = "回答";
  form.append(input, submit);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    submitQuestionAnswer(card, input.value);
  });
  card.appendChild(form);

  newMessageBlock(card);
  input.focus();
}

async function submitQuestionAnswer(card, answer) {
  const questionId = card.dataset.questionId;
  const value = String(answer || "").trim();
  if (!questionId || !value || card.dataset.answered === "1") return;
  card.dataset.answered = "1";
  // 立即锁定输入，避免重复提交；服务端 user_answered 事件补最终状态。
  const form = card.querySelector(".question-form");
  if (form) {
    const input = form.querySelector("input");
    if (input) input.disabled = true;
    const button = form.querySelector("button");
    if (button) button.disabled = true;
  }
  setRunPhase("thinking", "已提交回答，继续执行…");
  try {
    await api(`/session/${state.sessionId}/answer/${questionId}`, {
      method: "POST",
      body: JSON.stringify({ answer: value }),
    });
    markQuestionResult(card, "user", value);
  } catch (error) {
    card.dataset.answered = "";
    if (form) {
      const input = form.querySelector("input");
      if (input) input.disabled = false;
      const button = form.querySelector("button");
      if (button) button.disabled = false;
    }
    addMessage("error", `回答提交失败：${friendlyError(error)}`);
  }
}

function markQuestionAnswered(payload) {
  const questionId = payload.question_id || "";
  const card = document.querySelector(
    `.msg.question[data-question-id="${CSS.escape(questionId)}"]`
  );
  if (!card) return;
  if (state.pendingQuestion === questionId) state.pendingQuestion = null;
  markQuestionResult(card, payload.source || "user", payload.answer || "");
}

function markQuestionResult(card, source, answer) {
  card.classList.add("is-answered");
  const form = card.querySelector(".question-form");
  if (form) form.remove();
  const options = card.querySelector(".question-options");
  if (options) options.remove();
  if (card.querySelector(".question-stamp")) return;
  const stamp = document.createElement("div");
  stamp.className = "question-stamp";
  if (source === "user") stamp.textContent = `已回答：${answer}`;
  else if (source === "timeout") stamp.textContent = "提问长时间未回答，助手已按已有信息继续";
  else if (source === "aborted") stamp.textContent = "回合已中止，该提问作废";
  else stamp.textContent = "回合已结束，该提问作废";
  card.appendChild(stamp);
}

/// 回合终态兜底：若提问卡仍未作答（SSE 断流/回合提前结束），显式作废，
/// 不让界面上留一张还能输入的「僵尸提问卡」。
function settlePendingQuestion(source = "ended") {
  const questionId = state.pendingQuestion;
  if (!questionId) return;
  state.pendingQuestion = null;
  const card = document.querySelector(
    `.msg.question[data-question-id="${CSS.escape(questionId)}"]`
  );
  if (card) markQuestionResult(card, source, "");
}

// ---------- diff 审阅 ----------

async function refreshDiff(sessionId) {
  const list = $("diffList");
  if (sessionId && sessionId !== state.sessionId) return;
  const generation = ++sessionDiffRefreshGeneration;
  $("revertBtn").disabled = true;
  if (!sessionId) {
    list.innerHTML = '<li class="sub">开始任务后，可以在这里检查文件变化。</li>';
    return;
  }
  list.innerHTML = '<li class="sub">正在读取文件变化…</li>';
  try {
    const diffs = await api(`/session/${sessionId}/diff`);
    if (generation !== sessionDiffRefreshGeneration || sessionId !== state.sessionId) return;
    list.innerHTML = "";
    let index = 0;
    for (const diff of diffs) {
      const li = document.createElement("li");
      li.className = "diff-item";
      const changed = diff.before != null && diff.after != null ? "修改" : diff.after != null ? "新增" : "删除";
      const button = document.createElement("button");
      button.type = "button";
      button.className = "diff-open";
      button.innerHTML = '<span class="file-change-kind">' + changed + '</span><strong>' + esc(diff.path) + '</strong><span aria-hidden="true">⌄</span>';
      button.title = "查看修改前后的内容：" + diff.path;
      button.setAttribute("aria-expanded", "false");
      const body = document.createElement("pre");
      body.className = "diff-body hidden";
      body.id = "diff-preview-" + generation + "-" + index++;
      body.textContent = diffText(diff);
      button.setAttribute("aria-controls", body.id);
      button.addEventListener("click", () => {
        const hidden = body.classList.toggle("hidden");
        button.setAttribute("aria-expanded", String(!hidden));
      });
      li.append(button, body);
      list.appendChild(li);
    }
    $("revertBtn").disabled = diffs.length === 0;
    if (!diffs.length) list.innerHTML = '<li class="sub">本次会话没有文件改动。文字结果保留在对话中。</li>';
    // G13：逐 hunk 审阅（服务端 /session/{id}/hunks + /hunks/apply）。
    // 失败不影响上面的整份 diff 视图（旧行为保持可用）。
    await renderHunkActions(sessionId, generation, list);
  } catch (_) {
    if (generation !== sessionDiffRefreshGeneration || sessionId !== state.sessionId) return;
    list.innerHTML = '<li class="sub">暂时无法读取文件变化，请稍后重新打开会话。</li>';
  }
}

// 逐 hunk 审阅行：把 `/session/{id}/hunks` 视图渲染成"接受全部 / 拒绝全部"操作。
// 逐条序号应用仍走同一路由（此处只暴露最常见的整文件操作，避免误点单条 hunk）。
async function renderHunkActions(sessionId, generation, list) {
  const review = window.OwoHunkReview;
  if (!review || !list || !list.isConnected) return;
  let view;
  try {
    view = await api("/session/" + encodeURIComponent(sessionId) + "/hunks");
  } catch (_) {
    return;
  }
  if (generation !== sessionDiffRefreshGeneration || sessionId !== state.sessionId) return;
  let rows;
  try {
    rows = review.buildRows(view);
  } catch (_) {
    return;
  }
  for (const row of rows) {
    if (!row.reviewable) continue;
    const item = document.createElement("li");
    item.className = "diff-item hunk-review";
    item.dataset.path = row.path;
    const title = document.createElement("div");
    title.className = "file-change-kind";
    title.textContent = row.path + " · " + row.hunks.length + " 个 hunk";
    title.title = row.hunks.map((hunk) => hunk.label).join("\n");
    const status = document.createElement("span");
    status.className = "sub hunk-review-status";
    item.append(title);
    if (row.conflict) {
      status.textContent = "文件已被改动，逐 hunk 应用会被拒绝（409）";
    } else {
      const accept = document.createElement("button");
      accept.type = "button";
      accept.textContent = "接受全部";
      accept.addEventListener("click", () => applyHunksForFile(row.path, "accept", row.hunks.length, status));
      const reject = document.createElement("button");
      reject.type = "button";
      reject.textContent = "拒绝全部";
      reject.addEventListener("click", () => applyHunksForFile(row.path, "reject", row.hunks.length, status));
      item.append(accept, reject);
      status.textContent = "可逐 hunk 处理（拒绝即还原为写前内容）";
    }
    item.append(status);
    list.appendChild(item);
  }
}

// 应用某个文件的全部 hunk（accept/reject），并在原地回显服务端结果。
async function applyHunksForFile(path, side, total, status) {
  const review = window.OwoHunkReview;
  const sessionId = state.sessionId;
  if (!review || !sessionId) return;
  const built = review.applyBody(path, side, Array.from({ length: total }, (_, index) => index));
  if (!built.ok) {
    if (status) status.textContent = built.error;
    return;
  }
  if (status) status.textContent = side === "accept" ? "正在接受…" : "正在还原…";
  try {
    const result = await api("/session/" + encodeURIComponent(sessionId) + "/hunks/apply", {
      method: "POST",
      body: JSON.stringify(built.body),
    });
    if (status) status.textContent = review.summarizeApply(result);
    await refreshDiff(sessionId);
  } catch (error) {
    const message = error && error.status === 409
      ? "文件已被改动，未写盘（请先处理该文件的编辑）"
      : "逐 hunk 应用失败" + (error && error.status ? "（" + error.status + "）" : "");
    if (status) status.textContent = message;
  }
}

// 符号检索入口（桌面）：默认落在**当前会话打开的项目**上（session_id），
// Daemon 自己的工作区未必是被打开的项目。面板/控制台都可用 window.owoSymbolSearch。
async function searchSymbols(query, options) {
  const search = window.OwoSymbolSearch;
  if (!search) return { ok: false, error: "符号检索模块未加载" };
  const built = search.buildQueryUri(query, Object.assign({ sessionId: state.sessionId }, options || {}));
  if (!built.ok) return { ok: false, error: built.error };
  try {
    const payload = await api(built.uri);
    return { ok: true, uri: built.uri, payload: payload, rows: search.buildRows(payload), summary: search.summarize(payload) };
  } catch (error) {
    return { ok: false, error: "符号检索失败：" + String(error && error.message ? error.message : error) };
  }
}
window.owoSymbolSearch = searchSymbols;

// 生成行级 diff 文本（统一格式，参照 git diff 风格）。
function diffText(diff) {
  const beforeLines = diff.before != null ? diff.before.split("\n") : [];
  const afterLines = diff.after != null ? diff.after.split("\n") : [];
  const lines = [];
  const maxLen = Math.max(beforeLines.length, afterLines.length);
  for (let index = 0; index < maxLen; index++) {
    const before = index < beforeLines.length ? beforeLines[index] : null;
    const after = index < afterLines.length ? afterLines[index] : null;
    if (before === null) lines.push(`+ ${after}`);
    else if (after === null) lines.push(`- ${before}`);
    else if (before !== after) {
      lines.push(`- ${before}`);
      lines.push(`+ ${after}`);
    } else {
      lines.push(`  ${before}`);
    }
  }
  return lines.join("\n");
}

async function revertAll() {
  if (!state.sessionId) return;
  const ok = await askConfirm({
    title: "撤销文件改动",
    message: "确定撤销本次会话产生的文件改动？这会恢复修改前的文件内容。",
    confirmText: "撤销",
    kind: "danger",
  });
  if (!ok) return;
  await api(`/session/${state.sessionId}/revert`, { method: "POST" });
  await refreshDiff(state.sessionId);
  addMessage("system", "已回滚全部改动");
}

// ---------- 技能中心 ----------

// 技能页（Codex 风格）：已安装网格（✓ 标记启用态）+ 查看/编辑弹窗。
async function refreshSkills() {
  const grid = $("skillGrid");
  if (!grid) return;
  try {
    const skills = await apiWithTimeout("/skills");
    const count = $("skillCount");
    if (count) count.textContent = skills.length ? `${skills.length} 个` : "暂无";
    grid.innerHTML = "";
    for (const skill of skills) grid.appendChild(skillCard(skill));
    if (!skills.length) grid.innerHTML = '<div class="sub">暂无技能</div>';
  } catch (error) {
    const count = $("skillCount");
    if (count) count.textContent = "加载失败";
    grid.innerHTML = `<div class="sub">加载技能失败：${esc(friendlyError(error, { resource: true }))}<br>请点击上方“刷新”重试。</div>`;
  }
}

function skillCard(skill) {
  const enabled = skill.enabled !== false;
  const card = document.createElement("div");
  card.className = "ps-card";
  const initial = esc((skill.name || "?").trim().slice(0, 1).toUpperCase());
  card.innerHTML =
    '<div class="ps-card-top">' +
    `<span class="ps-tile" aria-hidden="true">${initial}</span>` +
    '<div class="ps-card-meta">' +
    `<strong>${esc(skill.name)}</strong>` +
    `<span class="sub">${enabled ? "已启用" : "已禁用"}</span>` +
    "</div>" +
    (enabled ? '<span class="ps-check" title="已启用">✓</span>' : "") +
    "</div>" +
    `<p class="ps-card-desc">${esc(skill.description || "暂无描述")}</p>` +
    '<div class="ps-card-foot">' +
    '<span class="sub">SKILL.md</span>' +
    '<span class="ps-card-actions">' +
    '<button type="button" class="ps-card-btn" data-act="view">查看</button>' +
    '<button type="button" class="ps-card-btn" data-act="edit">编辑</button>' +
    `<button type="button" class="ps-card-btn" data-act="toggle">${enabled ? "禁用" : "启用"}</button>` +
    "</span>" +
    "</div>";
  const [viewBtn, editBtn, toggleBtn] = card.querySelectorAll("button");
  viewBtn.addEventListener("click", async () => {
    try {
      const detail = await api(`/skills/${encodeURIComponent(skill.name)}`);
      openModal({
        title: skill.name,
        body: `<div class="ps-skill-path sub">${esc(detail.path || "")}</div><pre class="ps-skill-content">${esc(detail.content || "（空）")}</pre>`,
        actions: [{ label: "关闭", kind: "ghost", onClick: ({ close }) => close() }],
      });
    } catch (error) {
      showToast(`查看失败：${friendlyError(error, { resource: true })}`);
    }
  });
  editBtn.addEventListener("click", async () => {
    try {
      const detail = await api(`/skills/${encodeURIComponent(skill.name)}`);
      openModal({
        title: `编辑 ${skill.name}`,
        body: `<p class="sub" style="margin:0 0 8px">${esc(detail.path || "")}（注册表内技能重启核心服务后生效）</p><textarea id="skillEditArea" rows="14" spellcheck="false">${esc(detail.content || "")}</textarea>`,
        actions: [
          { label: "取消", kind: "ghost", onClick: ({ close }) => close() },
          {
            label: "保存",
            kind: "primary",
            onClick: async ({ close }) => {
              const content = $("skillEditArea").value;
              try {
                await api(`/skills/${encodeURIComponent(skill.name)}`, {
                  method: "POST",
                  body: JSON.stringify({ content }),
                });
                showToast(`技能 ${skill.name} 已保存`);
                close();
              } catch (error) {
                showToast(`保存失败：${friendlyError(error, { resource: true })}`);
              }
            },
          },
        ],
      });
    } catch (error) {
      showToast(`读取失败：${friendlyError(error, { resource: true })}`);
    }
  });
  toggleBtn.addEventListener("click", async () => {
    try {
      await api(`/skills/${encodeURIComponent(skill.name)}/enabled`, {
        method: "POST",
        body: JSON.stringify({ enabled: !enabled }),
      });
      showToast(`技能 ${skill.name} 已${enabled ? "禁用" : "启用"}（即时生效）`);
      await refreshSkills();
    } catch (error) {
      showToast(`操作失败：${friendlyError(error, { resource: true })}`);
    }
  });
  return card;
}

// ---------- 情景记忆 ----------

async function refreshObservations() {
  try {
    const data = await api("/memory/observations?limit=30");
    const list = $("observationList");
    list.innerHTML = "";
    for (const observation of data.observations || []) {
      const li = document.createElement("li");
      li.innerHTML =
        `<strong>${esc(observation.app_id)}｜${esc(observation.kind)}</strong>` +
        `<span class="sub">${esc(observation.summary)}</span>` +
        `<span class="sub">${esc((observation.ts || "").slice(0, 19).replace("T", " "))}</span>`;
      list.appendChild(li);
    }
    if (!(data.observations || []).length) list.innerHTML = '<li class="sub">暂无观察记录</li>';
  } catch (error) {
    $("observationList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

async function recallMemory() {
  const query = $("recallQ").value.trim();
  if (!query) return;
  try {
    const data = await api(`/memory/recall?q=${encodeURIComponent(query)}&top_k=8`);
    const list = $("recallList");
    list.innerHTML = "";
    for (const hit of data.hits || []) {
      const li = document.createElement("li");
      const score = hit.confidence != null ? `（${(hit.confidence * 100).toFixed(0)}%）` : "";
      li.innerHTML =
        `<strong>${esc(hit.app_id || "")} ${score}</strong>` +
        `<span class="sub">${esc(hit.summary || "")}</span>` +
        `<span class="sub">${esc((hit.ts || "").slice(0, 19).replace("T", " "))}</span>`;
      list.appendChild(li);
    }
    if (!(data.hits || []).length) list.innerHTML = '<li class="sub">无匹配结果</li>';
  } catch (error) {
    $("recallList").innerHTML = `<li class="sub">检索失败：${esc(error.message)}</li>`;
  }
}

// ---------- 技能健康度 ----------

async function refreshSkillHealth() {
  const container = $("skillHealth");
  if (!container) return;
  try {
    const data = await apiWithTimeout("/skills/health");
    const skills = data.skills || [];
    container.innerHTML = "";
    for (const skill of skills) {
      const row = document.createElement("div");
      row.className = "ps-row";
      const badge = skill.state === "active" ? "✅" : skill.state === "degraded" ? "⚠️" : "⛔";
      row.innerHTML =
        `<span class="ps-row-icon">${badge}</span>` +
        '<div class="ps-row-meta">' +
        `<strong>${esc(skill.name)}</strong>` +
        `<span class="sub">${esc(skill.state)} ｜ 成功 ${skill.successes}/${skill.attempts} ｜ 成功率 ${(skill.success_rate * 100).toFixed(0)}% ｜ 模板命中 ${(skill.template_hit_rate * 100).toFixed(0)}% ｜ 连续失败 ${skill.consecutive_failures}</span>` +
        "</div>";
      container.appendChild(row);
    }
    if (!skills.length) container.innerHTML = '<div class="sub">暂无技能健康度数据</div>';
  } catch (error) {
    container.innerHTML = `<div class="sub">加载技能健康度失败：${esc(friendlyError(error))}<br>请点击“刷新”重试。</div>`;
  }
}

// ---------- 插件与技能页：tabs 切换与刷新入口 ----------

function initPluginSkillTabs() {
  const tabs = document.querySelectorAll(".ps-tab");
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      for (const item of tabs) {
        const active = item === tab;
        item.classList.toggle("active", active);
        item.setAttribute("aria-selected", String(active));
      }
      for (const pane of document.querySelectorAll(".ps-pane")) {
        pane.hidden = pane.dataset.psPane !== tab.dataset.psTab;
      }
    });
  }
  const on = (id, fn) => {
    const el = $(id);
    if (el) el.addEventListener("click", fn);
  };
  on("pluginRefreshBtn", () => {
    refreshPlugins();
    refreshPluginMarket();
  });
  on("marketRefreshBtn", () => refreshPluginMarket());
  on("skillRefreshBtn", () => {
    refreshSkills();
    refreshSkillHealth();
  });
}

// ---------- Eval 评估 ----------

async function runEval() {
  const suiteId = $("evalSuite").value;
  const button = $("evalRunBtn");
  const box = $("evalReport");
  button.dataset.coreBusy = "true";
  button.dataset.coreBusy = "true";
  button.disabled = true;
  button.textContent = "运行中…";
  box.className = "owo-result";
  box.innerHTML = '<span class="owo-result-empty">评估运行中（调用真实模型，请稍候）…</span>';
  try {
    const report = await api("/eval/run", {
      method: "POST",
      body: JSON.stringify({ suite_id: suiteId }),
    });
    const rate = Number(report.pass_rate || 0) * 100;
    const allPass = report.passed === report.total;
    let html =
      '<div class="owo-result-head">' +
      `<span class="owo-tag ${allPass ? "ok" : "bad"}">通过 <strong>${report.passed}/${report.total}</strong></span>` +
      `<span class="owo-tag">${rate.toFixed(1)}%</span>` +
      `<span class="owo-tag">总耗时 ${((report.total_duration_ms || 0) / 1000).toFixed(1)}s</span>` +
      `<span class="owo-tag">${esc(report.suite || suiteId)}</span>` +
      "</div>" +
      '<div class="owo-rows">';
    for (const caseResult of report.cases || []) {
      const ok = !!caseResult.passed;
      html +=
        '<div class="owo-row">' +
        `<span class="owo-row-mark ${ok ? "ok" : "bad"}">${ok ? "✔" : "✘"}</span>` +
        `<span class="owo-row-main" title="${esc(caseResult.name)}">${esc(caseResult.name)}</span>` +
        `<span class="owo-row-meta">${((caseResult.duration_ms || 0) / 1000).toFixed(1)}s · ${caseResult.steps || 0} 步</span>` +
        (caseResult.error ? `<span class="owo-row-note">${esc(caseResult.error)}</span>` : "") +
        "</div>";
    }
    html += "</div>";
    if (!(report.cases || []).length) html += '<span class="owo-result-empty">该套件没有用例</span>';
    box.innerHTML = html;
  } catch (error) {
    box.className = "owo-result";
    box.innerHTML = `<span class="owo-result-empty">评估失败：${esc(error.message)}</span>`;
  } finally {
    button.textContent = "运行评估";
    delete button.dataset.coreBusy;
    window.OwoCoreActionAvailability?.update(
      window.OwoCoreActionAvailability?.isAvailable() === true,
    );
  }
}

// ---------- Traces 可观测（v0.5.6） ----------

async function refreshTraces() {
  try {
    const data = await api("/traces");
    const list = $("traceList");
    list.innerHTML = "";
    const traces = data.traces || [];
    for (let index = 0; index < traces.length; index++) {
      const trace = traces[index];
      const li = document.createElement("li");
      const final = trace.has_final ? "✅" : "—";
      const usage = trace.usage && trace.usage.total_tokens ? ` ｜ ${trace.usage.total_tokens} tokens` : "";
      li.innerHTML =
        `<strong>${index} ${esc(trace.prompt_preview || trace.prompt || "")}</strong>` +
        `<span class="sub">${final} ${(trace.duration_ms / 1000).toFixed(1)}s ｜ ${trace.steps} 步 ｜ ${esc(trace.model)}${usage}</span>` +
        `<span class="sub">${esc((trace.started_at || "").slice(0, 19).replace("T", " "))}</span>`;
      li.addEventListener("click", () => showTrace(index));
      list.appendChild(li);
    }
    if (!traces.length) list.innerHTML = '<li class="sub">暂无轨迹（完成回合后自动记录）</li>';
  } catch (error) {
    $("traceList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

async function showTrace(index) {
  const box = $("traceReplay");
  box.className = "owo-result";
  box.innerHTML = '<span class="owo-result-empty">回放加载中…</span>';
  try {
    const trace = await api(`/traces/${index}`);
    const usage = trace.usage || {};
    const tags = [
      `<span class="owo-tag">${esc(trace.model || "未知模型")}</span>`,
      `<span class="owo-tag">${((trace.duration_ms || 0) / 1000).toFixed(1)}s</span>`,
      `<span class="owo-tag">${trace.steps || 0} 步</span>`,
    ];
    if (usage.total_tokens) {
      tags.push(
        `<span class="owo-tag">${usage.total_tokens} tokens` +
          (usage.prompt_tokens || usage.completion_tokens
            ? `（↑${usage.prompt_tokens || 0} ↓${usage.completion_tokens || 0}）`
            : "") +
          "</span>"
      );
    }
    let html =
      '<div class="owo-result-head">' + tags.join("") + "</div>" +
      '<div class="owo-rows">' +
      '<div class="owo-row"><span class="owo-row-main" title="' + esc(trace.prompt || "") + '">' +
      esc(trace.prompt || "（无 prompt）") +
      "</span></div>" +
      "</div>" +
      '<ol class="owo-timeline">';
    // token 流式增量合并成一条，避免时间线被几十条 delta 淹没。
    let deltaCount = 0;
    let deltaChars = 0;
    const flushDelta = () => {
      if (!deltaCount) return;
      html +=
        '<li><strong>流式输出</strong> <code>' +
        `${deltaCount} 段 / ${deltaChars} 字符` +
        "</code></li>";
      deltaCount = 0;
      deltaChars = 0;
    };
    for (const event of trace.events || []) {
      const type = event.type || "?";
      if (type === "token_delta") {
        deltaCount += 1;
        deltaChars += (event.delta || "").length;
        continue;
      }
      flushDelta();
      if (type === "model_call") {
        html += '<li class="ev-tool"><strong>模型调用</strong></li>';
      } else if (type === "tool_start") {
        html += `<li class="ev-tool"><strong>工具开始</strong> <code>${esc(event.tool || "")}</code></li>`;
      } else if (type === "tool_result") {
        html +=
          `<li class="${event.ok ? "ev-ok" : "ev-bad"}"><strong>工具结果</strong> ` +
          `<code>${esc(event.tool || "")}${event.ok ? "" : "（失败：" + esc(event.error || "未知") + "）"}</code></li>`;
      } else if (type === "permission_request") {
        html +=
          `<li class="ev-warn"><strong>审批请求</strong> <code>${esc(event.tool || "")}` +
          `${event.reason ? " · " + esc(event.reason) : ""}</code></li>`;
      } else if (type === "compaction") {
        html += `<li class="ev-warn"><strong>上下文压缩</strong> <code>${esc(event.summary || "")}</code></li>`;
      } else if (type === "final") {
        html += `<li class="ev-ok"><strong>最终汇报</strong> <code>${esc((event.text || "").slice(0, 160))}</code></li>`;
      } else {
        html += `<li><code>${esc(type)}</code></li>`;
      }
    }
    flushDelta();
    html += "</ol>";
    if (!(trace.events || []).length) html += '<span class="owo-result-empty">该轨迹没有事件</span>';
    box.innerHTML = html;
  } catch (error) {
    box.className = "owo-result";
    box.innerHTML = `<span class="owo-result-empty">回放失败：${esc(friendlyError(error, { resource: true }))}</span>`;
  }
}

async function exportSession(format) {
  if (!state.sessionId) {
    showToast("请先选择会话", "error");
    return;
  }
  try {
    const response = await apiRaw(
      `/session/${state.sessionId}/export/${format}`
    );
    if (!response.ok) throw new Error(await response.text());
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `session-${state.sessionId.slice(0, 8)}.${format === "html" ? "html" : "md"}`;
    link.click();
    URL.revokeObjectURL(url);
    addMessage("system", `已导出会话为 ${format.toUpperCase()}`);
  } catch (error) {
    addMessage("error", `导出失败：${friendlyError(error, { resource: true })}`);
  }
}

// ---------- 子代理 / 项目规则 / MCP 管理（v0.5.5，对标 Codex @explore/@subagent、AGENTS.md、/mcp） ----------

async function runSubagent() {
  const prompt = $("subagentPrompt").value.trim();
  if (!prompt) {
    addMessage("system", "请输入子代理任务");
    return;
  }
  const mode = $("subagentMode").value;
  const readOnly = mode === "read_only";
  // 命令执行需显式选择"通用子代理 + 命令执行"：无审批通道的子代理默认拒绝执行命令。
  const allowCommands = mode === "full_commands";
  const button = $("subagentRunBtn");
  const box = $("subagentResult");
  button.disabled = true;
  button.textContent = "运行中…";
  box.className = "owo-result";
  box.innerHTML = '<span class="owo-result-empty">子代理执行中（调用真实模型，请稍候）…</span>';
  try {
    const result = await api("/subagent/run", {
      method: "POST",
      body: JSON.stringify({ prompt, read_only: readOnly, allow_commands: allowCommands }),
    });
    const modeTag = result.read_only
      ? "只读探索"
      : result.allow_commands
        ? "通用子代理 + 命令执行"
        : "通用子代理";
    box.innerHTML =
      '<div class="owo-result-head">' +
      `<span class="owo-tag ${result.read_only || !result.allow_commands ? "" : "warn"}">${modeTag}</span>` +
      `<span class="owo-tag">${((result.duration_ms || 0) / 1000).toFixed(1)}s</span>` +
      "</div>" +
      `<div class="owo-result-body">${renderMarkdown(result.text || "（无汇报）")}</div>`;
  } catch (error) {
    box.className = "owo-result";
    box.innerHTML = `<span class="owo-result-empty">子代理失败：${esc(error.message)}</span>`;
  } finally {
    button.textContent = "运行子代理";
    delete button.dataset.coreBusy;
    window.OwoCoreActionAvailability?.update(
      window.OwoCoreActionAvailability?.isAvailable() === true,
    );
  }
}

async function refreshProjectRules() {
  try {
    const data = await api("/project/rules");
    const info = $("projectRulesInfo");
    info.innerHTML = "";
    for (const rule of data.rules || []) {
      const badge = rule.exists ? (rule.injected ? "✅ 注入" : "⚠️ 未注入") : "— 不存在";
      const div = document.createElement("div");
      div.textContent = `${rule.name}：${badge}`;
      info.appendChild(div);
      if (rule.name === "AGENTS.md") {
        const editor = $("agentsEditor");
        if (!editor.dataset.seeded) {
          editor.value = rule.content || "";
          editor.dataset.seeded = "1";
        }
      }
    }
  } catch (error) {
    $("projectRulesInfo").textContent = friendlyError(error);
  }
}

async function saveAgentsRules() {
  const content = $("agentsEditor").value;
  const button = $("agentsSaveBtn");
  button.dataset.coreBusy = "true";
  button.disabled = true;
  button.textContent = "保存中…";
  try {
    const result = await api("/project/rules", {
      method: "POST",
      body: JSON.stringify({ content }),
    });
    addMessage("system", `已保存 AGENTS.md（${result.chars} 字符），下次会话注入生效`);
    await refreshProjectRules();
  } catch (error) {
    addMessage("error", `保存失败：${error.message}`);
  } finally {
    button.textContent = "保存 AGENTS.md";
    delete button.dataset.coreBusy;
    window.OwoCoreActionAvailability?.update(
      window.OwoCoreActionAvailability?.isAvailable() === true,
    );
  }
}

async function generateAgentsTemplate() {
  try {
    const result = await api("/project/rules/template", { method: "POST" });
    showToast(`已生成 AGENTS.md 模板（${result.chars} 字符）`, "ok");
    $("agentsEditor").dataset.seeded = "0";
    await refreshProjectRules();
  } catch (error) {
    const msg = String((error && error.message) || error || "");
    if (msg.startsWith("409")) showToast("AGENTS.md 已存在，未生成模板（幂等）", "error");
    else showToast(`生成失败：${friendlyError(error)}`, "error");
  }
}

async function refreshMcp() {
  try {
    const data = await api("/mcp");
    const list = $("mcpList");
    const configured = data.servers || [];
    const connected = data.connected || [];
    list.innerHTML = "";
    let rendered = 0;
    for (const server of configured) {
      const li = document.createElement("li");
      const target = server.transport === "http" ? server.url : server.command;
      const isUp = connected.includes(server.name);
      li.innerHTML = `<strong>${esc(server.name)}</strong><span class="sub">${esc(server.transport)} ｜ ${esc(target || "")}${server.args && server.args.length ? " " + esc(server.args.join(" ")) : ""}${isUp ? " ｜ 已连接" : ""}</span>`;
      const removeBtn = document.createElement("button");
      removeBtn.textContent = "移除";
      removeBtn.addEventListener("click", async () => {
        try {
          await api("/mcp/remove", {
            method: "POST",
            body: JSON.stringify({ name: server.name }),
          });
          await refreshMcp();
          addMessage("system", `已移除 MCP 服务器 ${server.name}`);
        } catch (error) {
          addMessage("error", `移除失败：${friendlyError(error, { resource: true })}`);
        }
      });
      li.appendChild(removeBtn);
      list.appendChild(li);
      rendered++;
    }
    // 已连接但不在持久化配置里的服务器（插件 manifest 声明的 stdio 服务器随插件启用接入）：
    // 不展示就会出现「实际连着 3 个、页面却说没有」的误导。
    for (const name of connected) {
      if (configured.some((server) => server.name === name)) continue;
      const li = document.createElement("li");
      li.innerHTML = `<strong>${esc(name)}</strong><span class="sub">已连接（来源：插件 manifest，随插件启用自动接入，不可单独移除）</span>`;
      list.appendChild(li);
      rendered++;
    }
    if (!rendered) {
      list.innerHTML =
        '<li class="sub">暂无 MCP 服务器：可在下方表单添加，或启用带 mcp 段的插件</li>';
    }
  } catch (error) {
    $("mcpList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

function syncMcpFields() {
  const transport = $("mcpTransport").value;
  for (const kind of ["stdio", "http"]) {
    const group = document.querySelector('[data-mcp-group="' + kind + '"]');
    const input = $(kind === "stdio" ? "mcpCommand" : "mcpUrl");
    const active = kind === transport;
    if (group) group.hidden = !active;
    if (input) {
      input.disabled = !active;
      input.required = active;
    }
  }
}

async function addMcpServer() {
  const name = $("mcpName").value.trim();
  const transport = $("mcpTransport").value;
  const command = $("mcpCommand").value.trim();
  const url = $("mcpUrl").value.trim();
  if (!name) return;
  if (transport === "stdio" && !command) {
    addMessage("error", "stdio 传输需要填写启动命令");
    return;
  }
  if (transport === "http" && !url) {
    addMessage("error", "http 传输需要填写 URL");
    return;
  }
  try {
    const result = await api("/mcp/add", {
      method: "POST",
      body: JSON.stringify({
        name,
        transport,
        command: transport === "stdio" ? command : "",
        url: transport === "http" && url ? url : null,
      }),
    });
    addMessage("system", `MCP 服务器 ${name} 已连接（${result.tools} 个工具）`);
    $("mcpName").value = "";
    $("mcpCommand").value = "";
    $("mcpUrl").value = "";
    await refreshMcp();
  } catch (error) {
    addMessage("error", `添加失败：${error.message}`);
  }
}

// ---------- 白名单 ----------

async function refreshWhitelist() {
  try {
    const entries = await api("/whitelist");
    const list = $("whitelistList");
    list.innerHTML = "";
    for (const entry of entries) {
      const li = document.createElement("li");
      li.innerHTML = `<strong>${esc(entry.name)}</strong><span class="sub">${esc(entry.app_id)} ｜ ${esc(entry.tier)} ｜ 操作:${entry.auto_ops_allowed ? "开" : "关"}</span>`;
      const removeBtn = document.createElement("button");
      removeBtn.textContent = "移除";
      removeBtn.addEventListener("click", async (event) => {
        event.stopPropagation();
        try {
          await api("/whitelist/manage", {
            method: "POST",
            body: JSON.stringify({ action: "remove", app_id: entry.app_id }),
          });
          await refreshWhitelist();
        } catch (error) {
          addMessage("error", `白名单删除失败：${error.message || error}`);
        }
      });
      li.appendChild(removeBtn);
      list.appendChild(li);
    }
  } catch (error) {
    $("whitelistList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

// ---------- Computer-use 任务级审批（P2，文档 7.3 语义：批准目标应用+描述+最长时长+允许动作） ----------

async function refreshComputerTasks() {
  try {
    const data = await api("/computer-use/tasks");
    const list = $("computerTaskList");
    list.innerHTML = "";
    const tasks = data.tasks || [];
    for (const task of tasks) {
      const li = document.createElement("li");
      const status = String(task.state || "unknown").toLowerCase();
      const elapsed = task.elapsed_ms != null ? `${Math.round(task.elapsed_ms / 1000)}s / ` : "";
      const cap = task.max_duration_ms != null ? `${Math.round(task.max_duration_ms / 1000)}s` : "无上限";
      li.innerHTML =
        `<strong>${esc(task.target_app || task.app || "")}：${esc(task.description || "")}</strong>` +
        `<span class="sub">${esc(status)} ｜ ${elapsed}${cap} ｜ 动作 ${esc((task.allowed_actions || []).join(",")) || "全部"}</span>`;
      if (status.startsWith("pending")) {
        const approve = document.createElement("button");
        approve.textContent = "批准";
        approve.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/approve`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `批准失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(approve);
        const deny = document.createElement("button");
        deny.textContent = "拒绝";
        deny.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/reject`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `拒绝失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(deny);
        const cancel = document.createElement("button");
        cancel.textContent = "取消";
        cancel.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/cancel`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `取消失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(cancel);
      } else if (status.startsWith("running")) {
        const pause = document.createElement("button");
        pause.textContent = "暂停";
        pause.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/pause`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `暂停失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(pause);
        const abort = document.createElement("button");
        abort.textContent = "终止";
        abort.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/cancel`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `终止失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(abort);
      } else if (status.startsWith("paused")) {
        const resume = document.createElement("button");
        resume.textContent = "恢复";
        resume.addEventListener("click", async () => {
          try {
            await api(`/computer-use/task/${encodeURIComponent(task.id)}/resume`, { method: "POST", body: JSON.stringify({}) });
            await refreshComputerTasks();
          } catch (error) {
            addMessage("error", `恢复失败：${friendlyError(error)}`);
          }
        });
        li.appendChild(resume);
      }
      list.appendChild(li);
    }
    if (!tasks.length) list.innerHTML = '<li class="sub">暂无 computer-use 任务</li>';
  } catch (error) {
    $("computerTaskList").innerHTML = `<li class="sub">${esc(friendlyError(error))}</li>`;
  }
}

async function createComputerTask() {
  const targetApp = $("cuApp").value.trim();
  const description = $("cuDesc").value.trim();
  if (!targetApp || !description) {
    addMessage("system", "请填写目标应用与任务描述");
    return;
  }
  const maxDurationMs = (parseInt($("cuMaxDur").value, 10) || 120) * 1000;
  const allowedActions = $("cuActions").value.split(",").map((s) => s.trim()).filter(Boolean);
  try {
    const result = await api("/computer-use/task", {
      method: "POST",
      body: JSON.stringify({
        target_app: targetApp,
        description,
        max_duration_ms: maxDurationMs,
        allowed_actions: allowedActions,
      }),
    });
    addMessage("system", `已创建 computer-use 任务（${result.id}），等待任务级审批`);
    $("cuApp").value = "";
    $("cuDesc").value = "";
    $("cuMaxDur").value = "";
    $("cuActions").value = "";
    await refreshComputerTasks();
  } catch (error) {
    addMessage("error", `创建失败：${friendlyError(error)}`);
  }
}

// ---------- 扩展面板（第四轮：notes / plugin-market / workflow / goal；第五轮：team / eval / observability / memory / command） ----------

// 挂载顺序（与 index.html 的 script 引入顺序一致）。
const PANEL_ORDER = [
  "home",
  "evidence",
  "notes",
  "automations",
  "plugin-market",
  "workflow",
  "custom-workflows",
  "goal",
  "team",
  "eval",
  "product-eval",
  "observability",
  "diagnostics-ledger",
  "memory",
  "command",
  "fleet",
  "capabilities",
  "permissions",
  "action-center",
  "project-history",
  "project-launcher",
  "workswarm",
  "about",
];

function panelHelpers() {
  return {
    baseUrl: API_BASE,
    snapshot: () => ({ workspace: $("workspace")?.value || "", settings: state.settings || {}, sessions: state.sessionIndex || [], sessionsKnown: state.sessionsKnown === true }),
    openPanel: openPanelById,
    downloadArtifact: async id => {
      if (window.owo?.downloadArtifact) return window.owo.downloadArtifact(id);
      const blob = await window.OwoApi.download("/artifacts/" + encodeURIComponent(id) + "/content?raw=true");
      const url = URL.createObjectURL(blob), link = document.createElement("a"); link.href = url; link.download = "artifact"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    },
    openChat: async () => { setSettingsPageVisible(false); await newSession(); $("prompt").focus(); },
    openSession: async id => { setSettingsPageVisible(false); await selectSession(id); },
    openTeam: id => { openPanelById("workswarm"); window.OwoPanels.workswarm.open(id); },
    get(path, options) {
      const requestOptions = Object.assign({}, options || {});
      // /health is intentionally public in the server contract. Tool panels must be
      // able to show service state without bootstrapping a desktop bearer first.
      if (path === "/health") requestOptions.public = true;
      return api(path, requestOptions);
    },
    post(path, body) {
      return api(path, { method: "POST", body: JSON.stringify(body || {}) });
    },
    put(path, body) {
      return api(path, { method: "PUT", body: JSON.stringify(body || {}) });
    },
    del(path) {
      return api(path, { method: "DELETE" });
    },
    // 任意 method/无 body 的请求（如自动化 toggle/clear 的空体 POST）。
    call(path, options) {
      return api(path, options);
    },
    // 面板内统一走样式化弹窗（原生 confirm/prompt 会阻塞渲染且无法定制）
    confirm: askConfirm,
    prompt: askText,
    notify(message, kind) {
      showToast(message, kind || "");
    },
    esc,
    friendlyError,
    renderMarkdown,
  };
}

let currentPanel = null;
const panelScrollPositions = new Map();
function unmountActivePanel() {
  const root = $("panelRoot");
  if (currentPanel && root) panelScrollPositions.set(currentPanel, root.scrollTop);
  const panel = window.OwoPanels?.[currentPanel];
  if (panel && typeof panel.dispose === "function") panel.dispose();
  currentPanel = null;
  markSidebarPanel(null);
  clearPanelLayoutObservers();
}

/// 主侧栏的工作入口跟随当前功能页高亮（功能页内主侧栏常驻，用户始终知道自己在哪）。
function markSidebarPanel(id) {
  for (const button of document.querySelectorAll(".codex-sidebar [data-workbench-panel]")) {
    const current = Boolean(id) && button.dataset.workbenchPanel === id;
    button.classList.toggle("is-current", current);
    if (current) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
}
const panelObserverScope = window.OwoObserverScope.createObserverScope();
let stopPanelAccessibility = null;

// The panel slot is shared, but each mount gets a fresh DOM root. Late replies from
// the previous panel can then update only its detached root, never the active page.
function clearPanelLayoutObservers() {
  panelObserverScope.clear();
  stopPanelAccessibility?.(); stopPanelAccessibility = null;
  panelTocSync = null;
}

function mountPanel(id, writeHash = true) {
  clearPanelLayoutObservers();
  const panel = window.OwoPanels && window.OwoPanels[id];
  const previousRoot = $("panelRoot");
  if (!panel || !previousRoot) return;
  if (currentPanel) panelScrollPositions.set(currentPanel, previousRoot.scrollTop);
  document.body.classList.add("panel-open");
  const description = window.OwoWorkbenchNavigation.find(id);
  const newTaskButton = $("workbenchPanelNewTask");
  if (newTaskButton) newTaskButton.hidden = id === "project-launcher";
  if ($("workbenchPanelTitle")) $("workbenchPanelTitle").textContent = description?.title || panel.title || id;
  if ($("workbenchPanelDescription")) $("workbenchPanelDescription").textContent = description?.description || "";
  // 每次替换根节点前都释放上一个实例，即使用户再次点开同一面板。
  // 工作流、目标云端订阅和 WorkSwarm 都有监听/SSE；同面板重挂载也必须 dispose。
  if (currentPanel) {
    const prev = window.OwoPanels[currentPanel];
    if (prev && typeof prev.dispose === "function") {
      try { prev.dispose(); } catch { /* 清理失败不打断挂载 */ }
    }
  }
  currentPanel = id;
  markSidebarPanel(id);
  // 面板深链：#<panel-id>，便于分享/直达/自动化验证（不改动其它状态）。
  // 引导时的默认挂载传 writeHash=false：否则地址栏被钉上 #notes，刷新后
  // applyDeepLink 又把它当深链 → 每次打开工作台都停在工具视图（用户实测反馈）。
  if (writeHash) workbenchView.setRoute(id);
  for (const button of document.querySelectorAll("#panelNav button")) {
    button.classList.toggle("active", button.dataset.panel === id);
    if (button.dataset.panel === id) {
      const section = button.closest("details");
      if (section) { section.open = true; if (section.dataset.searchWasOpen != null) section.dataset.searchWasOpen = "true"; }
    }
    if (button.dataset.panel === id) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
  const root = previousRoot.cloneNode(false);
  previousRoot.replaceWith(root);
  const showMountError = (error) => {
    if (currentPanel !== id || $("panelRoot") !== root) return;
    if (panel && typeof panel.dispose === "function") {
      try { panel.dispose(); } catch { /* failed mount cleanup is best-effort */ }
    }
    root.replaceChildren();
    const card = document.createElement("section");
    card.className = "panel-empty-state panel-mount-error";
    card.setAttribute("role", "alert");
    const title = document.createElement("strong");
    title.textContent = "无法打开「" + (panel.title || id) + "」";
    const detail = document.createElement("span");
    detail.textContent = "这个功能页启动时遇到问题。可以重试；其他工作区功能仍可继续使用。";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "secondary";
    retry.textContent = "重新加载此页面";
    retry.addEventListener("click", () => mountPanel(id, writeHash));
    card.append(title, detail, retry);
    root.appendChild(card);
    console.error("[panel] mount failed:", id, error && error.message ? error.message : "unknown");
  };
  try {
    window.OwoPanelRuntime.mount(panel, root, panelHelpers(), showMountError);
    layoutPanel(root);
    stopPanelAccessibility = window.OwoAccessibility?.watch(root);
    const heading = root.querySelector("h2");
    if (heading && description && heading.textContent.trim() === description.title) heading.hidden = true;
    window.OwoCoreActionAvailability?.update(window.OwoCoreActionAvailability.isAvailable(), root);
    requestAnimationFrame(() => { if ($("panelRoot") === root) root.scrollTop = panelScrollPositions.get(id) || 0; });
  } catch (error) {
    showMountError(error);
  }
}

/// 扩展面板统一排版：把面板根容器的"卡片型"子元素排成响应式两列网格，
/// 宽内容（表格/编辑区/表单/代码块）整行跨列。各面板根容器类名不一
/// （`.stack` / 自有类 / 直接挂在 section 下），因此在挂载后按 DOM 结构判定，
/// 免去逐个改面板 HTML。
function layoutPanel(root) {
  const section = root.querySelector("section[data-panel]");
  if (!section) return;
  // A panel may opt into a deliberate, panel-owned grid when its content has a
  // stable information architecture that the generic content-width heuristic
  // cannot infer (for example, the Team package tools).
  if (section.dataset.layout === "custom") return;
  const isBox = (el) => el.tagName === "DIV" || el.tagName === "SECTION";
  const wrapper = Array.from(section.children).find(isBox);
  if (wrapper) buildPanelToc(section, wrapper);
  // 先试「单一根容器」（.stack 等），不行再退回把 section 自身当容器
  // （notes/team/fleet/plugin-market 的骨架直接挂在 section 下）。
  const candidates = [];
  if (wrapper && wrapper.children.length >= 3) candidates.push(wrapper);
  if (section !== wrapper) candidates.push(section);
  for (const container of candidates) {
    const children = Array.from(container.children).filter((el) => el.tagName !== "STYLE");
    if (children.length < 3) continue;
    const wideFlags = children.map((child) => panelBlockIsWide(child));
    const cards = wideFlags.filter((flag) => !flag).length;
    // 可分的卡片太少（几乎全是整行块）就不折腾，保持原单列。
    if (cards < 2 || cards / children.length < 0.2) continue;
    // 标题与紧随其后的分区内容作为一个逻辑行，避免标题全宽但内容只落左列。
    const spanFlags = window.OwoPanelRuntime.layoutFlags(children, wideFlags);
    children.forEach((child, index) => {
      if (spanFlags[index]) child.classList.add("owo-span-all");
    });
    container.classList.add("owo-cols");
    // 自愈：
    // ① 内容溢出（超宽表格/长串）→ 整行跨列，避免横向滚动；
    // ② 异常高的块（长表/图表/长编辑器）→ 限高内滚，避免把整页拉成"一长条"。
    // 面板内容多为异步加载，块高度在挂载后才长出来 → 用 ResizeObserver 持续自愈。
    const heal = () => {
      for (const child of children) {
        if (child.scrollWidth > child.clientWidth + 4) {
          child.classList.add("owo-span-all");
        } else if (child.getBoundingClientRect().height > 700) {
          child.classList.add("owo-tall");
        }
      }
    };
    requestAnimationFrame(heal);
    let pending = false;
    panelObserverScope.observe(container, () => {
      if (pending) return;
      pending = true;
      requestAnimationFrame(() => {
        pending = false;
        heal();
      });
    });
    return;
  }
}

/// 当前面板目录的滚动联动函数（document 捕获阶段滚动监听只注册一次）。
let panelTocSync = null;

/// 面板分区目录（锚点侧栏）：分区标题（`.sub` / H2~H4）≥3 个时，
/// 在面板左侧生成吸顶目录，点击滚到对应分区——长面板不必一路往下找。
function buildPanelToc(section, container) {
  const headings = Array.from(container.children).filter(
    (el) => !el.hasAttribute("data-core-action-hint") && (el.classList.contains("sub") || /^H[2-4]$/.test(el.tagName))
  );
  if (headings.length < 3) return;
  const nav = document.createElement("nav");
  nav.className = "owo-toc";
  nav.setAttribute("aria-label", "面板分区");
  const items = [];
  headings.forEach((heading, index) => {
    if (!heading.id) heading.id = `owo-sec-${index}-${Math.random().toString(36).slice(2, 7)}`;
    const label = (heading.textContent || "").trim().replace(/\s+/g, " ");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "owo-toc-item";
    button.textContent = label.length > 16 ? `${label.slice(0, 16)}…` : label || `分区 ${index + 1}`;
    button.title = label;
    button.addEventListener("click", () => {
      heading.scrollIntoView({ behavior: "smooth", block: "start" });
      setActiveTocItem(items, button);
    });
    items.push({ button, heading });
    nav.appendChild(button);
  });
  // 滚动联动高亮（rAF 节流）：视口顶部最近的已越过分区即为当前分区。
  // 注意滚动发生在内层容器（`body.tools-open #sidebar` 是滚动容器，html/body 不滚），
  // 因此用 document 的捕获阶段监听（scroll 不冒泡但可捕获），一次注册常驻。
  const sync = () => {
    let current = items[0];
    for (const item of items) {
      // 阈值略高于吸顶条高度：点目录跳转后（标题停在 scroll-margin-top 处）即为当前项。
      if (item.heading.getBoundingClientRect().top <= 90) current = item;
    }
    setActiveTocItem(items, current && current.button);
  };
  panelTocSync = sync;
  if (!document.body.dataset.owoTocBound) {
    document.body.dataset.owoTocBound = "1";
    let ticking = false;
    document.addEventListener(
      "scroll",
      () => {
        if (ticking || !panelTocSync) return;
        ticking = true;
        requestAnimationFrame(() => {
          ticking = false;
          panelTocSync();
        });
      },
      { capture: true, passive: true }
    );
  }
  section.insertBefore(nav, container);
  section.classList.add("owo-toc-layout");
  addSectionFolding(headings);
  sync();
  // 面板内容异步加载：块高度在挂载后才长出来，用 ResizeObserver 复查折叠条件与高亮。
  let revisiting = false;
  panelObserverScope.observe(container, () => {
    if (revisiting) return;
    revisiting = true;
    requestAnimationFrame(() => {
      revisiting = false;
      addSectionFolding(headings);
      sync();
    });
  });
}

/// 长分区折叠（内容级"精简短页"的安全做法：不删内容，可展开）：
/// 某分区（标题到下一个标题之间）的块总高 >700px 时，在标题尾部加「收起/展开」。
/// 幂等——已加过按钮的标题不重复添加（ResizeObserver 自愈会多次调用）。
function addSectionFolding(headings) {
  headings.forEach((heading, index) => {
    if (heading.querySelector(".owo-fold")) return;
    const next = headings[index + 1];
    const blocks = [];
    for (let el = heading.nextElementSibling; el && el !== next; el = el.nextElementSibling) {
      if (el.tagName !== "STYLE") blocks.push(el);
    }
    if (blocks.length < 2) return;
    const total = blocks.reduce((sum, el) => sum + el.getBoundingClientRect().height, 0);
    if (total < 700) return;
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "owo-fold";
    toggle.textContent = "收起";
    toggle.title = "折叠该分区（内容不丢，可随时展开）";
    toggle.addEventListener("click", (event) => {
      event.stopPropagation();
      const collapsed = blocks.every((el) => el.classList.contains("owo-folded"));
      for (const el of blocks) el.classList.toggle("owo-folded", !collapsed);
      toggle.textContent = collapsed ? "收起" : "展开";
    });
    heading.appendChild(toggle);
  });
}

function setActiveTocItem(items, button) {
  for (const item of items) item.button.classList.toggle("active", item.button === button);
}

/// 该面板块是否应当整行跨列（宽内容 / 标题 / 工具条）。
/// 判定偏保守：宁可整行（不会破版），也不要硬塞进窄列。
/// 注意单行 `input/select` 不算宽内容（否则可编辑行永远分不了栏）。
function panelBlockIsWide(el) {
  if (/^(H[1-4]|BUTTON|FORM|TABLE|TEXTAREA|PRE)$/.test(el.tagName)) return true;
  if (el.tagName === "UL" || el.tagName === "OL") return el.children.length > 6;
  if (
    el.matches("canvas, svg, .owo-editor") ||
    el.querySelector(
      "table, textarea, form, pre, canvas, iframe, .owo-editor, .owo-mtr-table"
    )
  ) {
    return true;
  }
  const classes = String(el.className || "").split(/\s+/);
  return classes.some(
    (name) =>
      name === "sub" ||
      name === "toolbar" ||
      name === "actions" ||
      name.endsWith("-head") ||
      name.endsWith("-toolbar") ||
      name.endsWith("-actions") ||
      name.endsWith("-bar")
  );
}

function panelFromHash() {
  const id = (location.hash || "").replace(/^#/, "");
  return id && window.OwoPanels && window.OwoPanels[id] ? id : null;
}

// 深链打开：确保工具视图可见，再把对应面板挂上。
function openPanelById(id) {
  if (!id || !window.OwoPanels || !window.OwoPanels[id]) return false;
  // showTools also leaves settings without creating an intermediate chat route.
  // Clear the sidebar group filter so the shared extension panel stays visible.
  window.OwoWorkbenchView.clearGroupFilters(document.body);
  setToolsVisible(true, true);
  // The focused page does not hydrate the hidden tools overview.
  mountPanel(id);
  // 深链落到面板本身：扩展面板区在工具视图下方，需要滚动过去才算"打开"。
  const target = $("panelRoot");
  if (target) $("sidebar").scrollTop = 0;
  return true;
}

function filterPanelNavigation(nav, query) {
  const searching = Boolean(String(query || "").trim());
  const matches = new Set(window.OwoWorkbenchNavigation.search(query, PANEL_ORDER).map(item => item.id));
  for (const button of nav.querySelectorAll("button[data-panel]")) button.hidden = !matches.has(button.dataset.panel);
  for (const section of nav.querySelectorAll(".workbench-nav-section")) {
    if (searching && section.dataset.searchWasOpen == null) section.dataset.searchWasOpen = String(section.open);
    const visible = Array.from(section.querySelectorAll("button[data-panel]")).some(button => !button.hidden);
    section.hidden = !visible;
    if (searching) section.open = visible;
    else if (section.dataset.searchWasOpen != null) {
      section.open = section.dataset.searchWasOpen === "true";
      delete section.dataset.searchWasOpen;
    }
  }
}

function initPanels() {
  const nav = $("panelNav");
  if (!nav) return;
  window.OwoPanels = window.OwoPanels || {};
  window.OwoPanels.baseUrl = API_BASE;
  nav.innerHTML = "";
  for (const group of window.OwoWorkbenchNavigation.sections(PANEL_ORDER)) {
    const section = document.createElement(group.primary ? "div" : "details");
    section.className = group.primary ? "workbench-nav-primary" : "workbench-nav-section";
    if (!group.primary) {
      const summary = document.createElement("summary");
      summary.textContent = group.label;
      section.appendChild(summary);
    }
    for (const entry of group.entries) {
      const id = entry.id;
      if (!window.OwoPanels[id]) continue;
      const button = document.createElement("button");
      button.textContent = entry.title;
      button.title = entry.description;
      button.dataset.panel = id;
      button.addEventListener("click", () => openPanelById(id));
      section.appendChild(button);
    }
    nav.appendChild(section);
  }
  $("panelSearch")?.addEventListener("input", (event) => filterPanelNavigation(nav, event.target.value));
  // 保留有效的工具/设置深链；旧面板、拼错的 hash 不得劫持默认会话页。
  const initialRoute = window.OwoWorkbenchView.resolveInitialRoute(location.hash, PANEL_ORDER);
  if (location.hash && initialRoute.kind === "chat") {
    history.replaceState(null, "", location.pathname + location.search);
  }
  // 首屏默认是对话视图；检查器面板直到用户显式打开才挂载并请求数据。
  // 旧行为自动挂载第一个面板（通常是 Notes），在设置门之前发出额外 API 请求。
  const root = $("panelRoot");
  if (root) {
    root.innerHTML = '<div class="panel-empty-state"><strong>选择工具面板</strong><span>需要查看项目状态或管理能力时，再从上方选择对应面板。</span></div>';
  }
}

window.addEventListener("hashchange", () => applyDeepLink());
window.addEventListener("popstate", () => { if (!applyDeepLink()) setSettingsPageVisible(false); });

function openWorkbenchSearch() {
  const entries = window.OwoWorkbenchNavigation.search("", PANEL_ORDER).filter(item => window.OwoPanels?.[item.id]);
  openModal({ title: "查找功能", body: '<label><span class="sr-only">搜索功能</span><input id="workbenchQuickSearch" type="search" placeholder="搜索任务、笔记、权限或工具…"></label><div class="workbench-quick-results">' + entries.map(item => '<button type="button" data-quick-panel="' + esc(item.id) + '"><strong>' + esc(item.title) + '</strong><span>' + esc(item.description) + '</span></button>').join("") + "</div>" });
  const body = $("modalBody");
  const input = $("workbenchQuickSearch");
  input.addEventListener("input", () => {
    const matches = new Set(window.OwoWorkbenchNavigation.search(input.value, PANEL_ORDER).map(item => item.id));
    for (const button of body.querySelectorAll("[data-quick-panel]")) button.hidden = !matches.has(button.dataset.quickPanel);
  });
  for (const button of body.querySelectorAll("[data-quick-panel]")) button.addEventListener("click", () => { closeModal(); openPanelById(button.dataset.quickPanel); });
  input.addEventListener("keydown", event => { if (event.key === "Enter") { const first = Array.from(body.querySelectorAll("[data-quick-panel]")).find(button => !button.hidden); if (first) first.click(); } });
}

// 深链：#<panel-id> 打开对应面板，#settings 打开设置页（便于分享与逐页截图验证）。
function applyDeepLink() {
  const raw = (location.hash || "").replace(/^#/, "");
  if (!raw) return false;
  if (raw === "settings") {
    setToolsVisible(false);
    setSettingsPageVisible(true);
    return true;
  }
  return openPanelById(panelFromHash());
}

// ---------- 事件绑定 ----------
