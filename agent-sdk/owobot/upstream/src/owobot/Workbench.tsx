import { GroupView } from "@/components/GroupView";
import { DesktopCapabilitiesProvider } from "@/components/DesktopCapabilities";
import { ThreadRefsProvider } from "@/components/ThreadRefs";
import { localizedTaskTitle } from "@/lib/task-title";
import { StoreProvider, openRoomThread, useStore } from "@/state/store";
import "./workbench.css";

function ProjectRoom({ groupId }: { groupId: string }) {
  const { state, dispatch } = useStore();
  const group = state.groups.find((candidate) => candidate.id === groupId);
  if (!group) {
    return <p className="owobot-connection" role="status">
      {state.connected ? "正在读取项目会话…" : "正在连接 OwOBot…"}
    </p>;
  }

  const tasks = (group.tasks ?? [])
    .filter((task) => !task.archivedAt)
    .sort((a, b) => (b.lastThreadOrderAt ?? b.createdAt) - (a.lastThreadOrderAt ?? a.createdAt));
  return <div className="owobot-workbench">
    <aside className="owobot-task-list">
      <button type="button" className="owobot-new-task" onClick={() => dispatch({ type: "newGroupTask", groupId })}>
        新建协作任务
      </button>
      <nav aria-label="协作任务">
        <ul>
          {tasks.map((task) => <li key={task.threadId}>
            <button type="button" aria-current={task.threadId === group.threadId ? "page" : undefined}
              onClick={() => openRoomThread(dispatch, { groupId, threadId: task.threadId }, state.groups)}>
              <span>{task.title === "New task" ? "新建任务" : localizedTaskTitle(task.title)}</span>
              {task.working && <small>执行中</small>}
            </button>
          </li>)}
        </ul>
      </nav>
    </aside>
    <div className="owobot-transcript">
      {!state.connected && <p className="owobot-connection" role="status">正在恢复连接…</p>}
      <GroupView group={group} />
    </div>
  </div>;
}

export function OwOBotWorkbench() {
  const groupId = new URLSearchParams(location.search).get("group");
  if (!groupId) return <p className="owobot-connection">请从 OwO 桌面工作台打开 OwOBot。</p>;
  return <DesktopCapabilitiesProvider>
    <StoreProvider>
      <ThreadRefsProvider><ProjectRoom groupId={groupId} /></ThreadRefsProvider>
    </StoreProvider>
  </DesktopCapabilitiesProvider>;
}
