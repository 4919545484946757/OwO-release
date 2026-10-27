'use strict';

const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');

const { createServer, MAX_BODY_BYTES } = require('../src/server.js');

/** @type {import('node:http').Server} */
let server;
let baseUrl;

before(async () => {
  server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  baseUrl = `http://127.0.0.1:${port}`;
});

after(async () => {
  await new Promise((resolve) => server.close(resolve));
});

/**
 * 发起请求并收集原始文本响应（便于断言 Content-Type / Content-Length）。
 * @param {string} method
 * @param {string} path
 * @param {string|Buffer|undefined} body
 * @param {Record<string,string>} [headers]
 */
function request(method, path, body, headers = {}) {
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : Buffer.isBuffer(body) ? body : Buffer.from(body, 'utf8');
    const req = http.request(
      `${baseUrl}${path}`,
      {
        method,
        headers: {
          ...(payload ? { 'Content-Type': 'application/json', 'Content-Length': String(payload.length) } : {}),
          ...headers,
        },
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let json;
          try {
            json = text === '' ? undefined : JSON.parse(text);
          } catch {
            json = undefined;
          }
          resolve({ status: res.statusCode, headers: res.headers, text, json });
        });
      }
    );
    req.on('error', reject);
    if (payload) req.write(payload);
    req.end();
  });
}

// ---- 验收用例 1-8（docs/api-design.md 第 9 节） ----

test('1. GET /health -> 200 {"ok":true}', async () => {
  const res = await request('GET', '/health');
  assert.equal(res.status, 200);
  assert.deepStrictEqual(res.json, { ok: true });
  assert.match(res.headers['content-type'], /^application\/json; charset=utf-8$/);
  assert.equal(Number(res.headers['content-length']), Buffer.byteLength(res.text, 'utf8'));
});

test('2. POST /echo 对象 -> 200 等价回显', async () => {
  const res = await request('POST', '/echo', JSON.stringify({ a: 1 }));
  assert.equal(res.status, 200);
  assert.deepStrictEqual(res.json, { a: 1 });
  assert.match(res.headers['content-type'], /application\/json/);
});

test('3. POST /echo 截断 JSON -> 400 bad_request', async () => {
  const res = await request('POST', '/echo', '{"a":1');
  assert.equal(res.status, 400);
  assert.equal(res.json.error, 'bad_request');
  assert.equal(typeof res.json.message, 'string');
});

test('4. POST /echo 空 body -> 400 bad_request', async () => {
  const res = await request('POST', '/echo', Buffer.alloc(0), { 'Content-Length': '0' });
  assert.equal(res.status, 400);
  assert.equal(res.json.error, 'bad_request');
});

test('5. GET /nope -> 404 not_found', async () => {
  const res = await request('GET', '/nope');
  assert.equal(res.status, 404);
  assert.equal(res.json.error, 'not_found');
});

test('6. GET /echo（方法不匹配）-> 404 not_found', async () => {
  const res = await request('GET', '/echo');
  assert.equal(res.status, 404);
  assert.equal(res.json.error, 'not_found');
});

test('7. POST /echo 数组 -> 200 等价回显', async () => {
  const res = await request('POST', '/echo', JSON.stringify([1, 2]));
  assert.equal(res.status, 200);
  assert.deepStrictEqual(res.json, [1, 2]);
});

test('8. GET /health?x=1 -> 200（忽略 query）', async () => {
  const res = await request('GET', '/health?x=1');
  assert.equal(res.status, 200);
  assert.deepStrictEqual(res.json, { ok: true });
});

// ---- 边界用例 ----

test('边界: POST /health（方法不匹配）-> 404 not_found', async () => {
  const res = await request('POST', '/health', JSON.stringify({ any: 1 }));
  assert.equal(res.status, 404);
  assert.equal(res.json.error, 'not_found');
});

test('边界: /health/ 尾斜杠不归一化 -> 404', async () => {
  const res = await request('GET', '/health/');
  assert.equal(res.status, 404);
  assert.equal(res.json.error, 'not_found');
});

test('边界: PUT /echo -> 404，不被当作 echo 成功', async () => {
  const res = await request('PUT', '/echo', JSON.stringify({ a: 1 }));
  assert.equal(res.status, 404);
  assert.equal(res.json.error, 'not_found');
});

test('边界: 顶层 JSON 字面量与 null 也原样回显', async () => {
  for (const [raw, expected] of [
    ['null', null],
    ['42', 42],
    ['"hi"', 'hi'],
    ['true', true],
  ]) {
    const res = await request('POST', '/echo', raw);
    assert.equal(res.status, 200, `raw=${raw}`);
    assert.deepStrictEqual(res.json, expected, `raw=${raw}`);
  }
});

test('边界: 多余尾随字符 -> 400', async () => {
  const res = await request('POST', '/echo', '{"a":1} trailing');
  assert.equal(res.status, 400);
  assert.equal(res.json.error, 'bad_request');
});

test('边界: UTF-8 多字节字符按字节计 Content-Length 且正确回显', async () => {
  const payload = { text: '你好，世界 🙂' };
  const res = await request('POST', '/echo', JSON.stringify(payload));
  assert.equal(res.status, 200);
  assert.deepStrictEqual(res.json, payload);
  assert.equal(Number(res.headers['content-length']), Buffer.byteLength(res.text, 'utf8'));
});

test(`边界: 请求体超过 ${MAX_BODY_BYTES} 字节 -> 413 payload_too_large`, async () => {
  const big = JSON.stringify({ blob: 'x'.repeat(MAX_BODY_BYTES) });
  assert.ok(Buffer.byteLength(big, 'utf8') > MAX_BODY_BYTES);
  const res = await request('POST', '/echo', big, { Connection: 'close' });
  assert.equal(res.status, 413);
  assert.equal(res.json.error, 'payload_too_large');
});

test('边界: 分块传输（chunked）编码的 JSON 也能正确回显', async () => {
  const body = JSON.stringify({ chunked: true, list: [1, 2, 3] });
  const parts = [body.slice(0, 5), body.slice(5, 12), body.slice(12)];

  const res = await new Promise((resolve, reject) => {
    const req = http.request(
      `${baseUrl}/echo`,
      { method: 'POST', headers: { 'Content-Type': 'application/json', 'Transfer-Encoding': 'chunked' } },
      (r) => {
        const chunks = [];
        r.on('data', (c) => chunks.push(c));
        r.on('end', () => resolve({ status: r.statusCode, text: Buffer.concat(chunks).toString('utf8') }));
      }
    );
    req.on('error', reject);
    for (const p of parts) req.write(p);
    req.end();
  });

  assert.equal(res.status, 200);
  assert.deepStrictEqual(JSON.parse(res.text), { chunked: true, list: [1, 2, 3] });
});

test('边界: 同一 keep-alive 连接连续两次请求均正常（无跨请求状态泄漏）', async () => {
  const agent = new http.Agent({ keepAlive: true, maxSockets: 1 });

  const doRequest = (method, path, body) =>
    new Promise((resolve, reject) => {
      const payload = body === undefined ? undefined : Buffer.from(body, 'utf8');
      const req = http.request(
        `${baseUrl}${path}`,
        {
          method,
          agent,
          headers: payload ? { 'Content-Type': 'application/json', 'Content-Length': String(payload.length) } : {},
        },
        (r) => {
          const chunks = [];
          r.on('data', (c) => chunks.push(c));
          r.on('end', () => resolve({ status: r.statusCode, json: JSON.parse(Buffer.concat(chunks).toString('utf8')) }));
        }
      );
      req.on('error', reject);
      if (payload) req.write(payload);
      req.end();
    });

  try {
    const first = await doRequest('POST', '/echo', JSON.stringify({ n: 1 }));
    const second = await doRequest('GET', '/health');
    assert.equal(first.status, 200);
    assert.deepStrictEqual(first.json, { n: 1 });
    assert.equal(second.status, 200);
    assert.deepStrictEqual(second.json, { ok: true });
  } finally {
    agent.destroy();
  }
});
