'use strict';

const http = require('node:http');

const MAX_BODY_BYTES = 1048576;
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8';

/**
 * Send a JSON response with the contract Content-Type.
 * @param {import('node:http').ServerResponse} res
 * @param {number} statusCode
 * @param {unknown} payload
 */
function sendJson(res, statusCode, payload) {
  const body = Buffer.from(JSON.stringify(payload), 'utf8');
  res.writeHead(statusCode, {
    'Content-Type': JSON_CONTENT_TYPE,
    'Content-Length': body.length,
  });
  res.end(body);
}

/**
 * Send an error response: { "error": "CODE", "message": "..." }.
 * @param {import('node:http').ServerResponse} res
 * @param {number} statusCode
 * @param {string} code
 * @param {string} message
 */
function sendError(res, statusCode, code, message) {
  sendJson(res, statusCode, { error: code, message });
}

/**
 * Extract the raw pathname (query string stripped) without decoding.
 * @param {string} rawUrl
 * @returns {string}
 */
function pathnameOf(rawUrl) {
  const url = rawUrl || '/';
  const hashIndex = url.indexOf('#');
  const withoutHash = hashIndex === -1 ? url : url.slice(0, hashIndex);
  const queryIndex = withoutHash.indexOf('?');
  return queryIndex === -1 ? withoutHash : withoutHash.slice(0, queryIndex);
}

/**
 * Check whether a Content-Type header value denotes application/json.
 * Case-insensitive, parameters (e.g. "; charset=utf-8") allowed.
 * @param {string | null | undefined} headerValue
 * @returns {boolean}
 */
function isJsonContentType(headerValue) {
  if (typeof headerValue !== 'string') return false;
  const mediaType = headerValue.split(';')[0].trim().toLowerCase();
  return mediaType === 'application/json';
}

/**
 * Read the request body with a 1 MiB cap.
 * Resolves { tooLarge: true } as soon as the limit is exceeded (remaining bytes
 * are not consumed), or { tooLarge: false, buffer } on normal completion.
 * @param {import('node:http').IncomingMessage} req
 * @returns {Promise<{ tooLarge: boolean, buffer?: Buffer }>}
 */
function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let total = 0;
    let settled = false;

    const finish = (value) => {
      if (settled) return;
      settled = true;
      req.removeListener('data', onData);
      req.removeListener('end', onEnd);
      req.removeListener('error', onError);
      resolve(value);
    };

    const fail = (err) => {
      if (settled) return;
      settled = true;
      req.removeListener('data', onData);
      req.removeListener('end', onEnd);
      req.removeListener('error', onError);
      reject(err);
    };

    function onData(chunk) {
      total += chunk.length;
      if (total > MAX_BODY_BYTES) {
        finish({ tooLarge: true });
        return;
      }
      chunks.push(chunk);
    }

    function onEnd() {
      finish({ tooLarge: false, buffer: Buffer.concat(chunks, total) });
    }

    function onError(err) {
      fail(err);
    }

    req.on('data', onData);
    req.on('end', onEnd);
    req.on('error', onError);
  });
}

/**
 * Handle a single request following the API contract.
 * Never throws for request-level errors; falls back to 500 INTERNAL_ERROR.
 * @param {import('node:http').IncomingMessage} req
 * @param {import('node:http').ServerResponse} res
 */
async function handleRequest(req, res) {
  try {
    const pathname = pathnameOf(req.url);
    const method = (req.method || 'GET').toUpperCase();
    const isHealth = pathname === '/health';
    const isEcho = pathname === '/echo';
    const declaredLength = Number(req.headers['content-length']);

    // Priority 1: payload size (declared or streamed) beats every other outcome.
    if (Number.isFinite(declaredLength) && declaredLength > MAX_BODY_BYTES) {
      sendError(res, 413, 'PAYLOAD_TOO_LARGE', 'Request body exceeds 1048576 bytes.');
      req.resume();
      return;
    }

    // Priority 2: unknown route.
    if (!isHealth && !isEcho) {
      sendError(res, 404, 'NOT_FOUND', 'Resource not found.');
      req.resume();
      return;
    }

    // Priority 3: method not allowed.
    if ((isHealth && method !== 'GET') || (isEcho && method !== 'POST')) {
      sendError(res, 405, 'METHOD_NOT_ALLOWED', 'Method not allowed for this resource.');
      req.resume();
      return;
    }

    if (isHealth) {
      req.resume();
      sendJson(res, 200, { ok: true });
      return;
    }

    // POST /echo
    // Priority 4: media type.
    if (!isJsonContentType(req.headers['content-type'])) {
      sendError(res, 415, 'UNSUPPORTED_MEDIA_TYPE', 'Content-Type must be application/json.');
      req.resume();
      return;
    }

    // Priority 5: body parsing / type.
    const body = await readBody(req);
    if (body.tooLarge) {
      sendError(res, 413, 'PAYLOAD_TOO_LARGE', 'Request body exceeds 1048576 bytes.');
      req.resume();
      return;
    }

    const text = body.buffer.toString('utf8');
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch {
      sendError(res, 400, 'INVALID_JSON', 'Request body is not valid JSON.');
      return;
    }

    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
      sendError(res, 400, 'INVALID_BODY_TYPE', 'Request body must be a JSON object.');
      return;
    }

    sendJson(res, 200, { echo: parsed });
  } catch {
    if (!res.headersSent) {
      sendError(res, 500, 'INTERNAL_ERROR', 'Unexpected server error.');
    } else {
      try {
        res.end();
      } catch {
        /* ignore */
      }
    }
  }
}

/**
 * Create the HTTP server (useful for tests and for starting on port 0).
 * @returns {import('node:http').Server}
 */
function createServer() {
  return http.createServer((req, res) => {
    // Reject client-level protocol errors without crashing the process.
    req.on('error', () => {
      try {
        res.destroy();
      } catch {
        /* ignore */
      }
    });
    handleRequest(req, res);
  });
}

const DEFAULT_PORT = 3000;
const HOST = '127.0.0.1';

/**
 * Start listening using PORT (default 3000) on 127.0.0.1.
 * @param {{ port?: number | string, host?: string }} [options]
 * @returns {Promise<import('node:http').Server>}
 */
function start(options = {}) {
  const envPort = process.env.PORT;
  const rawPort = options.port !== undefined ? options.port : envPort;
  const port = rawPort === undefined || rawPort === '' ? DEFAULT_PORT : Number(rawPort);
  const host = options.host || HOST;
  const server = createServer();
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, host, () => {
      server.removeListener('error', reject);
      resolve(server);
    });
  });
}

module.exports = {
  MAX_BODY_BYTES,
  JSON_CONTENT_TYPE,
  createServer,
  handleRequest,
  start,
};

if (require.main === module) {
  start()
    .then((server) => {
      const address = server.address();
      const shownPort = typeof address === 'object' && address ? address.port : DEFAULT_PORT;
      process.stdout.write(`api-service listening on http://${HOST}:${shownPort}\n`);
    })
    .catch((err) => {
      process.stderr.write(`api-service failed to start: ${err && err.message}\n`);
      process.exitCode = 1;
    });
}
