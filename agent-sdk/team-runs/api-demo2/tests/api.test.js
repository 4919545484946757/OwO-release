'use strict';

const test = require('node:test');
const assert = require('node:assert');
const http = require('node:http');

const { createServer, ERROR_CODES, DEFAULT_MAX_BODY_BYTES } = require('../src/server');

/** 启动服务在随机端口（listen(0)），返回 base URL 与关闭函数 */
async function startServer(options) {
  const server = createServer(options);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  return {
    origin: `http://127.0.0.1:${port}`,
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections?.();
        server.close(() => resolve());
      }),
  };
}

/** 发起真实 HTTP 请求，返回 statusCode / headers / rawBody / json */
function request(origin, method, path, { headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const url = new URL(path, origin);
    const req = http.request(
      {
        host: url.hostname,
        port: url.port,
        method,
        path: url.pathname + url.search,
        headers,
      },
      (res) => {
        const chunks = [];
        res.on('data', (chunk) => chunks.push(chunk));
        res.on('end', () => {
          const rawBody = Buffer.concat(chunks);
          let json;
          try {
            json = JSON.parse(rawBody.toString('utf8'));
          } catch {
            json = undefined;
          }
          resolve({ statusCode: res.statusCode, headers: res.headers, rawBody, json });
        });
        res.on('error', reject);
      },
    );
    req.on('error', reject);
    if (body !== undefined) req.write(body);
    req.end();
  });
}

function assertJsonResponse(res) {
  assert.strictEqual(res.headers['content-type'], 'application/json; charset=utf-8');
  assert.strictEqual(
    Number(res.headers['content-length']),
    res.rawBody.length,
    'Content-Length 必须等于实际体长度',
  );
  assert.ok(!res.rawBody.toString('utf8').endsWith('\n'), '响应体不得以换行结尾');
}

function assertErrorShape(res, code) {
  assertJsonResponse(res);
  assert.strictEqual(res.statusCode, ERROR_CODES[code]);
  assert.ok(res.json && typeof res.json === 'object' && !Array.isArray(res.json));
  assert.ok(res.json.error && typeof res.json.error === 'object');
  assert.strictEqual(res.json.error.code, code);
  assert.strictEqual(typeof res.json.error.message, 'string');
  assert.ok(res.json.error.message.length > 0);
  // 错误体顶层只允许 error 字段
  assert.deepStrictEqual(Object.keys(res.json), ['error']);
}

test('GET /health → 200 {"ok":true}', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, 'GET', '/health');
    assertJsonResponse(res);
    assert.strictEqual(res.statusCode, 200);
    assert.deepStrictEqual(res.json, { ok: true });
    assert.strictEqual(res.rawBody.toString('utf8'), '{"ok":true}');
  } finally {
    await app.close();
  }
});

test('GET /health?x=1 丢弃查询串，仍为 200', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, 'GET', '/health?x=1&y=2');
    assertJsonResponse(res);
    assert.strictEqual(res.statusCode, 200);
    assert.deepStrictEqual(res.json, { ok: true });
  } finally {
    await app.close();
  }
});

test('POST /echo 回显 JSON 并带 ISO-8601 UTC receivedAt（可注入 now）', async () => {
  const fixed = new Date('2025-01-01T00:00:00.000Z');
  const app = await startServer({ now: () => fixed });
  try {
    const payload = { a: 1, b: [true, null, 'x'], nested: { c: 2 } };
    const res = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    assertJsonResponse(res);
    assert.strictEqual(res.statusCode, 200);
    assert.deepStrictEqual(Object.keys(res.json), ['receivedAt', 'body']);
    assert.strictEqual(res.json.receivedAt, '2025-01-01T00:00:00.000Z');
    assert.deepStrictEqual(res.json.body, payload);
  } finally {
    await app.close();
  }
});

test('POST /echo 顶层标量 / 数组 / null 原样透传', async () => {
  const app = await startServer();
  try {
    for (const raw of ['null', '0', '"s"', 'true', '[]', '[1,2]', '1.5']) {
      const res = await request(app.origin, 'POST', '/echo', {
        headers: { 'Content-Type': 'application/json; charset=utf-8' },
        body: raw,
      });
      assertJsonResponse(res);
      assert.strictEqual(res.statusCode, 200, `body=${raw}`);
      assert.deepStrictEqual(res.json.body, JSON.parse(raw), `body=${raw}`);
    }
  } finally {
    await app.close();
  }
});

test('POST /echo 空体 → 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'application/json', 'Content-Length': '0' },
      body: '',
    });
    assertErrorShape(res, 'INVALID_JSON');
    assert.strictEqual(res.statusCode, 400);
  } finally {
    await app.close();
  }
});

test('POST /echo 非法 JSON（截断 / 尾随多余值）→ 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    for (const raw of ['{"a":', '{"a":1}{"b":2}', 'not json', '{} trailing']) {
      const res = await request(app.origin, 'POST', '/echo', {
        headers: { 'Content-Type': 'application/json' },
        body: raw,
      });
      assertErrorShape(res, 'INVALID_JSON');
      assert.strictEqual(res.statusCode, 400, `body=${raw}`);
    }
  } finally {
    await app.close();
  }
});

test('POST /echo 非法 UTF-8 字节 → 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'application/json' },
      body: Buffer.from([0x22, 0xff, 0xfe, 0x22]),
    });
    assertErrorShape(res, 'INVALID_JSON');
    assert.strictEqual(res.statusCode, 400);
  } finally {
    await app.close();
  }
});

test('POST /echo 缺失或非 JSON Content-Type → 415（解析前判定）', async () => {
  const app = await startServer();
  try {
    const noType = await request(app.origin, 'POST', '/echo', { body: '{"a":1}' });
    assertErrorShape(noType, 'UNSUPPORTED_MEDIA_TYPE');
    assert.strictEqual(noType.statusCode, 415);

    // 体非法也不改变：415 优先于 400
    const textPlain = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'text/plain' },
      body: '<<<not json>>>',
    });
    assertErrorShape(textPlain, 'UNSUPPORTED_MEDIA_TYPE');
    assert.strictEqual(textPlain.statusCode, 415);
  } finally {
    await app.close();
  }
});

test('POST /echo 超过 maxBodyBytes → 413 + Connection: close', async () => {
  const app = await startServer({ maxBodyBytes: 16 });
  try {
    const body = JSON.stringify('x'.repeat(32)); // 34 字节 > 16
    const res = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'application/json', 'Content-Length': String(body.length) },
      body,
    });
    assertErrorShape(res, 'PAYLOAD_TOO_LARGE');
    assert.strictEqual(res.statusCode, 413);
    assert.strictEqual(res.headers.connection, 'close');
  } finally {
    await app.close();
  }
});

test('POST /echo 体长度恰好等于上限且合法 → 200', async () => {
  const app = await startServer({ maxBodyBytes: 16 });
  try {
    const exact = '"0123456789abcd"'; // 16 字节，含两端引号
    assert.strictEqual(Buffer.byteLength(exact, 'utf8'), 16);
    const res = await request(app.origin, 'POST', '/echo', {
      headers: { 'Content-Type': 'application/json', 'Content-Length': String(exact.length) },
      body: exact,
    });
    assertJsonResponse(res);
    assert.strictEqual(res.statusCode, 200);
    assert.strictEqual(res.json.body, '0123456789abcd');
  } finally {
    await app.close();
  }
});

test('默认上限为 1048576 字节（导出常量）', () => {
  assert.strictEqual(DEFAULT_MAX_BODY_BYTES, 1048576);
});

test('未知路径任意方法 → 404 NOT_FOUND；尾斜杠与大小写不折叠', async () => {
  const app = await startServer();
  try {
    const cases = [
      ['GET', '/'],
      ['GET', '/health/'],
      ['GET', '/Health'],
      ['GET', '/healthx'],
      ['DELETE', '/nope'],
      ['POST', '/unknown'],
    ];
    for (const [method, path] of cases) {
      const res = await request(app.origin, method, path);
      assertErrorShape(res, 'NOT_FOUND');
      assert.strictEqual(res.statusCode, 404, `${method} ${path}`);
      assert.strictEqual(res.headers.allow, undefined, `${method} ${path} 不应带 Allow`);
    }
  } finally {
    await app.close();
  }
});

test('已注册路径方法不匹配 → 405 + Allow 头', async () => {
  const app = await startServer();
  try {
    const headHealth = await request(app.origin, 'HEAD', '/health');
    assert.strictEqual(headHealth.statusCode, 405);
    assert.strictEqual(headHealth.headers.allow, 'GET');

    const postHealth = await request(app.origin, 'POST', '/health', {
      headers: { 'Content-Type': 'application/json' },
      body: '{}',
    });
    assertErrorShape(postHealth, 'METHOD_NOT_ALLOWED');
    assert.strictEqual(postHealth.statusCode, 405);
    assert.strictEqual(postHealth.headers.allow, 'GET');

    const getEcho = await request(app.origin, 'GET', '/echo');
    assertErrorShape(getEcho, 'METHOD_NOT_ALLOWED');
    assert.strictEqual(getEcho.statusCode, 405);
    assert.strictEqual(getEcho.headers.allow, 'POST');
  } finally {
    await app.close();
  }
});

test('ERROR_CODES 常量集合与状态码映射固定', () => {
  assert.deepStrictEqual(ERROR_CODES, {
    NOT_FOUND: 404,
    METHOD_NOT_ALLOWED: 405,
    UNSUPPORTED_MEDIA_TYPE: 415,
    INVALID_JSON: 400,
    PAYLOAD_TOO_LARGE: 413,
    INTERNAL_ERROR: 500,
  });
});

test('createServer 返回未监听的 http.Server', () => {
  const server = createServer();
  assert.strictEqual(server.listening, false);
  assert.strictEqual(typeof server.listen, 'function');
  server.close();
});
