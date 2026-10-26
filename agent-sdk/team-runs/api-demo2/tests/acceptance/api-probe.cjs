'use strict';
const { createServer } = require('../../src/server.js');

const MAX = 1048576;
const J = 'application/json';

function request(port, method, path, headers, body) {
  return new Promise((resolve) => {
    const http = require('node:http');
    const req = http.request({ host: '127.0.0.1', port, method, path, headers }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () =>
        resolve({
          status: res.statusCode,
          connection: res.headers.connection || null,
          allow: res.headers.allow || null,
          contentType: res.headers['content-type'] || null,
          text: Buffer.concat(chunks).toString('utf8'),
        })
      );
      res.on('error', () => resolve({ status: res.statusCode, text: '<res-error>' }));
    });
    req.on('error', (err) => resolve({ status: 'ERR', text: String(err && err.message) }));
    if (body) req.write(body);
    req.end();
  });
}

async function caseRun(label, method, path, headers, body) {
  const server = createServer().listen(0);
  await new Promise((r) => server.once('listening', r));
  const port = server.address().port;
  const r = await request(port, method, path, headers, body);
  server.close();
  console.log(
    JSON.stringify([
      label,
      r.status,
      { connection: r.connection, allow: r.allow, contentType: r.contentType },
      r.text.slice(0, 140),
    ])
  );
}

(async () => {
  await caseRun('GET /health', 'GET', '/health');
  await caseRun('GET /health?x=1', 'GET', '/health?x=1');
  await caseRun('POST /health', 'POST', '/health');
  await caseRun('GET /Nope', 'GET', '/Nope');
  await caseRun('GET /echo', 'GET', '/echo');
  await caseRun('POST /echo bad-json', 'POST', '/echo', { 'content-type': J, 'content-length': 4 }, '{bad');
  await caseRun('POST /echo empty-body', 'POST', '/echo', { 'content-type': J, 'content-length': 0 }, '');
  await caseRun('POST /echo ok', 'POST', '/echo', { 'content-type': J, 'content-length': 7 }, '{"a":1}');
  await caseRun('POST /echo 415 text-plain', 'POST', '/echo', { 'content-type': 'text/plain', 'content-length': 1 }, 'x');
  await caseRun(
    'POST /echo 413 oversize',
    'POST',
    '/echo',
    { 'content-type': J, 'content-length': MAX + 10 },
    '{"pad":"' + 'a'.repeat(MAX - 4) + '"}'
  );
  const exact = `{"pad":"${'a'.repeat(MAX - 11)}"}`;
  await caseRun(
    'POST /echo exactly-limit',
    'POST',
    '/echo',
    { 'content-type': J, 'content-length': Buffer.byteLength(exact) },
    exact
  );
})();
