/* 统一 Task API 客户端（Agent V2 Phase 5）。
 * 面板不允许各自拼 /agent-tasks 路径或各自解释状态：所有任务读写都经过这里，
 * 响应形状与错误在这里收口。依赖注入（get/post）便于 Node 契约测试与 Tauri 复用。
 */
(function (root) {
  "use strict";

  function TaskApiError(message, code, status) {
    const error = new Error(message);
    error.name = "TaskApiError";
    error.code = code || "";
    error.status = status || 0;
    return error;
  }

  function taskOf(body) {
    if (!body || typeof body !== "object" || !body.task || typeof body.task !== "object") {
      throw TaskApiError("任务响应不完整", "INCOMPLETE_RESPONSE");
    }
    if (typeof body.task.task_id !== "string" || !body.task.task_id) {
      throw TaskApiError("任务响应缺少 task_id", "INCOMPLETE_RESPONSE");
    }
    return body.task;
  }

  function eventsOf(body) {
    if (!body || typeof body !== "object" || !Array.isArray(body.events)) {
      throw TaskApiError("事件响应不完整", "INCOMPLETE_RESPONSE");
    }
    return body.events.filter((event) => event && typeof event.sequence === "number");
  }

  function query(params) {
    const parts = [];
    for (const key of Object.keys(params || {})) {
      const value = params[key];
      if (value == null || value === "") continue;
      parts.push(encodeURIComponent(key) + "=" + encodeURIComponent(String(value)));
    }
    return parts.length ? "?" + parts.join("&") : "";
  }

  function taskPath(id, suffix) {
    if (typeof id !== "string" || !id) throw TaskApiError("缺少任务 ID", "INVALID_REQUEST");
    return "/agent-tasks/" + encodeURIComponent(id) + (suffix || "");
  }

  /**
   * @param {{get: Function, post: Function}} transport
   */
  function createTaskApiClient(transport) {
    const t = transport || {};
    if (typeof t.get !== "function" || typeof t.post !== "function") {
      throw TaskApiError("TaskApiClient 需要 get/post", "INVALID_REQUEST");
    }
    return {
      list(params) {
        return t.get("/agent-tasks" + query(params)).then((body) => {
          if (!body || !Array.isArray(body.tasks)) {
            throw TaskApiError("任务列表响应不完整", "INCOMPLETE_RESPONSE");
          }
          return body;
        });
      },
      get(id) {
        return t.get(taskPath(id)).then((body) => {
          const task = taskOf(body);
          return {
            task,
            approvals: Array.isArray(body.approvals) ? body.approvals : [],
            latest_checkpoint: body.latest_checkpoint || null,
            artifacts: Array.isArray(body.artifacts) ? body.artifacts : [],
          };
        });
      },
      events(id, options) {
        const opts = options || {};
        return t
          .get(taskPath(id, "/events" + query({ after: opts.after, limit: opts.limit })))
          .then((body) => ({ events: eventsOf(body) }));
      },
      start(id) {
        return t.post(taskPath(id, "/start"), {}).then((body) => body || {});
      },
      pause(id, reason) {
        return t.post(taskPath(id, "/pause"), { reason: reason || "用户暂停" }).then((body) => body || {});
      },
      resume(id) {
        return t.post(taskPath(id, "/resume"), {}).then((body) => body || {});
      },
      cancel(id, reason) {
        return t.post(taskPath(id, "/cancel"), { reason: reason || "用户取消" }).then((body) => body || {});
      },
      verify(id, evidence) {
        return t.post(taskPath(id, "/verify"), evidence || {}).then((body) => body || {});
      },
      approve(id, approvalId, decision) {
        if (typeof approvalId !== "string" || !approvalId) {
          throw TaskApiError("缺少审批 ID", "INVALID_REQUEST");
        }
        return t
          .post(taskPath(id, "/approvals/" + encodeURIComponent(approvalId)), {
            decision: decision,
            decided_by: "desktop",
          })
          .then((body) => body || {});
      },
      create(body) {
        return t.post("/agent-tasks", body || {}).then(taskOf);
      },
    };
  }

  const api = { createTaskApiClient };
  root.OwoTaskApi = Object.freeze(api);
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
