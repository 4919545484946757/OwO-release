/* 执行工作区客户端（Agent V2 Phase 6）。
 *
 * 工作区租约、Diff 与「应用/拒绝/回滚/释放」全部经这里访问 `/workspaces`：
 * 面板不拼路径、不自己判断租约状态。只有服务端状态机允许的动作才会成功；
 * 前端按钮只负责把意图发出去，失败原因原样返回。
 *
 * 依赖注入（get/post/del）便于 Node 契约测试与不同传输复用。
 */
(function (root) {
  "use strict";

  function WorkspaceApiError(message, code, status) {
    const error = new Error(message);
    error.name = "WorkspaceApiError";
    error.code = code || "";
    error.status = status || 0;
    return error;
  }

  function leaseOf(body) {
    const lease = body && (body.workspace || body.lease);
    if (!lease || typeof lease !== "object") {
      throw WorkspaceApiError("工作区响应不完整", "INCOMPLETE_RESPONSE");
    }
    if (typeof lease.workspace_id !== "string" || !lease.workspace_id) {
      throw WorkspaceApiError("工作区响应缺少 workspace_id", "INCOMPLETE_RESPONSE");
    }
    if (typeof lease.path !== "string" || !lease.path) {
      throw WorkspaceApiError("工作区响应缺少 path", "INCOMPLETE_RESPONSE");
    }
    return lease;
  }

  function leasePath(id, suffix) {
    if (typeof id !== "string" || !id) {
      throw WorkspaceApiError("缺少工作区 ID", "INVALID_REQUEST");
    }
    return "/workspaces/" + encodeURIComponent(id) + (suffix || "");
  }

  function diffOf(body) {
    if (!body || typeof body !== "object") {
      throw WorkspaceApiError("Diff 响应不完整", "INCOMPLETE_RESPONSE");
    }
    const diff = body.diff || body;
    if (!diff || typeof diff !== "object" || typeof diff.patch !== "string") {
      throw WorkspaceApiError("Diff 响应缺少 patch", "INCOMPLETE_RESPONSE");
    }
    return {
      diff,
      empty: typeof body.empty === "boolean" ? body.empty : diff.patch.length === 0,
    };
  }

  /**
   * @param {{get: Function, post: Function, del: Function}} transport
   */
  function createWorkspaceApiClient(transport) {
    const t = transport || {};
    const hasGet = typeof t.get === "function";
    const hasPost = typeof t.post === "function";
    const hasDel = typeof t.del === "function" || typeof t.delete === "function";
    if (!hasGet || !hasPost || !hasDel) {
      throw WorkspaceApiError("WorkspaceApiClient 需要 get/post/del", "INVALID_REQUEST");
    }
    const del = typeof t.del === "function" ? t.del : t.delete;

    return {
      list() {
        return t.get("/workspaces").then((body) => {
          if (!body || !Array.isArray(body.workspaces)) {
            throw WorkspaceApiError("工作区列表响应不完整", "INCOMPLETE_RESPONSE");
          }
          return { workspaces: body.workspaces.filter((lease) => lease && lease.workspace_id) };
        });
      },
      get(id) {
        return t.get(leasePath(id)).then(leaseOf);
      },
      create(kind, taskId) {
        const body = { kind: kind || "git_worktree" };
        if (taskId) body.task_id = taskId;
        return t.post("/workspaces", body).then(leaseOf);
      },
      diff(id) {
        return t.get(leasePath(id, "/diff")).then(diffOf);
      },
      apply(id) {
        return t.post(leasePath(id, "/apply"), {}).then((body) => {
          if (!body || body.ok === false) {
            throw WorkspaceApiError((body && body.error) || "应用改动失败", body && body.code);
          }
          return body;
        });
      },
      reject(id) {
        return t.post(leasePath(id, "/reject"), {}).then(leaseOf);
      },
      rollback(id) {
        return t.post(leasePath(id, "/rollback"), {}).then(leaseOf);
      },
      release(id) {
        return del(leasePath(id)).then(leaseOf);
      },
    };
  }

  const api = { createWorkspaceApiClient };
  root.OwoWorkspaceApi = Object.freeze(api);
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
