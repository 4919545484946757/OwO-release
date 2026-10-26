'use strict';

/**
 * API 契约回归测试（node:test + 临时端口 listen(0)，无外部服务依赖）
 * 覆盖：成功路径（GET /health、POST /echo 各种 JSON 值）、400、404，
 * 以及边界：405 + Allow、415、413（>1MiB）、恰好 1 MiB、query 命中、尾斜杠 404。
 */

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');

const { createServer, MAX_BODY_BYTES } = require('../src/server.js');

let server;
let baseUrl;

const JSON_CT = 'application/json; charset=utf-8';

function rawRequest({ method, path, headers, body }) {
  const url = new URL(path, baseUrl);
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port,
        path: url.pathname + url.search,
        method,
        headers,
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
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

test.before(async () => {
  server = createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  baseUrl = `http://127.0.0.1:${port}`;
});

test.after(async () => {
  await new Promise((resolve) => server.close(resolve));
});

test('GET /health 返回 200 且 JSON 深等于 {"ok":true}', async () => {
  const res = await fetch(`${baseUrl}/health`);
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-type'), JSON_CT);
  assert.deepEqual(await res.json(), { ok: true });
});

test('GET /health?probe=1 命中（忽略 query）', async () => {
  const res = await fetch(`${baseUrl}/health?probe=1`);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { ok: true });
});

test('POST /echo 回显对象', async () => {
  const res = await fetch(`${baseUrl}/echo`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message: 'hi', n: 42 }),
  });
  assert.equal(res.status, 200);
  assert.equal(res.headers.get('content-type'), JSON_CT);
  assert.deepEqual(await res.json(), { echo: { message: 'hi', n: 42 } });
});

test('POST /echo 回显空对象必须保留 echo 键', async () => {
  const res = await fetch(`${baseUrl}/echo`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  });
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.deepEqual(body, { echo: {} });
  assert.ok(Object.prototype.hasOwnProperty.call(body, 'echo'));
});

test('POST /echo 回显数组/字符串/数字/布尔/null 且 echo 键必现', async () => {
  const cases = [[1, 2, 3], 'hello', 0, false, true, null];
  for (const value of cases) {
    const res = await fetch(`${baseUrl}/echo`, {
      method: 'POST',
      headers: { 'Content-Type': 'Application/JSON; charset=utf-8' },
      body: JSON.stringify(value),
    });
    assert.equal(res.status, 200, `value=${JSON.stringify(value)}`);
    const body = await res.json();
    assert.ok(Object.prototype.hasOwnProperty.call(body, 'echo'), `echo 键缺失 value=${JSON.stringify(value)}`);
    assert.deepEqual(body, { echo: value });
  }
});

test('POST /echo 语法错误 → 400 INVALID_JSON', async () => {
  const res = await fetch(`${baseUrl}/echo`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{bad',
  });
  assert.equal(res.status, 400);
  assert.equal(res.headers.get('content-type'), JSON_CT);
  const body = await res.json();
  assert.equal(body.error.code, 'INVALID_JSON');
  assert.equal(body.error.path, '/echo');
  assert.ok(body.error.message.length > 0);
});

test('POST /echo 空体与仅空白 → 400 INVALID_JSON', async () => {
  for (const body of ['', '   \n\t ']) {
    const res = await fetch(`${baseUrl}/echo`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
    });
    assert.equal(res.status, 400, `body=${JSON.stringify(body)}`);
    const parsed = await res.json();
    assert.equal(parsed.error.code, 'INVALID_JSON');
    assert.equal(parsed.error.path, '/echo');
  }
});

test('GET /nope → 404 NOT_FOUND，path 为 pathname（不含 query）', async () => {
  const res = await fetch(`${baseUrl}/nope?x=1`);
  assert.equal(res.status, 404);
  assert.equal(res.headers.get('content-type'), JSON_CT);
  const body = await res.json();
  assert.equal(body.error.code, 'NOT_FOUND');
  assert.equal(body.error.path, '/nope');
  assert.ok(body.error.message.length > 0);
});

test('边界：/health/ 尾斜杠不宽容 → 404', async () => {
  const res = await fetch(`${baseUrl}/health/`);
  assert.equal(res.status, 404);
  assert.equal((await res.json()).error.code, 'NOT_FOUND');
});

test('边界：POST /health 与 GET /echo → 405 + Allow 头', async () => {
  const postHealth = await fetch(`${baseUrl}/health`, { method: 'POST' });
  assert.equal(postHealth.status, 405);
  assert.equal(postHealth.headers.get('allow'), 'GET');
  assert.equal((await postHealth.json()).error.code, 'METHOD_NOT_ALLOWED');

  const getEcho = await fetch(`${baseUrl}/echo`);
  assert.equal(getEcho.status, 405);
  assert.equal(getEcho.headers.get('allow'), 'POST');
  assert.equal((await getEcho.json()).error.code, 'METHOD_NOT_ALLOWED');
});

test('边界：Content-Type 非 application/json 或缺失 → 415', async () => {
  const text = await fetch(`${baseUrl}/echo`, {
    method: 'POST',
    headers: { 'Content-Type': 'text/plain' },
    body: 'hi',
  });
  assert.equal(text.status, 415);
  assert.equal((await text.json()).error.code, 'UNSUPPORTED_MEDIA_TYPE');

  const missing = await rawRequest({
    method: 'POST',
    path: '/echo',
    headers: { 'Content-Length': 2 },
    body: '{}',
  });
  assert.equal(missing.status, 415);
  assert.equal(missing.json.error.code, 'UNSUPPORTED_MEDIA_TYPE');
});

test('边界：请求体超过 1 MiB → 413 PAYLOAD_TOO_LARGE', async () => {
  const big = 'a'.repeat(MAX_BODY_BYTES + 1024);
  const res = await rawRequest({
    method: 'POST',
    path: '/echo',
    headers: {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(JSON.stringify(big)),
    },
    body: JSON.stringify(big),
  }).catch((err) => ({ error: err }));
  if (res.error) {
    // 服务端超限后主动断开剩余请求体属文档允许行为
    assert.match(String(res.error.code), /ECONNRESET|EPIPE/);
    return;
  }
  assert.equal(res.status, 413);
  assert.equal(res.json.error.code, 'PAYLOAD_TOO_LARGE');
});

test('边界：恰好 1 MiB 以内仍然成功回显', async () => {
  const near = 'b'.repeat(MAX_BODY_BYTES - 64);
  const res = await fetch(`${baseUrl}/echo`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ near }),
  });
  assert.equal(res.status, 200);
  assert.equal((await res.json()).echo.near.length, near.length);
});
