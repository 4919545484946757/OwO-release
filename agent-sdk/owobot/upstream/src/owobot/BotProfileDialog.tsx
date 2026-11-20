// OwO extension: create a project Bot or update its appearance.
import { useEffect, useRef, useState, type FormEvent } from "react";
import { Check, X } from "lucide-react";
import { api, ApiError } from "@/state/api";
import type { Bot } from "@/state/store";
import { OWOBOT_STYLES, OWOBOT_STYLE_IDS, owobotStyleForColor, type OwobotStyle } from "../../shared/owobot-profiles";
import { BotMark } from "./BotMark";

export function BotProfileDialog({ groupId, bot, onClose }: { groupId: string; bot?: Bot; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const requestId = useRef<string | null>(null);
  const [name, setName] = useState(bot?.name ?? "");
  const [instructions, setInstructions] = useState("");
  const [style, setStyle] = useState<OwobotStyle>(bot ? owobotStyleForColor(bot.color, bot.name) : "sky");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const modal = dialog.current;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    modal?.showModal();
    return () => {
      modal?.close();
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (saving || !name.trim()) return;
    setSaving(true);
    setError("");
    requestId.current ??= crypto.randomUUID();
    const route = `/api/groups/${encodeURIComponent(groupId)}/bots`;
    try {
      await api(bot ? `${route}/${encodeURIComponent(bot.id)}` : route, {
        method: "POST",
        body: JSON.stringify(bot ? { style } : { requestId: requestId.current, name: name.trim(), instructions, style }),
      });
      onClose();
    } catch (failure) {
      setError(failure instanceof ApiError && failure.status >= 400 && failure.status < 500 && failure.status !== 403
        ? failure.message
        : "暂时无法保存。请检查本机工作台连接后重试。");
      setSaving(false);
    }
  }

  return <dialog ref={dialog} className="owobot-profile-dialog" aria-labelledby="owobot-profile-title"
    onCancel={(event) => { event.preventDefault(); if (!saving) onClose(); }}>
    <form onSubmit={(event) => void save(event)}>
      <header className="owobot-profile-header">
        <h2 id="owobot-profile-title">{bot ? "更换 Bot 外观" : "新建 Bot"}</h2>
        <button className="owobot-icon-button" type="button" aria-label="关闭" disabled={saving} onClick={onClose}>
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      <div className="owobot-profile-preview">
        <BotMark color={OWOBOT_STYLES[style].color} size={56} />
        <div><strong>{name.trim() || "新 Bot"}</strong><span>{OWOBOT_STYLES[style].name}</span></div>
      </div>
      {!bot && <div className="owobot-profile-fields">
        <label>
          名称
          <input value={name} onChange={(event) => setName(event.target.value)} maxLength={24}
            placeholder="例如 资料助手" autoFocus required disabled={saving} />
        </label>
        <label>
          工作职责 <span>选填</span>
          <textarea value={instructions} onChange={(event) => setInstructions(event.target.value)} maxLength={400}
            rows={3} placeholder="例如 整理项目资料，回答时注明出处" disabled={saving} />
        </label>
      </div>}
      <fieldset className="owobot-style-picker" disabled={saving}>
        <legend>外观</legend>
        <div className="owobot-style-grid">
          {OWOBOT_STYLE_IDS.map((id) => <label key={id} className="owobot-style-choice">
            <input className="sr-only" type="radio" name="appearance" value={id} checked={style === id}
              onChange={() => setStyle(id)} />
            <BotMark color={OWOBOT_STYLES[id].color} size={36} />
            <span>{OWOBOT_STYLES[id].name}</span>
            {id === style && <Check size={12} className="owobot-style-selected" aria-hidden="true" />}
          </label>)}
        </div>
      </fieldset>
      {error && <p className="owobot-profile-error" role="alert">{error}</p>}
      <footer className="owobot-profile-actions">
        <button className="owobot-secondary-button" type="button" disabled={saving} onClick={onClose}>取消</button>
        <button className="owobot-primary-button" type="submit" disabled={saving || !name.trim()}>
          {saving ? "保存中…" : bot ? "保存外观" : "创建 Bot"}
        </button>
      </footer>
    </form>
  </dialog>;
}
