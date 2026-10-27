# WorkSwarm Echo API 设计文档

版本：v1（草案）
状态：已定稿，供下游实现与测试直接消费

## 1. 目标

用 Node.js 标准库 `http` 实现一个零依赖的最小 HTTP API 服务，提供健康检查与 JSON 回显两个端点，行为可被 `node:test` 用例逐条断言：

- `GET /health` 返回 `{"ok":true}`。
- `POST /echo` 接收 JSON 请求体，原样回显为 JSON 响应。
- 未知路径返回 404，且响应体同样是 JSON。
- 非法 JSON 请求体返回 400，且响应体同样是 JSON。

## 2. 非目标（Non-goals）

- 不做鉴权、限流、TLS、CORS、压缩、连接复用调优。
- 不引入框架（Express/Koa）或任何第三方依赖。
- 不做持久化、外部调用、日志聚合、指标上报。
- 不实现除 `/health`、`/echo` 之外的任何路由（包括 `/`、`/favicon.ico`）。
- 不保证 HTTP/2、流式大文件上传。

## 3. 资源与路由

| 方法 | 路径 | 说明 | 请求体 | 成功状态码 |
| --- | --- | --- | --- | --- |
| GET | `/health` | 健康检查 | 无 | 200 |
| POST | `/echo` | JSON 回显 | JSON | 200 |

路由匹配规则（实现必须遵守）：

- 比较 `req.url` 的路径部分，忽略查询串（`?` 之后）与末尾多余斜杠的差异不做隐式归一化——**下文以精确匹配为规范**：`/health`、`/ech` 之外一律不匹配。
- 路径匹配区分大小写（`/Health` 视为未知路径 → 404）。
- 方法不匹配（如 `GET /echo`、`POST /health`）一律按未知路由处理，返回 404，不使用 405。

## 4. 请求/响应契约

### 4.1 GET /health

请求：无请求体，客户端若携带请求体应被忽略。

响应 `200`：

```http
Content-Type: application/json; charset=utf-8
```

```json
{"ok":true}
```

字段约束：

- `ok`：布尔，固定 `true`。下游断言必须使用 `body.ok === true`。

### 4.2 POST /echo

请求：

- `Content-Type` 应为 `application/json`（实现不强制校验 Content-Type，但缺失时仍按 JSON 解析）。
- 请求体为合法 JSON（对象、数组、字符串、数字、布尔或 null 均可）。

响应 `200`：

```json
<请求体解析后的值，原样序列化>
```

字段约束（下游必须遵守）：

- 响应体是请求体 JSON 文本解析后的值再序列化的结果；键顺序不保证与请求一致，比较应基于解析后的深比较。
- 顶层为 `null` 时也合法回显 `null`。
- 空请求体（长度 0）视为非法输入 → 400，见 4.3。
- 响应 `Content-Type` 同样为 `application/json; charset=utf-8`。

### 4.3 错误响应统一格式

所有错误响应体为 JSON 对象，形状固定：

```json
{"error": {"code": "<ERROR_CODE>", "message": "<人类可读说明>"}}
```

字段约束（下游必须遵守）：

- 必有 `error` 对象；`error.code` 为下表中的稳定字符串常量；`error.message` 为非空字符串，仅用于调试，**不得**作为断言依据。
- 错误响应同样设置 `Content-Type: application/json; charset=utf-8`。

## 5. 错误码

| HTTP 状态码 | error.code | 触发条件 |
| --- | --- | --- |
| 400 | `INVALID_JSON` | POST /echo 请求体不是合法 JSON（含空请求体、截断 JSON） |
| 404 | `NOT_FOUND` | 路径不存在；或路径存在但方法不匹配（含 `GET /echo`、`POST /health` 以及任意其他方法） |
| 500 | `INTERNAL_ERROR` | 处理过程中出现未预期异常（兜底） |

错误体示例：

```json
{"error":{"code":"NOT_FOUND","message":"Route not found"}}
```

```json
{"error":{"code":"INVALID_JSON","message":"Request body is not valid JSON"}}
```

## 6. 状态码汇总

- `200`：`GET /health`、`POST /echo` 成功。
- `400`：请求体 JSON 解析失败（仅 `/echo`）。
- `404`：未知路由或方法不匹配。
- `500`：未捕获异常兜底。

不使用 3xx、401、403、405、413、415、422。

## 7. 边界行为

- 请求体大小：实现应设置上限（建议 1 MiB）。超限时返回 `400` + `INVALID_JSON`（本项目不引入 413，以保持错误码集合最小化）。
- 字符编码：按 UTF-8 解码；非法 UTF-8 序列导致 JSON 解析失败时同样返回 400 `INVALID_JSON`。
- JSON 语法严格：不接受尾随逗号、注释、单引号；`NaN`/`Infinity` 视为非法。
- `/echo` 对 JSON 字符串 `"..."` 与数字、布尔、null 顶层值一并回显，不做包裹。
- 未知路由的响应体**不**回显请求内容。
- 所有响应在长度已知时设置 `Content-Length`，并显式 `end()` 结束。
- 监听端口通过环境变量 `PORT` 配置，缺省 `3000`；绑定 `127.0.0.1`。
- 并发请求之间不得共享可变状态。

## 8. 下游必须遵守的契约要点（Checklist）

1. `/health` 成功响应体解析后必须深等于 `{"ok": true}`。
2. `/echo` 成功响应体必须与请求 JSON 解析值深相等。
3. 任意 404 场景响应体必须含 `error.code === "NOT_FOUND"`。
4. 非法 JSON 的 400 响应体必须含 `error.code === "INVALID_JSON"`。
5. 所有响应（含错误）`Content-Type` 必须是 `application/json; charset=utf-8`。
6. 测试使用 `node:test` + `node:assert`，不引入第三方测试库；服务需支持注入端口或使用 `server.listen(0)` 随机端口以便测试。
7. 实现文件使用 `node:http`，不使用框架。
