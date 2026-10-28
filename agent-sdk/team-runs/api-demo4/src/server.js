'use strict';

/**
 * WorkSwarm Echo API — 零依赖实现（仅 node:http / node:url）
 * 契约来源：docs/api-design.md
 *
 * 路由：
 *   GET  /health -> 200 {"ok":true}
 *   POST /echo   -> 200 <请求体 JSON 解析值>
 *   其余         -> 404 NOT_FOUND
 *   非法 JSON    -> 400 INVALID_JSON
 *   未预期异常   -> 500 INTERNAL_ERROR
 */

const http = require('node:http');

const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';
const MAX_BODY_BYTES = 1024 * 1024; // 1 MiB
const DEFAULT_PORT = 3000;
const DEFAULT_HOST = '127.0.0.1';

/**
 * 从 req.url 中取出路径部分（忽略查询串）；不抛异常。
 * @param {string|undefined} url
 * @returns {string}
 */
function pathOf(url) {
  if (typeof url !== 'string' || url.length === 0) return '';
  const q = url.indexOf('?');
  return q === -1 ? url : url.slice(0, q);
}

/**
 * 统一发送 JSON 响应：显式设置 Content-Type 与 Content-Length 后 end()。
 * @param {http.ServerResponse} res
 * @param {number} statusCode
 * @param {unknown} payload
 */
function sendJson(res, statusCode, payload) {
  const body = JSON.stringify(payload);
  const buffer = Buffer.from(body, 'utf8');
  res.statusCode = statusCode;
  res.setHeader('Content-Type', JSON_CONTENT_TYPE);
  res.setHeader('Content-Length', String(buffer.length));
  res.end(buffer);
}

/**
 * @param {http.ServerResponse} res
 * @param {number} statusCode
 * @param {string} code
 * @param {string} message
 */
function sendError(res, statusCode, code, message) {
  sendJson(res, statusCode, { error: { code, message } });
}

/**
 * 收集请求体；超限或解码失败时回调错误，不共享任何可变全局状态。
 * @param {http.IncomingMessage} req
 * @returns {Promise<string>}
 */
function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let settled = false;

    const fail = (err) => {
      if (settled) return;
      settled = true;
      reject(err);
    };

    req.on('data', (chunk) => {
      if (settled) return;
      const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      size += buf.length;
      if (size > MAX_BODY_BYTES) {
        fail(new BodyError('Request body exceeds 1 MiB limit'));
        return;
      }
      chunks.push(buf);
    });

    req.on('end', () => {
      if (settled) return;
      settled = true;
      const buffer = Buffer.concat(chunks);
      // 严格 UTF-8 解码：非法字节序列视为非法输入
      try {
        resolve(new TextDecoder('utf-8', { fatal: true }).decode(buffer));
      } catch {
        reject(new BodyError('Request body is not valid UTF-8'));
      }
    });

    req.on('error', (err) => fail(err instanceof Error ? err : new Error(String(err))));
    req.on('aborted', () => fail(new BodyError('Request aborted')));
  });
}

/** 请求体相关的客户端错误（-> 400 INVALID_JSON） */
class BodyError extends Error {
  constructor(message) {
    super(message);
    this.name = 'BodyError';
  }
}

/**
 * 严格解析 JSON：拒绝 NaN/Infinity 等非标准字面量。
 * JSON.parse 本身已拒绝 NaN/Infinity/尾随逗号/注释/单引号，这里直接复用。
 * @param {string} text
 * @returns {{ok: true, value: unknown} | {ok: false, reason: string}}
 */
function parseStrictJson(text) {
  if (typeof text !== 'string' || text.trim() === '') {
    return { ok: false, reason: 'Request body is empty' };
  }
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch {
    return { ok: false, reason: 'Request body is not valid JSON' };
  }
}

/**
 * POST /echo 处理
 * @param {http.IncomingMessage} req
 * @param {http.ServerResponse} res
 */
async function handleEcho(req, res) {
  let text;
  try {
    text = await readBody(req);
  } catch (err) {
    if (err instanceof BodyError) {
      sendError(res, 400, 'INVALID_JSON', err.message || 'Request body is not valid JSON');
      return;
    }
    throw err;
  }

  const parsed = parseStrictJson(text);
  if (!parsed.ok) {
    sendError(res, 400, 'INVALID_JSON', parsed.reason);
    return;
  }
  sendJson(res, 200, parsed.value);
}

/**
 * 请求处理器（纯函数式：无跨请求可变状态），可直接被 node:test 调用。
 * @param {http.IncomingMessage} req
 * @param {http.ServerResponse} res
 */
function handler(req, res) {
  const path = pathOf(req.url);
  const method = (req.method || '').toUpperCase();

  // 快速收敛所有响应头，避免各分支遗漏
  Promise.resolve()
    .then(() => {
      if (path === '/health' && method === 'GET') {
        // 忽略任何请求体
        req.resume();
        sendJson(res, 200, { ok: true });
        return;
      }
      if (path === '/echo' && method === 'POST') {
        return handleEcho(req, res);
      }
      // 未知路径 / 方法不匹配 / 任意其他方法 -> 404（不用 405）
      req.resume();
      sendError(res, 404, 'NOT_FOUND', 'Route not found');
      return;
    })
    .catch(() => {
      if (res.headersSent) {
        res.destroy();
        return;
      }
      req.resume();
      sendError(res, 500, 'INTERNAL_ERROR', 'Internal server error');
    });
}

/**
 * 创建服务实例（便于测试使用 listen(0) 随机端口）。
 * @returns {http.Server}
 */
function createServer() {
  return http.createServer(handler);
}

/**
 * 启动服务。
 * @param {{port?: number, host?: string}} [options]
 * @returns {{server: http.Server, port: number}} 注意：port 在 listen 完成后才确定，需用 server.address()
 */
function start(options = {}) {
  const envPort = process.env.PORT;
  const parsedPort = envPort === undefined || envPort === '' ? NaN : Number(envPort);
  const port = options.port !== undefined
    ? options.port
    : (Number.isInteger(parsedPort) && parsedPort >= 0 ? parsedPort : DEFAULT_PORT);
  const host = options.host !== undefined ? options.host : DEFAULT_HOST;
  const server = createServer();
  server.listen(port, host);
  return { server, port };
}

if (require.main === module) {
  const { server, port } = start();
  server.on('listening', () => {
    const address = server.address();
    const actual = address && typeof address === 'object' ? address.port : port;
    process.stdout.write(`Echo API listening on http://${DEFAULT_HOST}:${actual}\n`);
  });
}

module.exports = {
  handler,
  createServer,
  start,
  parseStrictJson,
  pathOf,
  sendJson,
  sendError,
  MAX_BODY_BYTES,
  JSON_CONTENT_TYPE,
  DEFAULT_PORT,
  DEFAULT_HOST,
};
