# REST API 设计（api-service）

## 1. 目标

提供一个最小的 REST API 服务（Node.js 原生 `http` 模块实现，测试使用 `node:test`）：

- `GET /health` → `200`，返回 `{"ok":true}`，用于健康检查 / 存活探针。
- `POST /echo` → 接收 JSON 请求体并原样回显。
- 未匹配的路由 → `404`。
- 请求体不是合法 JSON（但目标是 `/echo`）→ `400`。

非目标：持久化、鉴权、限流、HTTPS、多实例编排、业务领域逻辑、请求体超大文件上传。

## 2. 资源与路由

| 方法 | 路径 | 用途 | 请求体 | 成功状态码 |
| --- | --- | --- | --- | --- |
| GET | `/health` | 健康检查 | 无 | 200 |
| POST | `/echo` | JSON 回显 | JSON 对象 | 200 |
| 任意 | 其它路径 | 未定义路由 | — | 404 |

路由匹配规则（下游必须遵守）：

- 路径比较基于 `URL` 解析后的 `url.pathname`，**忽略 query string**（`/echo?x=1` 视为 `/echo`）。
- 路由大小写敏感，不做尾斜杠归一化：`/health/` 视为未匹配 → `404`。
- 方法不匹配（如 `GET /echo`、`POST /health`）→ `404`（本服务不实现 405），如需 405 必须由上游契约变更决定。
- 其它方法（`PUT` / `DELETE` / `PATCH` / `OPTIONS` / `HEAD` 等）一律 → `404`。

## 3. 请求 / 响应 JSON

### 3.1 `GET /health`

请求：无 body（发送 body 可忽略）。

响应 `200`，`Content-Type: application/json; charset=utf-8`：

```json
{"ok":true}
```

约定：响应体为单键对象 `ok`，值必须是布尔 `true`（不能是字符串 `"true"` 或 `1`）。

### 3.2 `POST /echo`

请求头要求：

- 建议 `Content-Type: application/json`。**注意：本设计不强制校验 Content-Type**（MUST 以 JSON 解析结果为准），以便简单集成；如后续需要严格校验，属于非目标中的后续演进。
- 请求体最大长度限制：`1048576` 字节（1 MiB）。超限 → `413`（见 4.1，标记为可选实现；若下游选择不实现 413，必须至少保证不崩溃并返回 400 或 413 之一，且必须在实现说明中标注）。

请求体示例：

```json
{"name":"OwO","n":1,"nested":{"a":[1,2,3]},"ok":false}
```

响应 `200`，`Content-Type: application/json; charset=utf-8`，响应体为**请求体 JSON 的等价序列化**（语义相等，键顺序、空白不保证与请求完全一致）：

```json
{"name":"OwO","n":1,"nested":{"a":[1,2,3]},"ok":false}
```

回显契约要点（下游必须遵守）：

- 回显的是**解析后的 JSON 值**，不是原始字符串切片；因此若请求体为 `{ "a" : 1 }`，响应可以是 `{"a":1}`。
- 顶层值允许任意合法 JSON 值：对象、数组、字符串、数字、布尔、`null`；不得因为顶层是数组或字面量而返回 400。
- 空 body（0 字节）视为非法 JSON → `400`。
- 非法 JSON（语法错误、被截断、多余尾随字符）→ `400`。
- 允许重复键：以解析器（`JSON.parse`）语义为准（后者覆盖前者），不额外报错。

## 4. 错误响应

所有错误响应统一为 JSON 对象：

```json
{"error":"<code>","message":"<human readable>"}
```

- `error`：稳定的机器可读错误码，小写下划线（在本 API 中固定为下面列出的枚举值）。
- `message`：人类可读说明，仅用于排障，**下游不得依赖其具体文案**。

### 4.1 错误码枚举

| HTTP 状态码 | error | 触发条件 | 备注 |
| --- | --- | --- | --- |
| 400 | `bad_request` | `POST /echo` 请求体为空或不是合法 JSON | 必须实现 |
| 400 | `bad_request` | 其它请求体不可解析/解析异常（包括超限降级为 400 时的兜底） | 必须实现 |
| 404 | `not_found` | 路径未匹配任何已定义路由，或方法不匹配 | 必须实现 |
| 405 | — | 本设计不使用 405 | 非目标 |
| 413 | `payload_too_large` | 请求体超过 1 MiB 上限 | 可选实现（若实现则必须用该状态码与错误码） |
| 415 | — | 本设计不校验 Content-Type，不使用 415 | 非目标 |
| 500 | `internal_error` | 未捕获异常，统一兜底，避免进程崩溃 | 必须实现 |

## 5. 状态码汇总

- 成功：`200`（`/health`、`/echo`）。
- 客户端错误：`400`（非法 JSON / 空体）、`404`（未知路由或方法不匹配）、可选 `413`（体过大）。
- 服务端错误：`500`（未捕获异常兜底）。

## 6. 边界与约束

1. **通用响应头**：所有 JSON 响应设置 `Content-Type: application/json; charset=utf-8`；`Content-Length` 必须与实际 body 字节数一致（UTF-8 字节而非字符数）。
2. **不崩溃**：请求体非法、路径奇怪、方法未知、客户端中途断开等情况不得导致进程退出；解析异常必须被 `try/catch` 捕获。
3. **并发**：同一进程内可并发处理多个请求；实现不得依赖全局可变状态（无跨请求共享的缓冲区复用）。
4. **端口与监听**：通过环境变量 `PORT` 指定，缺省 `3000`；监听地址缺省 `0.0.0.0`（`HOST` 可覆盖）。启动成功后可选打印 `listening` 日志（不作为契约）。
5. **优雅关闭**：收到 `SIGTERM` / `SIGINT` 时调用 `server.close()` 并退出；不作为对外 API 契约，但属于非崩溃要求的一部分。
6. **URL 编码**：`pathname` 使用原始（percent-encoded）形式比较；不实现路径参数解码（本 API 无路径参数）。
7. **HTTP/1.1 keep-alive**：必须正确结束响应的可读流读取（`req.resume()` 或读取完 body），避免连接挂起。
8. **字符编码**：请求体按 UTF-8 解码（使用 `Buffer` 拼接后再 `toString('utf8')`，不要按 chunk 边界切片解码）。

## 7. 非目标

- 不做鉴权/鉴权头解析（即任何请求头都不影响 `200` 语义，除非上面明确写了）。
- 不做 CORS 预检、不做 `OPTIONS` 处理。
- 不做 gzip/压缩、不做缓存头、不做 ETag。
- 不做路由注册框架、中间件体系、OpenAPI 文档生成（本文件即契约来源）。
- 不做持久化与幂等键。
- 不做 405 / 415 / 429 / 503 语义。
- 不实现 `/echo` 的白名单字段（任意 JSON 均可回显）。

## 8. 下游必须遵守的字段（契约摘要）

| 项 | 必须值 |
| --- | --- |
| `/health` 响应体 | `{"ok":true}`（`ok` 为布尔 true） |
| `/echo` 请求方法 | `POST` |
| `/echo` 成功状态码 | `200` |
| `/echo` 响应体 | 请求 JSON 的等价序列化（任意合法 JSON 顶层值） |
| 非法/空 JSON 状态码 | `400` |
| 非法/空 JSON 错误码 | `bad_request` |
| 未知路由/方法不匹配状态码 | `404` |
| 未知路由错误码 | `not_found` |
| 错误体形状 | `{"error":"<code>","message":"<text>"}` |
| 响应 Content-Type | `application/json; charset=utf-8` |
| 监听端口默认值 | `3000`（可用 `PORT` 覆盖） |
| 请求体上限 | `1048576` 字节（可选 413 `payload_too_large`） |
| 实现运行环境 | Node.js 原生 `http`，无第三方运行时依赖 |
| 测试 | `node:test`（`node --test`），不使用外部测试框架 |

## 9. 验收用例（供 test 角色映射为 `node:test`）

1. `GET /health` → 200，body 解析后 `deepStrictEqual({ok:true})`，`Content-Type` 含 `application/json`。
2. `POST /echo` 体 `{"a":1}` → 200，body 解析后等于 `{a:1}`。
3. `POST /echo` 体 `{"a":1`（截断）→ 400，`error === "bad_request"`。
4. `POST /echo` 空 body → 400，`error === "bad_request"`。
5. `GET /nope` → 404，`error === "not_found"`。
6. `GET /echo`（方法不匹配）→ 404，`error === "not_found"`。
7. `POST /echo` 体为 `[1,2]` → 200，body 解析后等于 `[1,2]`。
8. `GET /health?x=1` → 200（query 被忽略）。
