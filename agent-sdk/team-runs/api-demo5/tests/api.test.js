'use strict';

const test = require('node:test');
const assert = require('node:assert');
const http = require('node:http');

const { createServer, MAX_BODY_BYTES, JSON_CONTENT_TYPE } = require('../src/server.js');

/**
 * Start the server on an ephemeral port (127.0.0.1).
 * @returns {Promise<{ server: import('node:http').Server, port: number, close: () => Promise<void> }>}
 */
async function startTestServer() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  return {
    server,
    port,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}

/**
 * Perform an HTTP request against 127.0.0.1:<port>.
 * @param {number} port
 * @param {{ method?: string, path?: string, headers?: Record<string, string>, body?: string | Buffer }} options
 * @returns {Promise<{ status: number, headers: Record<string, string | string[] | undefined>, text: string, json: any }>}
 */
function request(port, options = {}) {
  const { method = 'GET', path = '/', headers = {}, body } = options;
  return new Promise((resolve, reject) => {
    const req = http.request(
      { host: '127.0.0.1', port, method, path, headers, agent: false },
      (res) => {
        const chunks = [];
        res.on('data', (chunk) => chunks.push(chunk));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let json;
          try {
            json = JSON.parse(text);
          } catch {
            json = undefined;
          }
          resolve({ status: res.statusCode, headers: res.headers, text, json });
        });
      },
    );
    req.on('error', reject);
    if (body !== undefined) req.write(body);
    req.end();
  });
}

const JSON_HEADERS = { 'Content-Type': 'application/json' };

test('GET /health returns 200 {"ok":true} with JSON content type', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, { path: '/health' });
  assert.strictEqual(res.status, 200);
  assert.strictEqual(res.headers['content-type'], JSON_CONTENT_TYPE);
  assert.deepStrictEqual(res.json, { ok: true });
  assert.strictEqual(Object.keys(res.json).length, 1);
});

test('GET /health with query string still returns 200 {"ok":true}', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, { path: '/health?x=1' });
  assert.strictEqual(res.status, 200);
  assert.deepStrictEqual(res.json, { ok: true });
});

test('POST /echo echoes a nested JSON object', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const payload = { hello: 'world', nested: { a: [1, 2, 3], b: null }, flag: false };
  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: JSON_HEADERS,
    body: JSON.stringify(payload),
  });
  assert.strictEqual(res.status, 200);
  assert.strictEqual(res.headers['content-type'], JSON_CONTENT_TYPE);
  assert.deepStrictEqual(res.json, { echo: payload });
});

test('POST /echo accepts content type with charset and empty object', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: { 'Content-Type': 'Application/JSON; charset=utf-8' },
    body: '{}',
  });
  assert.strictEqual(res.status, 200);
  assert.deepStrictEqual(res.json, { echo: {} });
});

test('POST /echo with invalid JSON returns 400 INVALID_JSON', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: JSON_HEADERS,
    body: '{"broken": ',
  });
  assert.strictEqual(res.status, 400);
  assert.strictEqual(res.json.error, 'INVALID_JSON');
  assert.strictEqual(typeof res.json.message, 'string');
  assert.strictEqual(res.headers['content-type'], JSON_CONTENT_TYPE);
});

test('POST /echo with empty body returns 400 INVALID_JSON', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: JSON_HEADERS,
    body: '',
  });
  assert.strictEqual(res.status, 400);
  assert.strictEqual(res.json.error, 'INVALID_JSON');
});

test('POST /echo with non-object JSON returns 400 INVALID_BODY_TYPE', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  for (const body of ['[]', '"str"', '1', 'null', 'true']) {
    const res = await request(ctx.port, {
      method: 'POST',
      path: '/echo',
      headers: JSON_HEADERS,
      body,
    });
    assert.strictEqual(res.status, 400, `expected 400 for body ${body}`);
    assert.strictEqual(res.json.error, 'INVALID_BODY_TYPE', `for body ${body}`);
  }
});

test('POST /echo without JSON content type returns 415 UNSUPPORTED_MEDIA_TYPE', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: { 'Content-Type': 'text/plain' },
    body: '{"a":1}',
  });
  assert.strictEqual(res.status, 415);
  assert.strictEqual(res.json.error, 'UNSUPPORTED_MEDIA_TYPE');
});

test('unknown path returns 404 NOT_FOUND including trailing-slash variants', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  for (const path of ['/', '/nope', '/health/', '/echo/', '/health/extra']) {
    const res = await request(ctx.port, { path });
    assert.strictEqual(res.status, 404, `expected 404 for ${path}`);
    assert.strictEqual(res.json.error, 'NOT_FOUND', `for ${path}`);
    assert.strictEqual(typeof res.json.message, 'string');
    assert.strictEqual(res.headers['content-type'], JSON_CONTENT_TYPE);
  }
});

test('wrong method on known path returns 405 METHOD_NOT_ALLOWED', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const postHealth = await request(ctx.port, { method: 'POST', path: '/health' });
  assert.strictEqual(postHealth.status, 405);
  assert.strictEqual(postHealth.json.error, 'METHOD_NOT_ALLOWED');

  const getEcho = await request(ctx.port, { path: '/echo' });
  assert.strictEqual(getEcho.status, 405);
  assert.strictEqual(getEcho.json.error, 'METHOD_NOT_ALLOWED');
});

test('payload over 1 MiB returns 413 PAYLOAD_TOO_LARGE (declared and streamed)', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const oversized = 'x'.repeat(MAX_BODY_BYTES + 1);
  const declared = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: { ...JSON_HEADERS, 'Content-Length': String(Buffer.byteLength(oversized)) },
    body: oversized,
  });
  assert.strictEqual(declared.status, 413);
  assert.strictEqual(declared.json.error, 'PAYLOAD_TOO_LARGE');

  // Chunked body with no Content-Length, exceeding the cap while streaming.
  const chunked = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: { ...JSON_HEADERS, 'Transfer-Encoding': 'chunked' },
    body: oversized,
  });
  assert.strictEqual(chunked.status, 413);
  assert.strictEqual(chunked.json.error, 'PAYLOAD_TOO_LARGE');
});

test('body exactly at 1 MiB limit is accepted', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const prefix = '{"data":"';
  const suffix = '"}';
  const filler = 'y'.repeat(MAX_BODY_BYTES - prefix.length - suffix.length);
  const body = prefix + filler + suffix;
  assert.strictEqual(Buffer.byteLength(body), MAX_BODY_BYTES);

  const res = await request(ctx.port, {
    method: 'POST',
    path: '/echo',
    headers: JSON_HEADERS,
    body,
  });
  assert.strictEqual(res.status, 200);
  assert.strictEqual(res.json.echo.data.length, filler.length);
});

test('413 takes priority over 404 for oversized body on unknown path', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const oversized = 'z'.repeat(MAX_BODY_BYTES + 10);
  const res = await request(ctx.port, {
    method: 'POST',
    path: '/does-not-exist',
    headers: { ...JSON_HEADERS, 'Content-Length': String(Buffer.byteLength(oversized)) },
    body: oversized,
  });
  assert.strictEqual(res.status, 413);
  assert.strictEqual(res.json.error, 'PAYLOAD_TOO_LARGE');
});

test('keep-alive sequential requests on one connection stay isolated', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const agent = new http.Agent({ keepAlive: true, maxSockets: 1 });
  t.after(() => agent.destroy());

  const send = (options) =>
    new Promise((resolve, reject) => {
      const req = http.request(
        { host: '127.0.0.1', port: ctx.port, agent, ...options },
        (res) => {
          const chunks = [];
          res.on('data', (c) => chunks.push(c));
          res.on('end', () =>
            resolve({
              status: res.statusCode,
              json: JSON.parse(Buffer.concat(chunks).toString('utf8')),
            }),
          );
        },
      );
      req.on('error', reject);
      if (options.body !== undefined) req.write(options.body);
      req.end();
    });

  const first = await send({ method: 'GET', path: '/health' });
  const second = await send({
    method: 'POST',
    path: '/echo',
    headers: JSON_HEADERS,
    body: '{"n":1}',
  });
  const third = await send({ method: 'GET', path: '/missing' });

  assert.deepStrictEqual(first.json, { ok: true });
  assert.deepStrictEqual(second.json, { echo: { n: 1 } });
  assert.strictEqual(third.status, 404);
  assert.strictEqual(third.json.error, 'NOT_FOUND');
});

test('very long URL (>= 8 KiB) returns 404 and does not crash', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const res = await request(ctx.port, { path: '/' + 'a'.repeat(9000) });
  assert.strictEqual(res.status, 404);
  assert.strictEqual(res.json.error, 'NOT_FOUND');
});

test('concurrent requests are isolated', async (t) => {
  const ctx = await startTestServer();
  t.after(() => ctx.close());

  const results = await Promise.all(
    Array.from({ length: 10 }, (_, index) =>
      request(ctx.port, {
        method: 'POST',
        path: '/echo',
        headers: JSON_HEADERS,
        body: JSON.stringify({ index }),
      }),
    ),
  );
  results.forEach((res, index) => {
    assert.strictEqual(res.status, 200);
    assert.deepStrictEqual(res.json, { echo: { index } });
  });
});
