/* 权限中心 · 编排层（§4.5 / §4.8）。
 *
 * 这个文件只回答一个问题：**"用户在权限中心做一件事，接下来该发生什么"**——
 * 按需加载、三态（loading/error/empty）、提交前校验、完全访问双确认 + 风险告知 +
 * 时长选择、撤销后的自动复查。它决定顺序与时机，不碰 DOM（渲染走注入的 render），
 * 不拼 URL（路径在 api 层），不做业务判断（判定在 domain 层）。
 *
 * 生命周期红线：
 *   - **进入面板才请求**（§8.2 首屏 ≤5 请求口径，权限数据绝不进首屏链路）；
 *   - 离开面板 `dispose()` 清掉全部挂起标记与延时 timer；
 *   - **禁止 setInterval**：本页不做任何周期轮询，刷新只有"手动"与"动作后复查"两种来源。
 */
(function (global) {
  "use strict";

  const RECHECK_DELAY_MS = 120;
  const PROFILE_LABEL_FALLBACK = "未知档位";

  function isPlainObject(value) {
    return Boolean(value) && typeof value === "object" && !Array.isArray(value);
  }

  function text(value) {
    return value == null ? "" : String(value);
  }

  function defaultNotify() {
    /* 缺省静默：宿主未注入 notify 时不得抛错打断主流程 */
  }

  /**
   * @param {{api:object, render:function, notify?:function, domain?:object, setTimeout?:function, clearTimeout?:function}} deps
   *   api 由 OwoPermissionsApi.create(fetchJson) 产出；render(state) 纯渲染。
   */
  function create(deps) {
    const options = isPlainObject(deps) ? deps : {};
    const api = options.api;
    if (!api || typeof api.overview !== "function") throw new Error("权限中心控制器需要 api（OwoPermissionsApi.create 的产物）");
    const render = typeof options.render === "function" ? options.render : function () {};
    const notify = typeof options.notify === "function" ? options.notify : defaultNotify;
    const domain = options.domain || global.OwoPermissionsDomain;
    const later = typeof options.setTimeout === "function" ? options.setTimeout : (fn, ms) => global.setTimeout(fn, ms);
    const cancelLater = typeof options.clearTimeout === "function" ? options.clearTimeout : (id) => global.clearTimeout(id);

    const state = {
      phase: "idle", // idle | loading | ready | error
      overview: null,
      dimensions: [],
      spec: null,
      draft: null,
      baselineDraft: null,
      pending: [],
      grants: [],
      recentDecisions: [],
      fullAccess: null,
      profile: "",
      readOnly: false,
      errors: [],
      notice: "",
      busy: false,
      confirming: null, // 完全访问待确认载荷（含风险清单与时长）
      verify: "", // 撤销后复查提示
      goneNotices: [],
      updatedAt: "",
      lastError: null,
    };

    const timers = new Set();
    let disposed = false;
    let inFlight = null;
    let loadToken = 0;

    function schedule(fn, ms) {
      const id = later(() => {
        timers.delete(id);
        if (!disposed) fn();
      }, ms);
      timers.add(id);
      return id;
    }

    function clearTimers() {
      for (const id of Array.from(timers)) cancelLater(id);
      timers.clear();
    }

    function paint() {
      if (disposed) return;
      render(snapshot());
    }

    function blockMutation() {
      let reason = "";
      if (disposed) reason = "权限页面已关闭，请重新打开后操作。";
      else if (state.phase !== "ready") reason = "权限数据尚未成功读取，暂不能修改。";
      else if (state.readOnly) reason = "核心当前处于只读模式，不能提交权限变更。";
      else if (state.busy) reason = "正在处理上一项权限操作，请稍后。";
      if (!reason) return null;
      state.notice = reason;
      paint();
      return { ok: false, blocked: true, reason: reason };
    }

    function snapshot() {
      return {
        phase: state.phase,
        profile: state.profile || PROFILE_LABEL_FALLBACK,
        profileLabel: domain ? domain.profileLabel(state.profile) : state.profile,
        readOnly: state.readOnly,
        dimensions: state.dimensions.slice(),
        spec: state.spec,
        draft: state.draft ? JSON.parse(JSON.stringify(state.draft)) : null,
        baselineDraft: state.baselineDraft,
        changedFields: domain ? domain.changesFromDraft(state.draft, state.baselineDraft) : [],
        pending: state.pending.slice(),
        grants: state.grants.slice(),
        recentDecisions: state.recentDecisions.slice(),
        fullAccess: state.fullAccess,
        errors: state.errors.slice(),
        notice: state.notice,
        busy: state.busy,
        confirming: state.confirming,
        verify: state.verify,
        goneNotices: state.goneNotices.slice(),
        updatedAt: state.updatedAt,
        lastError: state.lastError,
        empty: isEmptyState(),
      };
    }

    /** 空态判据：三个列表全空且没有任何**权威**维度时才叫空（0 ≠ 未知）。 */
    function isEmptyState() {
      return (
        state.phase === "ready" &&
        !state.pending.length &&
        !state.grants.length &&
        !state.recentDecisions.length &&
        // 不能用 `!dimensions.length`：domain 为了骨架稳定会把四维补成占位行，
        // 补出来的行不是事实，把它当"有数据"就等于永远不空、且界面像有配置。
        !state.dimensions.some((row) => row && !row.synthesized)
      );
    }

    function absorb(json) {
      const overview = isPlainObject(json) ? json : {};
      state.overview = overview;
      state.profile = text(overview.profile);
      state.readOnly = overview.read_only === true;
      state.dimensions = domain ? domain.dimensionsFromOverview(overview) : (Array.isArray(overview.dimensions) ? overview.dimensions : []);
      state.spec = domain ? domain.specFromOverview(overview) : (isPlainObject(overview.spec) ? overview.spec : null);
      state.draft = state.spec ? JSON.parse(JSON.stringify(state.spec)) : defaultDraft();
      state.baselineDraft = JSON.parse(JSON.stringify(state.draft));
      state.pending = domain ? domain.listField(overview, "pending") : (Array.isArray(overview.pending) ? overview.pending : []);
      state.grants = domain ? domain.listField(overview, "grants") : (Array.isArray(overview.grants) ? overview.grants : []);
      state.recentDecisions = domain ? domain.listField(overview, "recent_decisions") : [];
      state.fullAccess = isPlainObject(overview.full_access) ? overview.full_access : null;
      state.updatedAt = new Date().toISOString();
      reportToStatusBar();
    }

    function defaultDraft() {
      return domain ? domain.draftFromOverview(state.overview) : { filesystem: "", command: "", network: "", persistence: "", scopes: [] };
    }

    /** §4.3 状态条权限段回灌（零额外请求；函数不存在时安全跳过）。 */
    function reportToStatusBar() {
      const bar = global.OwoStatusBar;
      if (!bar || typeof bar.reportPermission !== "function") return;
      try {
        bar.reportPermission({
          profile: state.profile ? (domain ? domain.profileLabel(state.profile) : state.profile) : null,
          pendingApprovals: state.pending.length,
          grants: state.grants.length,
        });
      } catch (error) {
        /* 状态条异常不得影响权限页本身 */
      }
    }

    function fail(error, fallbackMessage, codeOverride) {
      const info = domain ? domain.normalizeError(error) : { code: "", message: text((error && error.message) || error) || fallbackMessage, status: 0 };
      const code = text(codeOverride) || info.code;
      state.phase = "error";
      state.busy = false;
      state.lastError = { code: code, message: info.message || fallbackMessage, status: info.status || 0 };
      state.errors = [info.message || fallbackMessage];
      if (code) state.errors.push("稳定错误码：" + code);
    }

    /**
     * 按需加载：进入面板调用一次即可（同代际合并为一次请求）。
     * HTTP 200 但结构不可用（无 profile / 无 dimensions）同样按错误处理——不假设 200 一定有数据。
     */
    function load(options) {
      const force = Boolean(options && options.force);
      if (!force && state.phase === "ready") return Promise.resolve(snapshot());
      if (!force && inFlight) return inFlight;
      const token = ++loadToken;
      state.phase = "loading";
      state.errors = [];
      state.notice = "";
      state.verify = "";
      paint();
      const started = Promise.resolve()
        .then(() => api.overview())
        .then(
          (json) => {
            if (inFlight === started) inFlight = null;
            // 陈旧响应不得覆盖新状态（dispose / 更新的 reload 之后一律丢弃）。
            if (disposed || token !== loadToken) return disposed ? null : snapshot();
            if (domain && !domain.overviewIsUsable(json)) {
              // 两种"不可用"要分开说：服务端在 200 里主动报失败（{"ok":false,"error":{…}}）
              // 时，稳定错误码与原消息都必须照面（§3.4）；真的什么都没返回时，
              // 才由我们自己说"缺少档位与维度矩阵"——不把猜测包装成服务端结论。
              const envelope = isPlainObject(json) ? json : {};
              const info = domain.normalizeError(envelope);
              const reported = Boolean(info.code);
              fail(
                new Error(reported ? info.message || "服务端报回失败" : "概览响应缺少档位与维度矩阵"),
                "权限概览返回了空数据（HTTP 200 但没有可用字段）",
                info.code,
              );
              paint();
              return snapshot();
            }
            absorb(json);
            state.phase = "ready";
            state.busy = false;
            state.lastError = null;
            paint();
            return snapshot();
          },
          (error) => {
            if (inFlight === started) inFlight = null;
            if (disposed || token !== loadToken) return disposed ? null : snapshot();
            fail(error, "无法读取权限概览");
            paint();
            return snapshot();
          },
        );
      inFlight = started;
      return started;
    }

    function reload() {
      return load({ force: true });
    }

    /** 编辑草稿中的某个维度（不落库，仅本地态；提交仍要过 validateSpec）。 */
    function setDimension(key, value) {
      const blocked = blockMutation();
      if (blocked) return blocked;
      if (!state.draft) state.draft = defaultDraft();
      state.draft[key] = value;
      state.errors = [];
      state.notice = "";
      paint();
    }

    function setScopes(scopes) {
      const blocked = blockMutation();
      if (blocked) return blocked;
      if (!state.draft) state.draft = defaultDraft();
      state.draft.scopes = (Array.isArray(scopes) ? scopes : []).map(text).filter(Boolean);
      paint();
    }

    /** 一步关闭完全访问：把不受限维度收回白名单档，不需要确认要素。 */
    function closeFullAccess() {
      const blocked = blockMutation();
      if (blocked) return Promise.resolve(blocked);
      const target = state.draft ? JSON.parse(JSON.stringify(state.draft)) : defaultDraft();
      if (text(target.command) === "unrestricted") target.command = "allowlisted";
      if (text(target.network) === "unrestricted") target.network = "allowlisted";
      state.draft = target;
      state.confirming = null;
      return submitDraft();
    }

    /** 打开完全访问二次确认：先给出风险清单 + 时长选择，未确认前不发请求。 */
    function requestFullAccess(spec) {
      const blocked = blockMutation();
      if (blocked) return null;
      const candidate = isPlainObject(spec) ? spec : state.draft;
      if (!candidate || !domain || !domain.needsFullAccessConfirm(candidate)) {
        // 不能静默 return：按钮就摆在那里，点了什么都不发生等于一个死控件
        // （实测真机第一轮就是这样把"确认卡未出现"读成断言失败的）。
        // 这里明说"当前草稿不含不受限维度"，并保留按钮的下一次可点状态。
        state.notice = "当前配置不含不受限的命令或网络维度，无需二次确认；把某一维改为「不受限」后再申请。";
        state.confirming = null;
        paint();
        return null;
      }
      const risks = state.fullAccess && Array.isArray(state.fullAccess.risk_notes) ? state.fullAccess.risk_notes.slice() : [];
      state.confirming = {
        spec: JSON.parse(JSON.stringify(candidate)),
        riskNotes: risks,
        durationSecs: domain.defaultDurationSecs(),
        durationOptions: domain.durationOptions(),
        requiresConfirm: !state.fullAccess || state.fullAccess.requires_confirm !== false,
      };
      state.errors = [];
      // 上一次"无需确认"的解释性提示不能留在真正开出来的卡上面。
      state.notice = "";
      paint();
      return state.confirming;
    }

    function chooseDuration(secs) {
      if (!state.confirming) return;
      const value = Number(secs);
      state.confirming.durationSecs = Number.isFinite(value) && value > 0 ? value : domain.defaultDurationSecs();
      paint();
    }

    function cancelConfirm() {
      state.confirming = null;
      state.notice = "";
      paint();
    }

    /**
     * 提交配置（唯一出口）。三道闸：
     *   ① validateSpec 非空 → 阻止提交并逐条展示中文原因；
     *   ② 需要完全访问确认但未确认 → 先渲染确认卡，不发请求；
     *   ③ 通过后才发请求（confirm/duration_secs 由 api 层补齐）。
     */
    function submitDraft() {
      const blocked = blockMutation();
      if (blocked) return Promise.resolve(blocked);
      const spec = state.draft;
      const problems = domain ? domain.validateSpec(spec) : [];
      if (problems.length) {
        state.errors = problems;
        state.notice = "";
        paint();
        notify(problems[0]);
        return Promise.resolve({ ok: false, validation: problems });
      }
      if (domain && domain.needsFullAccessConfirm(spec) && !state.confirming) {
        requestFullAccess(spec);
        return Promise.resolve({ ok: false, needsConfirmation: true });
      }
      const confirming = state.confirming;
      const options = confirming
        ? { confirm: true, durationSecs: confirming.durationSecs }
        : {};
      state.busy = true;
      state.errors = [];
      state.notice = "正在提交权限配置…";
      paint();
      return Promise.resolve()
        .then(() => api.submitSpec(spec, options))
        .then((result) => {
          state.busy = false;
          state.confirming = null;
          if (result && result.ok === false) {
            const info = domain ? domain.normalizeError(result.error || result) : { code: "", message: "提交失败" };
            state.errors = [info.message || "提交失败"];
            if (info.code) state.errors.push("稳定错误码：" + info.code);
            state.notice = "";
            paint();
            notify(info.code ? info.message + "（" + info.code + "）" : info.message);
            return result;
          }
          state.notice = result && result.denials_added
            ? "已保存权限配置：新增 " + result.denials_added + " 条拒绝规则（维度只能收紧）。"
            : "已保存权限配置。";
          return reload().then(() => snapshot());
        })
        .catch((error) => {
          state.busy = false;
          state.confirming = null;
          const info = domain ? domain.normalizeError(error) : { code: "", message: text(error && error.message) || "提交失败" };
          state.errors = [info.message || "提交失败"];
          if (info.code) state.errors.push("稳定错误码：" + info.code);
          state.notice = "";
          paint();
          notify(info.code ? info.message + "（" + info.code + "）" : info.message);
          return { ok: false };
        });
    }

    /**
     * 审批四动作（拒绝 / 仅本次 / 本任务 / 工作区长期）。
     * 条目已消失（404/gone）是**正常终态**：清掉本地条目 + 给一条提示，不进错误态。
     */
    function respondApproval(item, action) {
      const blocked = blockMutation();
      if (blocked) return Promise.resolve(blocked);
      const actions = domain ? domain.APPROVAL_ACTIONS : [];
      const decision = actions.find((row) => row.action === action);
      if (!decision) return Promise.resolve({ ok: false, error: { message: "未知的审批动作" } });
      if (!isPlainObject(item) || !item.request_id) return Promise.resolve({ ok: false, error: { message: "审批条目缺少标识" } });
      const mayRemember = item.destructive === false && item.level === "read";
      if (decision.allow && decision.scope && decision.scope !== "once" && !mayRemember) {
        return Promise.resolve({ ok: false, error: { message: "该操作只支持本次审批，不允许保存为授权。" } });
      }
      state.busy = true;
      state.notice = "正在提交审批结果…";
      paint();
      return Promise.resolve()
        .then(() => api.respondApproval(item.session_id, item.request_id, { allow: decision.allow, scope: decision.scope }))
        .then(
          (result) => {
            state.busy = false;
            if (result && result.ok === false) {
              const info = domain ? domain.normalizeError(result.error || result) : { code: "", message: "审批失败" };
              if (domain && domain.isPendingGone(info.code)) return markGone(item, info.code);
              state.errors = [info.message || "审批失败"];
              if (info.code) state.errors.push("稳定错误码：" + info.code);
              state.notice = "";
              paint();
              return result;
            }
            dropPending(item);
            state.notice = decision.allow ? "已" + decision.label + "允许该操作。" : "已拒绝该操作。";
            paint();
            notify(state.notice);
            return reload().then(() => snapshot());
          },
          (error) => {
            state.busy = false;
            if (domain && domain.isErrorGone(error)) return markGone(item, "gone");
            fail(error, "审批失败");
            paint();
            return { ok: false };
          },
        );
    }

    function dropPending(item) {
      state.pending = state.pending.filter((row) => row !== item && text(row.request_id) !== text(item.request_id));
    }

    function markGone(item, code) {
      dropPending(item);
      state.goneNotices = ["该请求已由其他窗口处理或已失效" + (code ? "（" + code + "）" : "") + "，已从待审批列表移除。"].concat(state.goneNotices).slice(0, 3);
      state.notice = "待审批列表已更新。";
      paint();
      return { ok: true, gone: true };
    }

    /**
     * 撤销授权 + **撤销后立即复查**。
     * 载荷先经 revokePayload 归一（三种粒度之一）；`revoked:0` 与"条目已不在列表"
     * 都是正常终态（幂等撤销），不得渲染成错误。
     */
    function revoke(input) {
      const blocked = blockMutation();
      if (blocked) return Promise.resolve(blocked);
      const normalized = domain ? domain.revokePayload(input) : { ok: true, payload: input };
      if (!normalized.ok) {
        state.errors = [normalized.error];
        paint();
        notify(normalized.error);
        return Promise.resolve({ ok: false, validation: [normalized.error] });
      }
      state.busy = true;
      state.errors = [];
      state.verify = "撤销后自动复查…";
      state.notice = "";
      paint();
      return Promise.resolve()
        .then(() => api.revoke(normalized.payload))
        .then(
          (result) => {
            if (result && result.ok === false) {
              const info = domain ? domain.normalizeError(result.error || result) : { code: "", message: "撤销失败" };
              state.busy = false;
              state.verify = "";
              if (domain && domain.isPendingGone(info.code)) {
                state.goneNotices = ["该授权此前已被撤销（" + info.code + "），列表即将同步。"].concat(state.goneNotices).slice(0, 3);
                // gone 分支同样走一次性复查 timer（保持"撤销后必复查"的口径一致）。
                return new Promise((resolve) => {
                  schedule(() => {
                    reload().then(() => resolve({ ok: true, gone: true })).catch(() => resolve({ ok: true, gone: true }));
                  }, RECHECK_DELAY_MS);
                });
              }
              state.errors = [info.message || "撤销失败"];
              if (info.code) state.errors.push("稳定错误码：" + info.code);
              paint();
              return result;
            }
            const revoked = Number((result && result.revoked) || 0);
            // 复查：重新拉 overview 并断言目标条目确实消失（延迟一拍，避开服务端事务未落盘）。
            return new Promise((resolve) => {
              schedule(() => {
                reload().then(() => resolve(finishRevoke(revoked, normalized))).catch(() => resolve(finishRevoke(revoked, normalized)));
              }, RECHECK_DELAY_MS);
            });
          },
          (error) => {
            state.busy = false;
            state.verify = "";
            if (domain && domain.isErrorGone(error)) {
              state.goneNotices = ["该授权此前已被撤销，列表即将同步。"].concat(state.goneNotices).slice(0, 3);
              return new Promise((resolve) => {
                schedule(() => {
                  reload().then(() => resolve({ ok: true, gone: true })).catch(() => resolve({ ok: true, gone: true }));
                }, RECHECK_DELAY_MS);
              });
            }
            fail(error, "撤销失败");
            paint();
            return { ok: false };
          },
        );
    }

    function finishRevoke(revoked, normalized) {
      state.busy = false;
      state.verify = "";
      const stillThere = normalized && normalized.payload && domain && domain.isGrantGone(state.grants, normalized.payload) === false;
      if (stillThere) {
        state.errors = ["复查发现该授权仍然存在：撤销可能未生效，请重试或刷新页面。"];
        state.notice = "";
      } else if (revoked > 0) {
        state.notice = "已撤销 " + revoked + " 条授权，复查确认列表中已不再出现。";
      } else {
        state.notice = "该范围内没有可撤销的授权（可能已被撤销），列表已同步。";
      }
      paint();
      if (!stillThere) notify(state.notice);
      return { ok: !stillThere, revoked: revoked };
    }

    /** 离开面板：清 timer、丢弃挂起的确认卡，后续回调一律不再渲染。 */
    function dispose() {
      disposed = true;
      clearTimers();
      state.confirming = null;
      state.busy = false;
      inFlight = null;
    }

    return {
      state: state,
      snapshot: snapshot,
      load: load,
      reload: reload,
      setDimension: setDimension,
      setScopes: setScopes,
      submitDraft: submitDraft,
      requestFullAccess: requestFullAccess,
      chooseDuration: chooseDuration,
      cancelConfirm: cancelConfirm,
      closeFullAccess: closeFullAccess,
      respondApproval: respondApproval,
      revoke: revoke,
      dispose: dispose,
      reportToStatusBar: reportToStatusBar,
      isEmpty: isEmptyState,
    };
  }

  global.OwoPermissionsController = {
    RECHECK_DELAY_MS: RECHECK_DELAY_MS,
    create: create,
  };
})(typeof window !== "undefined" ? window : globalThis);

// Node（CommonJS）测试环境导出；浏览器无 module 定义，此分支不生效。
if (typeof module !== "undefined" && module.exports) {
  module.exports = (typeof window !== "undefined" ? window : globalThis).OwoPermissionsController;
}
