'use strict';

/**
 * 节点健康检查与回显服务
 * 实现依据：docs/api-design.md（v2）
 * 运行时：仅使用 Node.js 内置模块（http / url），零第三方依赖。
 *
 * 导出（契约 9.1，不可改名）：createServer() / start(env?) / MAX_BODY_BYTES(=1048576)
 */

const http = require('node:http');

const MAX_BODY_BYTES = 1024 * 1024; // 1 MiB，严格大于才拒绝
const DEFAULT_PORT = 3000;
const DEFAULT_HOST = '127.0.0.1';
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';

const ROUTES = new Map([
  ['/health', new Set(['GET'])],
  ['/echo', new Set(['POST'])],
]);

class HttpError extends Error {
  constructor(status, code, message, details) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

function sendJson(res, status, payload, extraHeaders) {
  if (res.writableEnded) return;
  const body = JSON.stringify(payload);
  const headers = {
    'Content-Type': JSON_CONTENT_TYPE,
    'Content-Length': Buffer.byteLength(body),
    ...extraHeaders,
  };
  res.writeHead(status, headers);
  res.end(body);
}

function sendError(res, status, code, message, pathname, extraHeaders, details) {
  const error = { code, message, path: pathname };
  if (details !== undefined) error.details = details;
  const headers = { ...(extraHeaders || {}) };
  if (status === 405) headers.Allow = allowHeaderFor(pathname);
  sendJson(res, status, { error }, headers);
}

function isJsonContentType(rawHeader) {
  // 重复头取第一个；忽略大小写与参数
  const raw = Array.isArray(rawHeader) ? rawHeader[0] : rawHeader;
  if (typeof raw !== 'string') return false;
  const mime = raw.split(';')[0].trim().toLowerCase();
  return mime === 'application/json';
}

function allowHeaderFor(pathname) {
  const methods = ROUTES.get(pathname);
  return methods ? [...methods].join(', ') : undefined;
}

/**
 * 按 pathname 解析请求目标：忽略 query string，
 * 并兼容绝对 URI 形式的 request-target。
 */
function resolvePathname(reqUrl) {
  const raw = String(reqUrl || '/');
  try {
    return new URL(raw, 'http://127.0.0.1').pathname;
  } catch {
    // 非法 URL：退回未解码片段，后续按未匹配路径处理（404 JSON），不抛异常
    const qIndex = raw.indexOf('?');
    const cut = qIndex >= 0 ? raw.slice(0, qIndex) : raw;
    return cut || '/';
  }
}

function collectBody(req, res) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let settled = false;

    const finish = (fn, arg) => {
      if (settled) return;
      settled = true;
      req.removeListener('data', onData);
      req.removeListener('end', onEnd);
      req.removeListener('error', onError);
      req.removeListener('aborted', onAborted);
      res.removeListener('close', onResClose);
      fn(arg);
    };

    function onData(chunk) {
      size += chunk.length;
      if (size > MAX_BODY_BYTES) {
        // 超限立即中断（413），不再继续缓冲
        finish(reject, new HttpError(
          413,
          'PAYLOAD_TOO_LARGE',
          'Request body exceeds the 1 MiB limit',
          { limitBytes: MAX_BODY_BYTES },
        ));
        req.pause();
        return;
      }
      chunks.push(chunk);
    }

    function onEnd() {
      finish(resolve, Buffer.concat(chunks, size));
    }

    function onError(err) {
      finish(reject, err);
    }

    function onAborted() {
      finish(reject, new HttpError(400, 'INVALID_JSON', 'Request aborted before body completed'));
    }

    function onResClose() {
      if (!settled && !res.writableEnded) {
        finish(reject, new HttpError(400, 'INVALID_JSON', 'Client disconnected'));
      }
    }

    req.on('data', onData);
    req.on('end', onEnd);
    req.on('error', onError);
    req.on('aborted', onAborted);
    res.on('close', onResClose);
  });
}

async function handle(req, res) {
  const pathname = resolvePathname(req.url);
  const methods = ROUTES.get(pathname);

  // 判定顺序遵循文档第 7 节
  // 1) 路径未命中 → 404（不读取请求体）
  if (!methods) {
    sendError(res, 404, 'NOT_FOUND', 'Route not found', pathname);
    return;
  }

  // 2) 路径命中但方法不支持 → 405 + Allow（不读取请求体）
  if (!methods.has(req.method)) {
    sendError(res, 405, 'METHOD_NOT_ALLOWED', `Method ${req.method} not allowed on ${pathname}`, pathname);
    return;
  }

  if (pathname === '/health') {
    // GET /health：忽略请求体，仅 {"ok":true}
    sendJson(res, 200, { ok: true });
    return;
  }

  // 以下为 POST /echo
  // 3) Content-Type 校验 → 415
  if (!isJsonContentType(req.headers['content-type'])) {
    sendError(res, 415, 'UNSUPPORTED_MEDIA_TYPE', 'Content-Type must be application/json', pathname);
    return;
  }

  // 4) 累积请求体，超 1 MiB → 413
  let buffer;
  try {
    buffer = await collectBody(req, res);
  } catch (err) {
    if (res.writableEnded) return;
    if (err instanceof HttpError) {
      sendError(res, err.status, err.code, err.message, pathname, undefined, err.details);
      // 超限后主动断开剩余请求体；由此触发的客户端 ECONNRESET/EPIPE 为允许行为
      if (!req.complete) req.destroy();
      return;
    }
    sendError(res, 500, 'INTERNAL_ERROR', 'Internal server error', pathname);
    return;
  }

  // 5) 空体 / 仅空白 / 非法 JSON → 400（顶层 null/0/false 不是空体）
  const text = buffer.toString('utf8');
  if (text.trim().length === 0) {
    sendError(res, 400, 'INVALID_JSON', 'Request body is empty', pathname);
    return;
  }

  let payload;
  try {
    payload = JSON.parse(text);
  } catch {
    sendError(res, 400, 'INVALID_JSON', 'Request body is not valid JSON', pathname);
    return;
  }

  // 6) 成功回显：显式包一层 { echo: <解析值> }，保证 echo 键必现
  sendJson(res, 200, { echo: payload });
}

function createServer() {
  return http.createServer((req, res) => {
    handle(req, res).catch(() => {
      if (!res.writableEnded) {
        sendError(res, 500, 'INTERNAL_ERROR', 'Internal server error', '/', undefined, undefined);
      }
    });
  });
}

function resolvePort(rawPort) {
  const port = Number.parseInt(rawPort ?? String(DEFAULT_PORT), 10);
  if (!Number.isInteger(port) || port < 0 || port > 65535) return DEFAULT_PORT;
  return port;
}

function start(env = process.env) {
  const host = env.HOST || DEFAULT_HOST;
  const port = resolvePort(env.PORT);
  const server = createServer();
  server.listen(port, host, () => {
    const addr = server.address();
    process.stdout.write(`listening on http://${addr.address}:${addr.port}\n`);
  });
  const shutdown = () => server.close(() => process.exit(0));
  process.on('SIGTERM', shutdown);
  process.on('SIGINT', shutdown);
  return server;
}

if (require.main === module) {
  start();
}

module.exports = { createServer, start, handle, MAX_BODY_BYTES, ROUTES };
