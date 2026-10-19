/* 用户自定义工作流的纯逻辑（桌面 Web）：解析/编辑/请求体/预览/发现文案。
 *
 * 无 DOM、无网络；面板与入口函数共用，node 测试直接覆盖。
 * 服务端是唯一权威（校验/评审/落盘都在服务端），这里只做"提交前的即时提示"与请求组装。
 */
(function (global) {
  "use strict";

  var REQUIRED_FIELDS = ["template_id", "name", "roles"];

  /** 解析编辑器里的 JSON；返回 {ok, template|error}。 */
  function parseTemplate(text) {
    var raw = String(text == null ? "" : text).trim();
    if (!raw) return { ok: false, error: "配置为空" };
    var value;
    try {
      value = JSON.parse(raw);
    } catch (error) {
      return { ok: false, error: "JSON 解析失败：" + (error && error.message ? error.message : error) };
    }
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return { ok: false, error: "配置必须是一个 JSON 对象（TeamTemplate）" };
    }
    return { ok: true, template: value };
  }

  /** 本地轻量检查（即时提示用；完整校验以服务端 /workflows/user/validate 为准）。 */
  function quickCheck(template) {
    var problems = [];
    if (!template || typeof template !== "object") return ["配置必须是对象"];
    REQUIRED_FIELDS.forEach(function (field) {
      var value = template[field];
      if (field === "roles") {
        if (!Array.isArray(value) || !value.length) problems.push("roles 至少要有 1 个角色");
      } else if (!value || !String(value).trim()) {
        problems.push(field + " 不能为空");
      }
    });
    var roles = Array.isArray(template.roles) ? template.roles : [];
    var seen = {};
    roles.forEach(function (role) {
      var name = role && role.role ? String(role.role).trim() : "";
      if (!name) {
        problems.push("存在未命名的角色");
        return;
      }
      if (seen[name]) problems.push("角色名重复：" + name);
      seen[name] = true;
      var assignee = role.assignee ? String(role.assignee) : "agent";
      if (["agent", "human", "worker"].indexOf(assignee) < 0) {
        problems.push("角色 " + name + " 的 assignee 非法：" + assignee);
      }
      (Array.isArray(role.depends_on) ? role.depends_on : []).forEach(function (dependency) {
        if (!seen[String(dependency).trim()] && !roles.some(function (other) {
          return other && String(other.role || "").trim() === String(dependency).trim();
        })) {
          problems.push("角色 " + name + " 依赖不存在的角色：" + dependency);
        }
      });
    });
    return problems;
  }

  /**
   * DAG 分层布局（画图用）：最长路径分层——无依赖层 0，其余 = 上游最大层 + 1。
   *
   * 返回 {ok, layers, nodes:[{id,assignee,layer,row}], edges}；有环返回
   * {ok:false, error}（不给错误图形；校验器也会报环）。未知依赖按"无上游"处理。
   */
  function layoutGraph(template) {
    var roles = template && Array.isArray(template.roles) ? template.roles : [];
    var byId = {};
    roles.forEach(function (role) {
      var id = String((role && role.role) || "").trim();
      if (id) byId[id] = role;
    });
    var layerOf = {};
    var visiting = {};
    function resolve(id) {
      if (Object.prototype.hasOwnProperty.call(layerOf, id)) return layerOf[id];
      if (visiting[id]) throw new Error("sync_cycle");
      visiting[id] = true;
      var role = byId[id];
      var depends = role && Array.isArray(role.depends_on) ? role.depends_on : [];
      var layer = 0;
      depends.forEach(function (dependency) {
        var upstream = String(dependency == null ? "" : dependency).trim();
        if (!byId[upstream]) return;
        layer = Math.max(layer, resolve(upstream) + 1);
      });
      delete visiting[id];
      layerOf[id] = layer;
      return layer;
    }
    try {
      Object.keys(byId).forEach(resolve);
    } catch (error) {
      return { ok: false, error: "角色依赖存在环，无法布局" };
    }
    var nodes = [];
    var edges = [];
    var rows = {};
    var depth = 0;
    Object.keys(byId)
      .sort(function (left, right) {
        return layerOf[left] - layerOf[right] || (left < right ? -1 : left > right ? 1 : 0);
      })
      .forEach(function (id) {
        var layer = layerOf[id];
        var row = rows[layer] || 0;
        rows[layer] = row + 1;
        depth = Math.max(depth, layer + 1);
        var role = byId[id];
        nodes.push({
          id: id,
          assignee: String((role && role.assignee) || "agent"),
          layer: layer,
          row: row,
        });
        var depends = role && Array.isArray(role.depends_on) ? role.depends_on : [];
        depends.forEach(function (dependency) {
          var upstream = String(dependency == null ? "" : dependency).trim();
          if (byId[upstream]) edges.push({ from: upstream, to: id });
        });
      });
    return { ok: true, layers: depth, nodes: nodes, edges: edges };
  }

  /** 依赖边（画图/预览用）：[{from, to}]。 */
  function buildEdges(template) {
    var edges = [];
    var roles = template && Array.isArray(template.roles) ? template.roles : [];
    roles.forEach(function (role) {
      (Array.isArray(role.depends_on) ? role.depends_on : []).forEach(function (dependency) {
        edges.push({ from: String(dependency), to: String(role.role || "") });
      });
    });
    return edges;
  }

  /** 角色/依赖/工具/写范围的纯文本预览行（无 DOM，便于测试与日志）。 */
  function previewLines(template) {
    var roles = template && Array.isArray(template.roles) ? template.roles : [];
    return roles.map(function (role) {
      var parts = ["- " + String(role.role || "?") + "（" + String(role.assignee || "agent") + "）"];
      var depends = Array.isArray(role.depends_on) ? role.depends_on : [];
      if (depends.length) parts.push("← " + depends.join(", "));
      var tools = Array.isArray(role.tools) ? role.tools : [];
      if (tools.length) parts.push("工具：" + tools.join(", "));
      var writes = Array.isArray(role.write_paths) ? role.write_paths : [];
      if (writes.length) parts.push("写范围：" + writes.join(", "));
      if (role.verify) parts.push("校验：" + role.verify);
      return parts.join(" ");
    });
  }

  /** 用这条工作流创建团队（POST /teams）：objective 必填，mode 缺省 team。 */
  function buildTeamCreateRequest(template, objective, acknowledgeRisk) {
    var text = String(objective == null ? "" : objective).trim();
    if (!text) return { ok: false, error: "请先填写这次要让团队达成的目标" };
    var id = template && template.template_id ? String(template.template_id).trim() : "";
    if (!id) return { ok: false, error: "工作流缺少 template_id" };
    var mode = template && template.mode ? String(template.mode) : "team";
    return {
      ok: true,
      path: "/teams",
      body: {
        objective: text,
        mode: mode,
        template_id: id,
        // 社区来源 + 有风险面时服务端要求显式确认；用户点过确认才为 true。
        acknowledge_risk: acknowledgeRisk === true,
      },
    };
  }

  /** 服务端校验请求体（POST /workflows/user/validate）。 */
  function buildValidateRequest(template) {
    return { path: "/workflows/user/validate", body: template };
  }

  /** 保存请求体（POST /workflows/user）。 */
  function buildSaveRequest(template) {
    return { path: "/workflows/user", body: template };
  }

  /** AI 生成草稿请求体（POST /workflows/draft）。 */
  function buildDraftRequest(description, constraints, refineOf, rounds, basedOn) {
    var text = String(description == null ? "" : description).trim();
    if (!text) return { ok: false, error: "请先描述这条工作流要完成什么" };
    var body = { description: text };
    var extra = String(constraints == null ? "" : constraints).trim();
    if (extra) body.constraints = extra;
    var base = String(basedOn == null ? "" : basedOn).trim();
    var refine = String(refineOf == null ? "" : refineOf).trim();
    if (base && refine) {
      // 服务端同样拒绝：一个基于已保存工作流，一个基于草稿，语义冲突。
      return { ok: false, error: "based_on（按运行证据改进）与 refine_of（基于草稿）不能同时使用" };
    }
    if (base) body.based_on = base;
    var requested = Number(rounds);
    if (Number.isFinite(requested) && requested > 1) {
      // 多轮收敛：走 converge（与 refine_of 互斥，服务端硬顶 5 轮）。
      body.max_rounds = Math.min(Math.floor(requested), 5);
      return { ok: true, path: "/workflows/draft/converge", body: body };
    }
    if (refine) body.refine_of = refine;
    return { ok: true, path: "/workflows/draft", body: body };
  }

  var ELEVATED_TOOLS = ["run_command", "web_fetch", "web_search", "browser_click", "browser_type", "computer_use"];
  var WRITE_TOOLS = ["write_file", "edit_file", "multi_edit", "apply_patch", "run_command", "kill_shell"];

  /**
   * 逐角色健康行（与宿主 `workflow_health` 的口径一致）：谁、能干什么、有什么风险标记。
   * 纯前端计算，供编辑器即时展示（服务端仍会给同一套 health）。
   */
  function roleHealthRows(template) {
    var roles = template && Array.isArray(template.roles) ? template.roles : [];
    return roles.map(function (role) {
      var tools = (Array.isArray(role.tools) ? role.tools : []).map(function (tool) {
        return String(tool).trim();
      }).filter(Boolean);
      var writePaths = (Array.isArray(role.write_paths) ? role.write_paths : []).map(function (path) {
        return String(path).trim();
      }).filter(Boolean);
      var assignee = String(role.assignee || "agent");
      var risks = [];
      tools.forEach(function (tool) {
        if (ELEVATED_TOOLS.indexOf(tool) >= 0) {
          risks.push("执行/联网工具:" + tool);
        } else if (tool.indexOf("mcp__") === 0) {
          risks.push("MCP 依赖:" + tool);
        }
      });
      var hasWrite = tools.some(function (tool) {
        return WRITE_TOOLS.indexOf(tool) >= 0;
      });
      if (assignee === "agent" && writePaths.length === 0 && hasWrite) {
        risks.push("未声明写范围（工作区任意写）");
      }
      if (assignee === "human") risks.push("需要人工确认");
      if (assignee === "agent" && tools.length === 0) risks.push("无工具（只产出文本）");
      return {
        role: String(role.role || "?"),
        assignee: assignee,
        tools: tools,
        writePaths: writePaths,
        risks: risks,
      };
    });
  }

  /**
   * 生成结果的健康提示行：`风险：高 · 执行/联网工具：run_command`（无 health 返回空数组）。
   * 用于"校验通过但风险不低"的显式提醒——通过 ≠ 安全。
   */
  function describeDraftHealth(draft) {
    var health = draft && draft.health;
    if (!health || typeof health !== "object") return [];
    var lines = ["风险：" + (riskLabel(health.risk_level) || "未知")];
    var elevated = Array.isArray(health.elevated_tools) ? health.elevated_tools : [];
    if (elevated.length) lines.push("执行/联网工具：" + elevated.join(", "));
    var mcp = Array.isArray(health.mcp_tools) ? health.mcp_tools : [];
    if (mcp.length) lines.push("MCP 依赖：" + mcp.join(", "));
    var wide = Array.isArray(health.whole_workspace_writers) ? health.whole_workspace_writers : [];
    if (wide.length) lines.push("工作区任意写：" + wide.join(", "));
    return lines;
  }

  /** 收敛轨迹 → 可读行（每轮角色数/错误数/是否被接受）。 */
  function describeTrajectory(trajectory) {
    var steps = Array.isArray(trajectory) ? trajectory : [];
    if (!steps.length) return [];
    return steps.map(function (step) {
      if (step && step.error) {
        return "  #" + String(step.round || "?") + " 失败：" + String(step.detail || step.error);
      }
      var risk = step && step.risk_level ? " / 风险 " + riskLabel(step.risk_level) : "";
      return (
        "  #" +
        String((step && step.round) || "?") +
        " 角色 " +
        String((step && step.roles) || 0) +
        " / 错误 " +
        String((step && step.errors) || 0) +
        risk +
        (step && step.valid ? "（通过）" : "") +
        (step && step.accepted === false ? "（未改善，停止）" : "")
      );
    });
  }

  /** 社区包相关请求体。 */
  function buildExportRequest(id, body) {
    var trimmed = String(id == null ? "" : id).trim();
    if (!trimmed) return { ok: false, error: "请选择要导出的工作流" };
    return { ok: true, path: "/workflows/user/" + encodeURIComponent(trimmed) + "/export", body: body || {} };
  }

  function buildImportRequest(packageB64, reviewOnly) {
    var text = String(packageB64 == null ? "" : packageB64).trim();
    if (!text) return { ok: false, error: "请粘贴社区包内容（base64）" };
    return {
      ok: true,
      path: reviewOnly ? "/workflows/import/review" : "/workflows/import",
      body: { package_b64: text },
    };
  }

  /** 校验/评审结果 → 可读行（含 level 前缀）。 */
  function describeReview(result) {
    var lines = [];
    if (!result || typeof result !== "object") return ["（没有结果）"];
    var errors = Array.isArray(result.errors) ? result.errors : [];
    errors.forEach(function (error) {
      lines.push("[错误] " + String(error));
    });
    var review = result.review && typeof result.review === "object" ? result.review : result;
    var findings = Array.isArray(review.findings) ? review.findings : [];
    findings.forEach(function (finding) {
      lines.push(
        "[" + String(finding.level || "info") + "] " + String(finding.category || "") + "：" + String(finding.detail || "")
      );
    });
    if (review.blocked === true) lines.push("[门禁] 评审未通过：存在阻止项，导入会被拒绝");
    if (!lines.length) lines.push("校验通过，没有发现需要关注的问题");
    return lines;
  }

  /**
   * 导入评审时与本地同名工作流的差异 → 可读行。
   * 输入是 /workflows/import(/review) 响应里的 diff_vs_local（没有本地同名时为 null）。
   */
  function describeLocalDiff(result) {
    var local = result && result.diff_vs_local;
    if (!local) return [];
    var lines = ["与本地同名工作流（" + String(local.local_origin || "?") + "，版本 " + String(local.local_version || "?") + " → " + String(local.incoming_version || "?") + "）的差异："];
    var summary = Array.isArray(local.summary) ? local.summary : [];
    summary.forEach(function (line) {
      lines.push("  " + String(line));
    });
    if (!summary.length) lines.push("  （没有差异）");
    return lines;
  }

  /** 「我的工作流」列表项 → 展示行。 */
  /**
   * 导入溯源 → 一行标签（`导入@MM-DD HH:MM · sha256 前 8 位`）；非社区导入返回空串。
   * 时间按 RFC3339 字符串截取展示（不臆造本地时区）。
   */
  function provenanceLabel(provenance) {
    if (!provenance || typeof provenance !== "object") return "";
    var importedAt = String(provenance.imported_at || "");
    var stamp = importedAt.length >= 16 ? importedAt.slice(5, 16).replace("T", " ") : importedAt;
    var sha = String(provenance.sha256 || "").slice(0, 8);
    var parts = [];
    if (stamp) parts.push("导入@" + stamp);
    if (sha) parts.push("sha256 " + sha);
    return parts.join(" · ");
  }

  /** RFC3339 → `MM-DD HH:MM`（不足长度原样返回；不臆造本地时区）。 */
  function compactStamp(value) {
    var text = String(value == null ? "" : value);
    return text.length >= 16 ? text.slice(5, 16).replace("T", " ") : text;
  }

  /**
   * 构造统计 URI（排序/条数；非法值忽略，limit 本地限幅 1..200）。
   */
  function buildStatsUri(sort, limit) {
    var query = [];
    if (["runs", "failed", "success_rate", "last"].indexOf(String(sort || "")) >= 0) {
      query.push("sort=" + sort);
    }
    var parsed = Number(limit);
    if (Number.isFinite(parsed) && parsed > 0) {
      query.push("limit=" + Math.min(Math.floor(parsed), 200));
    }
    return "/workflows/stats" + (query.length ? "?" + query.join("&") : "");
  }

  /**
   * `/workflows/stats` 的单行 → 展示结构（纯函数，面板与测试共用）。
   * 成功率无终态时为 null → 显示 "—"（不编造 0%）。
   */
  function describeStatsRow(item) {
    var rate = item && typeof item.success_rate === "number" ? Math.round(item.success_rate * 100) + "%" : "—";
    var failure = item && item.last_failure && typeof item.last_failure === "object" ? item.last_failure : null;
    return {
      id: String((item && item.template_id) || "?"),
      name: String((item && item.name) || ""),
      origin: String((item && item.origin) || "?"),
      riskLabel: riskLabel(item && item.risk_level),
      runs: Number((item && item.runs) || 0),
      succeeded: Number((item && item.succeeded) || 0),
      failed: Number((item && item.failed) || 0),
      cancelled: Number((item && item.cancelled) || 0),
      running: Number((item && item.running) || 0),
      awaitingHuman: Number((item && item.awaiting_human) || 0),
      successRate: rate,
      attention: Array.isArray(item && item.attention) ? item.attention.map(String) : [],
      lastRunFailed: !!(item && item.last_run_failed === true),
      lastRunAt: compactStamp(item && item.last_run_at),
      lastFailure: failure
        ? {
            teamId: String(failure.team_id || ""),
            at: compactStamp(failure.at),
            event: String(failure.event || ""),
            detail: String(failure.detail || "").slice(0, 120),
          }
        : null,
    };
  }

  /** 风险等级 → 中文标签（未知返回空串，不臆造）。 */
  function riskLabel(level) {
    if (level === "low") return "低";
    if (level === "medium") return "中";
    if (level === "high") return "高";
    return "";
  }

  var LAST_RUN_LABELS = {
    created: "已创建",
    running: "进行中",
    awaiting_human: "等待人工",
    succeeded: "成功",
    failed: "失败",
    cancelled: "已取消",
  };

  /// 草稿的上一次验证结果（采纳后沿用同一目标再跑的结论）→ 一行文案。
  function describeVerifyOutcome(draft) {
    var outcome = draft && (draft.verify_outcome || (draft.draft && draft.draft.verify_outcome));
    if (!outcome || typeof outcome !== "object") return "";
    var status = String(outcome.status || "?");
    var baseline = outcome.compared_status ? String(outcome.compared_status) : "";
    var verdict =
      baseline && baseline === status
        ? "未康复（与基线相同）"
        : baseline
          ? "状态有变化"
          : "无基线对比";
    return "上次验证：" + status + (baseline ? "（基线 " + baseline + "）" : "") + " · " + verdict;
  }

  /// 最近一次运行（GET /workflows/user/<id>/last-run）：给出可复用目标与状态标签。
  function describeLastRun(payload) {
    var run = payload && payload.last_run;
    if (!run) {
      return { hasRun: false, teamId: "", objective: "", status: "", statusLabel: "", hint: "从未运行" };
    }
    var status = String(run.status || "");
    return {
      hasRun: true,
      teamId: String(run.team_id || ""),
      objective: String(run.objective || ""),
      status: status,
      statusLabel: LAST_RUN_LABELS[status] || status || "?",
      createdAt: String(run.created_at || ""),
      // 复用按钮的提示文案：目标来自运行时快照，不是猜的。
      hint: "沿用上次目标再跑（team " + String(run.team_id || "?") + "）",
    };
  }

  function describeWorkflow(item) {
    var template = (item && item.template) || {};
    var roles = Array.isArray(template.roles) ? template.roles.length : 0;
    var findings = item && Array.isArray(item.findings) ? item.findings : [];
    return {
      id: String(template.template_id || "?"),
      name: String(template.name || ""),
      origin: String((item && item.origin) || "?"),
      roles: roles,
      valid: !item || item.valid !== false,
      warnings: Number((item && item.warnings) || 0),
      blocked: !!(item && item.blocked === true),
      findings: findings,
      risk: String((item && item.health && item.health.risk_level) || ""),
      riskLabel: riskLabel(item && item.health ? item.health.risk_level : ""),
      provenance: item && item.provenance ? item.provenance : null,
      provenanceLabel: provenanceLabel(item && item.provenance ? item.provenance : null),
      // 运行期告警（列表接口带出）：从未成功/高失败率/长期低成功率/最近失败。
      attention: item && Array.isArray(item.attention) ? item.attention.map(String) : [],
      runs: Number((item && item.runs) || 0),
      tag: item && item.valid === false ? "校验未通过" : "可创建团队",
    };
  }

  /**
   * 建队前的一次性知情确认文案：有 warn 级发现时返回多行文本，否则 null。
   * blocked（schema 错误等）返回拒绝文案——那种情况不应建队。
   */
  function buildTeamCreateConfirm(item) {
    var row = describeWorkflow(item);
    if (row.blocked) {
      var blocking = row.findings
        .filter(function (finding) {
          return finding && finding.level === "block";
        })
        .map(function (finding) {
          return String(finding.detail || "");
        })
        .join("；");
      return "该工作流存在阻止级问题，不能建队：" + (blocking || "请先修正校验问题");
    }
    var warnings = row.findings.filter(function (finding) {
      return finding && finding.level === "warn";
    });
    if (!warnings.length) return null;
    return (
      "该工作流会用到以下能力面，确认后才会创建团队：\n" +
      warnings
        .map(function (finding) {
          return "- " + String(finding.category || "") + "：" + String(finding.detail || "");
        })
        .join("\n")
    );
  }

  global.OwoWorkflowEditor = {
    parseTemplate: parseTemplate,
    quickCheck: quickCheck,
    buildEdges: buildEdges,
    layoutGraph: layoutGraph,
    previewLines: previewLines,
    buildValidateRequest: buildValidateRequest,
    buildTeamCreateRequest: buildTeamCreateRequest,
    buildSaveRequest: buildSaveRequest,
    buildDraftRequest: buildDraftRequest,
    buildExportRequest: buildExportRequest,
    buildImportRequest: buildImportRequest,
    describeReview: describeReview,
    describeTrajectory: describeTrajectory,
    describeDraftHealth: describeDraftHealth,
    roleHealthRows: roleHealthRows,
    describeLocalDiff: describeLocalDiff,
    describeWorkflow: describeWorkflow,
    describeLastRun: describeLastRun,
    describeVerifyOutcome: describeVerifyOutcome,
    riskLabel: riskLabel,
    describeStatsRow: describeStatsRow,
    buildStatsUri: buildStatsUri,
    provenanceLabel: provenanceLabel,
    buildTeamCreateConfirm: buildTeamCreateConfirm,
    _test: {
      parseTemplate: parseTemplate,
      quickCheck: quickCheck,
      buildEdges: buildEdges,
      layoutGraph: layoutGraph,
      previewLines: previewLines,
      buildDraftRequest: buildDraftRequest,
      buildTeamCreateRequest: buildTeamCreateRequest,
      buildExportRequest: buildExportRequest,
      buildImportRequest: buildImportRequest,
      describeReview: describeReview,
      describeTrajectory: describeTrajectory,
      describeDraftHealth: describeDraftHealth,
      roleHealthRows: roleHealthRows,
      describeLocalDiff: describeLocalDiff,
      describeWorkflow: describeWorkflow,
      describeLastRun: describeLastRun,
      describeVerifyOutcome: describeVerifyOutcome,
      riskLabel: riskLabel,
      describeStatsRow: describeStatsRow,
      buildStatsUri: buildStatsUri,
      provenanceLabel: provenanceLabel,
      buildTeamCreateConfirm: buildTeamCreateConfirm,
    },
  };
})(typeof window !== "undefined" ? window : globalThis);
