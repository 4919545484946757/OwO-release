// Modified for OwO: a roster and conversation list share the local workbench.
import { useEffect, useMemo, useRef, useState } from "react";
import { MessageCircle, MessageSquarePlus, Palette, Plus } from "lucide-react";
import { GroupView } from "@/components/GroupView";
import { DesktopCapabilitiesProvider } from "@/components/DesktopCapabilities";
import { ThreadRefsProvider } from "@/components/ThreadRefs";
import { t } from "@/lib/i18n";
import { localizedTaskTitle } from "@/lib/task-title";
import { leadComposerDraft } from "@/lib/drafts";
import { StoreProvider, openRoomThread, useStore, type Bot } from "@/state/store";
import { isOwobotCustomBot } from "../../shared/owobot-profiles";
import { BotMark } from "./BotMark";
import { BotProfileDialog } from "./BotProfileDialog";
import { DeliveryPanel } from "./DeliveryPanel";
import "./workbench.css";

function profileSummary(bot: Bot) {
  if (isOwobotCustomBot(bot)) return bot.description?.trim() || bot.soul?.trim();
  if (bot.name === "组长") return t("owo.role.lead");
  if (bot.name === "执行员") return t("owo.role.worker");
  if (bot.name === "评审员") return t("owo.role.reviewer");
  return bot.title?.trim() || bot.description?.trim();
}

function ProjectRoom({ groupId, requestedThread }: { groupId: string; requestedThread: string | null }) {
  const { state, dispatch } = useStore();
  const workbench = useRef<HTMLDivElement>(null);
  const [profile, setProfile] = useState<"new" | Bot | null>(null);
  const group = state.groups.find((candidate) => candidate.id === groupId);
  const snippets = useMemo(() => {
    const latest = new Map<string, string>();
    for (const message of group?.messages ?? []) {
      if (message.from?.botId && message.role === "bot" && message.kind === "text" && message.text?.trim()) {
        latest.set(message.from.botId, message.text.trim().slice(0, 120));
      }
    }
    return latest;
  }, [group?.messages]);
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
  const botsById = new Map(state.bots.map((bot) => [bot.id, bot]));
  const members = group.memberIds.flatMap((id) => botsById.get(id) ?? []);
  const threadId = group.threadId;
  function mention(bot: Bot) {
    leadComposerDraft(`group:${groupId}:${threadId}`, `@${bot.name} `);
    requestAnimationFrame(() => {
      const field = workbench.current?.querySelector<HTMLTextAreaElement>(".owobot-transcript textarea");
      field?.focus();
      if (field) field.setSelectionRange(field.value.length, field.value.length);
    });
  }

  return <div ref={workbench} className="owobot-workbench">
    <aside className="owobot-task-list" aria-label="项目会话和 Bot">
      <section className="owobot-roster" aria-labelledby="owobot-roster-title">
        <div className="owobot-sidebar-heading">
          <h2 id="owobot-roster-title">项目 Bot</h2>
          <button className="owobot-add-bot" type="button" aria-label="新建 Bot" title="新建 Bot"
            disabled={!state.connected} onClick={() => setProfile("new")}><Plus size={17} aria-hidden="true" /></button>
        </div>
        <ul className="owobot-roster-list">
          {members.map((bot) => {
            const preview = snippets.get(bot.id) || profileSummary(bot);
            return <li key={bot.id} className="owobot-roster-row">
            <button className="owobot-mention-bot" type="button" aria-label={`提及 ${bot.name}`}
              title={`在这条会话里提及 ${bot.name}`} onClick={() => mention(bot)}>
              <BotMark color={bot.color} name={bot.name} size={38} />
              <span className="owobot-roster-copy"><span className="owobot-roster-name">{bot.name}</span>
                {preview && <small>{preview}</small>}
              </span>
              {group.working && group.busyBotId === bot.id
                ? <span className="owobot-bot-working" title="回复中" aria-label="回复中" />
                : null}
            </button>
            <button type="button" className="owobot-icon-button owobot-style-button" disabled={!state.connected}
              aria-label={`更换 ${bot.name} 的外观`} title={`更换 ${bot.name} 的外观`}
              onClick={() => setProfile(bot)}><Palette size={14} aria-hidden="true" /></button>
          </li>;
          })}
        </ul>
      </section>
      <nav className="owobot-conversations" aria-labelledby="owobot-conversations-title">
        <div className="owobot-sidebar-heading">
          <h2 id="owobot-conversations-title">会话</h2>
          <button type="button" className="owobot-icon-button owobot-new-task" aria-label="新建协作任务" title="新建协作任务"
            disabled={!state.connected} onClick={() => dispatch({ type: "newGroupTask", groupId })}>
            <MessageSquarePlus size={16} aria-hidden="true" />
          </button>
        </div>
        <ul>
          {tasks.map((task) => <li key={task.threadId}>
            <button type="button" className="owobot-conversation" aria-current={task.threadId === group.threadId ? "page" : undefined}
              onClick={() => openRoomThread(dispatch, { groupId, threadId: task.threadId }, state.groups)}>
              <MessageCircle size={15} aria-hidden="true" />
              <span>{task.title === "New task" ? "新会话" : localizedTaskTitle(task.title)}</span>
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
    {profile && <BotProfileDialog key={profile === "new" ? "new" : profile.id} groupId={groupId}
      bot={profile === "new" ? undefined : profile} onClose={() => setProfile(null)} />}
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
