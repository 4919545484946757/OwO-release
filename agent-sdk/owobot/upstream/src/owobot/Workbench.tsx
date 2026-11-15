import { useEffect, useRef } from "react";
import { GroupView } from "@/components/GroupView";
import { DesktopCapabilitiesProvider } from "@/components/DesktopCapabilities";
import { ThreadRefsProvider } from "@/components/ThreadRefs";
import { localizedTaskTitle } from "@/lib/task-title";
import { StoreProvider, openRoomThread, useStore } from "@/state/store";
import { DeliveryPanel } from "./DeliveryPanel";
import "./workbench.css";

function ProjectRoom({ groupId, requestedThread }: { groupId: string; requestedThread: string | null }) {
  const { state, dispatch } = useStore();
  const group = state.groups.find((candidate) => candidate.id === groupId);
  const requestedSwitch = useRef(false);
  const reachedRequestedTask = useRef(false);
  if (requestedThread && group?.threadId === requestedThread) reachedRequestedTask.current = true;
  const openingRequestedTask = requestedThread && !reachedRequestedTask.current;
  const hasRequestedThread = group?.tasks?.some((task) => task.threadId === requestedThread);
  useEffect(() => {
    if (!requestedThread || !openingRequestedTask || !hasRequestedThread || requestedSwitch.current) return;
    requestedSwitch.current = true;
    dispatch({ type: "switchGroupTask", groupId, threadId: requestedThread });
  }, [dispatch, groupId, hasRequestedThread, openingRequestedTask, requestedThread]);
  if (!group || openingRequestedTask) {
    return <p className="owobot-connection" role="status">
      {!state.connected ? "正在连接 OwOBot…"
        : openingRequestedTask && group && !hasRequestedThread ? "无法找到这个项目任务。"
          : "正在读取项目会话…"}
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
    <DeliveryPanel key={group.threadId} thread={group.threadId} working={group.working === true} message={group.messages.at(-1)?.id} />
  </div>;
}

export function OwOBotWorkbench() {
  const params = new URLSearchParams(location.search);
  const groupId = params.get("group");
  if (!groupId) return <p className="owobot-connection">请从 OwO 桌面工作台打开 OwOBot。</p>;
  return <DesktopCapabilitiesProvider>
    <StoreProvider>
      <ThreadRefsProvider><ProjectRoom groupId={groupId} requestedThread={params.get("thread")} /></ThreadRefsProvider>
    </StoreProvider>
  </DesktopCapabilitiesProvider>;
}
