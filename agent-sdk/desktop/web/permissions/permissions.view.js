/* 权限中心 · 渲染层（§4.5 / §4.8）。
 *
 * 这个文件只回答一个问题：**"权限中心该长成什么样、用户点的东西怎么变成一次动作"**。
 * 它注册 `window.OwoPanels.permissions`，把 controller 的状态快照渲染成 DOM，
 * 并用**事件委托**（挂在容器上的一个 click/change 监听）把动作转回 controller。
 *
 * 边界（§4.8）：零直接网络调用、零 Tauri invoke、零业务判断——网络在 api 层、
 * 判定在 domain 层、时机在 controller 层；本文件只做字符串模板与 data-* 派发。
 */
(function (global) {
  "use strict";

  const ID = "permissions";

  function defaultEsc(value) {
    return String(value == null ? "" : value).replace(/[&<>"']/g, function (ch) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch];
    });
  }

  function isPlainObject(value) {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
  }

  function text(value) {
    return value == null ? "" : String(value);
  }

  let H = {}; // 宿主注入的 helpers（esc 等），缺省自建
  let rootEl = null;
  let controller = null;
  let delegated = false;

  function esc(value) {
    return (H.esc || defaultEsc)(value);
  }

  function domain() {
    return global.OwoPermissionsDomain;
  }

  function find(list, index) {
    const rows = Array.isArray(list) ? list : [];
    return rows[index] || null;
  }
  // ---------- 片段 ----------

  function styleBlock() {
    return (
      "<style>" +
      ".owo-perm-grid{display:grid;gap:10px}" +
      ".owo-perm-table{width:100%;border-collapse:collapse;font-size:12px}" +
      ".owo-perm-table th,.owo-perm-table td{border:1px solid #ddd;padding:5px 7px;text-align:left;vertical-align:top}" +
      ".owo-perm-card{border:1px solid #ddd;border-radius:8px;padding:8px 10px;margin:6px 0;background:#fafbfc}" +
      ".owo-perm-kv{display:flex;gap:6px;align-items:baseline;font-size:12px}" +
      ".owo-perm-kv span{color:#666;min-width:84px}" +
      ".owo-perm-bad{color:#c62828}" +
      ".owo-perm-warn{color:#b26a00}" +
      ".owo-perm-empty{padding:8px 10px;border:1px dashed #ccc;border-radius:8px;color:#666;font-size:12px}" +
      ".owo-perm-risk{margin:4px 0 4px 18px;padding:0}" +
      ".owo-perm-locked{background:#f5f6f8;color:#555}" +
      ".owo-perm-grid button:disabled,.owo-perm-grid select:disabled,.owo-perm-grid input:disabled{opacity:.55;cursor:not-allowed}" +
      "</style>"
    );
  }

  function renderHeader(snap) {
    const d = domain();
    return (
      '<div class="owo-perm-kv"><span>当前档位</span><strong>' +
      esc(d ? d.profileLabel(snap.profile) : snap.profile) +
      "</strong></div>" +
      '<div class="sub">档位由服务端展开为权威策略矩阵；本页只展示生效值，不在前端重算。</div>' +
      (snap.readOnly ? '<div class="owo-perm-warn">核心处于只读降级：写入类授权可能不生效。</div>' : "")
    );
  }

  /**
   * 四维开关表：每行显示「当前生效值 + 来源 + 人类可读范围摘要」。
   * 不可独立配置的维度**诚实标注**（纯文本 + 「不可单独配置」），绝不渲染假控件。
   */
  function renderDimensionTable(snap) {
    const d = domain();
    const rows = Array.isArray(snap.dimensions) ? snap.dimensions : [];
    if (!rows.length) return '<div class="owo-perm-empty">未收到服务端的权限维度矩阵，暂不展示可配置项。</div>';
    const draft = snap.draft || {};
    const editable = canMutate(snap);
    const body = rows
      .map((row) => {
        const key = text(row.key);
        const effective = d ? d.summarizeScope(row.effective) : text(row.effective);
        const source = d ? d.sourceLabel(row.source) : text(row.source);
        const summary = text(row.summary) || "（服务端未提供范围说明）";
        const control = row.configurable ? dimensionControl(key, draft[key], editable) : honestNonConfigurable(effective);
        return (
          '<tr' + (Array.isArray(snap.changedFields) && snap.changedFields.includes(key) ? ' class="owo-perm-changed"' : "") + ">" +
          "<th>" + esc(text(row.label) || (d ? d.dimensionLabel(key) : key)) + "</th>" +
          "<td>" + esc(effective || "未知") + "</td>" +
          "<td>" + esc(source) + "</td>" +
          "<td>" + esc(summary) + "</td>" +
          "<td>" + control + (Array.isArray(snap.changedFields) && snap.changedFields.includes(key) ? '<span class="owo-perm-change-note">将从 ' + esc(effective) + " 变为 " + esc(d ? d.summarizeScope(draft[key]) : draft[key]) + "</span>" : "") + "</td>" +
          "</tr>"
        );
      })
      .join("");
    return (
      '<table class="owo-perm-table"><thead><tr><th>维度</th><th>当前生效</th><th>来源</th><th>范围摘要</th><th>可配置项</th></tr></thead>' +
      "<tbody>" +
      body +
      "</tbody></table>" +
      '<div class="sub">改动只影响本工作区；维度只能收紧，不能绕过档位既有禁令（服务端权威校验）。</div>'
    );
  }

  function dimensionControl(key, value, editable) {
    const d = domain();
    if (!d) return "";
    if (key === "persistence") {
      return selectControl("perm-dimension-persistence", d.PERSISTENCE_VALUES, text(value), d.persistenceLabel, editable);
    }
    const allowed =
      key === "filesystem" ? d.FILESYSTEM_VALUES : key === "command" ? d.COMMAND_VALUES : key === "network" ? d.NETWORK_VALUES : null;
    if (!allowed) return "";
    return selectControl("perm-dimension-" + key, allowed, text(value), d.summarizeScope, editable);
  }

  function selectControl(name, values, current, labeler, editable) {
    const d = domain();
    const placeholder = values.includes(current) ? "" : '<option value="" selected>未返回生效值</option>';
    const options = values
      .map((value) => '<option value="' + esc(value) + '"' + (value === current ? " selected" : "") + ">" + esc(labeler ? labeler(value) : value) + "</option>")
      .join("");
    const label = d ? d.dimensionLabel(name.replace("perm-dimension-", "")) + "配置" : name;
    return '<select name="' + esc(name) + '" aria-label="' + esc(label) + '"' + (editable ? "" : " disabled") + '>' + placeholder + options + "</select>";
  }

  /** 不可配置维度的诚实呈现：没有 <select>/<input>，只有文字与原因。 */
  function honestNonConfigurable(effective) {
    return '<span class="owo-perm-warn" data-perm-nonconfigurable="1">不可单独配置（当前：' + esc(effective || "未知") + "）</span>";
  }

  function renderScopes(snap) {
    const editable = canMutate(snap);
    const disabled = editable ? "" : " disabled";
    const scopes = snap.draft && Array.isArray(snap.draft.scopes) ? snap.draft.scopes : [];
    const chips = scopes
      .map((scope, index) => '<li><code>' + esc(scope) + '</code> <button type="button" class="secondary" data-perm-action="drop-scope" data-index="' + index + '"' + disabled + '>移除</button></li>')
      .join("");
    return (
      "<h4>允许范围（工作区相对路径）</h4>" +
      (chips ? "<ul>" + chips + "</ul>" : '<div class="owo-perm-empty">未添加额外范围：使用档位默认范围即可。</div>') +
      '<div class="inline"><input id="permScopeInput" placeholder="如 src/** （工作区内相对路径）" aria-label="新增允许范围"' + disabled + '>' +
      '<button type="button" id="permAddScope" data-perm-action="add-scope"' + disabled + '>添加范围</button></div>'
    );
  }

  function renderFullAccess(snap) {
    const editable = canMutate(snap);
    const disabled = editable ? "" : " disabled";
    const info = isPlainObject(snap.fullAccess) ? snap.fullAccess : {};
    const risks = Array.isArray(info.risk_notes) ? info.risk_notes : [];
    const active = info.active === true;
    const confirming = snap.confirming;
    const parts = [
      "<h4>完全访问</h4>",
      '<div class="owo-perm-kv"><span>状态</span><strong class="' + (active ? "owo-perm-bad" : "") + '">' +
        (active ? "已开启" : "未开启") +
        "</strong></div>",
      risks.length
        ? '<ul class="owo-perm-risk">' + risks.map((note) => "<li>" + esc(note) + "</li>").join("") + "</ul>"
        : '<div class="sub">服务端未提供风险清单；开启前仍会列出范围与时长。</div>',
    ];
    if (confirming) {
      const durations = (Array.isArray(confirming.durationOptions) ? confirming.durationOptions : [])
        .map(
          (option) =>
            '<label class="inline"><input type="radio" name="perm-duration" value="' +
            esc(option.secs) +
            '"' +
            (Number(option.secs) === Number(confirming.durationSecs) ? " checked" : "") +
            disabled + "> " +
            esc(option.label) +
            "</label>",
        )
        .join("");
      parts.push(
        '<div class="owo-perm-card" data-perm-confirm="1">' +
          "<b>二次确认：完全访问</b>" +
          '<div class="sub">三要素缺一不可 —— 范围、时长、风险。</div>' +
          '<div class="owo-perm-kv"><span>范围</span><code>' +
          esc(JSON.stringify({ filesystem: confirming.spec.filesystem, command: confirming.spec.command, network: confirming.spec.network })) +
          "</code></div>" +
          '<div class="owo-perm-kv"><span>时长</span></div>' +
          '<div class="inline" data-perm-durations="1">' +
          durations +
          "</div>" +
          "<ul class=\"owo-perm-risk\">" +
          (confirming.riskNotes.length ? confirming.riskNotes.map((note) => "<li>" + esc(note) + "</li>").join("") : "<li>不受限的命令与网络访问可能造成不可逆的数据外发或本地破坏。</li>") +
          "</ul>" +
          '<div class="inline"><button type="button" class="primary danger" data-perm-action="confirm-full-access"' + disabled + '>确认开启（含时长）</button>' +
          '<button type="button" class="secondary" data-perm-action="cancel-confirm">取消</button></div>' +
          "</div>",
      );
    } else {
      parts.push(
        '<div class="inline"><button type="button" class="primary" data-perm-action="request-full-access"' + disabled + '>申请完全访问（需二次确认）</button>' +
          (active || hasUnrestricted(snap.draft)
            ? '<button type="button" class="secondary" data-perm-action="close-full-access"' + disabled + '>一步关闭完全访问</button>'
            : "") +
          "</div>",
      );
    }
    return parts.join("");
  }

  function hasUnrestricted(spec) {
    if (!isPlainObject(spec)) return false;
    return text(spec.command) === "unrestricted" || text(spec.network) === "unrestricted";
  }

  /** 待审批列表：会话/工具/级别/理由/参数摘要 + 四个动作。空列表是空态，不是错误态。 */
  function renderPending(snap) {
    const d = domain();
    const groups = d ? d.groupByDimension(snap.pending) : [];
    const editable = canMutate(snap);
    const disabled = editable ? "" : " disabled";
    const notices = (snap.goneNotices || []).map((note) => '<div class="sub owo-perm-warn">' + esc(note) + "</div>").join("");
    if (!groups.length) {
      return (
        "<h4>待审批请求</h4>" +
        notices +
        '<div class="owo-perm-empty" data-perm-empty="pending">当前没有等待审批的请求。出现新的工具申请时会自动列在这里。</div>'
      );
    }
    const actions = d ? d.APPROVAL_ACTIONS : [];
    const html = groups
      .map((group) => {
        let seq = -1;
        const items = group.items
          .map((item) => {
            seq += 1;
            const availableActions =
              item.destructive === false && item.level === "read"
                ? actions
                : actions.filter((action) => !action.allow || action.scope === "once");
            const buttons = availableActions
              .map(
                (action) =>
                  '<button type="button" class="' +
                  (action.allow ? "allow" : "deny") +
                  '" data-perm-action="approval" data-approval="' +
                  esc(action.action) +
                  '" data-pending="' +
                  seq +
                  '" title="' +
                  esc(action.label) +
                  '"' + disabled + '>' +
                  esc(action.label) +
                  "</button>",
              )
              .join("");
            return (
              '<div class="owo-perm-card" id="owo-perm-pending-' + seq + '" data-request-id="' + esc(item.request_id) + '">' +
              "<b>" + esc(text(item.tool) || "未知工具") + "</b>" +
              '<div class="owo-perm-kv"><span>会话</span><code>' + esc(text(item.session_id) || "—") + "</code></div>" +
              '<div class="owo-perm-kv"><span>级别</span>' + esc(text(item.level) || "—") + "</div>" +
              '<div class="owo-perm-kv"><span>理由</span>' + esc(text(item.reason) || "（未提供）") + "</div>" +
              (item.explain ? '<div class="owo-perm-kv"><span>说明</span>' + esc(text(item.explain)) + "</div>" : "") +
              '<div class="owo-perm-kv"><span>参数摘要</span><code>' + esc(d ? d.argsSummary(item.redacted_args) : JSON.stringify(item.redacted_args || {})) + "</code></div>" +
              (item.risk_note ? '<div class="owo-perm-kv"><span>风险</span><span class="owo-perm-warn">' + esc(text(item.risk_note)) + "</span></div>" : "") +
              '<div class="inline approval-actions">' +
              buttons +
              "</div>" +
              "</div>"
            );
          })
          .join("");
        return '<div class="owo-perm-group" data-dimension="' + esc(group.key) + '"><h5>' + esc(group.label) + "</h5>" + items + "</div>";
      })
      .join("");
    return "<h4>待审批请求</h4>" + notices + html;
  }

  /** 已授予权限 + 三粒度撤销（单条 / 按工具 / 当前工作区全部）。 */
  function renderGrants(snap) {
    const d = domain();
    const editable = canMutate(snap);
    const disabled = editable ? "" : " disabled";
    const grants = Array.isArray(snap.grants) ? snap.grants : [];
    if (!grants.length) {
      return '<h4>已授予的权限</h4><div class="owo-perm-empty" data-perm-empty="grants">暂无长期授权记录。审批时选择「本任务」或「工作区长期」会出现在这里。</div>';
    }
    const rows = grants
      .map((grant, index) => {
        const expiry = grant.expires_at ? "到期：" + esc(text(grant.expires_at)) : "无固定到期";
        const uses = grant.remaining_uses == null ? "次数不限" : "剩余 " + esc(grant.remaining_uses) + " 次";
        return (
          "<tr" + ' data-grant-row="' + esc(index) + '">' +
          "<td>" + esc(text(grant.tool_id) || "—") +
            (grant.effective === false ? '<div class="owo-perm-warn">历史授权已失效，当前不会自动放行</div>' : "") +
            "</td>" +
          "<td>" + esc(d ? d.persistenceLabel(grant.scope) : text(grant.scope)) + "</td>" +
          "<td>" + expiry + "</td>" +
          "<td>" + uses + "</td>" +
          "<td>" + scopeDetail(grant) + "</td>" +
          '<td><button type="button" class="secondary" data-perm-action="revoke-grant" data-grant="' + esc(grant.grant_id) + '"' + disabled + '>撤销此条</button></td>' +
          "</tr>"
        );
      })
      .join("");
    const toolGroups = new Map();
    for (const grant of grants) {
      const key = text(grant.tool_id) || "(未标注工具)";
      if (!toolGroups.has(key)) toolGroups.set(key, 0);
      toolGroups.set(key, toolGroups.get(key) + 1);
    }
    const perTool = Array.from(toolGroups.entries())
      .map(
        ([toolId, count]) =>
          '<button type="button" class="secondary" data-perm-action="revoke-tool"' +
          (text(toolId) ? ' data-tool="' + esc(toolId) + '"' : "") +
          '"' + disabled + '>撤销「' +
          esc(toolId) +
          "」全部（" +
          count +
          " 条）</button>",
      )
      .join(" ");
    return (
      "<h4>已授予的权限</h4>" +
      '<table class="owo-perm-table"><thead><tr><th>工具 / 状态</th><th>有效期</th><th>到期</th><th>次数</th><th>范围</th><th>撤销</th></tr></thead><tbody>' +
      rows +
      "</tbody></table>" +
      '<div class="inline">' +
      perTool +
      ' <button type="button" class="secondary" data-perm-action="revoke-all"' + disabled + '>撤销当前工作区全部授权</button>' +
      "</div>"
    );
  }

  function scopeDetail(grant) {
    const parts = [];
    if (grant.path_scope) parts.push("路径：" + text(grant.path_scope));
    if (grant.host_scope) parts.push("主机：" + text(grant.host_scope));
    return esc(parts.join(" ｜ ") || "（跟随档位范围）");
  }

  function renderDecisions(snap) {
    const rows = Array.isArray(snap.recentDecisions) ? snap.recentDecisions : [];
    if (!rows.length) return '<h4>最近审批决定</h4><div class="owo-perm-empty" data-perm-empty="decisions">暂无审批历史。</div>';
    const body = rows
      .slice(0, 12)
      .map(
        (row) =>
          "<tr><td>" +
          esc(text(row.ts).slice(11, 19) || text(row.ts) || "—") +
          "</td><td>" +
          esc(text(row.tool) || "—") +
          '</td><td class="' +
          (row.approved ? "" : "owo-perm-bad") +
          '">' +
          (row.approved ? "允许" : "拒绝") +
          "</td><td>" +
          esc(text(row.detail) || "—") +
          "</td></tr>",
      )
      .join("");
    return '<h4>最近审批决定</h4><table class="owo-perm-table"><thead><tr><th>时间</th><th>工具</th><th>决定</th><th>说明</th></tr></thead><tbody>' + body + "</tbody></table>";
  }

  function canMutate(snap) {
    return Boolean(snap && snap.phase === "ready" && snap.readOnly !== true && snap.busy !== true);
  }

  function renderStates(snap) {
    if (snap.phase === "loading") {
      return '<div class="owo-perm-empty" data-perm-phase="loading">正在读取权限概览…（进入本页才请求，不参与首屏链路）</div>';
    }
    if (snap.phase === "error") {
      const info = isPlainObject(snap.lastError) ? snap.lastError : {};
      return (
        '<div class="owo-perm-empty owo-perm-bad" data-perm-phase="error">权限数据读取失败：' +
        esc(text(info.message) || "未知错误") +
        (info.code ? "（稳定错误码：" + esc(info.code) + "）" : "") +
        '</div>'
      );
    }
    if (snap.empty) {
      return '<div class="owo-perm-empty" data-perm-phase="empty">服务端已连接，但当前没有任何权限事实（无待审批、无长期授权、无维度矩阵）。</div>';
    }
    return "";
  }

  function renderErrors(snap) {
    if (snap.phase === "error") return "";
    const errors = Array.isArray(snap.errors) ? snap.errors.filter(Boolean) : [];
    if (!errors.length) return "";
    return '<div class="owo-perm-bad" data-perm-errors="1">' + errors.map((line) => "<div>" + esc(line) + "</div>").join("") + "</div>";
  }

  // ---------- 整页渲染 ----------

  function render(root, snap) {
    if (!root || !isPlainObject(snap)) return;
    const ready = snap.phase === "ready";
    const editable = canMutate(snap);
    const facts = ready
      ? "<h4>权限档位与生效范围</h4>" +
        renderHeader(snap) +
        renderDimensionTable(snap) +
        renderScopes(snap) +
        renderFullAccess(snap) +
        renderPending(snap) +
        renderGrants(snap) +
        renderDecisions(snap)
      : '<div class="owo-perm-empty owo-perm-locked" data-perm-data-unavailable="1">只有成功读取到服务端权限概览后，才会显示权限状态与变更操作。读取失败时不推测当前授权。</div>';
    root.innerHTML =
      styleBlock() +
      '<section data-panel="permissions" class="stack">' +
      '<div class="inline">' +
      '<button type="button" class="primary" data-perm-action="reload"' + (snap.phase === "loading" || snap.busy ? " disabled" : "") + '>刷新权限概览</button>' +
      '<button type="button" class="primary" data-perm-action="submit"' + (editable ? "" : ' disabled title="权限概览成功读取且当前为可写模式后才能保存"') + '>' + (snap.changedFields?.length ? '保存 ' + snap.changedFields.length + ' 项改动' : '保存权限配置') + '</button>' +
      '<span class="sub" data-perm-status="1">' +
      esc(snap.verify || snap.notice ||
        (snap.phase === "loading" ? "正在读取权限概览…" :
          snap.phase === "error" ? "权限概览不可用" :
            snap.updatedAt ? "更新于 " + text(snap.updatedAt).slice(11, 19) : "尚未加载")) +
      "</span>" +
      "</div>" +
      renderStates(snap) +
      renderErrors(snap) +
      facts +
      "</section>";
    bindOnce(root);
  }

  // ---------- 事件委托（一个监听吃全部动作，重绘不需要重新绑定）----------

  function bindOnce(root) {
    if (delegated && rootEl === root) return;
    delegated = true;
    root.addEventListener("click", (event) => onAction(event, root));
    root.addEventListener("change", (event) => onChange(event));
  }

  /**
   * 动作派发：返回 controller 调用的原样结果（Promise 或 undefined）。
   * 委托监听器不 await，因此测试可以直接拿到 handler 的返回值做断言。
   */
  function dispatch(node, root) {
    const action = text(node.getAttribute("data-perm-action"));
    if (!action) return undefined;
    if (typeof node.blur === "function") node.blur();
    switch (action) {
      case "reload":
        return controller.reload();
      case "submit":
        return controller.submitDraft();
      case "request-full-access":
        return controller.requestFullAccess(controller.snapshot().draft);
      case "confirm-full-access":
        // 确认卡已收起时不得凭空提交。
        return controller.snapshot().confirming ? controller.submitDraft() : undefined;
      case "cancel-confirm":
        return controller.cancelConfirm();
      case "close-full-access":
        return controller.closeFullAccess();
      case "approval": {
        const snap = controller.snapshot();
        // 用序号定位：request_id 可能含引号/空格，属性选择器在真实 DOM 与假 DOM shim
        // 里行为不一致（本仓 shim 只支持 #id），所以把索引放在按钮上。
        const index = Number(node.getAttribute("data-pending"));
        const item = Number.isFinite(index) ? find(snap.pending, index) : null;
        const requestId = item ? text(item.request_id) : "";
        return controller.respondApproval(item || { request_id: requestId }, text(node.getAttribute("data-approval")));
      }
      case "revoke-grant":
        return controller.revoke({ grant_id: text(node.getAttribute("data-grant")) });
      case "revoke-tool": {
        const toolId = text(node.getAttribute("data-tool"));
        // 未标注工具的授权不提供"按工具撤销"（防误伤面扩大）。
        return toolId ? controller.revoke({ tool_id: toolId }) : undefined;
      }
      case "revoke-all":
        return controller.revoke({ all: true });
      case "add-scope":
        return addScope(root);
      case "drop-scope": {
        const snap = controller.snapshot();
        const scopes = ((snap.draft || {}).scopes || []).slice();
        const index = Number(node.getAttribute("data-index"));
        if (Number.isFinite(index) && index >= 0 && index < scopes.length) scopes.splice(index, 1);
        return controller.setScopes(scopes);
      }
      default:
        return undefined;
    }
  }

  function onAction(event, root) {
    if (!controller) return undefined;
    const target = event && event.target;
    if (!target || typeof target.closest !== "function") return undefined;
    const node = target.closest("[data-perm-action]");
    if (!node) return undefined;
    return dispatch(node, root);
  }

  function addScope(root) {
    const input = root.querySelector("#permScopeInput");
    if (!input) return;
    const value = text(input.value).trim();
    if (!value) return;
    const snap = controller.snapshot();
    const scopes = ((snap.draft || {}).scopes || []).slice();
    if (!scopes.includes(value)) scopes.push(value);
    controller.setScopes(scopes);
    input.value = "";
  }

  function onChange(event) {
    if (!controller) return;
    const node = event && event.target;
    if (!node || !node.getAttribute) return;
    const name = text(node.getAttribute("name"));
    if (name.indexOf("perm-dimension-") === 0) {
      controller.setDimension(name.slice("perm-dimension-".length), text(node.value));
      return;
    }
    if (name === "perm-duration") {
      controller.chooseDuration(node.value);
    }
  }

  // ---------- 面板契约（mount/dispose，与 panels/*.panel.js 同构）----------

  function mount(root, helpers) {
    rootEl = root;
    H = isPlainObject(helpers) ? helpers : {};
    if (!rootEl) return;
    const domainModule = domain();
    const apiModule = global.OwoPermissionsApi;
    const controllerModule = global.OwoPermissionsController;
    if (!domainModule || !apiModule || !controllerModule) {
      rootEl.innerHTML = '<div class="owo-perm-empty">权限中心模块未完整加载（domain/api/controller 缺一）。</div>';
      return;
    }
    const transport = typeof H.fetchJson === "function" ? H.fetchJson : defaultTransport;
    const api = apiModule.create({ fetchJson: transport });
    controller = controllerModule.create({
      api: api,
      domain: domainModule,
      render: (snap) => render(rootEl, snap),
      notify: typeof H.notify === "function" ? H.notify : undefined,
    });
    delegated = false;
    bindOnce(rootEl);
    // 按需加载：只有挂载到这条路由时才发第一个请求。
    controller.load();
  }

  function defaultTransport(path, options) {
    const client = global.OwoApi;
    if (!client || typeof client.request !== "function") return Promise.reject(new Error("统一 API 客户端未就绪"));
    // 边界说明：本函数只是**注入缺口的兜底**——把请求转交给 core/api-client.js
    // （宿主通过 helpers.fetchJson 注入时根本不会走到这里）。视图自身不构造任何 URL、
    // 不发任何请求：路径与 body 都在 permissions.api.js 里定型。
    return client.request(path, options || {});
  }

  /** 真机验收只读访问器：确认视图自身没有网络出口（§4.8 分层红线的机器可查形态）。 */
  function transportKind() {
    return typeof H.fetchJson === "function" ? "injected" : "unified-client";
  }

  function dispose() {
    if (controller) controller.dispose();
    controller = null;
    rootEl = null;
    delegated = false;
  }

  const panel = {
    id: ID,
    title: "权限中心",
    mount: mount,
    dispose: dispose,
    _test: {
      render: render,
      renderDimensionTable: renderDimensionTable,
      renderPending: renderPending,
      renderGrants: renderGrants,
      renderFullAccess: renderFullAccess,
      renderStates: renderStates,
      onAction: onAction,
      onChange: onChange,
      transportKind: transportKind,
      setController: function (next) {
        controller = next;
      },
      setRoot: function (next) {
        rootEl = next;
        delegated = false;
      },
      getController: function () {
        return controller;
      },
    },
  };

  function registerOwoPermissionsView() {
    global.OwoPanels = global.OwoPanels || {};
    global.OwoPanels.permissions = panel;
    return panel;
  }

  registerOwoPermissionsView();
  global.registerOwoPermissionsView = registerOwoPermissionsView;
  global.OwoPermissionsView = panel;
})(typeof window !== "undefined" ? window : globalThis);

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效。
if (typeof module !== "undefined" && module.exports) {
  module.exports = (typeof window !== "undefined" ? window : globalThis).OwoPermissionsView;
}
