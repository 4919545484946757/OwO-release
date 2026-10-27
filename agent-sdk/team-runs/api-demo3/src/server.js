'use strict';

const http = require('node:http');

const MAX_BODY_BYTES = 1048576; // 1 MiB，见 docs/api-design.md 4.1 / 6
const JSON_TYPE = 'application/json; charset=utf-8';

/**
 * 写出统一 JSON 响应。
 * @param {http.ServerResponse} res
 * @param {number} status
 * @param {unknown} payload
 * @param {{extraHeaders?: Record<string,string|number>}} [options]
 */
function sendJson(res, status, payload, options = {}) {
  const body = Buffer.from(JSON.stringify(payload), 'utf8');
  if (res.headersSent || res.writableEnded) return;
  res.writeHead(status, {
    'Content-Type': JSON_TYPE,
    'Content-Length': String(body.length),
    ...(options.extraHeaders || {}),
  });
  res.end(body);
}

function sendError(res, status, code, message, options) {
  sendJson(res, status, { error: code, message }, options);
}

/**
 * 读取请求体：按 Buffer 拼接后再整体 UTF-8 解码（不按 chunk 边界切字符串）。
 * 超过上限时停止累积并返回 tooLarge=true，同时继续消费流以保持 keep-alive 连接可用。
 * @param {http.IncomingMessage} req
 * @param {number} limit
 * @returns {Promise<{text: string, size: number, tooLarge: boolean}>}
 */
function readBody(req, limit = MAX_BODY_BYTES) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let tooLarge = false;

    req.on('data', (chunk) => {
      size += chunk.length;
      if (tooLarge) return; // 已超限：丢弃后续字节，但仍需读完流
      if (size > limit) {
        tooLarge = true;
        chunks.length = 0;
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => {
      if (tooLarge) {
        resolve({ text: '', size, tooLarge: true });
        return;
      }
      resolve({ text: Buffer.concat(chunks).toString('utf8'), size, tooLarge: false });
    });
    req.on('error', (err) => reject(err));
    req.on('aborted', () => reject(new Error('request aborted')));
  });
}

async function handleHealth(req, res) {
  req.resume(); // 丢弃可能的 body，保证 keep-alive 连接可复用
  sendJson(res, 200, { ok: true });
}

async function handleEcho(req, res) {
  let body;
  try {
    body = await readBody(req);
  } catch {
    // 客户端中途断开等：无法安全写响应，直接返回
    if (!res.writableEnded) res.destroy();
    return;
  }

  if (body.tooLarge) {
    sendError(res, 413, 'payload_too_large', `request body exceeds ${MAX_BODY_BYTES} bytes`, {
      extraHeaders: { Connection: 'close' },
    });
    return;
  }

  if (body.text.trim() === '') {
    sendError(res, 400, 'bad_request', 'request body must be valid JSON (empty body)');
    return;
  }

  let parsed;
  try {
    parsed = JSON.parse(body.text);
  } catch {
    sendError(res, 400, 'bad_request', 'request body is not valid JSON');
    return;
  }

  // 顶层任意合法 JSON 值（对象/数组/字面量/null）均原样回显
  sendJson(res, 200, parsed);
}

/**
 * 路由：基于 URL.pathname 精确比较（忽略 query，大小写敏感，不归一化尾斜杠）。
 * @param {http.IncomingMessage} req
 * @param {http.ServerResponse} res
 */
async function handleRequest(req, res) {
  try {
    const url = new URL(req.url || '/', 'http://localhost');
    const pathname = url.pathname;
    const method = (req.method || 'GET').toUpperCase();

    if (method === 'GET' && pathname === '/health') {
      await handleHealth(req, res);
      return;
    }
    if (method === 'POST' && pathname === '/echo') {
      await handleEcho(req, res);
      return;
    }

    // 未知路径或方法不匹配（不使用 405）
    req.resume();
    sendError(res, 404, 'not_found', `no route for ${method} ${pathname}`);
  } catch {
    req.resume();
    sendError(res, 500, 'internal_error', 'unexpected server error');
  }
}

function createServer() {
  const server = http.createServer((req, res) => {
    req.on('error', () => {
      /* 客户端断开，忽略，避免未处理的 error 事件导致进程退出 */
    });
    res.on('error', () => {
      /* 响应侧断开，忽略 */
    });
    handleRequest(req, res).catch(() => {
      if (!res.headersSent) {
        sendError(res, 500, 'internal_error', 'unexpected server error');
      } else if (!res.writableEnded) {
        res.destroy();
      }
    });
  });
  return server;
}

function startServer() {
  const port = Number.parseInt(process.env.PORT || '3000', 10);
  const host = process.env.HOST || '0.0.0.0';
  const server = createServer();

  server.listen(port, host, () => {
    const address = server.address();
    const actualPort = address && typeof address === 'object' ? address.port : port;
    console.log(`listening on http://${host}:${actualPort}`);
  });

  const shutdown = () => {
    server.close(() => process.exit(0));
    // 兜底：连接迟迟不关闭时强制退出
    setTimeout(() => process.exit(0), 5000).unref();
  };
  process.on('SIGTERM', shutdown);
  process.on('SIGINT', shutdown);

  return server;
}

if (require.main === module) {
  startServer();
}

module.exports = {
  createServer,
  startServer,
  handleRequest,
  readBody,
  sendJson,
  sendError,
  MAX_BODY_BYTES,
  JSON_TYPE,
};
