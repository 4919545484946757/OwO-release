import { useEffect, useRef, useState } from "react";
import { api, ApiError } from "@/state/api";

type Decision = "apply" | "reject";

interface Delivery {
  taskId: string;
  revision: number;
  revisions: number[];
  status: "working" | "staged" | "changesRequested" | "reviewed" | "applying" | "applied" | "rejected";
  files: { path: string; before: string | null; after: string }[];
  review: { approved: boolean; note: string; reviewedAt: string } | null;
  decidedAt: string | null;
}

const statusText: Record<Delivery["status"], string> = {
  working: "待交付",
  staged: "待评审",
  changesRequested: "需修改",
  reviewed: "待用户批准",
  applying: "应用中断",
  applied: "已应用",
  rejected: "已拒绝",
};

const errorText: Record<string, string> = {
  delivery_revision_changed: "交付已更新，请核对最新版本。",
  delivery_workspace_changed: "工作区文件已发生变化。请核对当前文件后重新暂存。",
  delivery_already_decided: "这个版本已处理。",
  delivery_requires_review: "请先完成评审。",
};

export function DeliveryPanel({ thread, working, message }: { thread: string; working: boolean; message?: string }) {
  const [delivery, setDelivery] = useState<Delivery | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [shownRevision, setShownRevision] = useState("");
  const deciding = useRef(false);
  const revision = useRef(0);
  const mounted = useRef(false);
  const path = "/api/delivery/" + encodeURIComponent(thread);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    const refresh = async () => {
      if (deciding.current) return;
      const request = ++revision.current;
      try {
        const url = shownRevision ? path + "?revision=" + encodeURIComponent(shownRevision) : path;
        const response = await api<Delivery>(url, { signal: controller.signal });
        if (!controller.signal.aborted && request === revision.current) setDelivery(response);
      } catch (err) {
        if (controller.signal.aborted || request !== revision.current) return;
        if (err instanceof ApiError && err.status === 404) setDelivery(null);
        else setError("暂时无法读取交付记录。");
      }
    };
    void refresh();
    const timer = working ? window.setInterval(() => void refresh(), 1800) : undefined;
    return () => {
      controller.abort();
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [path, working, message, shownRevision]);

  if (!delivery) return null;
  const isLatest = !shownRevision && delivery.revision === (delivery.revisions.at(-1) ?? 0);
  const decide = async (action: Decision) => {
    if (!isLatest || deciding.current) return;
    deciding.current = true;
    ++revision.current;
    setPending(true);
    setError("");
    try {
      const view = await api<Delivery>(path + "/decision", {
        method: "POST",
        body: JSON.stringify({
          revision: delivery.revision, action,
          idempotencyKey: "desktop-" + action + "-" + delivery.revision,
        }),
      });
      if (mounted.current) setDelivery(view);
    } catch (err) {
      if (!mounted.current) return;
      // Read the journal after an interrupted request so the same approved revision can resume.
      const message = err instanceof Error ? err.message : "";
      setError(errorText[message] || "操作未完成，请检查交付状态。");
      try {
        const current = await api<Delivery>(path);
        if (mounted.current) setDelivery(current);
      } catch {
        // Preserve the last verified view until the next update.
      }
    } finally {
      deciding.current = false;
      if (mounted.current) setPending(false);
    }
  };

  return <aside className="owobot-delivery" aria-label="产物与审批">
    <details open={delivery.revision > 0 || undefined}>
      <summary>
        <span>产物与审批</span>
        <span className="owobot-delivery-status" role="status">{statusText[delivery.status]}</span>
      </summary>
      <div className="owobot-delivery-content">
        <p className="owobot-task-id" title={delivery.taskId}>{delivery.taskId}</p>
        {delivery.revision > 0 && <p>版本 {delivery.revision} · {delivery.files.length} 个文件</p>}
        {delivery.revisions.length > 1 && <label className="owobot-delivery-versions">
          <span>交付版本</span>
          <select value={shownRevision} onChange={(event) => setShownRevision(event.target.value)} disabled={pending}>
            <option value="">最新</option>
            {delivery.revisions.map((number) => <option key={number} value={number}>{number}</option>)}
          </select>
        </label>}
        {delivery.review && <div className="owobot-review">
          <strong>{delivery.review.approved ? "评审通过" : "评审要求修改"}</strong>
          <p>{delivery.review.note}</p>
        </div>}
        {delivery.files.map((file) => <details className="owobot-delivery-file" key={file.path}>
          <summary>{file.path}</summary>
          {file.before !== null && <div><strong>修改前</strong><pre>{file.before}</pre></div>}
          <div><strong>{file.before === null ? "新建文件" : "提议内容"}</strong><pre>{file.after}</pre></div>
        </details>)}
        {error && <p role="alert" className="owobot-delivery-error">{error}</p>}
        <div className="owobot-delivery-actions">
          {isLatest && (delivery.status === "reviewed" || delivery.status === "applying") &&
            <button type="button" disabled={pending} onClick={() => void decide("apply")}>
              {delivery.status === "applying" ? "继续应用" : "批准并应用"}
            </button>}
          {isLatest && ["staged", "changesRequested", "reviewed"].includes(delivery.status) &&
            <button type="button" disabled={pending} onClick={() => void decide("reject")}>拒绝</button>}
        </div>
      </div>
    </details>
  </aside>;
}
