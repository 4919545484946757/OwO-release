'use strict';

/**
 * 宿主行为验证入口（评审返修项）
 *
 * 背景：当前执行环境下 `node --test <file>` 不产出任何用例（TAP 输出 `1..0`，pass 0/fail 0），
 * 因此不能把 test runner 的自动发现当作行为证据。本脚本改用「同进程显式登记」的方式执行行为用例：
 *   - 直接 require tests/api.test.js，让 13 个 node:test 用例在当前进程登记；
 *   - 由 node:test 默认 runner 在本进程退出前真实执行；
 *   - 任一用例失败 → 进程退出码非 0。
 *
 * 复现命令（工作区根，零第三方依赖，仅 loopback + listen(0)）：
 *   node -e "require('./tests/hash-verify.test.js')"
 *   node tests/hash-verify.test.js
 *
 * 本文件同时校验被测源码/测试文件的内容哈希，保证证据与最终文件一一对应。
 */

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const ROOT = path.resolve(__dirname, '..');
const SERVER_PATH = 'src/server.js';
const API_TEST_REL = 'tests/api.test.js';
const API_TEST_MODULE = './api.test.js';

// 期望哈希：与本次交付落盘内容一一对应；源码/测试变更后必须同步更新，否则本用例失败
const EXPECTED_SHA256 = {
  [SERVER_PATH]: '848e1b421ae01915d5a417779c55671aac3e6aafc7187a85856feecc1994b355',
  [API_TEST_REL]: '3de524c565c5530c455a4e0e3ca8e54ecd2dde322647f955995be28b52bbf2bc',
};

function sha256File(relativePath) {
  return crypto
    .createHash('sha256')
    .update(fs.readFileSync(path.join(ROOT, relativePath)))
    .digest('hex');
}

test('被测文件哈希与交付记录一致（src/server.js + tests/api.test.js）', () => {
  for (const [rel, expected] of Object.entries(EXPECTED_SHA256)) {
    assert.equal(sha256File(rel), expected, `${rel} 内容已变更，须重新登记哈希并重跑行为验证`);
  }
});

test('src/server.js 仅依赖 Node 内置模块且导出契约不变', () => {
  const source = fs.readFileSync(path.join(ROOT, SERVER_PATH), 'utf8');
  const requires = [...source.matchAll(/require\(\s*['"]([^'"]+)['"]\s*\)/g)].map((m) => m[1]);
  assert.ok(requires.length > 0, '应至少 require 一个内置模块');
  for (const id of requires) {
    assert.ok(id.startsWith('node:'), `出现非内置依赖：${id}`);
  }
  const pkgPath = path.join(ROOT, 'package.json');
  if (fs.existsSync(pkgPath)) {
    const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
    assert.deepEqual(pkg.dependencies ?? {}, {}, '不应声明第三方 dependencies');
  }
  const { MAX_BODY_BYTES, createServer, start } = require('../src/server.js');
  assert.equal(MAX_BODY_BYTES, 1048576, 'MAX_BODY_BYTES 契约值必须为 1048576');
  assert.equal(typeof createServer, 'function');
  assert.equal(typeof start, 'function');
});

// 同进程登记并执行全部行为用例（13 个）：任一失败都会写回本进程的测试结果
require(API_TEST_MODULE);
