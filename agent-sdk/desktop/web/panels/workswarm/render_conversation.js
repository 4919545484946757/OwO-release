// ============================================================================
// WorkSwarm 交接对话 / 引用选择器渲染（HTML 视图构建）
//   desktop/web/panels/workswarm/render_conversation.js
//
// 从 render.js 拆出的两个末端纯 HTML 构建器：
//   - teamConversationHtml：按发言人/时间顺序渲染已持久化的团队交接消息；
//   - refPickerHtml：结构化引用选择器（已选行 + 搜索候选 + allowFree 输入）。
//
// esc/short 由调用方经 ctx 注入（render.js 的 bindEsc/bindShort 可变绑定），
// 保持浏览器（H.esc DOM 实现）与 Node 单测（defaultEsc 正则实现）行为一致。
//
// 加载约定与 domain/format/render 相同：
//   - 浏览器由 index.html 预加载，挂 win.OwoWorkswarmRenderConversation；
//   - Node 单测 require 同目录模块（module.exports 导出）。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;

  /// §8.1 引用选择器（artifact/evidence/member 引用）：已选结构化行（可移除）
  /// + 搜索过滤后的候选按钮（可点击添加）。allowFree 时支持输入任意引用
  /// （如证据链接/描述）后点「添加」成为一行。
  /// opts = {kind, rows:[string], options:[{value,label}], query, allowFree}
  function refPickerHtml(ctx, opts) {
    var esc = ctx.esc;
    var short = ctx.short;
    var kind = String((opts && opts.kind) || "");
    var rows = (opts && opts.rows) || [];
    var options = (opts && opts.options) || [];
    var query = (opts && opts.query) == null ? "" : String(opts.query);
    var allowFree = !!(opts && opts.allowFree);
    var rowsHtml = rows.length
      ? rows.map(function (r) {
          var v = String(r);
          return (
            '<span class="owo-ws-refrow"><span class="owo-ws-mono" title="' + esc(v) + '">' +
            esc(short(v)) + "</span>" +
            '<button type="button" class="owo-ws-mini" data-core-action data-ws-ref-del="' + esc(kind + ":" + v) +
            '" title="移除该引用">×</button></span>'
          );
        }).join(" ")
      : '<span class="hint">暂无引用行</span>';
    var sug = options
      .map(function (o) {
        return (
          '<button type="button" class="owo-ws-mini" data-core-action data-ws-ref-add="' + esc(kind + ":" + (o && o.value)) +
          '" title="' + esc((o && o.label) || (o && o.value) || "") + '">' +
          esc(short((o && o.label) || (o && o.value) || "")) + "</button>"
        );
      })
      .join(" ");
    return (
      '<div class="owo-ws-refpick" data-ws-refpick="' + esc(kind) + '">' +
      '<div class="owo-ws-refrows">' + rowsHtml + "</div>" +
      '<div class="owo-ws-inline">' +
      '<input class="owo-ws-refq" data-ws-ref-q="' + esc(kind) + '" size="34" placeholder="' +
      (allowFree ? "搜索候选或输入引用后点添加" : "搜索候选引用") + '" value="' + esc(query) + '">' +
      (allowFree ? '<button type="button" class="owo-ws-mini" data-ws-ref-free="' + esc(kind) + '">添加</button>' : "") +
      "</div>" +
      (sug ? '<div class="owo-ws-refsug">' + sug + "</div>" : '<div class="hint">无匹配候选</div>') +
      "</div>"
    );
  }

  // Compact local display; preserve the original timestamp for exact provenance.
  function timeHtml(esc, value) {
    var raw = String(value || ""), date = new Date(raw);
    if (!raw) return "";
    if (!isFinite(date.getTime())) return "<time>" + esc(raw) + "</time>";
    var text = date.toLocaleString("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
    return '<time datetime="' + esc(raw) + '" title="' + esc(raw) + '">' + esc(text) + "</time>";
  }

  function teamConversationHtml(ctx, handoffs, members, markdown, errorText, onlyRole) {
    var esc = ctx.esc;
    if (errorText) {
      return '<div class="owo-ws-chat-state bad" role="alert">' + esc(errorText) + "</div>";
    }
    if (handoffs === null) {
      return '<div class="owo-ws-chat-state hint" aria-live="polite">正在读取团队交接记录…</div>';
    }
    if (!Array.isArray(handoffs) || !handoffs.length) {
      return '<div class="owo-ws-chat-state hint">暂无已保存的交接消息。逐轮内部对话只有在服务端持久化后才能展示。</div>';
    }
    var memberMap = {};
    (Array.isArray(members) ? members : []).forEach(function (m) {
      if (m && m.member_id) memberMap[String(m.member_id)] = m;
    });
    var filterRole = String(onlyRole || "").trim();
    var rows = handoffs.filter(function (handoff) {
      if (!filterRole || filterRole === "all") return true;
      var id = String((handoff && handoff.from_member) || "");
      return String((memberMap[id] && memberMap[id].role) || id || "团队成员") === filterRole;
    }).sort(function (a, b) {
      return String((a && a.created_at) || "").localeCompare(String((b && b.created_at) || ""));
    });
    return '<div class="owo-ws-chat-log" role="log" aria-label="团队交接对话">' +
      rows.map(function (h, index) {
        h = h || {};
        var senderId = String(h.from_member || "");
        var receiverId = String(h.to_member || "");
        var member = memberMap[senderId] || {};
        var role = String(member.role || senderId || "团队成员");
        var initial = role.trim().slice(0, 1) || "团";
        var slug = role.toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "member";
        var summary = String(h.completed_summary || "");
        var body = typeof markdown === "function"
          ? markdown(summary)
          : '<div class="md-p">' + esc(summary).replace(/\n/g, "<br>") + "</div>";
        var receiver = receiverId
          ? '<span class="owo-ws-chat-to">发给 ' + esc((memberMap[receiverId] && memberMap[receiverId].role) || receiverId) + "</span>"
          : "";
        var extras = "";
        [
          ["遗留问题", h.open_issues],
          ["下一步", h.suggested_next_actions],
          ["已知风险", h.known_risks],
        ].forEach(function (pair) {
          var values = Array.isArray(pair[1]) ? pair[1].filter(Boolean) : [];
          if (!values.length) return;
          extras += '<div class="owo-ws-chat-extra"><b>' + pair[0] + "</b><ul>" +
            values.map(function (value) { return "<li>" + esc(value) + "</li>"; }).join("") +
            "</ul></div>";
        });
        var refs = Array.isArray(h.output_artifact_refs) ? h.output_artifact_refs.filter(Boolean) : [];
        return '<article class="owo-ws-chat-row speaker-' + esc(slug) + '" data-handoff-index="' + index + '">' +
          '<div class="owo-ws-chat-avatar" aria-hidden="true">' + esc(initial) + "</div>" +
          '<div class="owo-ws-chat-card"><header class="owo-ws-chat-head"><b>' + esc(role) + "</b>" +
          '<span class="owo-ws-chat-id">' + esc(senderId) + "</span>" + receiver +
          timeHtml(esc, h.created_at) + "</header>" +
          '<div class="owo-ws-chat-body">' + body + "</div>" + extras +
          (refs.length ? '<div class="owo-ws-chat-refs">关联产物：' + refs.map(esc).join(" · ") + "</div>" : "") +
          "</div></article>";
      }).join("") + "</div>";
  }

  // Keep only a recent rendering window; older public observations remain in the
  // daemon's cursor-paged run history. This is not another transcript database.
  var REPLAY_EVENTS = 1024, REPLAY_BYTES = 2 * 1024 * 1024;
  function boundReplay(replay, priorTruncated) {
    var rows = [];
    Object.keys(replay).forEach(function(sessionId) {
      var session = replay[sessionId];
      (session.events || []).forEach(function(event) { rows.push({ sessionId: sessionId, role: session.role, event: event }); });
    });
    rows.sort(function(a,b) {
      return String(a.event.created_at || "").localeCompare(String(b.event.created_at || "")) ||
        Number(a.event.seq)-Number(b.event.seq) || a.sessionId.localeCompare(b.sessionId);
    });
    var kept = [], bytes = 0, truncated = !!priorTruncated, encoder = new TextEncoder();
    for (var i=rows.length-1;i>=0;i--) {
      var size = encoder.encode(JSON.stringify(rows[i].event)).byteLength;
      if (kept.length >= REPLAY_EVENTS || size > REPLAY_BYTES-bytes) { truncated=true; continue; }
      bytes += size; kept.push(rows[i]);
    }
    var result = {};
    kept.reverse().forEach(function(row) {
      if (!result[row.sessionId]) result[row.sessionId] = { role: row.role, events: [] };
      result[row.sessionId].events.push(row.event);
    });
    Object.defineProperty(result,"windowInfo",{enumerable:false,value:{truncated:truncated,events:kept.length,bytes:bytes}});
    return result;
  }

  function mergeReplay(previous, sessions) {
    var map = Object.assign({}, previous || {});
    (sessions || []).forEach(function (session) {
      var old = map[session.session_id] || { role: session.role || "成员", events: [] };
      var bySeq = {};
      old.events.forEach(function (event) { bySeq[event.seq] = event; });
      (session.events || []).forEach(function (event) { bySeq[event.seq] = event; });
      map[session.session_id] = {
        role: session.role || old.role,
        events: Object.keys(bySeq).map(function (seq) { return bySeq[seq]; }).sort(function (a, b) { return a.seq - b.seq; }),
      };
    });
    return boundReplay(map, previous && previous.windowInfo && previous.windowInfo.truncated);
  }

  /// 成员对话（按时间合并所有角色）。`onlyRole` 非空时只渲染该角色——
  /// 多节点并行时按角色过滤比按时间线硬读更清楚（用户反馈"看不出是谁在说"）。
  function workerConversationHtml(ctx, replay, markdown, onlyRole) {
    var esc = ctx.esc;
    var rows = [];
    var filterRole = String(onlyRole == null ? "" : onlyRole).trim();
    Object.keys(replay || {}).forEach(function (sessionId) {
      var session = replay[sessionId], pending = "", start = null;
      var role = String((session && session.role) || "成员");
      var toolRows = Object.create(null);
      if (filterRole && filterRole !== "all" && role !== filterRole) return;
      function textRow(text, event) {
        if (!text) return;
        event = event || {};
        rows.push({ time: event.created_at || "", seq: Number(event.seq) || 0,
          html: '<article class="owo-ws-chat-row"><div class="owo-ws-chat-avatar" aria-hidden="true">' + esc(role.slice(0, 1)) +
          '</div><div class="owo-ws-chat-card"><header class="owo-ws-chat-head"><b>' + esc(role) +
          '</b>' + timeHtml(esc, event.created_at) + '</header><div class="owo-ws-chat-body">' +
          (typeof markdown === "function" ? markdown(text) : '<div class="md-p">' + esc(text).replace(/\n/g, "<br>") + "</div>") + "</div></div></article>" });
      }
      function flushPending(event) {
        if (pending) textRow(pending, start || event || {});
        pending = "";
        start = null;
      }
      (session.events || []).forEach(function (event) {
        var p = event.payload || {}, kind = p.type;
        if (kind === "token_delta") { if (!start) start = event; pending += p.delta || ""; return; }
        if (kind === "final") { textRow(p.text || pending, start || event); pending = ""; start = null; return; }
        flushPending(event);
        if (kind === "tool_use") {
          var toolRow = {
            time: event.created_at || "", seq: Number(event.seq) || 0, role: role,
            sessionId: sessionId, eventKey: sessionId + ":" + (Number(event.seq) || 0), id: String(p.id || ""), tool: String(p.tool || "工具"),
            args: typeof p.args === "string" ? p.args : JSON.stringify(p.args, null, 2),
            result: null,
          };
          rows.push(toolRow);
          if (toolRow.id) toolRows[toolRow.id] = toolRow;
        } else if (kind === "tool_result") {
          var resultRow = p.id ? toolRows[String(p.id)] : null;
          if (!resultRow) {
            resultRow = {
              time: event.created_at || "", seq: Number(event.seq) || 0, role: role,
              sessionId: sessionId, eventKey: sessionId + ":" + (Number(event.seq) || 0), id: String(p.id || ""), tool: String(p.tool || "工具"),
              args: "", result: null,
            };
            rows.push(resultRow);
          }
          resultRow.result = {
            ok: p.ok === true, error: String(p.error || ""),
            preview: String(p.preview || ""), outputs: p.outputs, time: event.created_at || "",
          };
        } else if (kind === "progress") {
          textRow(p.message, event);
        } else if (kind === "plan_update") {
          textRow("工作计划\n" + JSON.stringify(p.steps, null, 2), event);
        }
      });
      flushPending();
    });
    rows.sort(function (a, b) { return a.time.localeCompare(b.time) || a.seq - b.seq; });
    var toolCalls = rows.filter(function (row) { return !!row.tool; });
    var toolFailures = toolCalls.filter(function (row) { return row.result && !row.result.ok; }).length;
    var toolRunning = toolCalls.filter(function (row) { return !row.result; }).length;
    var summary = toolCalls.length
      ? '<div class="owo-ws-chat-tools-summary hint">已执行 ' + toolCalls.length + ' 个工具' +
        (toolFailures ? ' · ' + toolFailures + ' 个失败' : '') +
        (toolRunning ? ' · ' + toolRunning + ' 个进行中' : '') + '</div>'
      : "";
    var windowNotice = replay && replay.windowInfo && replay.windowInfo.truncated
      ? '<p class="hint" role="status">仅展示近期消息窗口；片段可能从中途开始。较早公开记录可在「执行记录 → 持久运行时间线」按页查看，旧运行缺失的历史会明确提示。</p>'
      : "";
    return windowNotice + summary + rows.map(function (row) {
      if (row.html) return row.html;
      var result = row.result;
      var state = !result ? "进行中" : (result.ok ? "已完成" : "失败");
      var stateClass = !result ? "rv-warn" : (result.ok ? "rv-ok" : "rv-bad");
      var detail = '<div class="owo-ws-tool-parts">';
      if (row.args) detail += '<section><b>参数</b><pre>' + esc(row.args) + "</pre></section>";
      if (result) {
        var media = result.ok ? mediaPreviewHtml(esc, row.tool, result.preview, result.outputs) : "";
        if (media) {
          detail += '<section><b>结果</b>' + media + "</section>";
        } else {
          var resultText = result.error || result.preview || (result.ok ? "工具已完成，没有可展示的文本预览。" : "工具失败，服务端没有返回错误详情。");
          detail += '<section><b>' + (result.ok ? "结果" : "失败详情") + "</b><pre>" + esc(resultText) + "</pre></section>";
        }
      } else {
        detail += '<div class="hint">等待工具返回结果。</div>';
      }
      detail += "</div>";
      return '<article class="owo-ws-chat-row"><div class="owo-ws-chat-avatar" aria-hidden="true">' + esc(row.role.slice(0, 1)) +
        '</div><div class="owo-ws-chat-card"><header class="owo-ws-chat-head"><b>' + esc(row.role) +
        '</b>' + timeHtml(esc, row.time) + '</header><details class="owo-ws-tool-detail" data-event-key="' +
        esc(row.eventKey) + '"><summary><b>' + esc(row.tool) + "</b> · <span class=\"owo-ws-badge " +
        stateClass + "\">" + state + "</span></summary>" + detail + "</details></div></article>";
    }).join("");
  }

  /// 当前回放里出现过的角色（按出现顺序去重），供"只看某成员"下拉框。
  function replayRoles(replay) {
    var seen = Object.create(null);
    var roles = [];
    Object.keys(replay || {}).forEach(function (sessionId) {
      var role = String((replay[sessionId] || {}).role || "").trim();
      if (role && !seen[role]) {
        seen[role] = true;
        roles.push(role);
      }
    });
    return roles;
  }

  function conversationRoles(replay, handoffs, members, selectedRole) {
    var roles = replayRoles(replay);
    var memberMap = Object.create(null);
    (members || []).forEach(function (member) { if (member) memberMap[member.member_id] = member; });
    (handoffs || []).forEach(function (handoff) {
      var id = String((handoff && handoff.from_member) || "");
      var role = String((memberMap[id] && memberMap[id].role) || id || "团队成员");
      if (roles.indexOf(role) < 0) roles.push(role);
    });
    // A quiet selected member may have fallen outside the recent rendering window.
    // Keep this one real member selectable instead of silently switching to "all".
    if (selectedRole && roles.indexOf(selectedRole) < 0 &&
        (members || []).some(function(member) { return member && member.role === selectedRole; })) {
      roles.push(selectedRole);
    }
    return roles;
  }

  /// 解析"上次选的成员过滤"：只在该角色本次仍存在时沿用，否则回到 `all`。
  /// 角色是用户自定义的（随时会改名/删角色），所以持久化值必须每次校验。
  function pickConversationRole(saved, roles) {
    var wanted = String(saved == null ? "" : saved).trim();
    if (!wanted || wanted === "all") return "all";
    return (Array.isArray(roles) ? roles : []).indexOf(wanted) >= 0 ? wanted : "all";
  }

  /// 媒体类工具（文生图/语音合成，含 MCP 同名工具）的预览是 JSON 字符串，
  /// 直接 <pre> 出来是一坨 base64。这里解析成路径摘要 + 内联播放器。
  var MEDIA_TOOL_KIND = { generate_image: "image", text_to_speech: "audio" };

  function mediaFormatBytes(bytes) {
    if (!(bytes > 0)) return "";
    if (bytes >= 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB";
    if (bytes >= 1024) return Math.round(bytes / 1024) + " KB";
    return bytes + " B";
  }

  function parseMediaPreview(preview) {
    var text = String(preview == null ? "" : preview).trim();
    var start = text.indexOf("{");
    var end = text.lastIndexOf("}");
    if (start < 0 || end <= start) return null;
    try {
      var value = JSON.parse(text.slice(start, end + 1));
      return value && typeof value === "object" ? value : null;
    } catch (error) {
      return null;
    }
  }

  function mediaPreviewHtml(esc, tool, preview, outputs) {
    if (Array.isArray(outputs) && win.OwoMediaResult) {
      var payload = { tool: tool, ok: true, outputs: outputs };
      var media = win.OwoMediaResult.mediaFromToolResult(payload);
      if (!media) return "";
      return '<div class="owo-ws-media" data-media-artifact="' + esc(JSON.stringify(payload)) + '">' + esc(win.OwoMediaResult.describeMedia(media)) + '</div>';
    }
    var kind = MEDIA_TOOL_KIND[String(tool || "")];
    if (!kind) return "";
    var value = parseMediaPreview(preview);
    if (!value || typeof value.path !== "string" || !value.path.trim()) return "";
    var path = value.path.trim();
    var bytes = Number(value.bytes);
    var label = (kind === "image" ? "已生成图片" : "已生成语音") +
      (bytes > 0 ? "（" + mediaFormatBytes(bytes) + "）" : "") + "：" + path;
    var url = typeof value.data_url === "string" ? value.data_url : "";
    var prefix = kind === "image" ? /^data:image\// : /^data:audio\//;
    var html = '<div class="owo-ws-media">' + esc(label);
    if (prefix.test(url)) {
      html += kind === "image"
        ? '<img class="owo-media-inline" loading="lazy" alt="' + esc(path) + '" src="' + esc(url) + '">'
        : '<audio class="owo-media-audio" controls preload="none" src="' + esc(url) + '"></audio>';
    }
    return html + "</div>";
  }

  var api = {
    timeHtml: timeHtml,
    mergeReplay: mergeReplay,
    boundReplay: boundReplay,
    replayLimits: { events: REPLAY_EVENTS, bytes: REPLAY_BYTES },
    replayRoles: replayRoles,
    conversationRoles: conversationRoles,
    pickConversationRole: pickConversationRole,
    workerConversationHtml: workerConversationHtml,
    mediaPreviewHtml: mediaPreviewHtml,
    refPickerHtml: refPickerHtml,
    teamConversationHtml: teamConversationHtml,
  };

  win.OwoWorkswarmRenderConversation = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})();
