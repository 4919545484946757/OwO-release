import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { ApiClient } from "../core/api-client.js";

const here = fileURLToPath(new URL(".", import.meta.url));

test("业务脚本不绕过统一 API 客户端", () => {
  const webRoot = join(here, "..");
  const files = [];
  const visit = (dir) => {
    for (const name of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, name.name);
      if (name.isDirectory() && !name.name.startsWith("tests")) visit(path);
      else if (name.isFile() && name.name.endsWith(".js")) files.push(path);
    }
  };
  visit(webRoot);
  const offenders = files.filter((path) => !path.endsWith(join("core", "api-client.js")) && /fetch\s*\(/.test(readFileSync(path, "utf8")));
  assert.deepEqual(offenders, [], "只有 core/api-client.js 可以直接访问网络");
});

test("工作台三态视图由单一控制器管理并在启动前加载", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const index = readFileSync(join(here, "../index.html"), "utf8");
  assert.match(index, /core\/workbench-view\.js/);
  const viewScript = index.search(/<script src="core\/workbench-view\.js(?:\?[^"]*)?"><\/script>/);
  assert.ok(viewScript >= 0 && viewScript < index.indexOf('<script src="app.js"></script>'));
  assert.match(app, /window\.OwoWorkbenchView\.create\(/);
  assert.match(app, /workbenchView\.showTools\(visible, preserveRoute\)/);
  assert.match(app, /workbenchView\.showSettings\(visible\)/);
});

test("请求自动带 token，401 只刷新并重试一次", async () => {
  const calls = [];
  const original = global.fetch;
  let protectedCalls = 0;
  let tokenRequests = 0;
  global.fetch = async (url, options = {}) => {
    calls.push({ url, options });
    if (String(url).endsWith("/auth/token")) {
      tokenRequests += 1;
      return new Response(JSON.stringify({ token: tokenRequests === 1 ? "token-a" : "token-b" }), { status: 200 });
    }
    protectedCalls += 1;
    if (protectedCalls === 1) return new Response("expired", { status: 401 });
    return new Response(JSON.stringify({ ok: true }), { status: 200, headers: { "content-type": "application/json" } });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    assert.deepEqual(await client.get("/sessions"), { ok: true });
    assert.equal(calls.filter((call) => call.url.endsWith("/auth/token")).length, 2);
    assert.equal(calls.filter((call) => call.url.endsWith("/sessions")).length, 2);
    assert.equal(calls[1].options.headers.get("Authorization"), "Bearer token-a");
    assert.equal(calls[3].options.headers.get("Authorization"), "Bearer token-b");
  } finally {
    global.fetch = original;
  }
});

test("公开健康检查不触发 bearer token 引导", async () => {
  const original = global.fetch;
  const calls = [];
  global.fetch = async (url, options = {}) => {
    calls.push({ url: String(url), options });
    return new Response(JSON.stringify({ healthy: true, version: "0.7" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:8529");
    assert.deepEqual(await client.get("/health", { public: true }), { healthy: true, version: "0.7" });
    assert.equal(calls.length, 1);
    assert.ok(calls[0].url.endsWith("/health"));
    assert.equal(calls[0].options.headers.get("Authorization"), null);
  } finally {
    global.fetch = original;
  }
});

test("桌面配对门拒绝普通浏览器时显示可操作的原因", async () => {
  const original = global.fetch;
  global.fetch = async () => new Response(
    JSON.stringify({ error: "desktop pairing required", code: "auth/pairing_required/not_retryable" }),
    { status: 403, headers: { "content-type": "application/json" } }
  );
  try {
    const client = new ApiClient("http://127.0.0.1:8529");
    await assert.rejects(client.get("/sessions"), (error) => {
      assert.equal(error.status, 403);
      assert.equal(error.code, "auth/pairing_required/not_retryable");
      assert.match(error.message, /Electron 工作台/);
      return true;
    });
  } finally {
    global.fetch = original;
  }
});

test("浏览器预览的 token 引导 404 显示 Electron 连接指引并保留诊断", async () => {
  const original = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  delete global.__TAURI_INTERNALS__;
  global.fetch = async (url) => String(url).endsWith("/auth/token")
    ? new Response("static preview has no auth route", { status: 404 })
    : new Response("unexpected route", { status: 404 });
  try {
    const client = new ApiClient("http://127.0.0.1:8766");
    await assert.rejects(client.get("/sessions"), (error) => {
      assert.equal(error.status, 404);
      assert.equal(error.code, "auth/bootstrap_route_unavailable");
      assert.match(error.message, /浏览器预览没有连接 Electron 核心/);
      assert.equal(error.body, "static preview has no auth route");
      return true;
    });
  } finally {
    global.fetch = original;
    if (originalTauri !== undefined) global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("上传和下载复用同一鉴权路径", async () => {
  const original = global.fetch;
  const seen = [];
  global.fetch = async (url, options = {}) => {
    seen.push({ url, options });
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "token" }), { status: 200 });
    return String(url).endsWith("/download")
      ? new Response("zip", { status: 200 })
      : new Response(JSON.stringify({ text: "ok" }), { status: 200 });
  };
  try {
    const client = new ApiClient("");
    const upload = await client.upload("/upload", new Blob(["wav"]), "audio/wav");
    const download = await client.download("/download");
    assert.equal(upload.text, "ok");
    assert.equal(await download.text(), "zip");
    assert.equal(seen.filter((item) => item.url === "/upload")[0].options.headers.get("Content-Type"), "audio/wav");
    assert.equal(seen.filter((item) => item.url === "/download")[0].options.headers.get("Authorization"), "Bearer token");
  } finally {
    global.fetch = original;
  }
});

test("Tauri 桌面端先向壳询问核心连接，携带实例身份与配对证明引导 token", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  const commands = [];
  const seen = [];
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      commands.push(command);
      if (command === "get_core_connection") {
        return {
          port: 6071,
          instanceId: "a1b2c3d4e5f60718293a4b5c6d7e8f90",
          pairing: "0123456789abcdef0123456789abcdef",
          apiVersion: "0.7",
          pid: 1234,
          buildId: "abc123",
          state: "ready",
        };
      }
      return "0123456789abcdef0123456789abcdef";
    },
  };
  global.fetch = async (url, options = {}) => {
    seen.push({ url, options });
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "token" }), { status: 200 });
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.get("/sessions");
    assert.deepEqual(commands, ["get_core_connection"], "配对证明改由连接描述符提供，无需再问 desktop_pairing");
    assert.equal(String(seen[0].url), "http://127.0.0.1:6071/auth/token", "基址必须切换到壳报告的动态端口");
    assert.equal(seen[0].options.headers.get("x-owo-desktop-instance"), "a1b2c3d4e5f60718293a4b5c6d7e8f90");
    assert.equal(seen[0].options.headers.get("X-Owo-Desktop-Pairing"), "0123456789abcdef0123456789abcdef");
    // §8.1：ledger 来源标签——web 出口一律自带 x-owo-client（含 token 引导请求）。
    assert.equal(seen[0].options.headers.get("x-owo-client"), "web");
    assert.deepEqual(global.__owoCoreDiagnostics, {
      state: "ready",
      port: 6071,
      pid: 1234,
      buildId: "abc123",
      // §6.1.4：旧壳不带 expectedBuildId → 归一为 null（比对降级为不可用，不误报）。
      expectedBuildId: null,
      // §4.6：同一份旧壳夹具也不带 generation → 归一为 null，台账据此显示"壳未上报"。
      generation: null,
      apiVersion: "0.7",
      instanceId: "a1b2c3d4e5f60718293a4b5c6d7e8f90",
    });
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("R3-B §3.4：非 ready 的连接快照不得被永久缓存（否则稳定码永远读不到）", async () => {
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  let queries = 0;
  // 壳的真实时序：第一次查询时核心还在启动（无码），之后才进 failed 带稳定码。
  const responses = [
    { state: "starting", attempt: 0, logPath: "C:\\logs\\desktop-core-1.log" },
    { state: "failed", errorCode: "storage/not_writable", message: "存储错误：数据目录不可写", logPath: "C:\\logs\\desktop-core-1.log" },
  ];
  global.__TAURI_INTERNALS__ = {
    invoke: async () => {
      const value = responses[Math.min(queries, responses.length - 1)];
      queries += 1;
      return value;
    },
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    const first = await client.ensureCoreConnection();
    assert.equal(first.state, "starting");
    assert.equal(global.__owoCoreDiagnostics.errorCode, undefined, "启动期本来就没有错误码（不得编造）");
    // 关键断言：陈旧快照不得被复用——否则错误卡只能渲染默认三出口，用户看不到真因。
    const second = await client.ensureCoreConnection();
    assert.equal(queries, 2, "非 ready 必须允许再查（ready 才长期缓存）");
    assert.equal(second.errorCode, "storage/not_writable");
    assert.equal(global.__owoCoreDiagnostics.errorCode, "storage/not_writable", "诊断面必须携带稳定码（UI 按码渲染动作）");
    // 在途合并仍要成立：并发两次调用只发一次 IPC。
    const before = queries;
    const [a, b] = await Promise.all([client.ensureCoreConnection(), client.ensureCoreConnection()]);
    assert.equal(queries, before + 1, "在途请求必须单飞合并，不得变成 IPC 轮询风暴");
    assert.equal(a.errorCode, b.errorCode);
  } finally {
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§6.1.4 诊断对象同时暴露 expected/actual build id（不一致时供错误页消费）", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "get_core_connection") {
        return {
          port: 6074,
          instanceId: "inst-mismatch",
          pairing: "0123456789abcdef0123456789abcdef",
          apiVersion: "0.7",
          pid: 4321,
          buildId: "actual-core-build",
          // 壳编译期期望值 ≠ core 实际上报值：诊断必须两个都带上，
          // 是否判定为不匹配由错误页/恢复流程消费方决定。
          expectedBuildId: "shell-expected-build",
          state: "ready",
        };
      }
      throw new Error("unexpected command " + command);
    },
  };
  global.fetch = async (url) => {
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "token" }), { status: 200 });
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.get("/sessions");
    assert.deepEqual(global.__owoCoreDiagnostics, {
      state: "ready",
      port: 6074,
      pid: 4321,
      buildId: "actual-core-build",
      expectedBuildId: "shell-expected-build",
      apiVersion: "0.7",
      // §4.6：代际由壳上报；这份桩描述符没带该字段 → 归一为 null（不是 0）。
      generation: null,
      instanceId: "inst-mismatch",
    });
    assert.notEqual(global.__owoCoreDiagnostics.buildId, global.__owoCoreDiagnostics.expectedBuildId);
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§4 壳注入 token：桌面模式冷启动零 /auth/token 请求，重连后失效", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  const seen = [];
  let connectionGeneration = 0;
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "get_core_connection") {
        connectionGeneration += 1;
        return {
          port: 6070 + connectionGeneration,
          instanceId: "inst-" + connectionGeneration,
          pairing: "0123456789abcdef0123456789abcdef",
          token: "injected-token-" + connectionGeneration,
          state: "ready",
        };
      }
      throw new Error("unexpected command " + command);
    },
  };
  global.fetch = async (url, options = {}) => {
    seen.push({ url: String(url), options });
    if (String(url).endsWith("/auth/token")) {
      return new Response(JSON.stringify({ token: "bootstrap-token" }), { status: 200 });
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.get("/sessions");
    assert.ok(
      !seen.some((item) => item.url.endsWith("/auth/token")),
      "壳注入 token 后不得再请求 /auth/token（§4 冷启动 ≤5）"
    );
    assert.equal(seen[0].options.headers.get("Authorization"), "Bearer injected-token-1");
    // 重连（core 被壳重启）：注入凭据必须失效并重新向壳取新 token。
    client.resetCoreConnection();
    await client.get("/sessions");
    assert.equal(seen.filter((i) => i.url.includes("127.0.0.1:6072")).length, 1, "重查连接后基址切换到新实例");
    assert.equal(seen.filter((i) => i.url.endsWith("/sessions"))[1].options.headers.get("Authorization"), "Bearer injected-token-2");
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§8.2 第5条：401 必须整体重查壳连接（旧注入 token 不得复用）", async () => {
  // core 每次启动换发 bearer。若 401 重试只清 this.token，injectedToken 会把
  // 上一代 token 再注入一遍 → 二次 401 → 界面永久"未授权"。这条测试锁死修复。
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  const seen = [];
  const commands = [];
  let connectionGeneration = 0;
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      commands.push(command);
      if (command === "get_core_connection") {
        connectionGeneration += 1;
        return {
          port: 6110 + connectionGeneration,
          instanceId: "inst-" + connectionGeneration,
          pairing: "0123456789abcdef0123456789abcdef",
          token: "injected-token-" + connectionGeneration,
          state: "ready",
        };
      }
      throw new Error("unexpected command " + command);
    },
  };
  global.fetch = async (url, options = {}) => {
    const text = String(url);
    seen.push({ url: text, options });
    if (text.includes(":6111")) {
      return new Response("expired", { status: 401 }); // 上一代端口：token 已换发
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    const result = await client.get("/sessions");
    assert.deepEqual(result, { ok: true }, "401 重试后应成功");
    assert.equal(
      commands.filter((c) => c === "get_core_connection").length,
      2,
      "401 后必须重新向壳查询连接（而非只清本地 token）"
    );
    assert.ok(
      !seen.some((item) => item.url.endsWith("/auth/token")),
      "桌面模式重连不得回落到公开引导端点"
    );
    const retry = seen[seen.length - 1];
    assert.ok(retry.url.includes(":6112"), `重试应打到新一代端口：${retry.url}`);
    assert.equal(retry.options.headers.get("Authorization"), "Bearer injected-token-2");
    assert.equal(
      seen.filter((item) => item.options.headers.get("x-owo-client")).length,
      seen.length,
      "每次尝试（含重试）都必须带来源标签"
    );
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§8.2 第5条：core 重启换端口后，网络失败必须触发重查连接并重试成功", async () => {
  // 旧端口上的 fetch 是**网络错误**（不是 401）：只修 401 分支时运行期重启会永久失联。
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  const seen = [];
  const commands = [];
  let generation = 0;
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      commands.push(command);
      if (command === "get_core_connection") {
        generation += 1;
        return {
          port: 6210 + generation,
          instanceId: "gen-" + generation,
          pairing: "0123456789abcdef0123456789abcdef",
          token: "tok-" + generation,
          state: "ready",
        };
      }
      throw new Error("unexpected command " + command);
    },
  };
  global.fetch = async (url, options = {}) => {
    const text = String(url);
    seen.push({ url: text, auth: options.headers.get("Authorization") });
    if (text.includes(":6211")) {
      throw new TypeError("Failed to fetch"); // 上一代端口：进程已没了
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    const result = await client.get("/sessions");
    assert.deepEqual(result, { ok: true }, "网络失败后应经重查连接自愈");
    assert.ok(
      commands.filter((c) => c === "get_core_connection").length >= 2,
      `连接层失败必须重查壳连接，实际命令：${commands.join(",")}`
    );
    assert.equal(seen[seen.length - 1].url, "http://127.0.0.1:6212/sessions");
    assert.equal(seen[seen.length - 1].auth, "Bearer tok-2", "重试必须带新一代 bearer");
    assert.equal(client.networkFailures, 0, "成功后网络失败计数归零");
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§8.2：POST 网络失败必须上抛且不得自动重放（避免重复回合/写入副作用）", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  let fetches = 0;
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      if (command === "get_core_connection") {
        return {
          port: 6410,
          instanceId: "post-once",
          pairing: "0123456789abcdef0123456789abcdef",
          token: "tok-post",
          state: "ready",
        };
      }
      throw new Error("unexpected command " + command);
    },
  };
  global.fetch = async () => {
    fetches += 1;
    throw new TypeError("Failed to fetch");
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await assert.rejects(
      () => client.post("/session/s1/turn", { prompt: "重复副作用防护" }),
      "POST 网络失败必须上抛由 UI 决定是否重发"
    );
    assert.equal(fetches, 1, "POST 网络失败不得自动重试（服务端可能已执行）");
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("§8.2：幂等请求遇到 429/5xx 自动重试一次，POST 不重放", async () => {
  const originalFetch = global.fetch;
  let getCalls = 0;
  let postCalls = 0;
  global.fetch = async (url, options = {}) => {
    if (String(url).endsWith("/auth/token")) {
      return new Response(JSON.stringify({ token: "tok-503" }), { status: 200 });
    }
    const method = String((options && options.method) || "GET").toUpperCase();
    if (method === "POST") {
      postCalls += 1;
      return new Response("busy", { status: 503 });
    }
    getCalls += 1;
    if (getCalls === 1) return new Response("busy", { status: 503 });
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    assert.deepEqual(await client.get("/sessions"), { ok: true });
    assert.equal(getCalls, 2, "GET 503 应自动重试一次并成功");
    await assert.rejects(
      () => client.post("/session/s1/turn", { prompt: "瞬时 503" }),
      /503/
    );
    assert.equal(postCalls, 1, "POST 503 不得自动重放（避免重复回合）");
  } finally {
    global.fetch = originalFetch;
  }
});

test("§8.2 第5条：核心始终不可达时网络失败只重试一次并受冷却约束（不得风暴重查）", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  let connectionQueries = 0;
  global.__TAURI_INTERNALS__ = {
    invoke: async () => {
      connectionQueries += 1;
      return {
        port: 6310,
        instanceId: "down",
        pairing: "0123456789abcdef0123456789abcdef",
        token: "tok-down",
        state: "ready",
      };
    },
  };
  global.fetch = async () => {
    throw new TypeError("Failed to fetch");
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    // 第 1 次请求：失败 → 重查连接 → 再失败 → 上抛（不得吞错，UI 才能显示未连接）。
    await assert.rejects(() => client.get("/sessions"), "核心不可达必须上抛");
    const afterFirst = connectionQueries;
    assert.ok(afterFirst >= 1, "首次失败应至少重查一次连接");
    // 第 2、3 次请求：处于重查冷却窗口内，不得再向壳发起查询（风暴放大防护）。
    await assert.rejects(() => client.get("/sessions"));
    await assert.rejects(() => client.get("/skills"));
    assert.equal(connectionQueries, afterFirst, "冷却期内不得重复重查壳连接");
    // 尝试计数：req1 两次尝试（重查前 1 次 + 重查后 1 次，重查时归零）、
    // req2/req3 各一次 → 稳定为 3。用它验证"失败被如实记录，不被吞掉"。
    assert.equal(client.networkFailures, 3, "网络失败计数供诊断读取（重查后归零再累计）");
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

test("核心未就绪时保留原基址，并暴露启动诊断供恢复流程消费", async () => {
  const originalFetch = global.fetch;
  const originalTauri = global.__TAURI_INTERNALS__;
  const originalDiagnostics = global.__owoCoreDiagnostics;
  const commands = [];
  global.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      commands.push(command);
      if (command === "get_core_connection") {
        return { port: 0, state: "failed", errorCode: "core/spawn_failed", message: "核心服务启动失败", logPath: "C:\\logs\\core.log" };
      }
      return "0123456789abcdef0123456789abcdef";
    },
  };
  global.fetch = async (url, options = {}) => {
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "token" }), { status: 200 });
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.get("/sessions");
    assert.ok(commands.includes("desktop_pairing"), "连接未就绪时回退到 desktop_pairing 证明");
    assert.ok(String(commands[0]) === "get_core_connection", "首个命令仍是连接询问");
    assert.deepEqual(global.__owoCoreDiagnostics, {
      state: "failed",
      errorCode: "core/spawn_failed",
      message: "核心服务启动失败",
      logPath: "C:\\logs\\core.log",
      // §4.6：壳未上报重启计数时归一为 null（UI 必须说"壳未上报"，不得显示 0）。
      attempt: null,
      generation: null,
    });
    // 恢复前重查连接：缓存与基址必须复位
    client.resetCoreConnection();
    assert.equal(client.baseUrl, "http://127.0.0.1:4096");
    assert.equal(client.coreInstanceId, null);
  } finally {
    global.fetch = originalFetch;
    global.__owoCoreDiagnostics = originalDiagnostics;
    if (originalTauri === undefined) delete global.__TAURI_INTERNALS__;
    else global.__TAURI_INTERNALS__ = originalTauri;
  }
});

function sseBody(chunks) {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk));
      controller.close();
    },
  });
}

test("§3.1 openEventStream：Bearer 头 + 401 单次刷新重试（token 不进 URL）", async () => {
  const original = global.fetch;
  const seen = [];
  let protectedCalls = 0;
  let tokenRequests = 0;
  global.fetch = async (url, options = {}) => {
    seen.push({ url: String(url), options });
    if (String(url).endsWith("/auth/token")) {
      tokenRequests += 1;
      return new Response(JSON.stringify({ token: tokenRequests === 1 ? "tok-a" : "tok-b" }), { status: 200 });
    }
    protectedCalls += 1;
    if (protectedCalls === 1) return new Response("expired", { status: 401 });
    return new Response(sseBody([]), { status: 200, headers: { "content-type": "text/event-stream" } });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.openEventStream("/events/stream", {});
    assert.equal(protectedCalls, 2, "401 后必须单次刷新重试");
    const [first, second] = seen.filter((item) => item.url.endsWith("/events/stream"));
    assert.equal(first.options.headers.get("Authorization"), "Bearer tok-a");
    assert.equal(second.options.headers.get("Authorization"), "Bearer tok-b");
    assert.equal(second.options.headers.get("Accept"), "text/event-stream");
    assert.ok(!String(second.url).includes("tok"), "token 不得出现在 URL 中");
  } finally {
    global.fetch = original;
  }
});

test("§3.1 openEventStream：Last-Event-ID 头续传 + SSE 逐帧解析（id/多行/注释帧）", async () => {
  const original = global.fetch;
  const inner = JSON.stringify({ domain: "mcp", version: 3 });
  const frameData = JSON.stringify({ seq: 1, kind: "invalidate", data: inner });
  global.fetch = async (url) => {
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "tok" }), { status: 200 });
    const body = sseBody([
      ': keep-alive\n\n',
      "event: invalidate\nid: 1\ndata: " + frameData + "\n\n",
      "event: heartbeat\nid: 2\ndata: {\"type\":\"heartbeat\"}\n\n",
    ]);
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    const frames = [];
    let opened = 0;
    await client.openEventStream("/events/stream", {
      lastEventId: 7,
      onOpen: () => { opened += 1; },
      onEvent: (frame) => frames.push(frame),
    });
    assert.equal(opened, 1, "流建立时必须回调 onOpen");
    assert.equal(frames.length, 2, "注释帧不得分发");
    assert.equal(frames[0].event, "invalidate");
    assert.equal(frames[0].id, "1");
    assert.equal(JSON.parse(frames[0].data).data, inner);
    assert.equal(frames[1].event, "heartbeat");
    assert.equal(frames[1].id, "2");
    // 复核续传头确实随请求发送。
  } finally {
    global.fetch = original;
  }
});

test("§4.6 壳诊断归一：generation/attempt 必须透出，旧壳缺失时保持 null", () => {
  const ready = ApiClient.buildCoreDiagnostics({
    state: "ready",
    port: 4096,
    pid: 12,
    instanceId: "inst",
    generation: 5,
  });
  assert.equal(ready.generation, 5, "ready 时也要带代际，否则台账日常看不到重启事实");
  assert.equal(ApiClient.buildCoreDiagnostics({ state: "ready", port: 1, pid: 2 }).generation, null,
    "旧壳未上报必须显式为 null，不得补 0（0 与未上报是两回事）");
  const starting = ApiClient.buildCoreDiagnostics({
    state: "starting",
    attempt: 2,
    generation: 1,
    errorCode: "core/exited",
    message: "core 退出",
    logPath: "C:\\logs\\core.log",
  });
  assert.equal(starting.attempt, 2, "同代自动重启次数与代际是两个事实，都要透出");
  assert.equal(starting.generation, 1);
  assert.equal(starting.errorCode, "core/exited");
});

test("§3.1 openEventStream：Last-Event-ID 头随请求发送", async () => {
  const original = global.fetch;
  const seen = [];
  global.fetch = async (url, options = {}) => {
    seen.push({ url: String(url), options });
    if (String(url).endsWith("/auth/token")) return new Response(JSON.stringify({ token: "tok" }), { status: 200 });
    return new Response(sseBody([]), { status: 200, headers: { "content-type": "text/event-stream" } });
  };
  try {
    const client = new ApiClient("http://127.0.0.1:4096");
    await client.openEventStream("/events/stream", { lastEventId: 42 });
    const request = seen.find((item) => item.url.endsWith("/events/stream"));
    assert.equal(request.options.headers.get("Last-Event-ID"), "42", "断线续传必须经 Last-Event-ID 头");
  } finally {
    global.fetch = original;
  }
});

// ---------- 桌面认证回归（真实故障） ----------
//
// 故障：窗口刷新后界面立刻退化为「服务未连接」+「列表加载失败：Failed to fetch」。
// 链路：刷新瞬间核心可能仍处于 starting → get_core_connection 返回 state!=ready
// → 若把这个 null 缓存下来，页面此后永远拿不到 pairing/instanceId
// → /auth/token 恒 403 → 业务请求恒 401。
// 症状特征是「首次打开正常、刷新后必坏」，最容易被误判成后端挂了。

test("index.html 引入 core/api-client.js（认证头唯一实现）", () => {
  const index = readFileSync(join(here, "../index.html"), "utf8");
  // 契约测试「业务脚本不绕过统一 API 客户端」规定全站仅 core/api-client.js 可 fetch，
  // 它承担桌面壳的 pairing / instance 两道认证门；不引入则 token 永远拿不到。
  assert.match(index, /<script src="core\/api-client\.js(?:\?[^"]*)?"><\/script>/);
});

test("index.html 不再按 __TAURI_INTERNALS__ 把 baseUrl 钉死到硬编码端口", () => {
  const index = readFileSync(join(here, "../index.html"), "utf8");
  // 壳用 `serve --port 0` 随机分配核心端口（实测出现过 10720/15201/27644/33885/38552/63797…）。
  // Electron 侧也提供 __TAURI_INTERNALS__（M11 兼容桥），据此判定"在壳内"就会把
  // baseUrl 钉死在 4096 之类的常量上 → 请求打错端口 → 全线 401。
  assert.doesNotMatch(
    index,
    /OWO_API_BASE\s*=\s*window\.OWO_API_BASE\s*\|\|\s*\([^)]*__TAURI_INTERNALS__[^)]*\)/,
    "不得依据 __TAURI_INTERNALS__ 回落硬编码端口"
  );
  assert.match(index, /window\.OWO_API_BASE\s*=\s*window\.OWO_API_BASE\s*\|\|\s*"";/);
});

test("工具面板把公开 /health 请求作为公开请求发送", () => {
  const domain = readFileSync(join(here, "../app-domain.js"), "utf8");
  const helpers = /function panelHelpers\(\) \{([\s\S]*?)\n\}/.exec(domain);
  assert.ok(helpers);
  assert.match(helpers[1], /get\(path, options\)[\s\S]*?path === "\/health"[\s\S]*?requestOptions\.public = true[\s\S]*?return api\(path, requestOptions\)/);
});

test("app.js 的 token 引导复用 ApiClient，不自带实例/配对实现", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  // 认证头只在 core/api-client.js 实现；app.js 通过 OwoApi.ensureCoreConnection 复用。
  assert.match(app, /window\.OwoApi/);
  assert.match(app, /ensureCoreConnection/);
  // 不得出现"把失败结果缓存下来"的形态：未就绪必须能重试。
  assert.doesNotMatch(
    app,
    /shellConnectionPromise\s*=\s*null;[\s\S]{0,80}state\s*===\s*"starting"/,
    "不得缓存未就绪的连接信息"
  );
});

test("发送时无会话会自动新建，而不是提示后放弃", () => {
  const domain = readFileSync(join(here, "../app-domain.js"), "utf8");
  // 用户按回车的意图是"把这句话说出去"，不该要求他先点一次「新建对话」。
  assert.doesNotMatch(
    domain,
    /async function sendPrompt\(\)\s*\{\s*if \(!state\.sessionId\)\s*\{\s*addMessage\("system",\s*"请先新建或选择一个会话"\)/,
    "sendPrompt 不应在无会话时只提示而不新建"
  );
  const sendBody = /async function sendPrompt\(\)\s*\{([\s\S]*?)\n\}/.exec(domain);
  assert.ok(sendBody, "app-domain.js 必须提供唯一 sendPrompt 实现");
  assert.match(sendBody[1], /if \(!state\.sessionId\)\s*\{[\s\S]{0,1000}await promptSessionStart\.run\(\(\) => newSession\(\)\)/);
});

test("侧栏项目分组可折叠且折叠态持久化", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const domain = readFileSync(join(here, "../app-domain.js"), "utf8");
  const css = readFileSync(join(here, "../style.css"), "utf8");
  assert.match(app, /collapsedGroups/, "折叠状态集合");
  assert.match(domain, /owo\.collapsedGroups/, "折叠状态持久化到 localStorage");
  assert.match(domain, /codex-group-caret/, "折叠箭头");
  assert.match(css, /\.codex-group-caret/, "折叠箭头样式");
  // 折叠只在「非搜索」期间生效，搜索应临时展开全部。
  assert.match(domain, /if \(query\) li\.classList\.toggle\("hidden", !li\.textContent\.toLowerCase\(\)\.includes\(query\)\)/);
});

// ---------- 首启门回归（真实故障：配好模型仍发不出） ----------
//
// 故障：用户在设置页填好端点与密钥、「测试连接」通过，发送按钮却仍禁用、
// 提示条"先连接你自己的模型服务"一直挂着。根因是前后端字段脱节：
//   * 前端 modelGateMissing() 读 settings.provider_ready / settings.provider.base_url；
//   * 服务端 Settings 结构里**没有 provider 段**，provider 信息只以 runtime 投影存在，
//     settings_get 也从不返回 provider_ready。
// 于是前端读的永远是 undefined → 恒判"未就绪"。
// 修法两侧配套：core 加 ProviderSettings + settings_get 投影 provider 段/provider_ready；
// 前端改读 runtime.{endpoint_kind,credential_source,provider_ready}。

test("首启门读服务端真实字段，不读不存在的 settings.provider_ready", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const body = app.slice(app.indexOf("function modelGateMissing"), app.indexOf("function updateModelGate"));
  assert.ok(body.length > 0, "应能找到 modelGateMissing 实现");
  // 必须优先看 runtime 段（服务端 effective_runtime_config 的产物）。
  assert.match(body, /settings\.runtime|runtime\s*=\s*settings\.runtime/);
  assert.match(body, /endpoint_kind\s*===\s*"local"/, "本地端点无需凭据，应放行");
  assert.match(body, /credential_source/);
});

test("本地端点（Ollama）不因缺密钥被拦", () => {
  const app = readFileSync(join(here, "../app.js"), "utf8");
  const body = app.slice(app.indexOf("function modelGateMissing"), app.indexOf("function updateModelGate"));
  // endpoint_kind=local 即放行：Ollama 之类本地服务不需要 API 密钥。
  assert.doesNotMatch(
    body,
    /endpoint_kind\s*===\s*"local"[\s\S]{0,40}return true/,
    "本地端点不得被判为缺配置"
  );
});
