# REST API 设计说明（api-service / Node 内置 http）

状态：草案 v1（architect 出品，供 implementer / tester / reviewer 遵循）
运行时约束：仅使用 Node.js 内置 `http` 模块，零第三方依赖；测试使用 `node:test`。

## 1. 目标

1. 提供一个最小可用的 HTTP JSON 服务，暴露健康检查与回声两个端点。
2. 语义明确、可机械校验：状态码、Content-Type、JSON 字段与错误体格式全部固定。
3. 可测试：服务以工厂函数导出，监听端口可注入（含 `0` 随机端口），便于 `node:test` 内启动/关闭。

## 2. 非目标（Non-goals）

- 不做认证、鉴权、限流、CORS、TLS、压缩、缓存。
- 不做持久化、数据库、日志聚合、指标端点（`/metrics`）。
- 不做路由参数、嵌套资源、分页、版本协商（`/v1/...`）。
- 不实现 HTTP/2、WebSocket、请求体流式落盘。
- 不追求生产级健壮性（优雅停机、超时策略可留待后续迭代）。

## 3. 资源与路由

| Method | Path | 说明 | 成功状态码 |
| --- | --- | --- | --- |
| GET | `/health` | 健康检查，返回固定体 | 200 |
| POST | `/echo` | 回显请求 JSON | 200 |

### 3.1 路由匹配规则（必须逐字实现）

- Path 比较基于 `new URL(req.url, 'http://localhost')` 取 `pathname`；查询字符串一律忽略（`/health?x=1` 等价 `/health`）。
- Path 匹配区分大小写（`/Health` → 404）。
- 尾随斜杠视为不同路径（`/health/` → 404），不自动重定向。
- **Method 不匹配已存在路径时返回 405**，并附带 `Allow` 头；其它任何未注册路径返回 404（`/`、`/healthx` 均为 404）。

## 4. 端点契约

### 4.1 GET /health

响应 `200`，`Content-Type: application/json; charset=utf-8`。

```json
{"ok":true}
```

- 字段：`ok`（boolean，恒为 `true`）。无其他字段。
- 无请求体、无查询参数要求。

### 4.2 POST /echo

请求：

- `Content-Type` 必须是 `application/json`（允许带 `; charset=utf-8` 等参数，比较时仅取 `;` 前的 MIME 主类型并 trim + 小写）。
- 请求体必须是合法 JSON（UTF-8）。
- 请求体上限 **1 MiB（1048576 字节）**，按实际累计字节计算。

响应成功 `200`，`Content-Type: application/json; charset=utf-8`：

```json
{
  "receivedAt": "2025-01-01T00:00:00.000Z",
  "body": <请求 JSON 解析后的原样值>
}
```

字段固定：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `receivedAt` | string | ISO-8601 UTC 时间戳（`new Date().toISOString()`） |
| `body` | any | 解析后的 JSON 值原样透传，不做包装、不做深拷贝改写、不做字段过滤 |

顶层 JSON 标量（`1`、`"x"`、`true`、`null`）与数组同样合法，原样放入 `body`。

## 5. 错误码（业务错误类型）

所有错误响应体形状固定为：

```json
{"error":{"code":"<CODE>","message":"<人类可读说明>"}}
```

| 业务 code | HTTP 状态码 | 触发条件 | 附加响应头 |
| --- | --- | --- | --- |
| `NOT_FOUND` | 404 | 路径未注册（含 `/`、`/health/`、任意未知路径） | — |
| `METHOD_NOT_ALLOWED` | 405 | 路径已注册但 method 不匹配（如 `POST /health`、`GET /echo`） | `Allow`，值为该路径允许的方法（`GET` 或 `POST`） |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | `POST /echo` 的 Content-Type 缺失或非 `application/json` | — |
| `INVALID_JSON` | 400 | `POST /echo` 请求体为空、非合法 JSON、或非法 UTF-8 | — |
| `PAYLOAD_TOO_LARGE` | 413 | `POST /echo` 请求体超过 1 MiB | `Connection: close`（响应后销毁连接） |
| `INTERNAL_ERROR` | 500 | 未预期异常（兜底） | — |

补充约定：

- 400/413/415 的判定顺序：**413 优先于 415 与 400**（按累计字节流超限时立即中止并响应 413）；415 判定在解析之前。
- 所有响应体均为单行 JSON（`JSON.stringify` 无缩进），末尾不加换行。
- 错误响应同样使用 `Content-Type: application/json; charset=utf-8`。

## 6. 状态码汇总

| 状态码 | 场景 |
| --- | --- |
| 200 | `/health` 成功；`/echo` 成功 |
| 400 | `INVALID_JSON` |
| 404 | `NOT_FOUND` |
| 405 | `METHOD_NOT_ALLOWED` |
| 413 | `PAYLOAD_TOO_LARGE` |
| 415 | `UNSUPPORTED_MEDIA_TYPE` |
| 500 | `INTERNAL_ERROR` |

不使用 201/204/301/302 等其他状态码。

## 7. 边界情况清单

1. `GET /health?foo=bar` → 200 `{"ok":true}`。
2. `HEAD /health` → 405（不实现 HEAD 快捷返回）。
3. `POST /health` → 405，`Allow: GET`。
4. `GET /echo` → 405，`Allow: POST`。
5. `POST /echo` 无 `Content-Type` → 415。
6. `POST /echo` `Content-Type: text/plain` → 415。
7. `POST /echo` `Content-Type: application/json; charset=utf-8` → 正常解析。
8. `POST /echo` 空体（0 字节）→ 400 `INVALID_JSON`。
9. `POST /echo` 体 `{"a":` → 400 `INVALID_JSON`。
10. `POST /echo` 体 `{"a":1}{"b":2}` → 400 `INVALID_JSON`（严禁只解析首个值）。
11. `POST /echo` 体为 `null` / `0` / `"s"` / `[]` → 200，`body` 原样透传。
12. `POST /echo` 体恰好 1048576 字节且合法 → 200；1048577 字节 → 413。
13. 请求体为非法 UTF-8 字节序列 → 400 `INVALID_JSON`。
14. 未知路径任意方法（如 `DELETE /nope`）→ 404（不返回 405）。
15. 客户端中途断开导致 `req` 报 `aborted`/`error` → 直接销毁连接，不再写响应，不抛出未捕获异常。
16. 响应统一显式设置 `Content-Length`，不要依赖隐式 chunked。

## 8. 实现接口（模块导出契约，implementer 必须遵守）

- 入口文件：`src/server.js`，导出 `createServer(options?)`，返回**未监听**的 `http.Server` 实例；`options` 允许覆盖 `maxBodyBytes`（默认 1048576）与 `now`（默认 `() => new Date()`）。
- 可执行入口：`src/index.js` 调用 `createServer()` 并 `listen(process.env.PORT || 3000)`，打印一行监听日志到 stdout。
- 测试：`test/*.test.js` 使用 `node:test` + `node:assert`，通过 `listen(0)` 取随机端口发起真实 HTTP 请求，断言**状态码 + Content-Type + JSON 结构**三层。
- 导出给测试的错误码常量（可选但推荐）：`ERROR_CODES = { NOT_FOUND, METHOD_NOT_ALLOWED, UNSUPPORTED_MEDIA_TYPE, INVALID_JSON, PAYLOAD_TOO_LARGE, INTERNAL_ERROR }`。

## 9. 下游必须遵守的字段与不变量

- 成功体只有两处：`{"ok":true}` 与 `{"receivedAt":<ISO string>,"body":<any>}`。
- 错误体固定嵌套：`error.code`（大写下划线字符串）与 `error.message`（非空字符串）。
- `Content-Type` 恒为 `application/json; charset=utf-8`（所有 200 与 4xx/5xx 响应）。
- 405 响应必须带 `Allow` 头；413 响应必须带 `Connection: close`。
- 路径匹配不做归一化（不折叠尾斜杠、不忽略大小写），但必须丢弃查询字符串。
- 其余路径与方法一律 404/405，不存在隐性兼容别名。
