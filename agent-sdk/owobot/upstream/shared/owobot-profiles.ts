// OwO extension: appearance choices shared by the local roster and renderer.
import type { MausColor } from "./wire.ts";

export const OWOBOT_CUSTOM_SECTION = "owobot-custom";

export const OWOBOT_STYLES = {
  sky: { name: "晴空", color: "blue" },
  grove: { name: "松林", color: "green" },
  ember: { name: "余烬", color: "orange" },
  tide: { name: "潮汐", color: "teal" },
  bloom: { name: "曙光", color: "coral" },
  mist: { name: "雾银", color: "silver" },
} as const satisfies Record<string, { name: string; color: MausColor }>;

export type OwobotStyle = keyof typeof OWOBOT_STYLES;
export const OWOBOT_STYLE_IDS = Object.keys(OWOBOT_STYLES) as OwobotStyle[];

export function owobotStyleForColor(color: MausColor, botName?: string): OwobotStyle {
  const chosen = OWOBOT_STYLE_IDS.find((style) => OWOBOT_STYLES[style].color === color);
  if (chosen) return chosen;
  switch (botName) {
    case "组长": return "sky";
    case "执行员": return "ember";
    case "评审员": return "grove";
    default: return "mist";
  }
}

export function isOwobotCustomBot(bot: { section?: string | null }): boolean {
  return bot.section === OWOBOT_CUSTOM_SECTION;
}
