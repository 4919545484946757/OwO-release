// OwO extension: local Bot marks used in the roster and conversation.
import type { CSSProperties } from "react";
import { owobotStyleForColor } from "../../shared/owobot-profiles";
import type { MausColor } from "../../shared/wire";

export function BotMark({ color, size = 34, name, label }: { color: MausColor; size?: number; name?: string; label?: string }) {
  const style = { "--owobot-mark-size": `${size}px` } as CSSProperties;
  return <span className="owobot-role-avatar" data-style={owobotStyleForColor(color, name)} style={style}
    role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
    <span />
  </span>;
}
