(function (root) {
  "use strict";
  const taskIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  function valid(ref) {
    return ref && Object.prototype.hasOwnProperty.call(ref, "taskId") &&
      (ref.taskId === null || (typeof ref.taskId === "string" && taskIdPattern.test(ref.taskId)));
  }
  function fromSearch(search) {
    const query = new URLSearchParams(search || "");
    if (query.getAll("panel").length !== 1 || query.get("panel") !== "agent-tasks" ||
        query.getAll("task").length > 1) return null;
    const ref = { taskId: query.has("task") ? query.get("task") : null };
    return valid(ref) ? ref : null;
  }
  function create({ isReady, openPanel, openTask, onError = () => {} }) {
    let pending = null;
    async function flush() {
      if (!pending || !isReady()) return false;
      const ref = pending;
      pending = null;
      try {
        if (openPanel("agent-tasks") === false) throw new Error("任务面板尚未就绪");
        if (ref.taskId !== null) await openTask(ref.taskId);
        return true;
      } catch (error) { onError(error); return false; }
    }
    function receive(ref) {
      if (!valid(ref)) return Promise.resolve(false);
      pending = { taskId: ref.taskId };
      return flush();
    }
    return { receive, flush };
  }
  const api = Object.freeze({ create, fromSearch, valid });
  root.OwoImeTasks = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
