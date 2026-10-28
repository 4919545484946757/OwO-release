# API 设计文档：节点健康检查与回显服务

状态：定稿（v2）　负责人：architect　下游实现者：implementer / tester
实现文件：`src/server.js`（Node 内置 `http`，零第三方依赖）；测试文件：`tests/api.test.js`（`node:test`）。

## 1. 目标

提供一个最小可用的 HTTP API 服务，满足：

1. `GET /health` 返回存活探测结果，响应体 JSON 深等于 **`{"ok":true}`**（仅此一个键）。
2. `POST /echo` 接收 JSON 请求体，返回 **`{"echo": <请求解析值>}`**。
3. 未匹配路由返回 **404**，响应体为 JSON（`error.code = "NOT_FOUND"`）。
4. 请求体非法（空体 / 非法 JSON）返回 **400**，响应体为 JSON（`error.code = "INVALID_JSON"`）。
5. 运行时：Node.js 内置 `http` 模块（不引入 Web 框架）；测试：`node:test`（+ 内置 `fetch`/`http`）。

## 2. 非目标（Out of Scope）

- 不做鉴权、限流、CORS、压缩、TLS 终止。
- 不做持久化、外部依赖、消息队列。
- 不做 GET/POST 之外的业务语义路由（如 PUT/DELETE/PATCH 的业务行为）。
- 不做 OpenAPI/Swagger 生成；`/health` 不探活下游依赖。
- 不做分页、批量接口、指标（metrics）端点。

## 3. 约定

- 传输：HTTP/1.1，UTF-8。
- **所有**响应体均为 JSON，且统一带 `Content-Type: application/json; charset=utf-8`（含 4xx/5xx）。
- 请求体上限：**1 MiB（1048576 字节）**，超出 → 413。
- 监听地址由环境变量控制，默认 `HOST=127.0.0.1`、`PORT=3000`（`PORT` 非整数时回落 3000）。
- 测试必须用临时端口（`listen(0)`），不得占用 3000。

## 4. 资源与路由

| 方法 | 路径 | 作用 | 请求体 | 成功状态码 |
| --- | --- | --- | --- | --- |
| GET | `/health` | 存活探测 | 无（有也忽略） | 200 |
| POST | `/echo` | JSON 回显 | JSON | 200 |
| 任意其他 | 任意未匹配路径 | 未找到 | 不读取 | 404 |

路由匹配规则：

- 以 **pathname** 匹配，忽略 query string（`/health?probe=1` 命中 `GET /health`）。
- 大小写敏感，不宽容尾斜杠：`/health/` → 404。
- 路径存在但方法不支持（`POST /health`、`GET /echo`、`HEAD /health`）→ **405**，并带 `Allow` 头（`/health` → `Allow: GET`；`/echo` → `Allow: POST`）。

## 5. 请求 / 响应模型

### 5.1 GET /health

请求：无请求体（携带则忽略，不报错）。

成功响应 **200**：

```json
{"ok":true}
```

| 字段 | 类型 | 必现 | 取值 |
| --- | --- | --- | --- |
| `ok` | boolean | 是 | 恒为 `true` |

响应体不得含多余键（如 `status`、`uptime`），以便字节级/深比较断言。

### 5.2 POST /echo

请求头：

- `Content-Type` 必须为 `application/json`（允许带参数如 `application/json; charset=utf-8`；比较忽略大小写与参数；重复头取第一个）。
- 缺失或类型不符（如 `text/plain`）→ **415**。

请求体：

- 必须为合法 JSON（RFC 8259），解析后可为**任意 JSON 值**：对象、数组、字符串、数字、布尔、`null`。
- 空体（0 字节或仅空白）→ **400**。
- 语法错误 → **400**。
- 超过 1 MiB → **413**。

成功响应 **200**（解析后回显，非原始字节）：

```json
{"echo": <请求 JSON 解析后的原值>}
```

| 字段 | 类型 | 必现 | 取值 |
| --- | --- | --- | --- |
| `echo` | 任意 JSON | 是 | 与请求 JSON **深比较相等**的值（键序、空白、转义形式不保证逐字节相同） |

示例：

```
POST /echo
Content-Type: application/json
{"message":"hi","n":42}
→ 200 {"echo":{"message":"hi","n":42}}
```

```
POST /echo  body: [1,2,3]        → 200 {"echo":[1,2,3]}
POST /echo  body: "hello"        → 200 {"echo":"hello"}
POST /echo  body: null           → 200 {"echo":null}
```

> 顶层 `null`、数字 `0`、`false` 均为合法 JSON，**不得**被当作“空体”返回 400。

### 5.3 未匹配路由 404

```json
{"error":{"code":"NOT_FOUND","message":"Route not found","path":"/unknown"}}
```

`path` 为请求 pathname（不含 query）。404 响应体必须是合法 JSON，不得返回 HTML 或空体。

## 6. 错误模型

所有 4xx/5xx 统一结构：

```json
{"error":{"code":"INVALID_JSON","message":"Human readable message","path":"/echo"}}
```

| 字段 | 类型 | 必现 | 说明 |
| --- | --- | --- | --- |
| `error` | object | 是 | 错误包装对象 |
| `error.code` | string | 是 | 大写下划线枚举，见下表 |
| `error.message` | string | 是 | 非空、面向人读；不承诺逐字稳定 |
| `error.path` | string | 是 | 请求 pathname |
| `error.details` | object | 否 | 可选补充，如 `{"limitBytes":1048576}` |

### 6.1 错误码表

| 状态码 | `code` | 触发条件 |
| --- | --- | --- |
| 400 | `INVALID_JSON` | 请求体为空/仅空白，或 JSON 解析失败（含请求体传输中断） |
| 404 | `NOT_FOUND` | pathname 未匹配任何路由 |
| 405 | `METHOD_NOT_ALLOWED` | 路径存在但方法不支持，响应带 `Allow` 头 |
| 413 | `PAYLOAD_TOO_LARGE` | 请求体超过 1 MiB |
| 415 | `UNSUPPORTED_MEDIA_TYPE` | `POST /echo` 的 `Content-Type` 缺失或非 `application/json` |
| 500 | `INTERNAL_ERROR` | 未预期异常兜底；仍为上述 JSON 结构，不泄漏堆栈 |

### 6.2 状态码矩阵（验收断言用）

| 场景 | 方法/路径 | Content-Type | 请求体 | 期望状态 | 期望 `error.code` |
| --- | --- | --- | --- | --- | --- |
| 健康 | GET /health | — | — | 200 | —（`{"ok":true}`） |
| 健康带 query | GET /health?probe=1 | — | — | 200 | — |
| 回显对象 | POST /echo | application/json | `{"a":1}` | 200 | — |
| 回显数组 | POST /echo | application/json | `[1,2]` | 200 | — |
| 回显标量/null | POST /echo | Application/JSON | `"s"` / `0` / `true` / `null` | 200 | — |
| 语法错误 | POST /echo | application/json | `{bad` | 400 | INVALID_JSON |
| 空体 | POST /echo | application/json | 空 | 400 | INVALID_JSON |
| 类型不符 | POST /echo | text/plain | `hi` | 415 | UNSUPPORTED_MEDIA_TYPE |
| 超大 | POST /echo | application/json | >1 MiB | 413 | PAYLOAD_TOO_LARGE |
| 未知路径 | GET /nope?x=1 | — | — | 404 | NOT_FOUND |
| 尾斜杠 | GET /health/ | — | — | 404 | NOT_FOUND |
| 方法不符（health） | POST /health | — | — | 405 | METHOD_NOT_ALLOWED（`Allow: GET`） |
| 方法不符（echo） | GET /echo | — | — | 405 | METHOD_NOT_ALLOWED（`Allow: POST`） |

## 7. 边界条件与判定顺序

实现必须按下述顺序判定，保证状态码可预测：

1. 解析 URL → 取 pathname。
2. 路径未命中 → 404（**不读取**请求体）。
3. 路径命中但方法不支持 → 405（**不读取**请求体）。
4. 仅 `POST /echo`：校验 `Content-Type` → 415。
5. 累积请求体，超 1 MiB 立即中断 → 413。
6. 0 字节 / 仅空白 / JSON 解析失败 → 400。
7. 成功 → 200 回显。

其他边界：

- 重复 `Content-Type` 头、大小写变体（`Application/JSON`）均按 `application/json` 处理。
- 413/400 后立即结束响应，避免继续缓冲；超限后服务端主动断开剩余请求体导致的客户端 `ECONNRESET`/`EPIPE` 属允许行为，但服务端不得抛未捕获异常。
- 客户端提前断开时不得产生未捕获异常（`unhandledRejection`/`uncaughtException`）。
- `HEAD /health` 按未支持方法处理 → 405（不额外实现 HEAD 语义）。
- 恰好 ≤ 1 MiB 的合法请求体必须成功回显（上限为严格大于才拒绝）。

## 8. 非功能要求

- 启动时向 stdout 打一行监听地址（可选），不影响响应。
- 支持 `SIGTERM`/`SIGINT` 优雅关闭（非验收阻断项）。
- 本地单请求 P95 延迟 < 50ms，无外部依赖。

## 9. 下游必须遵守的字段（契约摘要）

实现与测试必须逐项满足：

1. `GET /health` → **200** → 响应体 JSON 深等于 `{"ok":true}`（唯一键 `ok`，布尔 `true`）。
2. `POST /echo` → **200** → 响应体形如 `{"echo": <请求解析值>}`，`echo` 键必现；空对象 `{}` 必须回显 `{"echo":{}}`（不得省略 `echo`）。
3. 未匹配路由 → **404** → `error.code === "NOT_FOUND"`，`error.path` 为 pathname。
4. 空体或非法 JSON → **400** → `error.code === "INVALID_JSON"`。
5. 方法不匹配 → **405** + `error.code === "METHOD_NOT_ALLOWED"` + `Allow` 头。
6. `Content-Type` 缺失或非 `application/json` → **415** + `error.code === "UNSUPPORTED_MEDIA_TYPE"`。
7. 请求体 > 1 MiB → **413** + `error.code === "PAYLOAD_TOO_LARGE"`。
8. 所有响应（含 4xx/5xx）均为 JSON，`Content-Type: application/json; charset=utf-8`。
9. 实现：Node 内置 `http`，零第三方依赖；测试：`node:test` + 内置 `fetch`/`http`，临时端口。
10. 默认监听 `HOST=127.0.0.1`、`PORT=3000`，环境变量可覆盖。

### 9.1 导出接口（测试依赖，不可改名）

`src/server.js` 必须 `module.exports` 导出：`createServer()`（返回未监听的 `http.Server`）、`start(env?)`、`MAX_BODY_BYTES`（值为 `1048576`）。测试通过 `createServer()` + `listen(0)` 启动。
