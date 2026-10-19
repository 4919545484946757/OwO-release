/**
 * OwO Agent SDK TypeScript 客户端（场景 4：第三方应用集成）。
 *
 * 用法：
 * ```ts
 * import { createClient } from "@owo-agent/client";
 * const api = createClient({ baseUrl: "http://127.0.0.1:4096" });
 * const session = await api.createSession({ workspace });
 * await api.runTurn({ id: session.id, prompt }, { onEvent });
 * ```
 */
import createFetchClient, { type Middleware } from "openapi-fetch";
import type { paths, components } from "./schema.js";

export interface ClientOptions {
  baseUrl: string;
  /** 每次请求前注入的自定义头（如鉴权令牌）。 */
  headers?: Record<string, string>;
}

export interface TurnStreamOptions {
  onEvent: (event: TurnEvent) => void;
  signal?: AbortSignal;
}

/** SSE 流式事件（对应服务端 TurnEvent JSON）。 */
export interface TurnEvent {
  type: string;
  [key: string]: unknown;
}

export type CompletionStatus = "response_complete" | "candidate" | "accepted" | "unverified" | "blocked" | "aborted";
export interface TurnCompletion {
  completionStatus: CompletionStatus;
  finalText: string;
  stats: TurnEvent;
}
export class TurnFailure extends Error {
  constructor(message: string, public readonly completionStatus: string) {
    super(message); this.name = "TurnFailure";
  }
}

export type ApiClient = ReturnType<typeof createFetchClient<paths>> & {
  /** 便捷方法：创建会话。 */
  createSession(input: { workspace: string; prompt?: string }): Promise<{
    id: string;
    [key: string]: unknown;
  }>;
  /** 便捷方法：发起 Agent 回合（SSE 流式，逐事件回调）。 */
  runTurn(
    input: { id: string; prompt: string; read_only?: boolean; turn_id?: string; model_connection?: components["schemas"]["CustomModelConnection"] },
    stream: TurnStreamOptions,
  ): Promise<TurnCompletion>;
  /** Local loopback server only; browser supplies Host/Origin. A 403 source rejection is not retried. */
  expression(input: { prompt: string }, signal?: AbortSignal): Promise<{ text: string }>;
  /** 便捷方法：健康检查。 */
  health(): Promise<boolean>;
};

/** 创建类型化 API 客户端。 */
export function createClient(options: ClientOptions): ApiClient {
  const baseUrl = options.baseUrl.replace(/\/+$/, "");
  const client = createFetchClient<paths>({
    baseUrl,
  });

  const auth: Middleware = {
    async onRequest({ request }) {
      if (options.headers) {
        for (const [key, value] of Object.entries(options.headers)) {
          request.headers.set(key, value);
        }
      }
    },
  };
  client.use(auth);
  const expression = async (input: { prompt: string }, signal?: AbortSignal) => {
    const response = await fetch(`${baseUrl}/expression`, {
      method: "POST", headers: { ...options.headers, "content-type": "application/json" },
      body: JSON.stringify(input), signal,
    });
    if (!response.ok) throw new Error(`Expression failed: ${response.status}`);
    const result = await response.json() as { text: string };
    if (typeof result.text !== "string" || [...result.text].length > 4096 || !result.text.trim()) {
      throw new Error("Invalid expression response");
    }
    return result;
  };
  Object.assign(client, { expression });

  const api = client as ApiClient;

  api.createSession = async (input) => {
    const { data, error } = await client.POST("/session", {
      body: input as never,
    });
    if (error) throw new Error(`createSession 失败：${JSON.stringify(error)}`);
    return data as { id: string; [key: string]: unknown };
  };

  api.runTurn = async (input, stream) => {
    const controller = new AbortController();
    const requestHeaders = new Headers(options.headers);
    requestHeaders.set("Content-Type", "application/json");
    const sessionPath = "/session/" + encodeURIComponent(input.id);
    const abortUrl = baseUrl + sessionPath + "/abort";
    const requestedTurnId = input.turn_id ?? (stream.signal && !stream.signal.aborted ? crypto.randomUUID() : undefined);
    if (requestedTurnId && !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(requestedTurnId)) {
      throw new Error("turn/invalid_id");
    }
    const canonicalTurnId = requestedTurnId?.toLowerCase();
    let scopedCancellationSupported = false;
    let turnSubmitted = false;
    const onAbort = () => {
      if (turnSubmitted && scopedCancellationSupported && canonicalTurnId) {
        void fetch(abortUrl, {
          method: "POST",
          headers: new Headers(requestHeaders),
          body: JSON.stringify({ turn_id: canonicalTurnId }),
          signal: AbortSignal.timeout(5000),
        }).catch(() => undefined);
      }
      controller.abort();
    };
    const ensureNotAborted = () => {
      if (controller.signal.aborted) {
        const error = new Error("Aborted"); error.name = "AbortError"; throw error;
      }
    };
    if (stream.signal) {
      if (stream.signal.aborted) {
        onAbort();
      } else {
        stream.signal.addEventListener("abort", onAbort, { once: true });
      }
    }
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    let streamEnded = false;
    let buffer = "";
    let turnId: string | null = null;
    let cursor = 0;
    let finalText: string | null = null;
    let terminal: TurnEvent | null = null;
    const observe = (event: TurnEvent, sequence?: number) => {
      if (sequence !== undefined && sequence <= cursor) return;
      if (event.type === "final") finalText = typeof event.text === "string" ? event.text : "";
      if (event.type === "turn_failed" || (event.type === "turn_stats" && terminal?.type !== "turn_failed")) terminal = event;
      stream.onEvent(event);
      if (sequence !== undefined && sequence > cursor) cursor = sequence;
    };
    const finish = (): TurnCompletion => {
      const completion = terminal;
      if (!completion) throw new Error("agentTurn 流结束但未收到 final 事件或宿主完成记录");
      const status = completion.completion_status;
      if (!["response_complete", "candidate", "accepted", "unverified", "blocked", "aborted"].includes(String(status))) {
        throw new Error("agentTurn 缺少有效的宿主完成状态");
      }
      if (completion.type === "turn_failed") {
        throw new TurnFailure(String(completion.message || "回合执行失败"), String(status));
      }
      if (finalText === null) throw new Error("agentTurn 宿主已结束回合，但缺少 final 事件");
      return { completionStatus: status as CompletionStatus, finalText, stats: completion };
    };

    try {
      ensureNotAborted();
      if (input.read_only === true || canonicalTurnId || input.model_connection) {
        const capability = await client.GET("/capabilities", { signal: controller.signal });
        const support = capability.data as { constraints?: { request_read_only?: boolean; scoped_turn_cancellation?: boolean; custom_model_connection?: boolean; custom_model_env_credentials?: boolean } } | undefined;
        ensureNotAborted();
        if (input.read_only === true && (!capability.response.ok || support?.constraints?.request_read_only !== true)) {
          throw new Error("permission/read_only_unsupported: no turn was submitted");
        }
        if (input.model_connection && (!capability.response.ok || support?.constraints?.custom_model_connection !== true)) {
          throw new Error("model_connection/unsupported: no turn was submitted");
        }
        if (input.model_connection?.api_key_env && support?.constraints?.custom_model_env_credentials !== true) throw new Error("model_connection/secure_credentials_unsupported: no turn was submitted");
        scopedCancellationSupported = capability.response.ok && support?.constraints?.scoped_turn_cancellation === true;
        if (canonicalTurnId && !scopedCancellationSupported) {
          throw new Error("turn/scoped_cancellation_unsupported: no turn was submitted");
        }
      }
      ensureNotAborted();
      turnSubmitted = true;
      const response = await fetch(baseUrl + sessionPath + "/turn", {
        method: "POST",
        headers: requestHeaders,
        body: JSON.stringify({ prompt: input.prompt, read_only: input.read_only, turn_id: canonicalTurnId, model_connection: input.model_connection }),
        signal: controller.signal,
      });
      if (!response.ok || !response.body) {
        throw new Error("agentTurn 失败：HTTP " + response.status);
      }
      turnId = response.headers.get("x-owo-turn-id");
      reader = response.body.getReader();
      if (canonicalTurnId && turnId !== canonicalTurnId) {
        throw new Error("turn/identity_mismatch");
      }
      ensureNotAborted();
      const decoder = new TextDecoder();
      while (true) {
        let chunk: ReadableStreamReadResult<Uint8Array>;
        try {
          chunk = await reader.read();
        } catch (error) {
          if (stream.signal?.aborted || controller.signal.aborted || !turnId) throw error;
          break; // The durable event journal can recover a dropped SSE connection.
        }
        if (chunk.done) {
          streamEnded = true;
          buffer += decoder.decode();
          if (buffer) buffer = consumeSseFrames(buffer + "\n\n", observe);
          break;
        }
        buffer += decoder.decode(chunk.value, { stream: true });
        buffer = consumeSseFrames(buffer, observe);
      }

      if (!terminal && turnId) {
        let emptyPages = 0;
        let replayFinished = false;
        while (!replayFinished) {
          if (stream.signal?.aborted) {
            const error = new Error("Aborted"); error.name = "AbortError"; throw error;
          }
          let page: TurnReplayPage | undefined;
          let replayStatus = 0;
          try {
            const replayResult = await client.GET("/session/{id}/turn/events", {
              params: {
                path: { id: input.id },
                query: { turn_id: turnId, after_seq: cursor, limit: 256 },
              },
              signal: stream.signal,
            });
            replayStatus = replayResult.response.status;
            page = replayResult.data as unknown as TurnReplayPage | undefined;
          } catch (error) {
            if (stream.signal?.aborted) throw error;
            await waitForTurnReplay(Math.min(2500, 200 * (2 ** Math.min(emptyPages++, 4))), stream.signal);
            continue;
          }
          if (replayStatus < 200 || replayStatus >= 300) throw new Error("agentTurn 重放失败：HTTP " + replayStatus);
          if (!page) throw new Error("agentTurn 重放响应缺少事件页");
          const records = Array.isArray(page.events) ? page.events : [];
          const pending = records
            .filter((record) => record && record.turn_id === turnId && Number.isSafeInteger(Number(record.seq)) && Number(record.seq) > cursor)
            .sort((left, right) => Number(left.seq) - Number(right.seq));
          for (const record of pending) {
            if (!record.payload || typeof record.payload !== "object") throw new Error("agentTurn 重放事件格式无效");
            observe(record.payload, Number(record.seq));
          }
          if (page.active !== true) {
            if (page.state === "interrupted") throw new Error("回合在写入完成结果前中断；已保留部分回答");
            if (page.state !== "completed" && page.state !== "failed") {
              throw new Error("回合重放结束但缺少宿主完成记录；结果尚未确认");
            }
            replayFinished = true;
          } else if (pending.length > 0) {
            emptyPages = 0; // Drain full replay pages without adding polling delay.
          } else {
            await waitForTurnReplay(Math.min(2500, 200 * (2 ** Math.min(emptyPages++, 4))), stream.signal);
          }
        }
      }
      return finish();
    } finally {
      if (reader) {
        if (!streamEnded) await reader.cancel().catch(() => undefined);
        reader.releaseLock();
      }
      stream.signal?.removeEventListener("abort", onAbort);
    }
  };

  api.health = async () => {
    const { response } = await client.GET("/health");
    return response.ok;
  };

  return api;
}

function consumeSseFrames(text: string, onEvent: (event: TurnEvent, sequence?: number) => void): string {
  const blocks = text.split(/\r?\n\r?\n/);
  const remainder = blocks.pop() ?? "";
  const encoder = new TextEncoder();
  if (encoder.encode(remainder).length > 1024 * 1024) throw new Error("SSE 事件超过 1 MiB 上限");
  for (const block of blocks) {
    let eventName = "message";
    let eventId: string | undefined;
    const data: string[] = [];
    for (const line of block.split(/\r?\n/)) {
      if (line.startsWith("event:")) eventName = line.slice(6).trim();
      if (line.startsWith("id:")) eventId = line.slice(3).trim();
      if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
    }
    const raw = data.join("\n");
    if (!raw || raw === "[DONE]") continue;
    if (encoder.encode(raw).length > 1024 * 1024) throw new Error("SSE 事件超过 1 MiB 上限");
    let event: TurnEvent;
    try { event = JSON.parse(raw) as TurnEvent; }
    catch { throw new Error("SSE 事件格式无效"); }
    if (!event || typeof event !== "object") throw new Error("SSE 事件格式无效");
    if (eventName !== "message") event = { ...event, type: eventName };
    const sequence = eventId && /^\d+$/.test(eventId) ? Number(eventId) : undefined;
    onEvent(event, sequence !== undefined && Number.isSafeInteger(sequence) && sequence > 0 ? sequence : undefined);
  }
  return remainder;
}

function waitForTurnReplay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const cleanup = () => signal?.removeEventListener("abort", abort);
    const timer = setTimeout(() => { cleanup(); resolve(); }, milliseconds);
    const abort = () => {
      clearTimeout(timer); cleanup();
      const error = new Error("Aborted"); error.name = "AbortError"; reject(error);
    };
    if (signal?.aborted) abort();
    else signal?.addEventListener("abort", abort, { once: true });
  });
}

interface TurnReplayPage {
  events?: Array<{ turn_id: string; seq: number; payload: TurnEvent }>;
  active?: boolean;
  state?: "active" | "completed" | "failed" | "interrupted";
}
