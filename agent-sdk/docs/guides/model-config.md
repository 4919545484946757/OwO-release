# 模型配置（config.json）

OwO Agent 的模型配置写在**一个独立配置文件**里，风格对齐 codex / opencode：
地址、模型名、API Key 三样东西都在同一个文件里，可以直接用记事本手改。

## 文件位置

```
%LOCALAPPDATA%\OwO\Agent\config.json
```

- 也可以用环境变量 `OWO_CONFIG_FILE` 指向别处（多套配置/排障用）。
- 文件**不在项目目录里**：设计上避免被 `git add` 提交进仓库。
- 首次保存时文件权限会被收紧为"仅当前用户"，因为里面可能明文存放密钥。
- 设置页「模型」里有「定位」按钮，直接在资源管理器里选中该文件。

## 文件结构

```json
{
  "version": 1,
  "model": {
    "provider": "bigmodel",
    "base_url": "https://open.bigmodel.cn/api/paas/v4",
    "name": "glm-5.3-flash",
    "api_key": "",
    "api_key_env": "OPENAI_API_KEY",
    "context_window": 128000,
    "max_output_tokens": 32000,
    "model_output_tokens": {
      "glm-5.3-flash": 24000,
      "qwen3.8-max": 32000
    },
    "temperature": 0.7,
    "timeout_secs": 180,
    "keep_recent": 20,
    "compaction": true,
    "models": ["glm-5.3-flash", "glm-4.6", "glm-4.5-air"]
  }
}
```

| 字段 | 说明 |
|---|---|
| `provider` | 服务提供方：`bigmodel` / `openai` / `deepseek` / `dashscope` / `ollama` / `custom` / `unset`。历史值 `cloud` 等价于 `bigmodel` |
| `base_url` | 接口地址（OpenAI 兼容）。留空则用该 provider 的默认地址 |
| `name` | 模型名称。留空则用该 provider 的默认模型（历史字段名 `model` 仍可读） |
| `api_key` | **可选**。填了就用它；留空则去读 `api_key_env` 指向的环境变量 |
| `api_key_env` | 凭据环境变量名，缺省 `OPENAI_API_KEY` |
| `context_window` | **上下文窗口（token）**。核心据此设置压缩预算；留空 = 核心默认 60000 |
| `max_output_tokens` | 默认单次回复最大输出 token；缺省为 32000（32k），有效范围 1–1,000,000 |
| `model_output_tokens` | 可选的模型名 → 输出 token 上限映射（每项 1–1,000,000）；精确匹配实际请求模型，未配置时回退到默认值 |
| `temperature` | 采样温度（0–2）；留空 = 核心默认 |
| `timeout_secs` | 单次模型请求超时秒数；留空 = 核心默认 |
| `keep_recent` | 压缩时保留最近多少条消息；留空 = 核心默认 20 |
| `compaction` | 是否启用上下文压缩（`true`/`false`）；留空 = 核心默认开启 |
| `models` | **可选的模型清单**：写在这里的模型名会作为界面建议项出现。界面候选 = 本清单 + 内置预设 + 任意手输 |

没有任何一项是"写死在代码里"的：上面每个字段都能改文件生效；界面上对应
「模型」页里的输入框，留空即"用核心默认"。

文件里出现我们不认识的字段会被**原样保留**（不会因为一次保存就被抹掉）。

## 字段怎么生效（壳 → 核心）

桌面壳读本文件后，把模型配置注入核心子进程的环境变量（核心按同名变量消费）：

| 配置字段 | 环境变量 |
|---|---|
| `base_url` | `OPENAI_BASE_URL` |
| `name` | `OPENAI_MODEL` |
| `api_key` / `api_key_env` | `OPENAI_API_KEY` |
| `context_window` | `OWO_MODEL_CONTEXT_WINDOW` |
| `max_output_tokens` | `OWO_MODEL_MAX_OUTPUT_TOKENS` |
| `model_output_tokens` | `OWO_MODEL_OUTPUT_TOKENS_BY_MODEL`（JSON 对象） |
| `temperature` | `OWO_MODEL_TEMPERATURE` |
| `timeout_secs` | `OWO_MODEL_TIMEOUT_SECS` |
| `keep_recent` | `OWO_AGENT_KEEP_RECENT` |
| `compaction` | `OWO_AGENT_COMPACTION` |

桌面端默认输出上限为 32000（32k）；`model_output_tokens` 可为不同模型单独覆盖，应用配置范围为 1–1,000,000。未填写的单模型项回退到默认值。实际可用上限仍取决于所选模型与服务商；若超出其限制，服务商可能拒绝请求。

## 凭据优先级

```
model.api_key（配置文件）
  → model.api_key_env 指向的环境变量（缺省 OPENAI_API_KEY）
    → OPENAI_API_KEY
      → DASHSCOPE_API_KEY（历史兼容，仅提示）
```

密钥纪律：密钥只在本机进程间流动（壳 → 核心子进程环境变量），
**不会回传前端页面、不写日志**；设置页只显示来源与掩码（如 `sk-a…mnop`）。

## 常见配置

**智谱 BigModel（默认）**

```json
{ "version": 1, "model": {
    "provider": "bigmodel",
    "base_url": "https://open.bigmodel.cn/api/paas/v4",
    "name": "glm-5.3-flash",
    "api_key_env": "OPENAI_API_KEY" } }
```

**OpenAI / DeepSeek / 阿里 DashScope（兼容模式）**：改 `provider` 与 `name` 即可，
`base_url` 可留空走默认（`https://api.openai.com/v1`、`https://api.deepseek.com/v1`、
`https://dashscope.aliyuncs.com/compatible-mode/v1`）。

**本地 Ollama（不需要密钥）**

```json
{ "version": 1, "model": {
    "provider": "ollama",
    "base_url": "http://127.0.0.1:11434/v1",
    "name": "qwen2.5:3b" } }
```

**自建 / 中转网关（OpenAI 兼容）**

```json
{ "version": 1, "model": {
    "provider": "custom",
    "base_url": "https://your-gateway.example.com/v1",
    "name": "your-model",
    "api_key": "sk-your-key" } }
```

## 生效方式

改完文件后需要**重启核心服务**才生效：

- 设置页：点「保存并重启核心」（推荐，会顺带校验地址/模型名形状）；
- 手改文件：托盘菜单「退出」后重新打开工作台；
- CLI：`owo-agent serve` 重启。

校验规则（保存时拒绝，避免写出连不上的配置）：

- `base_url` 必须以 `http://` 或 `https://` 开头；
- `provider: custom` 必须填 `base_url`；
- 模型名不能含空格；环境变量名不能含空格或 `=`。

## 与工作区设置的区别

- `config.json`（本文件，**全局**）：模型地址、模型名、凭据 —— 跟"用哪个模型"有关；
- `<workspace>/settings.json`（**按项目**）：只读模式、危险命令黑名单、
  数据出境开关、技能启停、可选工具能力 —— 跟"在这个项目里怎么干活"有关。

会话级临时换模型（只影响一条会话、不重启核心）：设置页「当前会话使用的模型」，
或直接 `POST /session/{id}/model`。
