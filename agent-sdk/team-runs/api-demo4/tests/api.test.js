'use strict';

/**
 * Echo API 测试（node:test + node:assert，无第三方依赖）
 * 运行：node --test
 * 每个测试用例都通过 listen(0) 取得随机端口，不依赖外部服务。
 */

const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');

const { createServer, MAX_BODY_BYTES, JSON_CONTENT_TYPE } = require('../src/server.js');

/**
 * 启动一个临时服务并返回基址与关闭函数。
 */
async function startServer() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const { port } = server.address();
  return {
    port,
    origin: `http://127.0.0.1:${port}`,
    async close() {
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

/**
 * 用 node:http 发起请求，返回 status / headers / 文本体，并记录解析后的 JSON。
 * @param {string} origin
 * @param {{method?: string, path: string, body?: string|Buffer, headers?: Record<string,string>}} options
 */
function request(origin, { method = 'GET', path, body, headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const url = new URL(path, origin);
    const req = http.request(
      {
        hostname: url.hostname,
        port: url.port,
        path: `${url.pathname}${url.search}`,
        method,
        headers,
      },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          let json;
          let jsonError;
          try {
            json = JSON.parse(text);
          } catch (err) {
            jsonError = err;
          }
          resolve({ status: res.statusCode, headers: res.headers, text, json, jsonError });
        });
      },
    );
    req.on('error', reject);
    if (body !== undefined) req.write(body);
    req.end();
  });
}

const JSON_HEADERS = { 'Content-Type': 'application/json' };

test('环境：不引入第三方依赖，只用 node:http', () => {
  const pkg = (() => {
    try {
      return require('../package.json');
    } catch {
      return { dependencies: undefined, devDependencies: undefined };
    }
  })();
  assert.equal(pkg.dependencies === undefined || Object.keys(pkg.dependencies).length === 0, true);
  assert.equal(pkg.devDependencies === undefined || Object.keys(pkg.devDependencies).length === 0, true);
});

test('GET /health -> 200 {"ok":true} 且 Content-Type 正确', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, { path: '/health' });
    assert.equal(res.status, 200);
    assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
    assert.equal(res.headers['content-length'], String(Buffer.byteLength(res.text, 'utf8')));
    assert.deepEqual(res.json, { ok: true });
    assert.equal(res.json.ok, true);
  } finally {
    await app.close();
  }
});

test('POST /echo 对象：响应与请求解析值深相等', async () => {
  const app = await startServer();
  try {
    const payload = { hello: 'world', n: 42, nested: { list: [1, true, null] } };
    const res = await request(app.origin, {
      method: 'POST',
      path: '/echo',
      headers: JSON_HEADERS,
      body: JSON.stringify(payload),
    });
    assert.equal(res.status, 200);
    assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
    assert.deepEqual(res.json, payload);
  } finally {
    await app.close();
  }
});

test('POST /echo 顶层字符串/数字/布尔/null 都原样回显（不包裹）', async () => {
  const app = await startServer();
  try {
    const cases = ['"plain string"', '123.5', 'true', 'null', '[1,"a",false]'];
    for (const raw of cases) {
      const res = await request(app.origin, {
        method: 'POST',
        path: '/echo',
        headers: JSON_HEADERS,
        body: raw,
      });
      assert.equal(res.status, 200, `body=${raw}`);
      assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
      assert.deepEqual(res.json, JSON.parse(raw), `body=${raw}`);
      // 回显为顶层值而非被包裹的对象
      assert.equal(res.text, JSON.stringify(JSON.parse(raw)));
    }
  } finally {
    await app.close();
  }
});

test('POST /echo 空请求体 -> 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, {
      method: 'POST',
      path: '/echo',
      headers: JSON_HEADERS,
      body: '',
    });
    assert.equal(res.status, 400);
    assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
    assert.equal(res.json.error.code, 'INVALID_JSON');
    assert.equal(typeof res.json.error.message, 'string');
    assert.ok(res.json.error.message.length > 0);
  } finally {
    await app.close();
  }
});

test('POST /echo 非法 JSON（截断/尾随逗号/单引号/NaN）-> 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    const invalids = ['{"a":1', '{"a":1,}', "{'a':1}", 'NaN', '{"a":Infinity}', '/*c*/{}'];
    for (const raw of invalids) {
      const res = await request(app.origin, {
        method: 'POST',
        path: '/echo',
        headers: JSON_HEADERS,
        body: raw,
      });
      assert.equal(res.status, 400, `body=${raw}`);
      assert.equal(res.json.error.code, 'INVALID_JSON', `body=${raw}`);
      assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
    }
  } finally {
    await app.close();
  }
});

test('未知路径 -> 404 NOT_FOUND 且不回显请求内容', async () => {
  const app = await startServer();
  try {
    const res = await request(app.origin, { path: '/nope' });
    assert.equal(res.status, 404);
    assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
    assert.equal(res.json.error.code, 'NOT_FOUND');
    assert.equal(typeof res.json.error.message, 'string');
    // 不实现 / 路由
    const root = await request(app.origin, { path: '/' });
    assert.equal(root.status, 404);
    assert.equal(root.json.error.code, 'NOT_FOUND');
  } finally {
    await app.close();
  }
});

test('方法不匹配 -> 404（不用 405）：GET /echo、POST /health、PUT /health', async () => {
  const app = await startServer();
  try {
    const cases = [
      { method: 'GET', path: '/echo' },
      { method: 'POST', path: '/health', headers: JSON_HEADERS, body: '{}' },
      { method: 'PUT', path: '/health', body: '{}' },
      { method: 'DELETE', path: '/echo' },
    ];
    for (const c of cases) {
      const res = await request(app.origin, c);
      assert.equal(res.status, 404, `${c.method} ${c.path}`);
      assert.equal(res.json.error.code, 'NOT_FOUND', `${c.method} ${c.path}`);
      assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE, `${c.method} ${c.path}`);
    }
  } finally {
    await app.close();
  }
});

test('边界：路径区分大小写且忽略查询串', async () => {
  const app = await startServer();
  try {
    const upper = await request(app.origin, { path: '/Health' });
    assert.equal(upper.status, 404);
    assert.equal(upper.json.error.code, 'NOT_FOUND');

    const withQuery = await request(app.origin, { path: '/health?verbose=1&x=%20' });
    assert.equal(withQuery.status, 200);
    assert.deepEqual(withQuery.json, { ok: true });

    const echoWithQuery = await request(app.origin, {
      method: 'POST',
      path: '/echo?trace=abc',
      headers: JSON_HEADERS,
      body: '{"a":1}',
    });
    assert.equal(echoWithQuery.status, 200);
    assert.deepEqual(echoWithQuery.json, { a: 1 });
  } finally {
    await app.close();
  }
});

test('边界：请求体超过 1 MiB -> 400 INVALID_JSON', async () => {
  const app = await startServer();
  try {
    const big = `"${'x'.repeat(MAX_BODY_BYTES + 1024)}"`;
    const res = await request(app.origin, {
      method: 'POST',
      path: '/echo',
      headers: { ...JSON_HEADERS, 'Content-Length': String(Buffer.byteLength(big)) },
      body: big,
    });
    assert.equal(res.status, 400);
    assert.equal(res.json.error.code, 'INVALID_JSON');
  } finally {
    await app.close();
  }
});

test('边界：所有响应（含错误）都带 Content-Length 且 Content-Type 为 JSON', async () => {
  const app = await startServer();
  try {
    const responses = [
      await request(app.origin, { path: '/health' }),
      await request(app.origin, { method: 'POST', path: '/echo', headers: JSON_HEADERS, body: '{"k":"v"}' }),
      await request(app.origin, { path: '/missing' }),
      await request(app.origin, { method: 'POST', path: '/echo', headers: JSON_HEADERS, body: '{bad' }),
    ];
    for (const res of responses) {
      assert.equal(res.headers['content-type'], JSON_CONTENT_TYPE);
      assert.equal(res.headers['content-length'], String(Buffer.byteLength(res.text, 'utf8')));
      assert.equal(res.jsonError, undefined, `响应必须是合法 JSON，实测: ${res.text}`);
    }
  } finally {
    await app.close();
  }
});

test('边界：并发请求之间不共享可变状态', async () => {
  const app = await startServer();
  try {
    const tasks = [];
    for (let i = 0; i < 12; i += 1) {
      tasks.push(
        request(app.origin, {
          method: 'POST',
          path: '/echo',
          headers: JSON_HEADERS,
          body: JSON.stringify({ i }),
        }).then((res) => {
          assert.equal(res.status, 200);
          assert.deepEqual(res.json, { i });
        }),
      );
    }
    await Promise.all(tasks);
  } finally {
    await app.close();
  }
});
