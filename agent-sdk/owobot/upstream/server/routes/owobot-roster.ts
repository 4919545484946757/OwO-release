// OwO extension: project roster mutations exposed through the desktop gateway.
import { z } from "zod";
import { isOwobotCustomBot, OWOBOT_CUSTOM_SECTION, OWOBOT_STYLES, OWOBOT_STYLE_IDS } from "../../shared/owobot-profiles.ts";
import type { WireBot } from "../../shared/wire.ts";
import type { BotRecord, Store } from "../store.ts";
import { PASS, type RouteHandler } from "./table.ts";

const style = z.enum(OWOBOT_STYLE_IDS);
const createProfile = z.object({
  requestId: z.string().uuid(),
  name: z.string().trim().min(1).max(24).regex(/^[\p{L}\p{N}][\p{L}\p{N} _-]*$/u),
  instructions: z.string().trim().max(400).optional(),
  style,
}).strict();
const updateAppearance = z.object({ style }).strict();
const reservedNames = new Set(["lead", "worker", "reviewer", "everyone", "all", "you", "所有人", "全体成员"]);

function nameKey(name: string): string {
  return name.normalize("NFKC").toLowerCase();
}

export function createOwobotRosterRoutes(
  store: Store,
  wireBot: (bot: BotRecord) => WireBot,
  maxBots: number,
): RouteHandler {
  return async ({ req, res, path, method, auth, readBody, json }) => {
    const match = path.match(/^\/api\/groups\/([\w-]+)\/bots(?:\/([\w-]+))?$/);
    if (method !== "POST" || !match) return PASS;
    if (auth.kind !== "session" || !auth.scopes.includes("admin")) {
      return json(res, 403, { error: "请从 OwO 工作台管理本机 Bot。" });
    }
    const body: unknown = await readBody(req);
    const room = store.group(match[1]!);
    if (!room || room.dm || !room.cwd) {
      return json(res, 404, { error: "找不到当前项目。" });
    }
    const botId = match[2];
    if (botId) {
      if (!room.memberIds.includes(botId) || !store.bot(botId)) {
        return json(res, 404, { error: "这个 Bot 不在当前项目里。" });
      }
      const appearance = updateAppearance.safeParse(body);
      if (!appearance.success) return json(res, 400, { error: "请选择有效的 Bot 外观。" });
      const bot = store.patchBot(botId, { color: OWOBOT_STYLES[appearance.data.style].color });
      return json(res, 200, { bot: wireBot(bot!) });
    }

    const profile = createProfile.safeParse(body);
    if (!profile.success) return json(res, 400, { error: "请使用 1 至 24 个字母、数字、汉字、空格、连字符或下划线作为名称。" });
    const { requestId, name, instructions = "", style: appearance } = profile.data;
    const key = nameKey(name);
    const named = store.bots.find((existing) => nameKey(existing.name) === key);
    const interrupted = named && !room.memberIds.includes(named.id) && isOwobotCustomBot(named)
      && named.cwd === room.cwd && named.name === name ? named : undefined;
    let bot = store.bot(requestId) ?? interrupted;
    if (bot) {
      if (!isOwobotCustomBot(bot) || bot.cwd !== room.cwd || bot.name !== name) {
        return json(res, 409, { error: "这个创建请求已经用于另一个 Bot。" });
      }
    } else {
      if (reservedNames.has(key) || named) {
        return json(res, 409, { error: "这个名称已使用，请换一个名称。" });
      }
      if (store.botByThread(requestId) || store.groupByThread(requestId)) {
        return json(res, 409, { error: "这个创建请求与现有会话冲突。" });
      }
      if (store.bots.length >= maxBots) return json(res, 409, { error: "这个项目已达到 Bot 数量上限。" });
      bot = store.createBot({
        name, soul: instructions, section: OWOBOT_CUSTOM_SECTION, cwd: room.cwd,
        color: OWOBOT_STYLES[appearance].color,
        modelSelection: { instanceId: "owo-agent", model: "agent-default" },
      }, { id: requestId, seedMessages: false });
    }
    if (!room.memberIds.includes(bot.id)) {
      store.patchBot(bot.id, { computer: "off", approvalMode: "ask" });
      store.patchGroup(room.id, { memberIds: [...room.memberIds, bot.id] });
    }
    return json(res, 200, { bot: wireBot(bot) });
  };
}
