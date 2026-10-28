// ============================================================================
// Action Center（待我处理）面板 —— desktop/web/panels/action-center.panel.js
//
// 七期第四路：把四类"需要人工处理"的事项聚合为一个日常工作台。
// 八期第四路：升级为**正式 Human Inbox 优先**（GET /human/inbox + claim/release/
// resolve 直接处理，口径见 AGENTS-COORD 八期冻结③）。
// 九期第二路（回退口径修订）：**只在 /human/inbox 404（未部署）时**回退七期客户端
// 聚合（legacyLoad）；接口在线时一律以 Inbox 为准——列表为空就是真的为空，不再
// 偷偷切换数据源（避免"聚合口径有、正式 Inbox 无"时两个视图互相矛盾）；其余失败
// 保留 Inbox 口径并展示错误行，面板永远可用。回退路径仍不新增服务端接口：
//   GET  /teams                          {teams:[{team_id,status,mode,interrupted,
//                                         active,members[],project_space_id,...}]}
//   GET  /teams/{id}                     {team,tasks,interrupted,audit_tail,
//                                         changes?,write_lease?,worker_profiles?}
//                                        （七期新字段为可选+可空，UI 双路径容错读取）
//   GET  /projects/{pid}/artifacts       {artifacts:[{artifact_id,version,kind,format,
//                                         review_state,validation?,team_id?,...}]}
//   POST /teams/{id}/steer               {command:"retry",step_id,note} —— 第三路冻结契约
//
// 四类待办（聚合口径，与 WorkSwarm 面板闸门一致）：
//   ① 等待 Human 结果：团队 awaiting_human，或存在人节点（runtime_binding.kind=human）
//      且步骤未到终态（succeeded/failed/aborted）的任务；
//   ② 待评审产物：review_state 归一化后为 pendingreview；以及 validation.valid===false
//      的校验未通过产物（未过校验不进入 PendingReview，同样需要人工关注）；
//   ③ 失败步骤（可重试）：Failed/Aborted 步骤，团队状态非 succeeded/cancelled
//      （与核心 steer_retry 目标闸门一致）；
//   ④ 写租约持有：write_lease 非空且未释放（单写租约，同一工作区同时仅一个写角色）。
//
// 容错原则：所有字段缺省 []/false/null；详情/产物请求失败各自降级为空并记错误行，
// 不阻塞其他类目；产物 404 视为"暂无产物"静默处理。
//
// Node 兼容：globalThis 回退 + module.exports 导出 _test 纯逻辑挂钩（浏览器零差异），
// 供 tests/action-center.panel.test.mjs 断言。
// ============================================================================
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels["action-center"] = (function () {
    var ID = "action-center";
    var MAX_DETAIL = 12;   // 团队详情并发上限（候选裁剪）
    var MAX_PROJECTS = 8;  // 项目产物并发上限

    // ---------- helpers（优先 app.js 注入，缺失时自建回退，与 workswarm/launcher 同款） ----------
    var H = {};
    var rootEl = null;
    var lifecycleGeneration = 0;
    var loadGeneration = 0;
    var inboxGeneration = 0;

    function isCurrentRequest(lifecycle, loadId, inboxId) {
      if (lifecycle !== lifecycleGeneration) return false;
      if (loadId != null && loadId !== loadGeneration) return false;
      if (inboxId != null && inboxId !== inboxGeneration) return false;
      return true;
    }
    function defaultGet(path) {
      return window.OwoApi.get(path);
    }

    function defaultPost(path, body) {
      return window.OwoApi.post(path, body || {});
    }

    function defaultEsc(s) {
      return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
        return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
      });
    }

    function defaultFriendlyError(e) {
      var msg = String((e && e.message) || e || "");
      if (/^404/.test(msg)) return "资源不存在（404）。";
      if (/^400/.test(msg)) return "请求被拒绝（400）：" + msg.slice(5, 160);
      if (/^401|^403/.test(msg)) return "无权访问（401/403）。请检查凭据与权限范围。";
      if (/^409/.test(msg)) return "状态冲突（409）：当前状态不允许该操作。" + msg.slice(4, 160);
      if (/Failed to fetch|NetworkError/i.test(msg)) return "网络错误：无法连接服务端，请确认服务已启动。";
      return msg || "未知错误";
    }

    function el(sel) {
      if (!rootEl) return null;
      return rootEl.querySelector(String(sel || ""));
    }

    function esc(s) {
      return (H.esc || defaultEsc)(s);
    }

    function friendly(e) {
      return (H.friendlyError && H.friendlyError(e)) || defaultFriendlyError(e);
    }

    // ---------- 状态 ----------
    var state = {
      loading: false,
      loadedOnce: false,
      teams: [],            // GET /teams 原始列表
      details: {},          // team_id -> 详情 payload（加载失败不写入）
      detailErrors: {},     // team_id -> 错误消息
      artifacts: {},        // pid -> { items, team_id, error }（404 → items=[] 且 error=""）
      items: { human: [], review: [], failed: [], lease: [] },
      errors: [],           // 顶层请求错误备注
      retryBusy: {},        // key(teamId/stepId) -> true
      retryResults: {},     // key -> { ok, text }
      lastLoadedAt: "",
      // 八期三路：正式 Human Inbox（/human/inbox）优先；仅 404（未部署）回退
      // 客户端聚合；在线为空/其余失败保持 Inbox 口径（九期修订）。
      inbox: [],            // HumanWorkItem 归一化列表（inbox 模式）
      inboxSource: "",      // "inbox" | "legacy" | ""（渲染分支 + 头部口径提示）
      inboxBusy: {},        // key(item_id:action) -> true
      inboxResults: {},     // item_id -> { ok, text }
      filters: { status: "", kind: "", q: "" },
      drafts: Object.create(null),
      notice: "",
      hostScope: "",
    };

    // ---------- 纯逻辑层（Node 测试挂钩覆盖） ----------
    var STEP_TERMINAL = { succeeded: true, failed: true, aborted: true };
    var STEP_RETRYABLE = { failed: true, aborted: true };

    // 团队状态归一：TeamRun 里是 snake_case（serde），创建响应/SSE state 帧是
    // Debug 形式（AwaitingHuman/...）——统一小写归一化（与 WorkSwarm 同款）。
    function normStatus(s) {
      var x = String(s == null ? "" : s).toLowerCase();
      if (x === "awaitinghuman") return "awaiting_human";
      return x;
    }

    var TEAM_NO_RETRY = { succeeded: true, cancelled: true };

    function isRetryableStep(t) {
      return !!t && !!STEP_RETRYABLE[normStatus(t.status)];
    }

    // 候选团队：可能存在"待我处理"项的团队。succeeded/cancelled 一律排除（终态且
    // 不可重试）；failed 保留（存在可 retry 的失败步骤）；其余状态保留。
    // 排序：awaiting_human > running > failed > 其他；同类内 updated_at 降序；截断。
    function candidateTeams(teams, cap) {
      var rank = { awaiting_human: 0, running: 1, failed: 2 };
      var list = (teams || []).filter(function (t) {
        if (!t || t.team_id == null) return false;
        var st = normStatus(t.status);
        return !TEAM_NO_RETRY[st];
      });
      list.sort(function (a, b) {
        var ra = rank[normStatus(a.status)];
        var rb = rank[normStatus(b.status)];
        if (ra !== rb) return (ra == null ? 3 : ra) - (rb == null ? 3 : rb);
        return String(b.updated_at || "").localeCompare(String(a.updated_at || ""));
      });
      return list.slice(0, cap == null ? MAX_DETAIL : cap);
    }

    // 项目空间 id：team.project_space_id 优先，缺省 "proj-"+team_id
    // （与 WorkSwarm loadArtifacts 的派生规则同款）；去重后截断。
    function projectIdsOf(teams, cap) {
      var seen = {};
      var out = [];
      (teams || []).forEach(function (t) {
        if (!t || t.team_id == null) return;
        var pid = t.project_space_id || "proj-" + t.team_id;
        if (pid && !seen[pid]) {
          seen[pid] = true;
          out.push({ pid: pid, team_id: String(t.team_id) });
        }
      });
      return out.slice(0, cap == null ? MAX_PROJECTS : cap);
    }

    // 人节点任务（与 WorkSwarm humanTasks 同闸门）：成员 runtime_binding.kind ===
    // "human" 且步骤未到终态。member 以 task.worker 对应 member_id。
    function humanTasksOf(detail) {
      var team = (detail && (detail.team || detail)) || {};
      var tasks = (detail && detail.tasks) || [];
      var byId = {};
      (team.members || []).forEach(function (m) {
        if (m && m.member_id) byId[m.member_id] = m;
      });
      return tasks.filter(function (t) {
        var m = t && byId[t.worker];
        return !!(m && m.runtime_binding && m.runtime_binding.kind === "human" && !STEP_TERMINAL[normStatus(t.status)]);
      });
    }

    // ① 等待 Human 结果：团队 awaiting_human，或存在未完成人节点任务。
    //    输出按团队聚合为一条（深链目标本来就是团队详情）。
    function humanItemsOf(detail) {
      var team = (detail && detail.team) || {};
      var st = normStatus(team.status);
      var hts = humanTasksOf(detail);
      if (st !== "awaiting_human" && !hts.length) return [];
      return [{
        kind: "human",
        team_id: String(team.team_id || ""),
        team_status: st,
        awaiting_team: st === "awaiting_human",
        tasks: hts.map(function (t) {
          return {
            task_id: String(t.task_id || ""),
            role: String(t.role || t.task_id || ""),
            status: normStatus(t.status),
          };
        }),
      }];
    }

    // ③ 失败步骤（可重试）：Failed/Aborted 步骤；团队 succeeded/cancelled 不提供
    // （与 shouldShowRetry 闸门一致）。error/failure_code 双字段容错。
    function failedItemsOf(detail) {
      var team = (detail && detail.team) || {};
      var st = normStatus(team.status);
      if (TEAM_NO_RETRY[st]) return [];
      var tasks = (detail && detail.tasks) || [];
      return tasks.filter(isRetryableStep).map(function (t) {
        return {
          kind: "failed",
          team_id: String(team.team_id || ""),
          team_status: st,
          step_id: String(t.task_id || ""),
          role: String(t.role || t.worker || t.task_id || ""),
          attempts: Number(t.attempts || 0),
          error: String(t.error || t.failure_code || ""),
          interrupted: !!(detail && detail.interrupted),
        };
      });
    }

    // ④ 写租约：七期新字段，双路径容错读取（详情顶层或 team 对象内）。
    function leaseOf(detail) {
      if (!detail) return null;
      var lease = detail.write_lease;
      if (!lease && detail.team && typeof detail.team === "object") lease = detail.team.write_lease;
      return lease && typeof lease === "object" ? lease : null;
    }

    function leaseItemsOf(detail) {
      var team = (detail && detail.team) || {};
      var lease = leaseOf(detail);
      if (!lease || lease.released_at_ms != null) return []; // 已释放 → 不再需要处理
      return [{
        kind: "lease",
        team_id: String(team.team_id || ""),
        holder_role: String(lease.holder_role || "—"),
        holder_step_id: String(lease.holder_step_id || "—"),
        acquired_at_ms: lease.acquired_at_ms == null ? null : Number(lease.acquired_at_ms),
      }];
    }

    // 评审状态归一（与 WorkSwarm normReviewState 同款）。
    function normReviewState(s) {
      return String(s == null ? "" : s).toLowerCase().replace(/[_\s-]/g, "");
    }

    // ② 待评审产物（单项目聚合）：pendingreview，或 validation.valid===false
    //    （校验未通过不进入 PendingReview，但同样需要人工处理）。
    //    entry = { items, team_id, error }；产物自带 team_id 优先，缺省回落到 pid 所属团队。
    function reviewItemsOf(pid, entry) {
      var fallbackTeam = entry && entry.team_id ? String(entry.team_id) : "";
      return ((entry && entry.items) || []).filter(function (a) {
        if (!a || a.artifact_id == null) return false;
        if (normReviewState(a.review_state) === "pendingreview") return true;
        var v = a.validation;
        return !!(v && typeof v === "object" && v.valid === false);
      }).map(function (a) {
        var v = a.validation && typeof a.validation === "object" ? a.validation : null;
        return {
          kind: "review",
          project_id: String(pid || ""),
          artifact_id: String(a.artifact_id),
          version: a.version == null ? "" : String(a.version),
          review_state: normReviewState(a.review_state),
          kind_name: String(a.kind || ""),
          format: String(a.format || ""),
          team_id: a.team_id == null ? fallbackTeam : String(a.team_id),
          validation_failed: !!(v && v.valid === false),
          validation_reason: v && v.reason != null ? String(v.reason) : "",
        };
      });
    }

    // 汇总（纯函数）：对 state 形状的输入做四类聚合；详情/产物加载失败的条目跳过
    // （错误已在 detailErrors/artifacts[].error 中记录，渲染层单独展示）。
    function aggregate(st) {
      var details = (st && st.details) || {};
      var human = [];
      var failed = [];
      var lease = [];
      Object.keys(details).forEach(function (tid) {
        var d = details[tid];
        if (!d) return;
        human = human.concat(humanItemsOf(d));
        failed = failed.concat(failedItemsOf(d));
        lease = lease.concat(leaseItemsOf(d));
      });
      var review = [];
      var arts = (st && st.artifacts) || {};
      Object.keys(arts).forEach(function (pid) {
        var entry = arts[pid];
        if (entry && !entry.error) review = review.concat(reviewItemsOf(pid, entry));
      });
      return { human: human, review: review, failed: failed, lease: lease };
    }

    // 第三路冻结契约：POST /teams/{id}/steer {"command":"retry","step_id":"...","note":"..."}
    // note 可省略（服务端缺省 "retry"）；这里给确定性中文缺省，便于审计可读。
    function buildRetryBody(stepId, note) {
      var sid = String(stepId == null ? "" : stepId).trim();
      var n = note == null ? "" : String(note).trim();
      return {
        command: "retry",
        step_id: sid,
        note: n || ("重试此节点：" + sid),
      };
    }

    // ===========================================================================
    // 八期三路：正式 Human Inbox（/human/inbox）——领取/释放/直接处理四类待办。
    // 口径（AGENTS-COORD 八期冻结③）：
    //   GET  /human/inbox                {items:[HumanWorkItem]}
    //   POST /human/inbox/{id}/claim | release
    //   POST /human/inbox/{id}/resolve   按 kind 分派（resolve 重放幂等 200）
    // HumanWorkItem: {item_id, kind: human_result|artifact_review|change_set|step_retry,
    //   team_id, project_id, target_id, status: open|claimed|resolved, assignee?,
    //   summary, created_at, claimed_at?, resolved_at?}
    // 回退原则：inbox 仅 404 → 回退客户端聚合（legacyLoad），面板永远可用。
    // ===========================================================================

    var INBOX_KIND_CN = {
      human_result: "人工结果",
      artifact_review: "待评审产物",
      change_set: "变更审批",
      step_retry: "失败步骤重试",
    };
    var INBOX_KIND_ORDER = ["human_result", "artifact_review", "change_set", "step_retry"];

    function normItemStatus(s) {
      return String(s == null ? "" : s).toLowerCase();
    }

    // 归一化 Inbox 条目：字段全部容错；item_id/kind 非法条目丢弃。
    function inboxItemsOf(payload) {
      var items = (payload && payload.items) || [];
      return items
        .filter(function (it) {
          return !!(it && it.item_id && Object.prototype.hasOwnProperty.call(INBOX_KIND_CN, it.kind));
        })
        .map(function (it) {
          return {
            item_id: String(it.item_id),
            kind: String(it.kind),
            team_id: it.team_id == null ? "" : String(it.team_id),
            project_id: it.project_id == null ? "" : String(it.project_id),
            target_id: it.target_id == null ? "" : String(it.target_id),
            status: normItemStatus(it.status) || "open",
            assignee: it.assignee == null ? "" : String(it.assignee),
            summary: String(it.summary || ""),
            role: it.detail && typeof it.detail.role === "string" ? it.detail.role : "",
            created_at: it.created_at == null ? "" : String(it.created_at),
            claimed_at: it.claimed_at == null ? "" : String(it.claimed_at),
            resolved_at: it.resolved_at == null ? "" : String(it.resolved_at),
          };
        });
    }

    // 分组展示：已处理（resolved）条目不再出现（服务端已收敛，这里双保险过滤）。
    function groupInbox(items) {
      var out = {};
      INBOX_KIND_ORDER.forEach(function (k) {
        out[k] = [];
      });
      (items || []).forEach(function (it) {
        if (it && Object.prototype.hasOwnProperty.call(INBOX_KIND_CN, it.kind) && it.status !== "resolved") out[it.kind].push(it);
      });
      return out;
    }

    function buildInboxActionPath(itemId, action) {
      return "/human/inbox/" + encodeURIComponent(String(itemId == null ? "" : itemId)) + "/" +
        String(action || "");
    }

    // 处理人标识（三路 claim/release 必填 user；resolve 审计/领取校验用）。
    // 桌面单用户场景：localStorage 持久化，缺省「本地用户」（Node 测试环境无
    // localStorage → 安全回退）。
    function currentUser() {
      try {
        var v = win.localStorage && win.localStorage.getItem("owo.user");
        if (v && String(v).trim()) return String(v).trim();
      } catch (e) {
        /* 无 localStorage（Node）→ 缺省 */
      }
      return "本地用户";
    }

    // resolve 请求体按 kind 分派（冻结③）：artifact_review → decision；
    // change_set → action(accept|reject)；step_retry → {}；human_result → {result}。
    function buildResolveBody(kind, action, extra) {
      if (kind === "artifact_review") {
        var d = String(action || "").trim();
        if (d !== "approve" && d !== "request_changes" && d !== "reject") {
          return { error: "unknown_decision" };
        }
        var body = { decision: d };
        var c = String((extra && extra.comment) == null ? "" : extra.comment).trim();
        if (c) body.comment = c;
        return body;
      }
      if (kind === "change_set") {
        var a = String(action || "").trim();
        if (a !== "accept" && a !== "reject") return { error: "unknown_action" };
        return { action: a };
      }
      if (kind === "human_result") {
        var r = String((extra && extra.result) == null ? "" : extra.result).trim();
        if (!r) return { error: "missing_result" };
        return { result: r };
      }
      if (kind === "step_retry") return {};
      return { error: "unknown_kind" };
    }

    function inboxStatusBadge(it) {
      var st = normItemStatus(it && it.status);
      if (st === "claimed") return '<span class="owo-ac-badge info">已领取</span>';
      if (st === "resolved") return '<span class="owo-ac-badge ok">已处理</span>';
      return '<span class="owo-ac-badge warn">待领取</span>';
    }

    function itemBusy(id) {
      return Object.keys(state.inboxBusy).some(function (key) {
        return key.slice(0, key.lastIndexOf(":")) === id && state.inboxBusy[key];
      });
    }
    function filteredInbox(items, filters) {
      var f = filters || {}, q = String(f.q || "").trim().toLocaleLowerCase();
      return (items || []).filter(function (it) {
        return it.status !== "resolved" && (!f.status || it.status === f.status) &&
          (!f.kind || it.kind === f.kind) &&
          (!q || [it.summary, it.team_id, it.project_id, it.target_id, it.assignee]
            .join(" ").toLocaleLowerCase().indexOf(q) >= 0);
      });
    }
    function inboxActionButtons(it) {
      var tid = it.item_id, busy = itemBusy(tid), mine = !it.assignee || it.assignee === currentUser();
      function btn(action, label, primary) {
        return '<button type="button" class="owo-ac-mini' + (primary ? " primary" : "") +
          '" data-core-action data-ic-act="' + esc(action) + '" data-ic-item="' + esc(tid) + '"' +
          (busy ? " disabled" : "") + ">" + esc(label) + "</button>";
      }
      var out = [];
      if (it.status === "open") out.push(btn("claim", "领取", true));
      else if (it.status === "claimed" && mine) {
        out.push(btn("release", "释放"));
        if (it.kind === "artifact_review") out.push(btn("approve", "批准", true), btn("request_changes", "要求修改"), btn("reject", "拒绝"));
        else if (it.kind === "change_set") out.push(btn("accept", "接受", true), btn("reject", "拒绝"));
        else if (it.kind === "step_retry") out.push(btn("resolve", "重试", true));
        else if (it.kind === "human_result") out.push(btn("resolve", "提交结果", true));
      }
      if (it.team_id) out.push('<button type="button" class="owo-ac-mini" data-ac-goto="' + esc(it.team_id) + '">打开任务</button>');
      return out.join("");
    }
    function inboxItemHtml(it) {
      var res = state.inboxResults[it.item_id], mine = !it.assignee || it.assignee === currentUser();
      var editable = it.status === "claimed" && mine && (it.kind === "human_result" || it.kind === "artifact_review");
      var field = it.kind === "human_result" ? "result" : "comment", draft = state.drafts[it.item_id] || {};
      return '<article class="owo-ac-item" data-ic-item-row="' + esc(it.item_id) + '">' +
        '<div class="owo-ac-line"><h3>' + esc(it.role || it.summary || it.target_id || "待办事项") + '</h3>' + inboxStatusBadge(it) + '</div>' +
        '<div class="owo-ac-sub hint">' + esc(INBOX_KIND_CN[it.kind]) + ' · ' + esc(it.team_id || it.project_id || "—") +
        (it.assignee ? ' · 领取人：' + esc(it.assignee) : "") + '</div>' +
        (editable ? '<label class="ac-draft-label">' + (field === "result" ? "处理结果" : "评审意见（选填）") +
          '<textarea rows="3" class="owo-ac-note" data-ic-note="' + esc(it.item_id) + '" data-ic-field="' + field +
          '" placeholder="' + (field === "result" ? "填写处理结果…" : "补充评审依据或需要修改的内容…") + '"' +
          (itemBusy(it.item_id) ? " disabled" : "") + '>' + esc(draft[field] || "") + '</textarea></label>' : "") +
        (!mine && it.status === "claimed" ? '<p class="hint">此事项由其他用户处理，释放后可重新领取。</p>' : "") +
        '<div class="owo-ac-actions">' + inboxActionButtons(it) + '</div>' +
        '<div class="owo-ac-result' + (res ? (res.ok ? " ok" : " bad") : "") + '" data-ic-result="' + esc(it.item_id) +
        '" aria-live="polite">' + (res && !res.ok ? esc(res.text) : "") + '</div></article>';
    }
    function inboxSectionsHtml(items) {
      var list = filteredInbox(items, state.filters);
      if (!list.length) return '<div class="owo-ac-empty">' +
        (inboxTotal(items) ? "没有匹配的待办。调整筛选条件后重试。" : "当前没有待办。") + "</div>";
      return list.map(inboxItemHtml).join("");
    }
    function filtersHtml() {
      return '<div class="ac-status-tabs" role="group" aria-label="待办状态">' +
        [["", "全部"], ["open", "待领取"], ["claimed", "已领取"]].map(function (pair) {
          return '<button type="button" data-ac-status="' + pair[0] + '" aria-pressed="' + (state.filters.status === pair[0]) +
            '">' + pair[1] + ' <span data-ac-count="' + pair[0] + '">0</span></button>';
        }).join("") + '</div><div class="ac-filters"><input type="search" id="ac-search" aria-label="搜索待办、任务或项目" ' +
        'placeholder="搜索待办、任务或项目" value="' + esc(state.filters.q) + '"><select id="ac-kind" aria-label="待办类型">' +
        '<option value="">全部类型</option>' + INBOX_KIND_ORDER.map(function (k) {
          return '<option value="' + k + '"' + (state.filters.kind === k ? " selected" : "") + '>' + esc(INBOX_KIND_CN[k]) + '</option>';
        }).join("") + '</select></div>';
    }
    function rememberInput(ev) {
      var t = ev.target;
      if (!t || !t.getAttribute) return;
      var id = t.getAttribute("data-ic-note"), field = t.getAttribute("data-ic-field");
      if (!id || (field !== "result" && field !== "comment")) return;
      if (!state.drafts[id]) state.drafts[id] = {};
      state.drafts[id][field] = t.value;
    }

    function inboxTotal(items) {
      return (items || []).filter(function (it) {
        return it.status !== "resolved";
      }).length;
    }

    function paintInboxResult(itemId) {
      var box = el("#ac-sections");
      if (!box) return;
      var res = state.inboxResults[itemId];
      var div = box.querySelector('[data-ic-result="' + String(itemId).replace(/"/g, '\\"') + '"]');
      if (div) {
        div.textContent = res ? res.text : "";
        div.className = "owo-ac-result" + (res ? (res.ok ? " ok" : " bad") : "");
      }
    }

    // 领取/释放/直接处理：claim|release 走对应端点；resolve 类动作统一 POST
    // /human/inbox/{id}/resolve（请求体按 kind 分派）。幂等重放（replayed）如实提示。
    function startInboxAction(itemId, action, extra) {
      var id = String(itemId == null ? "" : itemId);
      var act = String(action || "");
      if (!id || !act) return Promise.resolve();
      var it = null;
      (state.inbox || []).forEach(function (x) {
        if (x.item_id === id) it = x;
      });
      if (!it || it.status === "resolved") return Promise.resolve();
      var isResolve = act !== "claim" && act !== "release";
      var key = id + ":" + act;
      if (itemBusy(id)) return Promise.resolve();
      if (it.status === "claimed" && it.assignee && it.assignee !== currentUser()) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      var body = {};
      if (isResolve) {
        body = buildResolveBody(it.kind, it.kind === "step_retry" ? "resolve" : act, extra);
        if (body && body.error) {
          state.inboxResults[id] = { ok: false, text: "无法构建请求：" + body.error };
          paintInboxResult(id);
          return Promise.resolve();
        }
      }
      var locks = state.inboxBusy, actionScope = state.hostScope;
      locks[key] = true;
      var post = H.post;
      render();
      var path = buildInboxActionPath(id, isResolve ? "resolve" : act);
      if (isResolve) {
        body.user = currentUser(); // 审计/领取校验（review 类兼作 reviewer 缺省）
      } else {
        body = { user: currentUser() }; // claim/release：ActorRequest {user}
      }
      return Promise.resolve()
        .then(function () { if (isCurrentRequest(lifecycle)) return post(path, body); })
        .then(function (resp) {
          if (!isCurrentRequest(lifecycle)) return;
          var replayed = !!(resp && resp.replayed);
          state.inboxResults[id] = {
            ok: true,
            text: isResolve
              ? replayed
                ? "已处理（幂等重放，无重复副作用）。"
                : "已处理。"
              : act === "claim"
                ? "已领取。"
                : "已释放。",
          };
          state.notice = (it.role || it.summary || "待办事项") + "：" + state.inboxResults[id].text;
          if (isResolve) delete state.drafts[id];
        })
        .catch(function (e) {
          if (!isCurrentRequest(lifecycle)) return;
          state.inboxResults[id] = { ok: false, text: "操作失败：" + friendly(e) };
        })
        .then(function () {
          if (!isCurrentRequest(lifecycle)) {
            delete locks[key];
            if (rootEl && actionScope === state.hostScope && state.inboxSource === "inbox") return refreshInbox();
            return;
          }
          var refreshed = state.inboxSource === "inbox" ? refreshInbox() : Promise.resolve();
          return Promise.resolve(refreshed).then(function () {
            delete locks[key];
            if (isCurrentRequest(lifecycle)) render();
          }, function () {
            delete locks[key];
            if (isCurrentRequest(lifecycle)) render();
          });
        });
    }

    // Inbox 重取（动作后刷新列表口径；九期：仅 404 回退 legacy，失败只记错误、
    // 保持当前 Inbox 口径，空列表也不再切换数据源）。
    function refreshInbox() {
      var lifecycle = lifecycleGeneration;
      var request = ++inboxGeneration;
      loadGeneration++;
      state.loading = true;
      state.errors = [];
      render();
      return Promise.resolve()
        .then(function () { return H.get("/human/inbox"); })
        .then(function (d) {
          if (!isCurrentRequest(lifecycle, null, request)) return;
          state.inbox = inboxItemsOf(d);
          state.loading = false;
          state.inboxSource = "inbox"; // 在线即以 Inbox 为准（空列表也是真实状态）
          state.lastLoadedAt = new Date().toLocaleTimeString();
          render();
        })
        .catch(function (e) {
          if (!isCurrentRequest(lifecycle, null, request)) return;
          if (isNotFound(e)) {
            // Inbox 未部署（404）→ 回退七期客户端聚合（面板永远可用）。
            state.inboxSource = "legacy";
            return legacyLoad(lifecycle, null, request);
          }
          state.loading = false;
          state.errors.push("Inbox 刷新失败（保持正式 Inbox 口径）：" + friendly(e));
          render();
        });
    }

    // ---------- 渲染（纯 HTML 构建器，render 只负责赋值与绑定） ----------
    function fmtWhen(ms) {
      var n = Number(ms);
      if (!isFinite(n) || n <= 0) return "";
      try {
        return new Date(n).toLocaleString();
      } catch (e) {
        return String(ms);
      }
    }

    var SEC_DEFS = [
      { key: "human", no: "①", title: "等待人工结果", hint: "有任务正在等待人工处理", empty: "暂无等待人工结果的任务" },
      { key: "review", no: "②", title: "待评审产物", hint: "有产物等待评审，或校验未通过", empty: "暂无待评审产物" },
      { key: "failed", no: "③", title: "失败步骤（可重试）", hint: "可一键重试失败节点", empty: "暂无可重试的失败步骤" },
      { key: "lease", no: "④", title: "写入权持有", hint: "有团队正持有写入权（同一时刻仅一个团队可写入）", empty: "当前没有团队持有写入权" },
    ];

    function itemHtml(item, busy, results) {
      var b = busy || {};
      var r = results || {};
      if (item.kind === "human") {
        var tasks = item.tasks || [];
        return (
          '<div class="owo-ac-item" data-kind="human">' +
          '<div class="owo-ac-line">' +
          '<span class="owo-ac-badge warn">等待人节点</span>' +
          "<b>" + esc(item.team_id || "—") + "</b>" +
          '<span class="hint">' + (item.awaiting_team ? "团队等待人工处理 · " : "") + tasks.length + " 个未完成人节点</span>" +
          "</div>" +
          (tasks.length
            ? '<div class="owo-ac-sub hint">' + tasks.map(function (t) {
                return esc(t.role || t.task_id) + "（" + esc(t.status) + "）";
              }).join("、") + "</div>"
            : "") +
          '<div class="owo-ac-actions">' +
          '<button type="button" class="owo-ac-mini primary" data-ac-goto="' + esc(item.team_id) + '">进入团队提交结果</button>' +
          '<span class="hint">在团队详情「人节点结果」区提交结果</span>' +
          "</div></div>"
        );
      }
      if (item.kind === "review") {
        var badge = item.validation_failed
          ? '<span class="owo-ac-badge bad">校验未通过</span>'
          : '<span class="owo-ac-badge ok">待评审</span>';
        var what = (item.kind_name || item.format || "artifact") + (item.version ? " · v" + esc(item.version) : "");
        return (
          '<div class="owo-ac-item" data-kind="review">' +
          '<div class="owo-ac-line">' + badge +
          "<b>" + esc(item.artifact_id) + "</b>" +
          '<span class="hint">' + what + " · 项目 " + esc(item.project_id) + "</span>" +
          "</div>" +
          (item.validation_failed
            ? '<div class="owo-ac-sub bad">校验：' + esc(item.validation_reason || "格式校验未通过") + "</div>"
            : "") +
          '<div class="owo-ac-actions">' +
          '<button type="button" class="owo-ac-mini primary" data-ac-goto-artifact data-team="' + esc(item.team_id) + '">进入评审</button>' +
          "</div></div>"
        );
      }
      if (item.kind === "failed") {
        var key = String(item.team_id) + "/" + String(item.step_id);
        var isBusy = !!b[key];
        var res = r[key];
        return (
          '<div class="owo-ac-item" data-kind="failed">' +
          '<div class="owo-ac-line">' +
          '<span class="owo-ac-badge bad">失败步骤</span>' +
          "<b>" + esc(item.step_id) + "</b>" +
          '<span class="hint">' + esc(item.role || "—") + " · 第 " + esc(item.attempts) + " 次尝试 · 团队 " + esc(item.team_id) + (item.interrupted ? " · 已中断" : "") + "</span>" +
          "</div>" +
          (item.error ? '<div class="owo-ac-sub bad">' + esc(item.error) + "</div>" : "") +
          '<div class="owo-ac-actions">' +
          '<button type="button" class="owo-ac-mini" data-core-action data-ac-retry data-ac-key="' + esc(key) + '" data-team="' + esc(item.team_id) + '" data-step="' + esc(item.step_id) + '"' + (isBusy ? " disabled" : "") + ">" + (isBusy ? "重试中…" : "重试") + "</button>" +
          '<button type="button" class="owo-ac-mini" data-ac-goto="' + esc(item.team_id) + '">进入团队</button>' +
          "</div>" +
          '<div class="owo-ac-result' + (res ? (res.ok ? " ok" : " bad") : "") + '" data-ac-result="' + esc(key) + '" aria-live="polite">' + (res ? esc(res.text) : "") + "</div>" +
          "</div>"
        );
      }
      if (item.kind === "lease") {
        return (
          '<div class="owo-ac-item" data-kind="lease">' +
          '<div class="owo-ac-line">' +
          '<span class="owo-ac-badge info">写租约持有中</span>' +
          "<b>" + esc(item.team_id || "—") + "</b>" +
          '<span class="hint">' + esc(item.holder_role) + " · 步骤 " + esc(item.holder_step_id) + (item.acquired_at_ms ? " · " + esc(fmtWhen(item.acquired_at_ms)) + " 起" : "") + "</span>" +
          "</div>" +
          '<div class="owo-ac-actions">' +
          '<button type="button" class="owo-ac-mini" data-ac-goto="' + esc(item.team_id) + '">进入团队</button>' +
          "</div></div>"
        );
      }
      return "";
    }

    function sectionsHtml(items, st) {
      items = items || {};
      var busy = (st && st.retryBusy) || {};
      var results = (st && st.retryResults) || {};
      return SEC_DEFS.map(function (def) {
        var list = items[def.key] || [];
        var body = list.length
          ? list.map(function (item) { return itemHtml(item, busy, results); }).join("")
          : '<div class="owo-ac-empty">' + esc(def.empty) + "</div>";
        return (
          '<section class="owo-ac-sec" data-ac-sec="' + def.key + '">' +
          "<h3>" + def.no + " " + esc(def.title) +
          ' <span class="owo-ac-count' + (list.length ? " has" : "") + '">' + list.length + "</span>" +
          ' <span class="hint">' + esc(def.hint) + "</span></h3>" +
          '<div class="owo-ac-items">' + body + "</div>" +
          "</section>"
        );
      }).join("");
    }

    function shellHtml() {
      return (
        '<div class="owo-ac">' +
        '<div class="owo-ac-head">' +
        '<span class="hint">领取待办，提交结果，继续推进任务。</span>' +
        '<button type="button" id="ac-refresh" class="owo-ac-mini">刷新</button>' +
        '<span id="ac-meta" class="hint" aria-live="polite"></span>' +
        "</div>" +
        '<div id="ac-meta-row"></div>' +
        '<div id="ac-controls" hidden></div>' +
        '<div id="ac-notice" class="ac-notice" role="status"></div>' +
        '<div id="ac-errors" aria-live="polite"></div>' +
        '<div id="ac-sections"><div class="owo-ac-empty">正在加载待办…</div></div>' +
        "</div>"
      );
    }

    // ---------- 行为层 ----------
    function totalOf(items) {
      items = items || {};
      return (items.human || []).length + (items.review || []).length + (items.failed || []).length + (items.lease || []).length;
    }

    function paintMeta(text) {
      var m = el("#ac-meta");
      if (m) m.textContent = text;
    }

    function paintErrors() {
      var box = el("#ac-errors");
      if (!box) return;
      var lines = state.errors.slice();
      Object.keys(state.detailErrors || {}).forEach(function (tid) {
        lines.push("团队 " + tid + " 详情加载失败：" + state.detailErrors[tid]);
      });
      Object.keys(state.artifacts || {}).forEach(function (pid) {
        var entry = state.artifacts[pid];
        if (entry && entry.error) lines.push("项目 " + pid + " 产物加载失败：" + entry.error);
      });
      box.innerHTML = lines.map(function (x) { return '<div class="owo-ac-err">' + esc(x) + "</div>"; }).join("");
    }

    function render() {
      if (!rootEl) return;
      var doc = rootEl.ownerDocument, active = doc && doc.activeElement;
      var focused = active && active.getAttribute && active.getAttribute("data-ic-note");
      var selection = focused ? [active.selectionStart, active.selectionEnd] : null;
      var box = el("#ac-sections");
      if (box) box.innerHTML = state.loading && !state.loadedOnce && !state.inbox.length
        ? '<div class="owo-ac-empty">正在加载待办…</div>'
        : state.errors.length && !state.inbox.length && state.inboxSource === "inbox"
          ? '<div class="owo-ac-empty">暂时无法获取待办，请刷新重试。</div>'
          : state.inboxSource === "inbox" ? inboxSectionsHtml(state.inbox) : sectionsHtml(state.items, state);
      if (focused && box && box.querySelectorAll) Array.prototype.some.call(box.querySelectorAll("[data-ic-note]"), function (input) {
        if (input.getAttribute("data-ic-note") !== focused || input.disabled) return false;
        input.focus();
        if (selection && input.setSelectionRange) input.setSelectionRange(selection[0], selection[1]);
        return true;
      });
      var controls = el("#ac-controls");
      if (controls) {
        controls.hidden = state.inboxSource !== "inbox";
        if (controls.querySelectorAll) Array.prototype.forEach.call(controls.querySelectorAll("[data-ac-status]"), function (button) {
          var status = button.getAttribute("data-ac-status");
          button.setAttribute("aria-pressed", String(state.filters.status === status));
          var count = button.querySelector("[data-ac-count]");
          if (count) count.textContent = filteredInbox(state.inbox, { status: status }).length;
        });
      }
      var refresh = el("#ac-refresh");
      if (refresh) { refresh.disabled = state.loading; refresh.textContent = state.loading ? "刷新中…" : "刷新"; }
      var notice = el("#ac-notice");
      if (notice) notice.textContent = state.notice;
      paintErrors();
      paintMeta(state.loading ? "加载中…" : state.inboxSource === "inbox"
        ? "共 " + inboxTotal(state.inbox) + " 项待办" + (state.lastLoadedAt ? " · 更新于 " + state.lastLoadedAt : "")
        : "共 " + totalOf(state.items) + " 项待办（兼容视图）" + (state.lastLoadedAt ? " · 更新于 " + state.lastLoadedAt : ""));
    }

    // 提交锁 + 结果回写（按钮/结果区由 sectionsHtml 重建，直接按 data 属性定位）。
    function paintRetry(key) {
      var box = el("#ac-sections");
      if (!box) return;
      var res = state.retryResults[key];
      var div = box.querySelector('[data-ac-result="' + key.replace(/"/g, '\\"') + '"]');
      if (div) {
        div.textContent = res ? res.text : "";
        div.className = "owo-ac-result" + (res ? (res.ok ? " ok" : " bad") : "");
      }
      if (!state.retryBusy[key]) {
        var btn = box.querySelector('[data-ac-retry][data-ac-key="' + key.replace(/"/g, '\\"') + '"]');
        if (btn) {
          btn.disabled = false;
          btn.textContent = "重试";
        }
      }
    }

    function submitRetry(teamId, stepId) {
      var key = String(teamId) + "/" + String(stepId);
      if (state.retryBusy[key]) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      state.retryBusy[key] = true;
      delete state.retryResults[key];
      paintRetry(key);
      var btn = el('#ac-sections [data-ac-retry][data-ac-key="' + key.replace(/"/g, '\\"') + '"]');
      if (btn) {
        btn.disabled = true;
        btn.textContent = "重试中…";
      }
      return Promise.resolve()
        .then(function () { return H.post("/teams/" + encodeURIComponent(teamId) + "/steer", buildRetryBody(stepId)); })
        .then(function () {
          if (!isCurrentRequest(lifecycle)) return;
          state.retryResults[key] = { ok: true, text: "重试指令已发送（command=retry），等待步骤重新调度。" };
        })
        .catch(function (e) {
          if (!isCurrentRequest(lifecycle)) return;
          state.retryResults[key] = { ok: false, text: "重试失败：" + friendly(e) };
        })
        .then(function () {
          delete state.retryBusy[key];
          if (!isCurrentRequest(lifecycle)) return;
          paintRetry(key);
          // 成功后重载聚合：失败步骤应随团队状态推进而消失。
          if (state.retryResults[key] && state.retryResults[key].ok) return load();
        });
    }

    // 深链：经导航按钮切到 WorkSwarm 面板，再用公开 open() 直达团队详情
    // （与 Project Launcher gotoTeam 同款）。teamId 为空时仅落在列表页。
    function gotoWorkswarm(teamId) {
      if (H.openTeam && teamId) { H.openTeam(teamId); return; }
      var doc = typeof document !== "undefined" ? document : null;
      var btn = doc ? doc.querySelector('#panelNav button[data-panel="workswarm"]') : null;
      if (btn) btn.click();
      var ws = win.OwoPanels && win.OwoPanels.workswarm;
      if (ws && typeof ws.open === "function" && teamId) {
        // open 内部有异步加载；给挂载一帧时间。
        setTimeout(function () {
          try {
            ws.open(teamId);
          } catch (e) {
            /* 面板打开失败不阻塞本面板 */
          }
        }, 60);
      }
    }

    // 404 识别（传输层把状态码嵌在 message 头部 "404: ..."）：仅 404 触发回退。
    function isNotFound(e) {
      return /^404/.test(String((e && e.message) || e || ""));
    }

    // 八期三路：Inbox 优先；九期修订——仅 404（未部署）回退客户端聚合，
    // 在线为空保持 Inbox 口径，其余失败展示错误行不切数据源。
    function load() {
      if (state.loading) return Promise.resolve();
      var lifecycle = lifecycleGeneration;
      var request = ++loadGeneration;
      inboxGeneration++;
      state.loading = true;
      state.errors = [];
      render();
      return H.get("/human/inbox")
        .then(function (d) {
          if (!isCurrentRequest(lifecycle, request)) return;
          state.inbox = inboxItemsOf(d);
          state.inboxSource = "inbox"; // 空列表也是 Inbox 的真实状态
          state.loading = false;
          state.loadedOnce = true;
          state.lastLoadedAt = new Date().toLocaleTimeString();
          render();
        })
        .catch(function (e) {
          if (!isCurrentRequest(lifecycle, request)) return;
          if (!isNotFound(e)) {
            // 非 404 失败：保持 Inbox 口径 + 错误行，不偷偷切换数据源。
            state.inboxSource = "inbox";
            state.errors.push("Inbox 加载失败（保持正式 Inbox 口径）：" + friendly(e));
            state.loading = false;
            state.loadedOnce = true;
            render();
            return;
          }
          // 404 = Inbox 尚未上线 → 回退七期客户端聚合。
          state.inboxSource = "legacy";
          return legacyLoad(lifecycle, request, null);
        });
    }

    // 七期客户端聚合路径（Inbox 明确 404 时的回退；原 load() 本体）。
    function legacyLoad(lifecycle, loadId, inboxId) {
      if (!isCurrentRequest(lifecycle, loadId, inboxId)) return Promise.resolve();
      state.inboxSource = "legacy";
      return H.get("/teams")
        .then(function (d) {
          if (!isCurrentRequest(lifecycle, loadId, inboxId)) return;
          state.teams = (d && d.teams) || [];
          var details = {};
          var detailErrors = {};
          return Promise.all(
            candidateTeams(state.teams).map(function (t) {
              return H.get("/teams/" + encodeURIComponent(t.team_id)).then(
                function (dd) {
                  details[String(t.team_id)] = dd || {};
                },
                function (e) {
                  detailErrors[String(t.team_id)] = friendly(e);
                }
              );
            })
          ).then(function () {
            if (!isCurrentRequest(lifecycle, loadId, inboxId)) return;
            state.details = details;
            state.detailErrors = detailErrors;
            var artifacts = {};
            return Promise.all(
              projectIdsOf(state.teams).map(function (p) {
                return H.get("/projects/" + encodeURIComponent(p.pid) + "/artifacts").then(
                  function (ad) {
                    artifacts[p.pid] = { items: (ad && ad.artifacts) || [], team_id: p.team_id, error: "" };
                  },
                  function (e) {
                    var msg = friendly(e);
                    var is404 = /^404/.test(String((e && e.message) || e || ""));
                    // 项目尚无产物（404）→ 静默视为空；其余错误记录展示。
                    artifacts[p.pid] = { items: [], team_id: p.team_id, error: is404 ? "" : msg };
                  }
                );
              })
            ).then(function () {
              if (!isCurrentRequest(lifecycle, loadId, inboxId)) return;
              state.artifacts = artifacts;
              finishLoad(lifecycle, loadId, inboxId);
            });
          });
        })
        .catch(function (e) {
          if (!isCurrentRequest(lifecycle, loadId, inboxId)) return;
          state.errors.push("团队列表加载失败：" + friendly(e));
          finishLoad(lifecycle, loadId, inboxId);
        });
    }

    function finishLoad(lifecycle, loadId, inboxId) {
      if (!isCurrentRequest(lifecycle, loadId, inboxId)) return;
      state.loading = false;
      state.loadedOnce = true;
      state.items = aggregate(state);
      state.lastLoadedAt = new Date().toLocaleTimeString();
      render();
    }

    function onSectionClick(ev) {
      var t = ev.target;
      if (!t || !t.closest) return;
      var inboxBtn = t.closest("[data-ic-act]");
      if (inboxBtn) {
        var itemId = inboxBtn.getAttribute("data-ic-item") || "";
        var act = inboxBtn.getAttribute("data-ic-act") || "";
        var extra = null;
        if (act !== "claim" && act !== "release") {
          // human_result：取同条目输入框中的结果文本；其余 kind 无额外字段。
          var noteInput = el('[data-ic-note="' + itemId.replace(/"/g, '\\"') + '"]');
          if (noteInput) { rememberInput({ target: noteInput }); extra = state.drafts[itemId]; }
        }
        startInboxAction(itemId, act, extra);
        return;
      }
      var retryBtn = t.closest("[data-ac-retry]");
      if (retryBtn) {
        submitRetry(retryBtn.getAttribute("data-team") || "", retryBtn.getAttribute("data-step") || "");
        return;
      }
      var goBtn = t.closest("[data-ac-goto]");
      if (goBtn) {
        gotoWorkswarm(goBtn.getAttribute("data-ac-goto") || "");
        return;
      }
      var artBtn = t.closest("[data-ac-goto-artifact]");
      if (artBtn) gotoWorkswarm(artBtn.getAttribute("data-team") || "");
    }

    function mount(root, helpers) {
      lifecycleGeneration++;
      loadGeneration++;
      inboxGeneration++;
      state.loading = false;
      rootEl = root;
      H = helpers || {};
      var scope = String(H.baseUrl || (win.OwoPanels && win.OwoPanels.baseUrl) || "");
      if (state.hostScope !== scope) { state.drafts = Object.create(null); state.inbox = []; state.inboxResults = {}; state.notice = ""; state.loadedOnce = false; state.inboxSource = "inbox"; state.inboxBusy = {}; }
      state.hostScope = scope;
      H.baseUrl = H.baseUrl || (win.OwoPanels && win.OwoPanels.baseUrl) || "";
      if (!H.get) H.get = defaultGet;
      if (!H.post) H.post = defaultPost;
      if (!H.esc) H.esc = defaultEsc;
      root.innerHTML = shellHtml();
      var box = el("#ac-sections");
      // 事件委托绑在持久容器 #ac-sections 上：innerHTML 重建子树不丢监听，
      // 重复挂载也不会累积（容器元素随 root.innerHTML 整体重建）。
      if (box) { box.addEventListener("click", onSectionClick); box.addEventListener("input", rememberInput); }
      var controls = el("#ac-controls");
      if (controls) {
        controls.innerHTML = filtersHtml();
        controls.addEventListener("click", function (ev) {
          var tab = ev.target.closest && ev.target.closest("[data-ac-status]");
          if (tab) { state.filters.status = tab.getAttribute("data-ac-status"); render(); }
        });
        controls.addEventListener("input", function (ev) {
          if (ev.target.id === "ac-search") { state.filters.q = ev.target.value; render(); }
        });
        controls.addEventListener("change", function (ev) {
          if (ev.target.id === "ac-kind") { state.filters.kind = ev.target.value; render(); }
        });
      }
      var refresh = el("#ac-refresh");
      if (refresh) refresh.onclick = function () { load(); };
      load();
    }

    function dispose() {
      lifecycleGeneration++;
      loadGeneration++;
      inboxGeneration++;
      rootEl = null;
      state.loading = false;
      // In-flight actions continue in the service after unmount; keep their locks until settled.
    }

    // ---------- 测试挂钩 ----------
    var TEST_API = {
      state: state,
      normStatus: normStatus,
      isRetryableStep: isRetryableStep,
      candidateTeams: candidateTeams,
      projectIdsOf: projectIdsOf,
      humanTasksOf: humanTasksOf,
      humanItemsOf: humanItemsOf,
      failedItemsOf: failedItemsOf,
      leaseOf: leaseOf,
      leaseItemsOf: leaseItemsOf,
      normReviewState: normReviewState,
      reviewItemsOf: reviewItemsOf,
      aggregate: aggregate,
      buildRetryBody: buildRetryBody,
      itemHtml: itemHtml,
      sectionsHtml: sectionsHtml,
      shellHtml: shellHtml,
      render: render,
      load: load,
      submitRetry: submitRetry,
      gotoWorkswarm: gotoWorkswarm,
      // 八期三路：Inbox 优先 + 回退口径
      filteredInbox: filteredInbox,
      rememberInput: rememberInput,
      itemBusy: itemBusy,
      normItemStatus: normItemStatus,
      inboxItemsOf: inboxItemsOf,
      groupInbox: groupInbox,
      buildInboxActionPath: buildInboxActionPath,
      buildResolveBody: buildResolveBody,
      inboxSectionsHtml: inboxSectionsHtml,
      inboxItemHtml: inboxItemHtml,
      startInboxAction: startInboxAction,
      refreshInbox: refreshInbox,
      dispose: dispose,
      getTransport: function () {
        return { get: H.get, post: H.post };
      },
      setTransport: function (t) {
        if (t && t.get) H.get = t.get;
        if (t && t.post) H.post = t.post;
      },
      setRoot: function (r) {
        rootEl = r;
      },
    };

    return {
      id: ID,
      title: "待我处理",
      mount: mount,
      dispose: dispose,
      // 九期：公开刷新入口——WorkSwarm 面板 ChangeSet accept/reject 后跨面板刷新
      // 待办（未挂载时安全跳过）。
      refreshInbox: function () { return refreshInbox(); },
      _test: TEST_API,
    };
  })();
})();

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效、零运行时差异。
if (typeof module !== "undefined" && module.exports) {
  var __acWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __acWin.OwoPanels["action-center"];
}
