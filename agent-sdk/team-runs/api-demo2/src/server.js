'use strict';

const http = require('node:http');
const { TextDecoder } = require('node:util');

/** 业务错误码 → HTTP 状态码（docs/api-design.md §5） */
const ERROR_CODES = Object.freeze({
  NOT_FOUND: 404,
  METHOD_NOT_ALLOWED: 405,
  UNSUPPORTED_MEDIA_TYPE: 415,
  INVALID_JSON: 400,
  PAYLOAD_TOO_LARGE: 413,
  INTERNAL_ERROR: 500,
});

const DEFAULT_MAX_BODY_BYTES = 1048576; // 1 MiB
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';

/** 已注册路由：pathname → 允许的 method 集合 */
const ROUTES = new Map([
  ['/health', new Set(['GET'])],
  ['/echo', new Set(['POST'])],
]);

const ERROR_MESSAGES = Object.freeze({
  NOT_FOUND: 'Resource not found',
  METHOD_NOT_ALLOWED: 'Method not allowed for this resource',
  UNSUPPORTED_MEDIA_TYPE: 'Content-Type must be application/json',
  INVALID_JSON: 'Request body must be valid JSON',
  PAYLOAD_TOO_LARGE: 'Request body exceeds the 1048576 byte limit',
  INTERNAL_ERROR: 'Internal server error',
});

/** 严格 UTF-8 解码器：非法字节序列直接抛错（对应 INVALID_JSON） */
const strictDecoder = new TextDecoder('utf-8', { fatal: true });

function sendJson(res, statusCode, payload, extraHeaders) {
  const body = Buffer.from(JSON.stringify(payload), 'utf8');
  const headers = {
    'Content-Type': JSON_CONTENT_TYPE,
    'Content-Length': String(body.length),
  };
  if (extraHeaders) {
    for (const [name, value] of Object.entries(extraHeaders)) {
      headers[name] = value;
    }
  }
  if (res.headersSent || res.writableEnded) return;
  res.writeHead(statusCode, headers);
  res.end(body);
}

function sendError(res, code, extraHeaders) {
  const statusCode = ERROR_CODES[code] || 500;
  const known = Object.prototype.hasOwnProperty.call(ERROR_CODES, code);
  const errorCode = known ? code : 'INTERNAL_ERROR';
  const message = known ? ERROR_MESSAGES[code] : ERROR_MESSAGES.INTERNAL_ERROR;
  sendJson(res, known ? statusCode : 500, { error: { code: errorCode, message } }, extraHeaders);
}

/** 取 MIME 主类型：截取 ';' 之前部分，trim + 小写 */
function mainMimeType(contentType) {
  if (typeof contentType !== 'string') return '';
  return contentType.split(';')[0].trim().toLowerCase();
}

/**
 * 解析 pathname，任何异常 URL 都按未注册路径处理（→ 404）。
 */
function pathnameOf(req) {
  try {
    return new URL(req.url, 'http://localhost').pathname;
  } catch {
    return null;
  }
}

function handleEcho(req, res, options) {
  const maxBodyBytes = options.maxBodyBytes;
  const now = options.now;

  // 415 判定在解析之前（但 413 优先级更高，见 data 事件累计字节判断）
  if (mainMimeType(req.headers['content-type']) !== 'application/json') {
    req.resume(); // 丢弃请求体，避免连接挂起
    sendError(res, 'UNSUPPORTED_MEDIA_TYPE');
    return;
  }

  const chunks = [];
  let received = 0;
  let settled = false;

  const finish413 = () => {
    if (settled) return;
    settled = true;
    sendError(res, 'PAYLOAD_TOO_LARGE', { Connection: 'close' });
    // 响应后销毁连接
    res.on('finish', () => req.destroy());
  };

  const finish400 = () => {
    if (settled) return;
    settled = true;
    sendError(res, 'INVALID_JSON');
  };

  req.on('data', (chunk) => {
    if (settled) return;
    received += chunk.length;
    if (received > maxBodyBytes) {
      chunks.length = 0;
      finish413();
      return;
    }
    chunks.push(chunk);
  });

  req.on('aborted', () => {
    settled = true;
    req.destroy();
  });

  req.on('error', () => {
    settled = true;
    req.destroy();
  });

  req.on('end', () => {
    if (settled) return;
    settled = true;

    if (received === 0) {
      sendError(res, 'INVALID_JSON');
      return;
    }

    let text;
    try {
      text = strictDecoder.decode(Buffer.concat(chunks));
    } catch {
      sendError(res, 'INVALID_JSON');
      return;
    }

    let value;
    try {
      value = JSON.parse(text);
    } catch {
      sendError(res, 'INVALID_JSON');
      return;
    }

    let receivedAt;
    try {
      receivedAt = new Date(now()).toISOString();
    } catch {
      sendError(res, 'INTERNAL_ERROR');
      return;
    }

    sendJson(res, 200, { receivedAt, body: value });
  });
}

/**
 * 创建未监听的 HTTP 服务。
 * @param {{maxBodyBytes?: number, now?: () => Date}} [options]
 * @returns {import('node:http').Server}
 */
function createServer(options = {}) {
  const resolved = {
    maxBodyBytes:
      Number.isInteger(options.maxBodyBytes) && options.maxBodyBytes >= 0
        ? options.maxBodyBytes
        : DEFAULT_MAX_BODY_BYTES,
    now: typeof options.now === 'function' ? options.now : () => new Date(),
  };

  return http.createServer((req, res) => {
    try {
      const pathname = pathnameOf(req);
      const allowed = pathname === null ? undefined : ROUTES.get(pathname);

      if (!allowed) {
        req.resume();
        sendError(res, 'NOT_FOUND');
        return;
      }

      const method = typeof req.method === 'string' ? req.method.toUpperCase() : '';
      if (!allowed.has(method)) {
        req.resume();
        sendError(res, 'METHOD_NOT_ALLOWED', { Allow: [...allowed].join(', ') });
        return;
      }

      if (pathname === '/health') {
        req.resume();
        sendJson(res, 200, { ok: true });
        return;
      }

      handleEcho(req, res, resolved);
    } catch {
      if (!res.headersSent) {
        try {
          sendError(res, 'INTERNAL_ERROR');
        } catch {
          res.destroy();
        }
      } else {
        res.destroy();
      }
    }
  });
}

module.exports = { createServer, ERROR_CODES, DEFAULT_MAX_BODY_BYTES };
