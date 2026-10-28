# API 设计文档（Node.js 原生 http 服务）

本文件是 `api-service` 项目的接口契约来源，下游实现与测试必须逐字遵守本文的路径、字段名、状态码与错误码。

## 1. 目标

- 使用 Node.js 内置 `http` 模块实现一个零第三方依赖的 HTTP 服务。
- 提供健康检查接口与 JSON 回显接口，保证可被自动化测试稳定断言。
- 使用 `node:test` + `node:assert` 编写测试，测试通过真实 `http.createServer` 监听临时端口发起请求。

## 2. 运行与约定

- 端口：由环境变量 `PORT` 指定，缺省 `3000`；测试中传 `0` 让系统分配可用端口。
- 监听地址：`127.0.0.1`（本机回环），避免测试暴露到外网。
- 所有响应统一携带 `Content-Type: application/json; charset=utf-8`，包括错误响应。**不存在 HTML 或纯文本响应**。
- 所有响应体都是 UTF-8 编码的 JSON 对象（顶层为 object，不是数组或裸值）。
- 字段命名：全部使用 snake_case 之外的既有字段仅限本文列出者；未列出的字段一律不返回。

## 3. 资源与路由

| 方法 | 路径 | 说明 | 成功状态码 |
| --- | --- | --- | --- |
| GET | `/health` | 健康检查 | 200 |
| POST | `/echo` | JSON 回显 | 200 |
| * | 其它任意未定义路径 | 未找到 | 404 |
| 任意非上述方法命中 `/health` 或 `/echo` | 同路径 | 方法不允许 | 405 |

- 路径匹配为**精确匹配**，区分查询串时忽略 `query string`（`/health?x=1` 等价于 `/health`，仍返回 200）。
- 路径**不忽略**末尾斜杠造成的差异：`/health/` 视为未定义路径，返回 404。
- 不做路由前缀匹配，不做正则路由。

## 4. 接口契约

### 4.1 GET /health

请求：无 body，无必需头，忽略 `Content-Type`。

响应 200：

```json
{"ok": true}
```

- 响应体必须**恰好**是上述对象的等价 JSON：唯一字段 `ok`，类型为布尔 `true`。
- 不得新增 `version`、`uptime`、`time` 等额外字段（保持断言稳定）；如需扩展须另起接口。

### 4.2 POST /echo

请求：

- `Content-Type` 必须为 `application/json`（大小写不敏感，允许带 `; charset=utf-8` 参数）。缺失或其它类型 → 415。
- body 必须是合法 JSON，且解析结果必须是 JSON 对象（object）：
  - 非法 JSON（语法错误、空 body、截断 body）→ 400。
  - 合法 JSON 但顶层不是对象（如 `[]`、`"str"`、`1`、`null`、`true`）→ 400，错误码 `INVALID_BODY_TYPE`。
- body 上限：`1 MiB`（`1048576` 字节）。超出上限 → 413，错误码 `PAYLOAD_TOO_LARGE`；实现可立即中止读取并返回，无需消费完剩余字节。
- 允许的 `Content-Type` 参数忽略；不处理 `charset` 以外的编码，非 UTF-8 声明不做特殊转换，按 UTF-8 解码。

响应 200：

```json
{"echo": {"hello": "world"}}
```

- 唯一字段 `echo`，值为请求解析后的 JSON 对象原样回显（键顺序与空白不保证，语义相等即可）。
- 回显保持嵌套结构不变，不做字段过滤、脱敏或类型转换。
- 顶层空对象 `{}` 是合法输入，返回 `{"echo": {}}`。

## 5. 错误响应格式

所有错误响应体都是统一结构，且**必须同时**包含 `error` 与 `message` 两个字段：

```json
{"error": "CODE", "message": "human readable message"}
```

- `error`：机器可读大写下划线错误码，见下表。
- `message`：英文可读说明，内容不参与断言（测试只断言 `error` 与状态码）。

## 6. 错误码与状态码映射

| HTTP 状态码 | error 错误码 | 触发条件 |
| --- | --- | --- |
| 400 | `INVALID_JSON` | body 不是合法 JSON（含空 body、语法错误） |
| 400 | `INVALID_BODY_TYPE` | JSON 合法但顶层不是对象 |
| 404 | `NOT_FOUND` | 路径未在路由表中定义 |
| 405 | `METHOD_NOT_ALLOWED` | 路径存在但方法不匹配 |
| 413 | `PAYLOAD_TOO_LARGE` | 请求体超过 1 MiB |
| 415 | `UNSUPPORTED_MEDIA_TYPE` | `POST /echo` 的 Content-Type 不是 application/json |
| 500 | `INTERNAL_ERROR` | 未捕获异常兜底，进程不崩溃 |

状态码优先级（同一请求多处违规时，按此顺序判定并只返回第一个错误）：
1. 413（超限，读取阶段即可判定）
2. 404（路由不存在）
3. 405（方法不匹配）
4. 415（媒体类型）
5. 400（body 解析/类型）

## 7. 边界与非目标

边界（属于本设计范围，必须实现）：

- 主机头、查询串、重复请求头、超长 URL（≥ 8 KiB）不导致崩溃；超长 URL 走 404 而非 414。
- 同一连接上的连续请求（keep-alive）不串扰响应。
- 并发请求相互隔离，互不影响 body 解析。

非目标（明确不做）：

- 认证/鉴权、限流、CORS 预检、TLS、压缩（gzip/br）。
- 持久化、日志采集、指标端点、优雅停机信号处理之外的运维能力。
- 请求体流式回传、非 JSON 内容类型（如 `text/plain`、`multipart/form-data`）支持。
- 非 UTF-8 字符集的解码转换。
- 框架与第三方依赖（不引入 express 等）。

## 8. 下游必须遵守的契约字段清单

- 路由：`GET /health`、`POST /echo`。
- 成功体：`{"ok": true}`、`{"echo": <object>}`。
- 错误体：必含 `error`、`message` 两个键。
- 错误码字面量：`INVALID_JSON`、`INVALID_BODY_TYPE`、`NOT_FOUND`、`METHOD_NOT_ALLOWED`、`PAYLOAD_TOO_LARGE`、`UNSUPPORTED_MEDIA_TYPE`、`INTERNAL_ERROR`。
- 状态码：200 / 400 / 404 / 405 / 413 / 415 / 500。
- 响应头：`Content-Type: application/json; charset=utf-8`。
- 体积上限：`1048576` 字节。
