import test from "node:test";
import assert from "node:assert/strict";
import { createClient } from "../src/index.js";

const BASE = process.env.OWO_TS_SDK_BASE ?? "http://127.0.0.1:4097";
const client = createClient({ baseUrl: BASE });
// 集成用例需要真实 Daemon：未显式指定地址/未开启集成开关时跳过，
// 让 `npm test` 在纯离线环境也能给出确定的绿色结果。
const INTEGRATION_ENABLED =
  Boolean(process.env.OWO_TS_SDK_BASE) || process.env.OWO_TS_SDK_INTEGRATION === "1";

test("health 返回可用", { skip: !INTEGRATION_ENABLED }, async () => {
  const ok = await client.health();
  assert.equal(ok, true);
});

test("创建会话并列出（场景 4 集成）", { skip: !INTEGRATION_ENABLED }, async () => {
  // X03 鉴权链路：业务 API 需 bearer；/auth/token 为公开同源引导端点
  // （浏览器侧由 CORS 白名单限制跨源读取；Node 直连可正常完成自举配对）。
  const boot = await fetch(`${BASE}/auth/token`);
  assert.ok(boot.ok, `/auth/token 引导应可用：HTTP ${boot.status}`);
  const { token } = (await boot.json()) as { token?: string };
  assert.ok(token, "引导响应应携带 token");
  const authorized = createClient({
    baseUrl: BASE,
    headers: { Authorization: `Bearer ${token}` },
  });
  const session = await authorized.createSession({
    workspace: process.cwd(),
    system_prompt: "SDK 集成测试",
  });
  assert.ok(session.id, "应返回会话 id");
  assert.equal(session.mode, "agent", "默认创建 Agent 会话");
  const list = await authorized.GET("/sessions");
  assert.ok(list.response.ok);
  const sessions = (list.data ?? []) as Array<{ id: string }>;
  assert.ok(
    sessions.some((s) => s.id === session.id),
    "新会话应出现在列表",
  );
});

test("纯对话可以不选工作区创建隔离会话", { skip: !INTEGRATION_ENABLED }, async () => {
  const boot = await fetch(`${BASE}/auth/token`);
  const { token } = (await boot.json()) as { token?: string };
  assert.ok(token, "引导响应应携带 token");
  const authorized = createClient({
    baseUrl: BASE,
    headers: { Authorization: `Bearer ${token}` },
  });
  const session = await authorized.createSession({ mode: "chat" });
  assert.equal(session.mode, "chat");
  assert.ok(session.workspace, "服务端应返回隔离工作区");
  assert.ok(session.id, "应返回会话 id");
  const detail = await authorized.GET("/session/{id}", {
    params: { path: { id: session.id } },
  });
  assert.ok(detail.response.ok);
  assert.equal((detail.data as { mode?: string }).mode, "chat");
});
