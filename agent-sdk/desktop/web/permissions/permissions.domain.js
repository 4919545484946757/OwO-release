/* 权限中心 · 领域层（§4.5 / §4.8）。
 *
 * 这个文件只回答一个问题：**"服务端给的权限事实，在业务上意味着什么"**——
 * 它把 GET /permissions/overview 的响应整理成前端可用的值对象，并回答
 * "这份配置合法吗 / 要不要二次确认 / 这条撤销该发什么载荷"这类判断。
 *
 * 边界（§4.8）：零 DOM、零 HTTP、零定时器；纯函数 + 常量，输入决定输出。
 * 权威判定矩阵来自服务端 dimensions[]，本层**绝不自行推导生效值**。
 */
(function (global) {
  "use strict";

  // ---- 契约枚举（与后端 spec 五字段逐字对齐，不得增删）----
  const PROFILES = ["read_only", "workspace", "auto_review", "full_access", "custom"];
  const FILESYSTEM_VALUES = ["none", "workspace_read", "workspace_write", "custom"];
  const COMMAND_VALUES = ["deny", "allowlisted", "unrestricted"];
  const NETWORK_VALUES = ["deny", "allowlisted", "unrestricted"];
  const PERSISTENCE_VALUES = ["once", "task", "workspace"];
  const DIMENSION_KEYS = ["filesystem", "command", "network", "persistence"];
  const SPEC_FIELDS = ["filesystem", "command", "network", "persistence", "scopes"];

  // 审批动作（§4.5.2 统一四态）：拒绝 / 仅本次 / 本任务 / 工作区长期。
  const APPROVAL_ACTIONS = [
    { action: "deny", label: "拒绝", allow: false, scope: null },
    { action: "once", label: "仅本次", allow: true, scope: "once" },
    { action: "task", label: "本任务", allow: true, scope: "task" },
    { action: "workspace", label: "工作区长期", allow: true, scope: "workspace" },
  ];

  const PROFILE_LABELS = {
    read_only: "只读",
    workspace: "工作区编辑",
    auto_review: "受控执行",
    full_access: "完全访问",
    custom: "自定义",
  };

  const EFFECTIVE_LABELS = {
    none: "禁止",
    deny: "禁止",
    workspace_read: "工作区内只读",
    workspace_write: "工作区内读写",
    allowlisted: "白名单内允许",
    unrestricted: "不受限",
    custom: "自定义",
    once: "仅本次",
    task: "本任务",
    workspace: "工作区长期",
  };

  // 授权有效期标签：新契约三值 + 旧审批条字面量兼容（同一术语口径，不另起一套）。
  const PERSISTENCE_LABELS = {
    once: "仅本次",
    task: "本任务",
    workspace: "工作区长期",
    session: "此会话",
    one_hour: "一小时内",
    always_readonly: "只读长期",
  };

  const SOURCE_LABELS = { profile: "档位展开", spec: "显式配置", explicit: "显式配置", server: "服务端判定" };

  function isPlainObject(value) {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
  }

  function text(value) {
    return value == null ? "" : String(value);
  }

  /**
   * overview → 结构化配置值对象。`spec: null` 表示从未显式提交过结构化配置，
   * 此时返回 null（调用方必须区分"没有配置"与"配置为空对象"）。
   */
  function specFromOverview(json) {
    const raw = isPlainObject(json) && isPlainObject(json.spec) ? json.spec : null;
    if (!raw) return null;
    const scopes = Array.isArray(raw.scopes) ? raw.scopes.map(text).filter(Boolean) : [];
    return {
      filesystem: text(raw.filesystem),
      command: text(raw.command),
      network: text(raw.network),
      persistence: text(raw.persistence),
      scopes: scopes,
    };
  }

  /** 维度展开表：以服务端 dimensions[] 为准，缺项时按 key 兜底占位（不编造生效值）。 */
  function dimensionsFromOverview(json) {
    const overview = isPlainObject(json) ? json : {};
    const list = Array.isArray(overview.dimensions) ? overview.dimensions : [];
    const byKey = new Map();
    for (const item of list) {
      if (!isPlainObject(item)) continue;
      const key = text(item.key);
      if (!key) continue;
      byKey.set(key, {
        key: key,
        label: text(item.label) || dimensionLabel(key),
        effective: text(item.effective),
        source: text(item.source),
        configurable: item.configurable !== false,
        summary: text(item.summary),
        // 这一维是服务端真的报上来的 —— 与下面的占位行严格区分。
        synthesized: false,
      });
    }
    const out = [];
    for (const key of DIMENSION_KEYS) {
      out.push(
        byKey.get(key) || {
          key: key,
          label: dimensionLabel(key),
          effective: "",
          source: "",
          configurable: false,
          summary: "",
          // 占位行只为让表格骨架稳定（四行不跳版）。它**不是**一条事实：
          // 服务端没说这一维时，前端绝不替它编一个范围出来（§4.5.1 的原始事故），
          // 空态判定也必须把它算作「没有可展示维度」。
          synthesized: true,
        },
      );
    }
    for (const [key, item] of byKey) {
      if (!DIMENSION_KEYS.includes(key)) out.push(item);
    }
    return out;
  }

  function draftFromOverview(json) {
    const spec = specFromOverview(json);
    if (spec) return spec;
    const values = { filesystem: FILESYSTEM_VALUES, command: COMMAND_VALUES, network: NETWORK_VALUES, persistence: PERSISTENCE_VALUES };
    const draft = { scopes: [] };
    const rows = dimensionsFromOverview(json);
    for (const key of DIMENSION_KEYS) {
      const row = rows.find(item => item.key === key && !item.synthesized);
      draft[key] = row && values[key].includes(row.effective) ? row.effective : "";
    }
    return draft;
  }
  function changesFromDraft(draft, baseline) {
    if (!draft || !baseline) return [];
    return SPEC_FIELDS.filter(key => JSON.stringify(draft[key]) !== JSON.stringify(baseline[key]));
  }
  function dimensionLabel(key) {
    switch (text(key)) {
      case "filesystem":
        return "文件系统";
      case "command":
        return "命令执行";
      case "network":
        return "网络访问";
      case "persistence":
        return "授权有效期";
      default:
        return text(key) || "未命名维度";
    }
  }

  /**
   * 校验结构化配置。返回 `[]` 即合法；否则是**中文原因数组**（逐条可展示）。
   * 这里只做表单级约束（枚举、类型、明显不可提交的组合）；
   * "只能收紧不能放宽"由服务端权威判定，前端不复制第二套推导。
   */
  function validateSpec(spec) {
    const errors = [];
    if (!isPlainObject(spec)) {
      errors.push("权限配置为空：至少需要选择四个维度的取值。");
      return errors;
    }
    const checks = [
      ["filesystem", FILESYSTEM_VALUES, "文件系统"],
      ["command", COMMAND_VALUES, "命令执行"],
      ["network", NETWORK_VALUES, "网络访问"],
      ["persistence", PERSISTENCE_VALUES, "授权有效期"],
    ];
    for (const [field, allowed, label] of checks) {
      const value = spec[field];
      if (value == null || value === "") {
        errors.push(label + "：请选择取值（可选：" + allowed.join(" / ") + "）。");
      } else if (!allowed.includes(String(value))) {
        errors.push(label + "：取值「" + text(value) + "」不在允许范围内。");
      }
    }
    if (!Array.isArray(spec.scopes)) {
      errors.push("范围列表必须是列表（工作区相对路径，一行一条）。");
    } else {
      const seen = new Set();
      spec.scopes.forEach((item, index) => {
        const value = text(item).trim();
        if (!value) {
          errors.push("范围列表第 " + (index + 1) + " 条为空，请删除或填写。");
          return;
        }
        if (/^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\") || value.startsWith("/")) {
          errors.push("范围「" + value + "」必须是工作区相对路径，不能是绝对路径或共享根路径。");
        }
        if (/(^|[\\/])\.\.([\\/]|$)/.test(value)) {
          errors.push("范围「" + value + "」包含越出工作区的上级跳转，已拒绝。");
        }
        if (seen.has(value)) errors.push("范围「" + value + "」重复。");
        seen.add(value);
      });
    }
    return errors;
  }

  /** 是否需要"完全访问"双确认：命令或网络任一不受限。 */
  function needsFullAccessConfirm(spec) {
    if (!isPlainObject(spec)) return false;
    return text(spec.command) === "unrestricted" || text(spec.network) === "unrestricted";
  }

  /** 完全访问三要素里的时长选项（秒 + 人类可读单位，禁止手填 RFC3339）。 */
  function durationOptions() {
    return [
      { secs: 600, label: "10 分钟" },
      { secs: 3600, label: "1 小时" },
      { secs: 4 * 3600, label: "4 小时" },
      { secs: 8 * 3600, label: "8 小时" },
    ];
  }

  function defaultDurationSecs() {
    return 3600;
  }

  /** 范围摘要：内部枚举 → 用户术语（未知值原样透出，不假装翻译成功）。 */
  function summarizeScope(scope) {
    const value = text(scope);
    if (!value) return "未设置";
    return EFFECTIVE_LABELS[value] || value;
  }

  /** 授权有效期标签（含旧审批条字面量兼容）。 */
  function persistenceLabel(scope) {
    const value = text(scope);
    if (!value) return "未标注";
    return PERSISTENCE_LABELS[value] || value;
  }

  function profileLabel(profile) {
    const value = text(profile);
    if (!value) return "未知档位";
    return PROFILE_LABELS[value] ? value + "（" + PROFILE_LABELS[value] + "）" : value;
  }

  function sourceLabel(source) {
    const value = text(source);
    if (!value) return "来源未标注";
    return SOURCE_LABELS[value] || value;
  }

  /** 待审批按维度归组：同一次渲染里让"同类请求"聚在一处，未知维度排最后。 */
  function groupByDimension(pending) {
    const list = Array.isArray(pending) ? pending.filter(isPlainObject) : [];
    const groups = new Map();
    for (const item of list) {
      const key = dimensionOfTool(text(item.tool));
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(item);
    }
    const ordered = [];
    for (const key of DIMENSION_KEYS) {
      if (groups.has(key)) ordered.push({ key: key, label: dimensionLabel(key), items: groups.get(key) });
      groups.delete(key);
    }
    for (const [key, items] of groups) ordered.push({ key: key, label: dimensionLabel(key), items: items });
    return ordered;
  }

  /** 工具 → 申请维度（用于分组展示，不参与任何生效判定）。 */
  function dimensionOfTool(tool) {
    const value = text(tool);
    if (/^(shell|run_command|exec|command)/i.test(value)) return "command";
    if (/^(browser|http|fetch|net|web)/i.test(value)) return "network";
    if (/file|path|dir|fs|diff|note/i.test(value)) return "filesystem";
    return "other";
  }

  /** 参数摘要：redacted_args 已是服务端脱敏结果，这里只做长度收敛。 */
  function argsSummary(redacted, limit) {
    const max = Number.isFinite(limit) ? limit : 160;
    if (!isPlainObject(redacted) || !Object.keys(redacted).length) return "（无参数摘要）";
    const parts = [];
    for (const [key, value] of Object.entries(redacted)) {
      const shown = typeof value === "object" && value !== null ? JSON.stringify(value) : text(value);
      parts.push(key + "=" + shown);
    }
    const joined = parts.join("，");
    return joined.length > max ? joined.slice(0, max) + "…" : joined;
  }

  /**
   * 撤销载荷归一（三种粒度，向后兼容）。
   * 只允许单条 / 按工具 / 全部三种形态之一，混传视为非法（防误伤面扩大）。
   */
  function revokePayload(input) {
    const value = isPlainObject(input) ? input : {};
    const grantId = text(value.grant_id).trim();
    const toolId = text(value.tool_id).trim();
    const all = value.all === true;
    const picked = [grantId ? "grant_id" : "", toolId ? "tool_id" : "", all ? "all" : ""].filter(Boolean);
    if (picked.length !== 1) {
      return { ok: false, error: "撤销必须且只能指定一种粒度：单条授权、按工具、或当前工作区全部。", payload: null };
    }
    if (grantId) return { ok: true, payload: { grant_id: grantId }, granularity: "grant" };
    if (toolId) return { ok: true, payload: { tool_id: toolId }, granularity: "tool" };
    return { ok: true, payload: { all: true }, granularity: "all" };
  }

  /** 撤销后复查判据：条目已不在列表 = 已消失（正常终态，不是错误）。 */
  function isGrantGone(grants, selector) {
    const list = Array.isArray(grants) ? grants.filter(isPlainObject) : [];
    const target = isPlainObject(selector) ? selector : {};
    const grantId = text(target.grant_id);
    const toolId = text(target.tool_id);
    if (grantId) return !list.some((grant) => text(grant.grant_id) === grantId);
    if (toolId) return !list.some((grant) => text(grant.tool_id) === toolId);
    return list.length === 0;
  }

  /**
   * 待审批条目"已经没了"的判据：404 / gone / not_found 一类。
   * 这是**正常态**（别的窗口已处理、任务已结束），必须走空态而不是错误态。
   */
  function isPendingGone(errorCode) {
    const value = text(errorCode).toLowerCase();
    if (!value) return false;
    if (/(?:^|\D)404(?:\D|$)/.test(value)) return true;
    return /gone|not_found|notfound|expired|no_longer_pending/.test(value);
  }

  /**
   * 错误归一：后端可能回 `{"error":{code,message}}` 或 `{"error":"字符串"}`，
   * 也可能只有 ApiError 的 status + 原始 body 文本（api-client 的 `"404: {json}"` 形态）。
   * 稳定错误码必须原样可见（§3.4），所以 code 一律透出、绝不翻译。
   */
  function normalizeError(error) {
    if (error == null) return { code: "", message: "未知错误", status: 0 };
    const status = statusOf(error);
    if (typeof error === "string") return fromText(error, status);
    const envelope = isPlainObject(error.error) ? error.error : null;
    if (envelope) {
      return { code: text(envelope.code), message: text(envelope.message) || text(envelope.code) || "未知错误", status: status };
    }
    let body = error.body != null ? error.body : error.responseBody;
    if (typeof body === "string" && body.trim()) {
      try {
        body = JSON.parse(body);
      } catch (_) {
        /* 非 JSON 正文：保留原文当 message */
      }
    }
    if (isPlainObject(body)) {
      const inner = isPlainObject(body.error) ? body.error : null;
      const code = text(inner ? inner.code : body.code);
      const message = inner
        ? text(inner.message) || text(inner.code)
        : body.error && !isPlainObject(body.error)
          ? text(body.error)
          : text(body.message);
      return { code: code, message: message || code || "未知错误", status: status };
    }
    if (typeof error.error === "string") return fromText(error.error, status);
    return fromText(text(error.message), status);
  }

  /** 从 `"404: {"error":{"code":"x"}}"` 这类纯文本里同时挖出状态码与稳定码。 */
  function fromText(raw, status) {
    const value = String(raw == null ? "" : raw);
    let rest = value;
    let found = Number.isFinite(Number(status)) && Number(status) > 0 ? Number(status) : 0;
    const prefixed = /^(\d{3}):\s*([\s\S]*)$/.exec(value);
    if (prefixed) {
      found = Number(prefixed[1]);
      rest = prefixed[2];
    }
    let code = "";
    let message = rest;
    const trimmed = rest.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        const parsed = JSON.parse(trimmed);
        if (isPlainObject(parsed)) {
          const info = normalizeError({ status: found, body: parsed });
          return { code: info.code, message: info.message, status: found };
        }
      } catch (_) {
        /* 不是 JSON：整串当消息 */
      }
    }
    const bare = /(?:^|[\s,"'])(code|error_code)\s*[:=]\s*"?([A-Za-z][A-Za-z0-9_/-]*)"?/.exec(rest);
    if (bare) {
      code = bare[2];
      message = rest.replace(bare[0], "").replace(/^[\s,;]+|[\s,;]+$/g, "") || code;
    }
    if (!message) message = found ? "服务返回 HTTP " + found : "未知错误";
    return { code: code, message: message, status: found };
  }

  /** 从 ApiError 的 `"404: body"` 消息里取状态码（api-client 的固定前缀格式）。 */
  function statusOf(error) {
    if (isPlainObject(error) && Number.isFinite(Number(error.status)) && Number(error.status) > 0) return Number(error.status);
    const match = /^(\d{3}):/.exec(text(isPlainObject(error) ? error.message : error));
    return match ? Number(match[1]) : 0;
  }

  /** 错误是否属于"条目已消失"类（合并 normalizeError + status 两条线索）。 */
  function isErrorGone(error) {
    const info = normalizeError(error);
    if (isPendingGone(info.code)) return true;
    return statusOf(error) === 404;
  }

  /** 概览完整性判定：HTTP 200 不等于有数据（profile/dimensions 缺失就是坏响应）。 */
  function overviewIsUsable(json) {
    if (!isPlainObject(json)) return false;
    // 服务端明确回报失败（{"ok":false,"error":…}）不是"空数据"，必须走错误态。
    if (json.ok === false) return false;
    if (!text(json.profile)) return false;
    // dimensions 是权威判定矩阵；服务端可能合法地返回**空数组**（例如尚未展开任何
    // 维度的旧配置），那是"没有可展示维度"而不是坏响应——缺字段才判不可用。
    if (!Array.isArray(json.dimensions)) return false;
    return true;
  }

  function listField(json, key) {
    return isPlainObject(json) && Array.isArray(json[key]) ? json[key].filter(isPlainObject) : [];
  }

  global.OwoPermissionsDomain = {
    PROFILES: PROFILES,
    FILESYSTEM_VALUES: FILESYSTEM_VALUES,
    COMMAND_VALUES: COMMAND_VALUES,
    NETWORK_VALUES: NETWORK_VALUES,
    PERSISTENCE_VALUES: PERSISTENCE_VALUES,
    DIMENSION_KEYS: DIMENSION_KEYS,
    SPEC_FIELDS: SPEC_FIELDS,
    APPROVAL_ACTIONS: APPROVAL_ACTIONS,
    PROFILE_LABELS: PROFILE_LABELS,
    PERSISTENCE_LABELS: PERSISTENCE_LABELS,
    specFromOverview: specFromOverview,
    draftFromOverview: draftFromOverview,
    changesFromDraft: changesFromDraft,
    dimensionsFromOverview: dimensionsFromOverview,
    dimensionLabel: dimensionLabel,
    dimensionOfTool: dimensionOfTool,
    validateSpec: validateSpec,
    needsFullAccessConfirm: needsFullAccessConfirm,
    durationOptions: durationOptions,
    defaultDurationSecs: defaultDurationSecs,
    summarizeScope: summarizeScope,
    persistenceLabel: persistenceLabel,
    profileLabel: profileLabel,
    sourceLabel: sourceLabel,
    groupByDimension: groupByDimension,
    argsSummary: argsSummary,
    revokePayload: revokePayload,
    isGrantGone: isGrantGone,
    isPendingGone: isPendingGone,
    isErrorGone: isErrorGone,
    normalizeError: normalizeError,
    statusOf: statusOf,
    overviewIsUsable: overviewIsUsable,
    listField: listField,
    isPlainObject: isPlainObject,
  };
})(typeof window !== "undefined" ? window : globalThis);

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效。
if (typeof module !== "undefined" && module.exports) {
  module.exports = (typeof window !== "undefined" ? window : globalThis).OwoPermissionsDomain;
}
