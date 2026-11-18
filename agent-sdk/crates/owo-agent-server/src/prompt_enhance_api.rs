//! 提示词增强：`/prompt/enhance`（用户草稿 → 任务类型识别 → 动态改写 → 结构化结果）。
//!
//! 与 `/expression` 同形的无状态单次模型调用：不建会话、不写历史、不传工具、
//! 不读工作区文件；上下文只采用请求里调用方声明的事实（工作区路径/项目名/
//! 会话模式），未声明的信息一律视为未知，模型不得编造。
//!
//! 模型接入复用核心网关，不引入第二套接入层：预设模型走 `model` 覆盖
//! （`complete_with_model`），桌面保存的自定义模型走 `model_connection`
//! （凭据仅环境变量引用），缺省回落核心默认 provider；连接非法按 400 拒绝，
//! 不悄悄换模型。
//!
//! 输出契约：模型必须返回严格 JSON（`ready` / `needs_clarification`，澄清问题
//! ≤3 个）。解析或校验失败允许带错误回灌重试一次；重试仍失败按 502 如实报错，
//! **绝不**回退到固定文本追加（那正是本次重构要消灭的行为）。
//!
//! 隐私：不落盘、不把提示词正文写进运行日志；凭据只在 provider 内部解析。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use owo_agent_core::gateway::{ChatMessage, ModelOutput, ModelProvider};
use owo_agent_protocol::{PromptEnhanceRequest, PromptEnhanceResponse, PromptEnhanceStatus};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// 交互式动作的模型调用超时（快速文本模型通常远低于此值；修复重试各计一次）。
const ENHANCE_TIMEOUT: Duration = Duration::from_secs(60);
/// 输入上限，与 `/expression` 保持一致。
const MAX_TEXT_CHARS: usize = 8192;
/// 增强结果字符上限：允许结构化改写留余量，但不接受无限输出。
const MAX_ENHANCED_CHARS: usize = 16384;
/// 澄清问题上限（产品契约：最多 3 个，每个都必须有实际必要性）。
const MAX_CLARIFY_QUESTIONS: usize = 3;
/// 单个澄清问题/摘要/类型标识的字符上限（防模型借字段倾倒内容）。
const MAX_QUESTION_CHARS: usize = 500;
const MAX_SUMMARY_CHARS: usize = 512;
const MAX_TASK_TYPE_CHARS: usize = 48;
/// JSON 解析/校验失败后的自动修复次数（带错误回灌一次，不做无限重试）。
const MAX_REPAIR_ATTEMPTS: usize = 1;

/// Prompt Engineer 的动态改写指令：任务类型分策略 + 保留原意 + 克制澄清 +
/// 严格 JSON 输出。这是本功能的"引擎"；改动它必须同步更新 openapi 描述。
const ENHANCE_SYSTEM_PROMPT: &str = r#"你是一名资深 Prompt Engineer。你的工作不是回答问题，也不是执行任务，而是理解用户真正想做什么，把原始输入重新组织成更清晰、更准确、可直接交给 AI 执行的提示词。

先判断任务类型，再按对应策略增强：
- coding（编程开发）：明确功能范围、实现要求、技术栈（优先采用上下文块中已给出的技术栈；未给出则保持开放，不要替用户指定）、兼容性与验收标准。
- frontend（前端设计）：页面结构、视觉风格、布局、交互与响应式要求。
- bugfix（Bug 修复）：问题表现、复现条件、排查范围（事件绑定/调用链/相关组件）、修复目标与验证方式；不扩大到无关模块的重构。
- docs（文档编写）：文档目的、受众、结构、内容要求与交付格式。
- data_analysis（数据分析）：数据来源、分析维度、指标口径与结果展示方式。
- research（深度研究）：研究对象、核心问题、资料范围与论证要求。
- creative（创意写作）：主题、受众、风格、叙事方式与创作边界。
- agent_task（Agent 任务）：最终目标、任务步骤、工具使用边界、执行限制与验收方式。
- translation（翻译/本地化）：保持翻译目标本身，只补充术语一致性与原文格式保留要求。
- general_qa（普通问答）：只优化表达与结构，保留问题本身，绝不扩展成复杂任务。

必须遵守：
1. 保留用户原始意图与明确要求，不改变任务目标，不擅自扩大或缩小范围。
2. 原文中的数字、路径、名称、代码、专有名词必须原样保留。
3. 不编造事实、技术栈、文件名或项目情况；上下文块未提供的信息不要当作已知，确有需要时在提示词中写成待确认项。
4. 不堆砌通用规则，不为显得专业而拉长篇幅；已经清楚的句子只做必要润色。
5. 使用与原文相同的语言（中英混合输入保持自然混合）。
6. 不执行任务、不回答问题、不解释增强过程。

澄清规则（严格执行）：
- 只有缺失信息会实质性影响任务目标、实现方式或交付结果时才提问；能从上下文块确定的不要问，能合理默认的不要问。
- 最多 3 个问题，每个都具体、必要、可直接回答；绝不为提问而提问。
- 不需要澄清时必须返回 ready。

输出格式（严格遵守：只输出一个 JSON 对象，不要用代码块包裹，不要输出任何其他文字）：
{"status":"ready","enhanced_prompt":"改写后的完整提示词","clarifying_questions":[],"change_summary":"一句话说明做了什么优化","task_type":"任务类型标识"}
需要澄清时：
{"status":"needs_clarification","enhanced_prompt":null,"clarifying_questions":["问题1","问题2"],"change_summary":"一句话说明缺什么关键信息","task_type":"任务类型标识"}"#;

fn error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// 用户消息 = 原始草稿 + 调用方已知上下文 +（可选）上一轮澄清问答。
/// 上下文块只在有事实时出现；缺失的信息不会出现在提示里，因此模型无从编造。
fn enhance_user_prompt(request: &PromptEnhanceRequest) -> String {
    let mut sections = vec![format!("## 原始输入\n{}", request.text.trim())];
    if let Some(context) = request.context.as_ref() {
        let mut facts = Vec::new();
        if let Some(workspace) = non_empty(&context.workspace) {
            facts.push(format!("- 工作区路径：{workspace}"));
        }
        if let Some(project) = non_empty(&context.project) {
            facts.push(format!("- 项目名：{project}"));
        }
        if let Some(mode) = non_empty(&context.session_mode) {
            let mode_desc = match mode {
                "agent" => "agent（可执行任务的智能体模式）",
                "chat" => "chat（纯对话模式，无工具执行）",
                other => other,
            };
            facts.push(format!("- 会话模式：{mode_desc}"));
        }
        if !facts.is_empty() {
            sections.push(format!(
                "## 已知上下文（仅为调用方声明的事实；未列出的一律未知，不得编造）\n{}",
                facts.join("\n")
            ));
        }
    }
    if !request.clarifications.is_empty() {
        let answers = request
            .clarifications
            .iter()
            .map(|entry| {
                format!(
                    "- 问：{}\n  答：{}",
                    entry.question.trim(),
                    entry.answer.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "## 用户已确认的澄清问答（把这些答案作为既定事实纳入改写，不要重复提问）\n{answers}"
        ));
    }
    sections.join("\n\n")
}

/// 修复回灌：同一份用户材料 + 上一次输出的具体问题，要求重新输出合法 JSON。
fn enhance_repair_prompt(request: &PromptEnhanceRequest, problem: &str) -> String {
    format!(
        "{}\n\n## 上一次输出的问题\n{}\n\n请重新输出，只输出一个符合格式要求的 JSON 对象。",
        enhance_user_prompt(request),
        problem
    )
}

fn bounded_optional_string(
    value: &Value,
    field: &str,
    limit: usize,
) -> Result<Option<String>, String> {
    let Some(text) = value[field].as_str() else {
        if value[field].is_null() {
            return Ok(None);
        }
        return Err(format!("{field} 必须是字符串或 null"));
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > limit {
        return Err(format!("{field} 超过 {limit} 字符上限"));
    }
    Ok(Some(text.to_string()))
}

fn parse_questions(value: &Value) -> Result<Vec<String>, String> {
    let Some(list) = value["clarifying_questions"].as_array() else {
        if value["clarifying_questions"].is_null() {
            return Ok(Vec::new());
        }
        return Err("clarifying_questions 必须是字符串数组".into());
    };
    let mut questions = Vec::with_capacity(list.len());
    for item in list {
        let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) else {
            return Err("clarifying_questions 中存在空问题".into());
        };
        if text.chars().count() > MAX_QUESTION_CHARS {
            return Err(format!("澄清问题超过 {MAX_QUESTION_CHARS} 字符上限"));
        }
        questions.push(text.to_string());
    }
    Ok(questions)
}

/// 解析并校验模型输出。宽松接受外层代码块/多余文字（extract_json_object 提取
/// 首个平衡对象），但 JSON 内部契约严格：ready 必须带非空 enhanced_prompt 且
/// 不得带问题；needs_clarification 必须 1..=3 个问题且不得带 enhanced_prompt。
fn parse_enhance_output(raw: &str) -> Result<PromptEnhanceResponse, String> {
    let Some(object) = owo_agent_core::workflow_draft::extract_json_object(raw) else {
        return Err("输出中未找到 JSON 对象（只输出一个 JSON 对象，不要代码块或额外文字）".into());
    };
    let value: Value =
        serde_json::from_str(object).map_err(|error| format!("JSON 解析失败：{error}"))?;
    let status = match value["status"].as_str() {
        Some("ready") => PromptEnhanceStatus::Ready,
        Some("needs_clarification") => PromptEnhanceStatus::NeedsClarification,
        other => {
            return Err(format!(
                "status 必须是 \"ready\" 或 \"needs_clarification\"，实际为 {other:?}"
            ))
        }
    };
    let change_summary = bounded_optional_string(&value, "change_summary", MAX_SUMMARY_CHARS)?;
    let task_type = bounded_optional_string(&value, "task_type", MAX_TASK_TYPE_CHARS)?;
    let questions = parse_questions(&value)?;
    match status {
        PromptEnhanceStatus::Ready => {
            let Some(prompt_text) = value["enhanced_prompt"]
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                return Err("ready 状态必须提供非空 enhanced_prompt".into());
            };
            if prompt_text.chars().count() > MAX_ENHANCED_CHARS {
                return Err(format!(
                    "enhanced_prompt 超过 {MAX_ENHANCED_CHARS} 字符上限"
                ));
            }
            if !questions.is_empty() {
                return Err("ready 状态不应包含 clarifying_questions".into());
            }
            Ok(PromptEnhanceResponse {
                status,
                enhanced_prompt: Some(prompt_text.to_string()),
                clarifying_questions: Vec::new(),
                change_summary,
                task_type,
            })
        }
        PromptEnhanceStatus::NeedsClarification => {
            if value["enhanced_prompt"]
                .as_str()
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
            {
                return Err("needs_clarification 状态不应提供 enhanced_prompt".into());
            }
            if questions.is_empty() {
                return Err("needs_clarification 状态必须提供 1-3 个澄清问题".into());
            }
            if questions.len() > MAX_CLARIFY_QUESTIONS {
                return Err(format!(
                    "澄清问题最多 {MAX_CLARIFY_QUESTIONS} 个，实际 {} 个",
                    questions.len()
                ));
            }
            Ok(PromptEnhanceResponse {
                status,
                enhanced_prompt: None,
                clarifying_questions: questions,
                change_summary,
                task_type,
            })
        }
    }
}

/// 一次非流式模型调用（无工具）。错误分类：超时 504、模型侧失败 502。
async fn complete_once(
    provider: &Arc<dyn ModelProvider>,
    model: Option<&str>,
    messages: Vec<ChatMessage>,
) -> Result<String, (StatusCode, Json<Value>)> {
    let completion = tokio::time::timeout(
        ENHANCE_TIMEOUT,
        provider.complete_with_model(model, &messages, &[]),
    )
    .await;
    match completion {
        Ok(Ok(ModelOutput::Text(text))) => Ok(text),
        Ok(Ok(ModelOutput::ToolCalls(_))) => Err(error(
            StatusCode::BAD_GATEWAY,
            "prompt_enhance/tool_calls_unexpected",
        )),
        Ok(Err(model_error)) => Err(error(
            StatusCode::BAD_GATEWAY,
            format!("prompt_enhance/provider_failed: {model_error}"),
        )),
        Err(_) => Err(error(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "prompt_enhance/timeout: 模型调用超时（>{}s）",
                ENHANCE_TIMEOUT.as_secs()
            ),
        )),
    }
}

pub(super) async fn enhance(
    State(state): State<Arc<crate::AppState>>,
    Json(request): Json<PromptEnhanceRequest>,
) -> Result<Json<PromptEnhanceResponse>, (StatusCode, Json<Value>)> {
    let text = request.text.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT_CHARS {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "prompt_enhance/invalid_input",
        ));
    }
    // 优雅关闭门：一次增强即一次模型调用，与 /expression 共用并发闸。
    let _permit = state
        .shutdown_gate
        .try_acquire_turn()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, "prompt_enhance/busy"))?;
    // Provider 选择与 turn 同语义：自定义连接优先（凭据仅环境变量引用），
    // 预设模型走 model 覆盖，缺省回落核心默认；连接非法直接 400，不悄悄换模型。
    let (provider, model_override) = match request.model_connection.as_ref() {
        Some(connection) => (
            owo_agent_core::gateway::custom_model_provider(connection)
                .map_err(|code| error(StatusCode::BAD_REQUEST, format!("prompt_enhance/{code}")))?,
            None,
        ),
        None => (
            state.agent.provider(),
            request
                .model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty()),
        ),
    };
    let messages = vec![
        ChatMessage::system(ENHANCE_SYSTEM_PROMPT.to_string()),
        ChatMessage::user(enhance_user_prompt(&request)),
    ];
    let first = complete_once(&provider, model_override, messages).await?;
    match parse_enhance_output(&first) {
        Ok(parsed) => Ok(Json(parsed)),
        Err(problem) => {
            // 一次修复机会：带上具体错误回灌。重试仍失败按 502 如实报错，
            // 不回退到固定模板追加（那是被本次重构明确禁止的行为）。
            let mut attempts = 0usize;
            let mut last_problem = problem;
            while attempts < MAX_REPAIR_ATTEMPTS {
                attempts += 1;
                let repair_messages = vec![
                    ChatMessage::system(ENHANCE_SYSTEM_PROMPT.to_string()),
                    ChatMessage::user(enhance_repair_prompt(&request, &last_problem)),
                ];
                let retry = complete_once(&provider, model_override, repair_messages).await?;
                match parse_enhance_output(&retry) {
                    Ok(parsed) => return Ok(Json(parsed)),
                    Err(problem) => last_problem = problem,
                }
            }
            Err(error(
                StatusCode::BAD_GATEWAY,
                format!("prompt_enhance/invalid_output: {last_problem}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 依序返回预设回复的桩 Provider（用于验证解析、修复与错误路径）。
    struct CannedProvider {
        responses: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for CannedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[owo_agent_core::ToolSpec],
        ) -> Result<ModelOutput, String> {
            let mut queue = self.responses.lock().unwrap();
            if queue.is_empty() {
                return Ok(ModelOutput::Text(String::new()));
            }
            Ok(ModelOutput::Text(queue.remove(0)))
        }
    }

    fn canned(responses: &[&str]) -> CannedProvider {
        CannedProvider {
            responses: std::sync::Mutex::new(
                responses.iter().map(|text| text.to_string()).collect(),
            ),
        }
    }

    fn make_state(provider: CannedProvider) -> (Arc<crate::AppState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let agent = owo_agent_core::Agent::new(
            Arc::new(provider),
            owo_agent_core::tools::ToolRegistry::new(),
            owo_agent_core::permissions::Policy::new(&workspace),
            Default::default(),
        );
        let store =
            owo_agent_core::sqlite_store::SqliteSessionStore::open(&workspace.join("index.db"))
                .unwrap();
        let state = Arc::new(crate::AppState::new(
            agent,
            store,
            workspace.join("traces"),
            temp.path().to_path_buf(),
            workspace,
        ));
        (state, temp)
    }

    fn request(text: &str) -> PromptEnhanceRequest {
        PromptEnhanceRequest {
            text: text.to_string(),
            model: None,
            model_connection: None,
            context: None,
            clarifications: Vec::new(),
        }
    }

    #[test]
    fn parse_accepts_ready_json_even_when_fenced() {
        let parsed = parse_enhance_output(
            "```json\n{\"status\":\"ready\",\"enhanced_prompt\":\"开发一个模拟交易网页，要求……\",\"clarifying_questions\":[],\"change_summary\":\"补全结构\",\"task_type\":\"frontend\"}\n```",
        )
        .unwrap();
        assert_eq!(parsed.status, PromptEnhanceStatus::Ready);
        assert!(parsed.enhanced_prompt.unwrap().contains("模拟交易"));
        assert!(parsed.clarifying_questions.is_empty());
        assert_eq!(parsed.task_type.as_deref(), Some("frontend"));
    }

    #[test]
    fn parse_accepts_needs_clarification_with_bounded_questions() {
        let parsed = parse_enhance_output(
            r#"{"status":"needs_clarification","enhanced_prompt":null,"clarifying_questions":["纯前端还是全栈？","有没有指定技术栈？"],"change_summary":"缺少影响实现方式的信息","task_type":"coding"}"#,
        )
        .unwrap();
        assert_eq!(parsed.status, PromptEnhanceStatus::NeedsClarification);
        assert_eq!(parsed.clarifying_questions.len(), 2);
        assert!(parsed.enhanced_prompt.is_none());
    }

    #[test]
    fn parse_rejects_broken_contract_shapes() {
        // 非 JSON
        assert!(parse_enhance_output("直接给了一段话").is_err());
        // 未知 status
        assert!(parse_enhance_output(r#"{"status":"maybe"}"#).is_err());
        // ready 缺 enhanced_prompt
        assert!(parse_enhance_output(r#"{"status":"ready","enhanced_prompt":"  "}"#).is_err());
        // ready 带澄清问题（自相矛盾）
        assert!(parse_enhance_output(
            r#"{"status":"ready","enhanced_prompt":"ok","clarifying_questions":["?"]}"#
        )
        .is_err());
        // needs_clarification 无问题
        assert!(parse_enhance_output(
            r#"{"status":"needs_clarification","clarifying_questions":[]}"#
        )
        .is_err());
        // 超过 3 个问题
        let four = r#"{"status":"needs_clarification","clarifying_questions":["1","2","3","4"]}"#;
        assert!(parse_enhance_output(four).is_err());
        // enhanced_prompt 超长
        let long_prompt = "x".repeat(MAX_ENHANCED_CHARS + 1);
        let oversized = format!(r#"{{"status":"ready","enhanced_prompt":"{long_prompt}"}}"#);
        assert!(parse_enhance_output(&oversized).is_err());
        // needs_clarification 不应携带 enhanced_prompt
        let contradictory = r#"{"status":"needs_clarification","enhanced_prompt":"不该有","clarifying_questions":["?"]}"#;
        assert!(parse_enhance_output(contradictory).is_err());
    }

    #[test]
    fn user_prompt_includes_only_declared_context() {
        let mut req = request("帮我优化这个页面");
        req.context = Some(owo_agent_protocol::PromptEnhanceContext {
            workspace: Some("D:/demo".into()),
            project: Some("demo".into()),
            session_mode: Some("agent".into()),
        });
        let prompt = enhance_user_prompt(&req);
        assert!(prompt.contains("## 原始输入\n帮我优化这个页面"));
        assert!(prompt.contains("工作区路径：D:/demo"));
        assert!(prompt.contains("agent（可执行任务的智能体模式）"));
        assert!(prompt.contains("未列出的一律未知，不得编造"));
        // 未声明上下文时不输出该节
        assert!(!enhance_user_prompt(&request("hi")).contains("已知上下文"));
        // 澄清答案纳入且要求不重复提问
        let mut clarified = request("帮我做一个网站");
        clarified.clarifications = vec![owo_agent_protocol::PromptClarificationAnswer {
            question: "给谁看？".into(),
            answer: "团队内部".into(),
        }];
        let text = enhance_user_prompt(&clarified);
        assert!(text.contains("问：给谁看？"));
        assert!(text.contains("答：团队内部"));
        assert!(text.contains("不要重复提问"));
    }

    #[tokio::test]
    async fn enhance_returns_parsed_ready_result() {
        let (state, _temp) = make_state(canned(&[
            r#"{"status":"ready","enhanced_prompt":"修复登录按钮无反应：排查点击事件绑定与接口调用链……","clarifying_questions":[],"change_summary":"组织排查结构","task_type":"bugfix"}"#,
        ]));
        let response = enhance(State(state), Json(request("帮我修复登录按钮没反应")))
            .await
            .unwrap()
            .0;
        assert_eq!(response.status, PromptEnhanceStatus::Ready);
        assert!(response.enhanced_prompt.unwrap().contains("登录按钮"));
        assert_eq!(response.task_type.as_deref(), Some("bugfix"));
    }

    #[tokio::test]
    async fn enhance_repairs_once_on_bad_json_then_succeeds() {
        let (state, _temp) = make_state(canned(&[
            "抱歉，我先解释一下我的思路……", // 首次输出非 JSON
            r#"{"status":"needs_clarification","enhanced_prompt":null,"clarifying_questions":["网站面向什么用户？"],"change_summary":"缺少关键信息","task_type":"general_qa"}"#,
        ]));
        let response = enhance(State(state), Json(request("帮我做一个网站")))
            .await
            .unwrap()
            .0;
        assert_eq!(response.status, PromptEnhanceStatus::NeedsClarification);
        assert_eq!(response.clarifying_questions.len(), 1);
    }

    #[tokio::test]
    async fn enhance_reports_invalid_output_after_repair_fails() {
        let (state, _temp) = make_state(canned(&["不是 JSON", "还是不是 JSON"]));
        let status = enhance(State(state), Json(request("随便写点什么内容")))
            .await
            .unwrap_err()
            .0;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn enhance_rejects_empty_and_oversized_text() {
        let (state, _temp) = make_state(canned(&[]));
        let status = enhance(State(state), Json(request("   ")))
            .await
            .unwrap_err()
            .0;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (state, _temp) = make_state(canned(&[]));
        let long = "长".repeat(MAX_TEXT_CHARS + 1);
        let status = enhance(State(state), Json(request(&long)))
            .await
            .unwrap_err()
            .0;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn enhance_rejects_invalid_custom_connection_without_falling_back() {
        let (state, _temp) = make_state(canned(&[
            r#"{"status":"ready","enhanced_prompt":"不应被调用"}"#,
        ]));
        let mut req = request("帮我做个页面");
        req.model_connection = Some(owo_agent_protocol::CustomModelConnection {
            model: "local".into(),
            base_url: "::not a url::".into(),
            api_format: "openai".into(),
            use_full_url: false,
            api_key: None,
            api_key_env: None,
            temperature: None,
            timeout_secs: None,
            input_capabilities: None,
        });
        let (status, body) = enhance(State(state), Json(req)).await.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("prompt_enhance/model_connection/"));
    }
}
