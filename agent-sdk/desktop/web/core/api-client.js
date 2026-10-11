/* OwO desktop API boundary.
 * All business code goes through this client so authentication, errors,
 * uploads/downloads and reconnect behavior stay consistent.
 */
(function (global) {
  "use strict";

  // §8.1：ledger 来源标签（服务端 request_ledger_api.CLIENT_HEADER / 消毒白名单同源）。
  const OWO_CLIENT_HEADER = "x-owo-client";
  const OWO_CLIENT_SOURCE = "web";
  // §8.2 第 5 条：连接层失败后重查壳连接的冷却窗口（防止核心真挂时轮询风暴）。
  const REHANDSHAKE_COOLDOWN_MS = 1000;

  class ApiError extends Error {
    constructor(message, details) {
      super(message);
      this.name = "ApiError";
      this.status = details && details.status ? details.status : 0;
      this.body = details && details.body;
      this.code = details && details.code;
      this.traceId = details && details.traceId;
    }
  }

  class ApiClient {
    constructor(baseUrl) {
      this.initialBaseUrl = String(baseUrl || "").replace(/\/+$/, "");
      this.baseUrl = this.initialBaseUrl;
      this.token = null;
      this.tokenPromise = null;
      this.pairingPromise = null;
      this.refreshPromise = null;
      this.unavailableUntil = 0;
      // §4.2/§6.1：Tauri 桌面端向壳询问核心连接（动态端口 + 实例身份 + 配对证明）。
      this.coreConnectionPromise = null;
      this.coreInstanceId = null;
      this.pairingOverride = null;
      // §4 首屏收敛：壳注入的短期 bearer token（正式桌面模式免去 GET /auth/token）。
      this.injectedToken = null;
      // §8.2 第 5 条：运行期重连状态（连续网络失败计数 + 重查冷却时刻）。
      this.networkFailures = 0;
      this.lastRehandshakeAt = 0;
    }

    /**
     * §8.2 第 5 条：core 被壳重启后**端口和 bearer 都会换**（每次启动换发 token）。
     * 打到旧端口的 fetch 以网络错误失败，根本不会命中 401 分支——只靠 401 重连
     * 不够，必须在连接层失败时整体重查壳连接（resetCoreConnection 会一并失效
     * 注入 token 与缓存描述符），再让调用方重试一次。
     * 冷却窗口用于避免"核心确实没起来"时每次轮询都重查一遍（风暴放大）。
     * @param {boolean} allowRetry 本次尝试是否还允许重试
     * @returns {boolean} 是否已重查连接（true 时调用方可重试一次）
     */
    handleNetworkFailure(allowRetry) {
      this.networkFailures += 1;
      if (!allowRetry) return false;
      const now = Date.now();
      if (now - this.lastRehandshakeAt < REHANDSHAKE_COOLDOWN_MS) return false;
      this.lastRehandshakeAt = now;
      this.networkFailures = 0;
      this.resetCoreConnection();
      return true;
    }

    url(path) {
      return this.baseUrl + path;
    }

    // §8.1：来源标签——服务端 ledger 只收 method/route_template/started_at/
    // duration_ms/status/source 六字段，本头即 source 的唯一来源（服务端消毒）。
    static withSourceTag(headers) {
      const out = new Headers(headers || {});
      if (!out.has(OWO_CLIENT_HEADER)) out.set(OWO_CLIENT_HEADER, OWO_CLIENT_SOURCE);
      return out;
    }

    // §4.2：解析 Tauri invoke 入口（公开 API 优先，兼容 internals）。
    static tauriInvokeOwner(scope) {
      const internal = scope && scope.__TAURI_INTERNALS__;
      const publicCore = scope && scope.__TAURI__ && scope.__TAURI__.core;
      const owner = publicCore && typeof publicCore.invoke === "function" ? publicCore : internal;
      return owner && typeof owner.invoke === "function" ? owner : null;
    }

    // §4.2：把壳返回的核心状态归一为诊断描述符（挂到 window.__owoCoreDiagnostics）。
    static buildCoreDiagnostics(descriptor) {
      if (!descriptor || typeof descriptor !== "object") return null;
      if (descriptor.state === "ready") {
        return {
          state: "ready",
          port: descriptor.port,
          pid: descriptor.pid,
          buildId: descriptor.buildId,
          // §6.1.4：壳编译期期望的 build id（缺失 = 旧壳，比对降级为不可用）。
          expectedBuildId: typeof descriptor.expectedBuildId === "string" ? descriptor.expectedBuildId : null,
          apiVersion: descriptor.apiVersion,
          instanceId: descriptor.instanceId,
          // §4.6：壳侧启动代际（旧壳没有该字段 → null，UI 必须显式说"壳未上报"）。
          // 判定先排除 null：`Number(null) === 0` 会把"未上报"伪装成"第 0 代"。
          generation:
            descriptor.generation === null || descriptor.generation === undefined || !Number.isFinite(Number(descriptor.generation))
              ? null
              : Number(descriptor.generation),
        };
      }
      return {
        state: descriptor.state || "unknown",
        errorCode: descriptor.errorCode,
        message: descriptor.message,
        logPath: descriptor.logPath,
        attempt: descriptor.attempt === null || descriptor.attempt === undefined || !Number.isFinite(Number(descriptor.attempt)) ? null : Number(descriptor.attempt),
        generation: descriptor.generation === null || descriptor.generation === undefined || !Number.isFinite(Number(descriptor.generation)) ? null : Number(descriptor.generation),
      };
    }

    // §4.2：向壳查询核心连接。仅在 Tauri 环境生效；结果缓存（单飞）。
    // - ready：切换 baseUrl 到实际端口，记录实例身份与配对证明；
    //   §4：壳注入的短期 token 存入 injectedToken（正式桌面冷启动不再请求 /auth/token）；
    // - 非 ready：仅更新启动诊断（错误码/文案/日志路径），沿用原 baseUrl。
    ensureCoreConnection() {
      if (this.coreConnectionPromise) return this.coreConnectionPromise;
      const owner = ApiClient.tauriInvokeOwner(global);
      if (!owner) return Promise.resolve(null);
      this.coreConnectionPromise = Promise.resolve(
        owner.invoke.call(owner, "get_core_connection")
      ).then((descriptor) => {
        const connection = descriptor && typeof descriptor === "object" ? descriptor : null;
        if (connection && connection.state === "ready" && connection.port > 0) {
          this.baseUrl = "http://127.0.0.1:" + connection.port;
          this.coreInstanceId = typeof connection.instanceId === "string" ? connection.instanceId : null;
          if (typeof connection.pairing === "string" && connection.pairing.length >= 32) {
            this.pairingOverride = connection.pairing;
          }
          if (typeof connection.token === "string" && connection.token.length > 0) {
            this.injectedToken = connection.token;
            this.token = connection.token;
          }
        }
        global.__owoCoreDiagnostics = ApiClient.buildCoreDiagnostics(connection);
        // R3-B（§3.4 终态可见性）：**只有 ready 才长期缓存**。历史缺陷：首屏第一次
        // 查询发生在核心还在启动时（state=starting，无 errorCode），此后 Promise 被
        // 永久复用 → 壳后来进入 failed(storage/not_writable / core/exited /
        // core/handshake_timeout) 的稳定码永远读不到，错误卡只能落到"默认三出口 +
        // 通用文案"。真机故障矩阵里 core-exit / core-hang / data-dir-unwritable 三条
        // 全因此红（不是产品没报错，是 UI 拿的是陈旧快照）。
        // 非 ready 时放开下一次重查：这是 Tauri IPC，不进 HTTP 台账，
        // 不影响 §8.2 首屏请求预算；在途请求仍由 coreConnectionPromise 单飞合并。
        if (!connection || connection.state !== "ready") {
          this.coreConnectionPromise = null;
        }
        return connection;
      }).catch((error) => {
        global.__owoCoreDiagnostics = {
          state: "unknown",
          message: String((error && error.message) || error),
        };
        this.coreConnectionPromise = null;
        return null;
      });
      return this.coreConnectionPromise;
    }

    // §4.2：核心被壳重启后端口/实例可能变化；恢复流程重查连接。
    // §4：注入 token 与缓存 token 一并失效（新实例必须重新向壳取新凭据）。
    resetCoreConnection() {
      this.coreConnectionPromise = null;
      this.coreInstanceId = null;
      this.pairingOverride = null;
      this.injectedToken = null;
      this.token = null;
      this.tokenPromise = null;
      this.baseUrl = this.initialBaseUrl;
    }

    emit(ready, detail) {
      if (typeof global.dispatchEvent !== "function") return;
      global.dispatchEvent(new CustomEvent("owo:connection", {
        detail: Object.assign({ ready: ready }, detail || {}),
      }));
    }

    async bootstrapToken(force) {
      if (!force && this.token) return this.token;
      // §4：先确保桌面连接已询问——token 注入发生在连接描述符到达时；
      // 非 Tauri 环境此调用是无害的立即 resolve。
      await this.ensureCoreConnection();
      if (!force && this.token) return this.token;
      // §4 首屏收敛：正式桌面模式壳已注入 token——不再发起 GET /auth/token
      //（冷启动总请求 = health 1 + 业务 4 = 5）；浏览器开发模式才走引导端点。
      if (this.injectedToken) {
        this.token = this.injectedToken;
        this.unavailableUntil = 0;
        return this.token;
      }
      if (this.tokenPromise) return this.tokenPromise;
      this.tokenPromise = this.ensureCoreConnection().then(() => this.desktopPairingProof()).then((pairing) => {
        const headers = ApiClient.withSourceTag({ Accept: "application/json" });
        if (pairing) headers.set("X-Owo-Desktop-Pairing", pairing);
        if (this.coreInstanceId) headers.set("x-owo-desktop-instance", this.coreInstanceId);
        return global.fetch(this.url("/auth/token"), { headers: headers });
      }).then(async (response) => {
        if (!response.ok) {
          const body = await response.text();
          let code = "";
          try {
            const payload = JSON.parse(body);
            code = payload && payload.code ? String(payload.code) : "";
          } catch (_) {
            /* 非 JSON 错误仍保留状态码，不把服务端原文直接灌进 UI。 */
          }
          let message = "token 引导失败（HTTP " + response.status + "）";
          if (code === "auth/pairing_required/not_retryable") {
            message = "此浏览器未获得桌面授权，请在 Electron 工作台中打开会话。";
          } else if (code === "auth/instance_mismatch/not_retryable") {
            message = "当前核心属于另一个桌面实例，请在 Electron 工作台重启核心后重试。";
          } else if (response.status === 404 || response.status === 405) {
            const hasDesktopBridge = Boolean(ApiClient.tauriInvokeOwner(global));
            message = hasDesktopBridge
              ? "Electron 已连接，但核心认证接口不可用（HTTP " + response.status + "）。请重启核心；如果问题仍在，请确认桌面端与核心服务版本一致。"
              : "当前浏览器预览没有连接 Electron 核心（HTTP " + response.status + "）。请通过 Electron 工作台启动完整服务后重试。";
            code = code || "auth/bootstrap_route_unavailable";
          }
          throw new ApiError(message, { status: response.status, body: body, code: code });
        }
        const data = await response.json();
        if (!data || !data.token) throw new ApiError("token 引导响应缺少 token");
        this.token = data.token;
        this.unavailableUntil = 0;
        this.emit(true, { reason: "token" });
        return this.token;
      }).catch((error) => {
        this.unavailableUntil = Date.now() + 1500;
        this.emit(false, { error: error });
        throw error;
      }).finally(() => {
        this.tokenPromise = null;
      });
      return this.tokenPromise;
    }

    async desktopPairingProof() {
      if (this.pairingPromise) return this.pairingPromise;
      if (this.pairingOverride) return this.pairingOverride;
      const owner = ApiClient.tauriInvokeOwner(global);
      if (!owner) return null;
      this.pairingPromise = Promise.resolve(owner.invoke.call(owner, "desktop_pairing"))
        .then((proof) => typeof proof === "string" && proof.length >= 32 ? proof : null)
        .catch(() => null);
      return this.pairingPromise;
    }

    async request(path, options) {
      const opts = Object.assign({}, options || {});
      const responseType = opts.responseType || "json";
      const retryAuth = opts.retryAuth !== false;
      const isPublic = opts.public === true;
      // 网络错误自动重试仅限幂等方法：POST/PATCH 可能在服务端已执行、只是响应丢失，
      // 盲目重放会重复发起回合/写入（POST /session/{id}/turn、/session 等）。401 重试
      // 仍然安全（鉴权中间件先于 handler 拒绝，未产生副作用）。
      const method = String(opts.method || "GET").toUpperCase();
      const networkRetrySafe =
        method === "GET" || method === "HEAD" || method === "PUT" || method === "DELETE";
      delete opts.responseType;
      delete opts.retryAuth;
      delete opts.public;
      const headers = ApiClient.withSourceTag(opts.headers || {});
      if (opts.json !== undefined) {
        opts.body = JSON.stringify(opts.json);
        delete opts.json;
        if (!headers.has("Content-Type")) headers.set("Content-Type", "application/json");
      }
      if (opts.body != null && typeof opts.body === "string" && !headers.has("Content-Type")) {
        headers.set("Content-Type", "application/json");
      }
      // Resolve the shell address even for public health checks before token bootstrap.
      await this.ensureCoreConnection();
      const execute = async (allowRetry) => {
        // 每次尝试使用独立 Headers，避免 401 重试回写前一次请求的观测对象。
        const requestHeaders = new Headers(headers);
        if (!isPublic && !requestHeaders.has("Authorization")) {
          const token = await this.bootstrapToken(false);
          requestHeaders.set("Authorization", "Bearer " + token);
        }
        let response;
        try {
          response = await global.fetch(this.url(path), Object.assign({}, opts, { headers: requestHeaders }));
        } catch (error) {
          // 连接层失败（端口没了 / 连接被拒）：可能是壳刚重启了 core。
          // 先整体重查连接再重试一次；非幂等方法不自动重放（见上）。
          if (networkRetrySafe && this.handleNetworkFailure(allowRetry)) {
            headers.delete("Authorization");
            return execute(false);
          }
          this.emit(false, { error: error });
          throw error;
        }
        if (response.status === 401 && allowRetry) {
          // §8.2 第 5 条：core 被壳重启后会换发 bearer（每次启动 mint_for_boot）。
          // 只清 this.token 不够——injectedToken 与缓存的连接描述符会把旧值再注入
          // 一遍，重试必然二次 401。必须整体重查壳连接（端口/实例/token 一并更新）。
          this.resetCoreConnection();
          headers.delete("Authorization");
          return execute(false);
        }
        if (!response.ok) {
          // 429/5xx 属于瞬时故障：幂等方法（GET/HEAD/PUT/DELETE）可安全重放一次，
          // 优先尊重 Retry-After（上限 2s，避免把 UI 卡死）；POST/PATCH 仍不重放。
          const retryableStatus =
            response.status === 429 ||
            response.status === 502 ||
            response.status === 503 ||
            response.status === 504;
          if (networkRetrySafe && retryableStatus && allowRetry) {
            const retryAfter = Number(response.headers.get("retry-after"));
            const delay =
              Number.isFinite(retryAfter) && retryAfter > 0
                ? Math.min(retryAfter * 1000, 2000)
                : 300;
            await response.text();
            await new Promise((resolve) => setTimeout(resolve, delay));
            headers.delete("Authorization");
            return execute(false);
          }
          const body = await response.text();
          throw new ApiError(response.status + ": " + body, {
            status: response.status,
            body: body,
            traceId: response.headers.get("x-trace-id"),
          });
        }
        this.unavailableUntil = 0;
        this.networkFailures = 0;
        this.emit(true, { status: response.status, path: path });
        if (responseType === "response") return response;
        if (responseType === "blob") return response.blob();
        if (responseType === "text") return response.text();
        if (responseType === "stream") return response;
        if (response.status === 204) return null;
        const text = await response.text();
        if (!text) return null;
        try { return JSON.parse(text); } catch (_) { return text; }
      };
      return execute(retryAuth);
    }

    // Static assets and explicit provider probes must never inherit daemon credentials,
    // pairing, source headers, retries, or the daemon's dynamic base URL.
    resource(url, options) {
      const opts = Object.assign({}, options || {});
      if (opts.credentials === undefined) opts.credentials = "omit";
      return global.fetch(url, opts);
    }

    get(path, options) { return this.request(path, Object.assign({}, options || {}, { method: "GET" })); }
    post(path, json, options) { return this.request(path, Object.assign({}, options || {}, { method: "POST", json: json || {} })); }
    put(path, json, options) { return this.request(path, Object.assign({}, options || {}, { method: "PUT", json: json || {} })); }
    patch(path, json, options) { return this.request(path, Object.assign({}, options || {}, { method: "PATCH", json: json || {} })); }
    delete(path, options) { return this.request(path, Object.assign({}, options || {}, { method: "DELETE" })); }
    upload(path, body, contentType, options) {
      return this.request(path, Object.assign({}, options || {}, {
        method: "POST", body: body, headers: Object.assign({ "Content-Type": contentType }, (options || {}).headers || {}),
      }));
    }
    download(path, options) { return this.request(path, Object.assign({}, options || {}, { responseType: "blob" })); }
    stream(path, options) { return this.request(path, Object.assign({}, options || {}, { responseType: "stream" })); }

    // §3.1：带认证的流式 SSE。EventSource 无法携带 Authorization 头，因此
    // /events/stream 与资源型 /…/events 一律经此 fetch-stream 订阅：
    // - Bearer 头 + Accept: text/event-stream + 可选 Last-Event-ID 续传头；
    // - 401 单次刷新 token 后重试（token 永不进入 URL/查询串/日志）；
    // - onEvent({event, data, id}) 逐帧回调，onOpen(status) 在流建立时回调；
    // - 返回在流结束时 resolve 的 Promise；调用方以 AbortController(signal) 取消。
    async openEventStream(path, options) {
      const opts = Object.assign({}, options || {});
      const signal = opts.signal || undefined;
      const onEvent = typeof opts.onEvent === "function" ? opts.onEvent : null;
      const onOpen = typeof opts.onOpen === "function" ? opts.onOpen : null;
      const headers = ApiClient.withSourceTag(opts.headers || {});
      headers.set("Accept", "text/event-stream");
      if (opts.lastEventId != null && opts.lastEventId !== "") {
        headers.set("Last-Event-ID", String(opts.lastEventId));
      }
      const connect = async () => {
        const requestHeaders = new Headers(headers);
        const token = await this.bootstrapToken(false);
        requestHeaders.set("Authorization", "Bearer " + token);
        return global.fetch(this.url(path), {
          method: "GET",
          headers: requestHeaders,
          signal,
          cache: "no-store",
        });
      };
      let response = await connect();
      if (response.status === 401) {
        // §3.1：401 单次刷新 token 重试（与 request() 的 401 重试语义一致）。
        // 必须整体重查壳连接：只清 this.token 时，缓存的连接描述符会把旧
        // injectedToken/端口/实例再注入一遍，重试必然二次 401（core 重启场景）。
        this.resetCoreConnection();
        response = await connect();
      }
      if (!response.ok || !response.body) {
        const body = await response.text().catch(() => "");
        const error = new ApiError(
          "事件流连接失败（HTTP " + response.status + "）" + (body ? ": " + body : ""),
          { status: response.status, body: body, traceId: response.headers.get("x-trace-id") }
        );
        if (response.status === 401) this.emit(false, { error: error });
        throw error;
      }
      this.emit(true, { status: response.status, path: path });
      if (onOpen) {
        try { onOpen(response.status); } catch (_) { /* onOpen 失败不影响流 */ }
      }
      if (!onEvent) return response;
      return ApiClient.parseSseStream(response.body, onEvent, signal);
    }

    // §3.1：text/event-stream 逐帧解析。跨 chunk 缓冲，按空行分帧；多行 data
    // 按规范拼接；忽略注释帧；id 行回传给调用方作 Last-Event-ID 续传依据。
    static async parseSseStream(body, onEvent, signal) {
      const reader = body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      const dispatch = (rawFrame) => {
        const frame = rawFrame.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
        let eventName = "message";
        const dataLines = [];
        let id = null;
        for (const line of frame.split("\n")) {
          if (!line || line.startsWith(":")) continue; // 注释/空行
          if (line.startsWith("event:")) eventName = line.slice(6).trim();
          else if (line.startsWith("data:")) dataLines.push(line.slice(5).replace(/^ /, ""));
          else if (line.startsWith("id:")) id = line.slice(3).trim();
        }
        if (eventName === "message" && !dataLines.length && id == null) return;
        onEvent({ event: eventName, data: dataLines.join("\n"), id: id });
      };
      try {
        for (;;) {
          if (signal && signal.aborted) {
            const abortError = new Error("Aborted");
            abortError.name = "AbortError";
            throw abortError;
          }
          const chunk = await reader.read();
          if (chunk.done) break;
          buf += decoder.decode(chunk.value, { stream: true });
          let idx;
          while ((idx = buf.indexOf("\n\n")) >= 0) {
            const frame = buf.slice(0, idx);
            buf = buf.slice(idx + 2);
            dispatch(frame);
          }
        }
      } finally {
        try { reader.releaseLock(); } catch (_) { /* 流已关闭 */ }
      }
    }
  }

  global.OwoApiError = ApiError;
  global.OwoApiClient = ApiClient;
  global.OwoApi = global.OwoApi || new ApiClient(global.OWO_API_BASE || "");
})(typeof window !== "undefined" ? window : globalThis);

if (typeof module !== "undefined" && module.exports) {
  module.exports = { ApiClient: globalThis.OwoApiClient, ApiError: globalThis.OwoApiError };
}
