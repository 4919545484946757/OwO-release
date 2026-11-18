"use strict";
/* 提示词增强（✨ 一键优化）的模型调用层。
 *
 * 交互对照 WorkBuddy「增强提示词」：用户写好草稿 → 点一下闪光按钮 → 输入框里的
 * 原文被直接替换为优化后的提示词 → 再点一下还原原文。这里只负责「把草稿发给
 * 当前配置的模型、拿回改写文本」，不负责任何 UI，也不创建会话/回合。
 *
 * 纯逻辑（buildRequest / parseCompletion / cleanOutput）不依赖 electron，
 * 可由 node --test 直接断言；网络请求通过 requestImpl 注入以便测试。
 */
const http = require("node:http");
const https = require("node:https");

const SYSTEM_PROMPT = [
  "你是一名「提示词改写专家」。把用户输入的原始提示词改写成更清晰、更具体、更完整、可直接交给 AI 执行的版本。",
  "硬性规则：",
  "1. 必须使用与原文相同的语言；原文中文就全程中文，原文英文就全程英文，不得翻译。",
  "2. 严格保留原意、原始诉求和所有已给出的具体信息（数字、路径、名称、引用、代码片段、专有名词），不得删除或改写它们。",
  "3. 只补全缺失的维度：任务目标、背景与对象、约束与边界、输出形式与结构、验收标准。按需选择，不必五项全加；已经写清楚的不要重复啰嗦。",
  "4. 不得编造任何事实、数据、文件名、引用、版本或结论；需要但不确定的背景，写成「如缺少关键信息，先向用户确认」这样的执行要求，不要替用户填空。",
  "5. 不增加无关内容，不寒暄，不解释改动，不加标题前缀，不加引号，不要用 Markdown 代码块包裹。",
  "6. 直接输出改写后的提示词正文，除此之外一个字都不要输出。",
].join("\n");

const MAX_INPUT = 4000;
const MAX_OUTPUT = 8000;
const MAX_RESPONSE_BYTES = 512 * 1024;
// 一键增强不该让用户干等：模型 5 秒内没回就由前端降级到本地补全。
const TIMEOUT_MS = 5000;

function cleanOutput(raw, original) {
  if (!raw) return "";
  let text = String(raw).trim();
  // 模型偶尔会自作主张套一层代码块或加前缀，统一剥掉。
  text = text.replace(/^```[A-Za-z0-9_-]*\s*\n?/, "").replace(/\n?```\s*$/, "").trim();
  text = text.replace(/^(优化|增强|改写)(后|过)?的?提示词\s*[:：]\s*/i, "").trim();
  text = text.replace(/^(Enhanced|Improved|Optimized)\s+prompt\s*:\s*/i, "").trim();
  text = text.replace(/^["“]([\s\S]*)["”]$/, "$1").trim();
  if (text.length > MAX_OUTPUT) text = text.slice(0, MAX_OUTPUT);
  // 明显偷懒/跑偏的结果不能回填：太短、或把原文核心内容整个丢掉。
  if (!text || text === original) return "";
  if (original.length >= 12 && text.length < original.length * 0.8) return "";
  return text;
}

function buildRequest(text, options = {}) {
  const source = String(text == null ? "" : text).trim();
  if (!source) throw Error("prompt/empty");
  if (source.length > MAX_INPUT) throw Error("prompt/too_long");
  const base = String(options.baseUrl || "").trim().replace(/\/+$/, "");
  if (!base) throw Error("prompt/missing_base_url");
  let url;
  try { url = new URL(base); } catch { throw Error("prompt/invalid_base_url"); }
  if (!["http:", "https:"].includes(url.protocol)) throw Error("prompt/invalid_base_url");
  const anthropic = options.apiFormat === "anthropic";
  const endpoint = base + (anthropic ? "/v1/messages" : "/chat/completions");
  const headers = { "content-type": "application/json" };
  if (anthropic) {
    headers["x-api-key"] = String(options.apiKey || "");
    headers["anthropic-version"] = "2023-06-01";
  } else if (options.apiKey) {
    headers.authorization = "Bearer " + String(options.apiKey);
  }
  const body = anthropic
    ? {
        model: String(options.model || ""),
        max_tokens: 2048,
        system: SYSTEM_PROMPT,
        messages: [{ role: "user", content: source.slice(0, MAX_INPUT) }],
      }
    : {
        model: String(options.model || ""),
        temperature: 0.3,
        max_tokens: 2048,
        stream: false,
        messages: [
          { role: "system", content: SYSTEM_PROMPT },
          { role: "user", content: source.slice(0, MAX_INPUT) },
        ],
      };
  return { endpoint, headers, body, anthropic };
}

function parseCompletion(body, anthropic) {
  let parsed = body;
  if (typeof body === "string") {
    try { parsed = JSON.parse(body); } catch { return ""; }
  }
  if (!parsed || typeof parsed !== "object") return "";
  if (anthropic) {
    const parts = Array.isArray(parsed.content) ? parsed.content : [];
    return parts.filter((item) => item && item.type === "text").map((item) => String(item.text || "")).join("\n");
  }
  const choice = Array.isArray(parsed.choices) ? parsed.choices[0] : null;
  const message = choice && (choice.message || choice.delta);
  return message ? String(message.content || "") : "";
}

function defaultRequest({ endpoint, headers, body }) {
  return new Promise((resolve, reject) => {
    let url;
    try { url = new URL(endpoint); } catch (error) { reject(error); return; }
    const payload = JSON.stringify(body);
    const req = (url.protocol === "https:" ? https : http).request(url, {
      method: "POST",
      headers: { ...headers, "content-length": Buffer.byteLength(payload) },
      timeout: TIMEOUT_MS,
    }, (response) => {
      let bytes = 0, data = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        bytes += Buffer.byteLength(chunk);
        if (bytes > MAX_RESPONSE_BYTES) { req.destroy(Error("prompt/response_too_large")); return; }
        data += chunk;
      });
      response.on("end", () => resolve({ status: response.statusCode || 0, body: data }));
    });
    req.on("timeout", () => req.destroy(Error("prompt/timeout")));
    req.on("error", reject);
    req.end(payload);
  });
}

async function enhancePrompt(options = {}) {
  const original = String(options.text == null ? "" : options.text).trim();
  const request = buildRequest(original, options);
  const impl = typeof options.requestImpl === "function" ? options.requestImpl : defaultRequest;
  const response = await impl(request);
  const status = Number(response && response.status) || 0;
  if (status < 200 || status >= 300) {
    const detail = String((response && response.body) || "").slice(0, 200);
    throw Error("prompt/http_" + status + (detail ? ":" + detail : ""));
  }
  // 非流式接口偶尔也会返回 SSE 分片；取最后一个有效的 data: 行兜底。
  let raw = String((response && response.body) || "");
  if (raw.includes("data:")) {
    const lines = raw.split(/\r?\n/).filter((line) => line.trim().startsWith("data:"));
    const last = lines.reverse().find((line) => line.trim() !== "data: [DONE]");
    if (last) raw = last.replace(/^\s*data:\s*/, "");
  }
  const text = cleanOutput(parseCompletion(raw, request.anthropic), original);
  if (!text) throw Error("prompt/empty_result");
  return { text, model: String(options.model || "") };
}

module.exports = {
  SYSTEM_PROMPT,
  MAX_INPUT,
  MAX_OUTPUT,
  cleanOutput,
  buildRequest,
  parseCompletion,
  enhancePrompt,
};
