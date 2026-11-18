import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const E = require("../src/main/prompt-enhancer.js");
const main = readFileSync(new URL("../src/main/main.js", import.meta.url), "utf8");
const preload = readFileSync(new URL("../src/main/preload.js", import.meta.url), "utf8");

const OK = (content) => ({
  status: 200,
  body: JSON.stringify({ choices: [{ message: { content } }] }),
});

test("system prompt preserves intent, language and forbids invented facts", () => {
  assert.match(E.SYSTEM_PROMPT, /相同的语言/);
  assert.match(E.SYSTEM_PROMPT, /严格保留原意/);
  assert.match(E.SYSTEM_PROMPT, /不得编造/);
  assert.match(E.SYSTEM_PROMPT, /直接输出改写后的提示词正文/);
});

test("buildRequest targets a single OpenAI-compatible completion, not a session turn", () => {
  const r = E.buildRequest("帮我写个邮件", {
    baseUrl: "https://api.openai.com/v1/", model: "gpt-4o-mini", apiKey: "sk-test",
  });
  assert.equal(r.endpoint, "https://api.openai.com/v1/chat/completions");
  assert.equal(r.headers.authorization, "Bearer sk-test");
  assert.equal(r.headers["content-type"], "application/json");
  assert.equal(r.body.stream, false);
  assert.equal(r.body.messages.length, 2);
  assert.equal(r.body.messages[0].role, "system");
  assert.equal(r.body.messages[1].content, "帮我写个邮件");
  assert.doesNotMatch(r.endpoint, /\/session|\/turn/);
});

test("buildRequest supports Anthropic message shape", () => {
  const r = E.buildRequest("hello", {
    baseUrl: "https://api.anthropic.com", model: "claude", apiKey: "k", apiFormat: "anthropic",
  });
  assert.equal(r.endpoint, "https://api.anthropic.com/v1/messages");
  assert.equal(r.headers["x-api-key"], "k");
  assert.equal(r.headers["anthropic-version"], "2023-06-01");
  assert.equal(r.anthropic, true);
  assert.equal(r.body.system, E.SYSTEM_PROMPT);
  assert.equal(r.body.messages.length, 1);
});

test("buildRequest rejects empty, oversized and invalid endpoints", () => {
  assert.throws(() => E.buildRequest("  ", { baseUrl: "https://x/v1", model: "m" }), /prompt\/empty/);
  assert.throws(() => E.buildRequest("x".repeat(E.MAX_INPUT + 1), { baseUrl: "https://x/v1", model: "m" }), /prompt\/too_long/);
  assert.throws(() => E.buildRequest("x", { baseUrl: "", model: "m" }), /prompt\/missing_base_url/);
  assert.throws(() => E.buildRequest("x", { baseUrl: "ftp://x/v1", model: "m" }), /prompt\/invalid_base_url/);
});

test("parseCompletion handles OpenAI, Anthropic and garbage", () => {
  assert.equal(E.parseCompletion(JSON.parse(OK("优化文本").body), false), "优化文本");
  assert.equal(E.parseCompletion(OK("优化文本").body, false), "优化文本");
  assert.equal(E.parseCompletion({ content: [{ type: "text", text: "A" }, { type: "text", text: "B" }] }, true), "A\nB");
  assert.equal(E.parseCompletion("not json", false), "");
  assert.equal(E.parseCompletion({}, false), "");
});

test("cleanOutput strips code fences and prefixes, rejects lazy rewrites", () => {
  const original = "帮我写个邮件";
  assert.equal(E.cleanOutput("优化后的提示词：帮我写一封给客户的道歉邮件，说明发货延迟，语气诚恳。", original),
    "帮我写一封给客户的道歉邮件，说明发货延迟，语气诚恳。");
  assert.equal(E.cleanOutput("```markdown\nRewrite this into a clear detailed brief with an explicit deliverable and acceptance criteria.\n```",
    "Rewrite this"), "Rewrite this into a clear detailed brief with an explicit deliverable and acceptance criteria.");
  assert.equal(E.cleanOutput(original, original), "", "与原文相同不能算优化");
  assert.equal(E.cleanOutput("嗯", "帮我写一封给客户的道歉邮件并说明原因"), "", "过短结果不能回填");
});

test("enhancePrompt returns the rewritten text on a successful non-streaming call", async () => {
  const result = await E.enhancePrompt({
    text: "帮我写个邮件", baseUrl: "https://example.test/v1", model: "m", apiKey: "k",
    requestImpl: async () => OK("帮我写一封给客户的道歉邮件，说明发货延迟，语气诚恳，控制在 300 字以内。"),
  });
  assert.equal(result.model, "m");
  assert.match(result.text, /道歉邮件/);
});

test("enhancePrompt also accepts SSE-shaped fragments", async () => {
  const frame = "data: " + JSON.stringify({ choices: [{ delta: { content: "把这段需求整理成包含目标、约束、输出格式与验收标准的完整提示词。" } }] });
  const result = await E.enhancePrompt({
    text: "整理需求", baseUrl: "https://example.test/v1", model: "m",
    requestImpl: async () => ({ status: 200, body: frame + "\n\ndata: [DONE]\n" }),
  });
  assert.match(result.text, /验收标准/);
});

test("enhancePrompt surfaces HTTP errors and empty results", async () => {
  await assert.rejects(
    E.enhancePrompt({ text: "x", baseUrl: "https://x/v1", model: "m", requestImpl: async () => ({ status: 401, body: "no key" }) }),
    /prompt\/http_401/
  );
  await assert.rejects(
    E.enhancePrompt({ text: "x", baseUrl: "https://x/v1", model: "m", requestImpl: async () => OK("x") }),
    /prompt\/empty_result/
  );
});

test("shell exposes one scoped IPC that never creates a session turn", () => {
  assert.match(main, /ipcMain\.handle\("prompt:enhance"/);
  assert.match(main, /promptEnhancer\.enhancePrompt/);
  assert.match(main, /prompt\/untrusted_frame/);
  assert.match(main, /shellCommands\.resolveModelConfig/);
  assert.match(preload, /enhancePrompt: input => ipcRenderer\.invoke\("prompt:enhance", input\)/);
});
