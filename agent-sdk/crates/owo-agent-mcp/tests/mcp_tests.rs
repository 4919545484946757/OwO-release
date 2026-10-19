use async_trait::async_trait;
use owo_agent_core::mcp::{McpClient, McpServerConfig};
use owo_agent_core::permissions::{AutoApprover, Policy};
use owo_agent_core::session::Session;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{
    Agent, AgentConfig, ChatMessage, ModelOutput, ModelProvider, ToolCall, ToolSpec,
};
use serde_json::json;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 进程生命周期测试专用 Provider：不参与推理。
struct IdleProvider;

struct ScriptedProvider {
    outputs: Mutex<VecDeque<ModelOutput>>,
}

#[async_trait]
impl ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.outputs
            .lock()
            .map_err(|_| "脚本输出锁中毒".to_string())?
            .pop_front()
            .ok_or_else(|| "脚本输出耗尽".to_string())
    }
}

/// 审查 N5：MCP 工具注册名是命名空间 mcp__{server}__{tool}（与 core 单一来源同源），
/// 测试不得再断言旧的 {server}_{tool} 拼接名。
fn mcp_name(server: &str, tool: &str) -> String {
    owo_agent_core::tools::mcp_tool_full_name(server, tool)
}

fn mcp_prefix(server: &str) -> String {
    owo_agent_core::tools::mcp_tool_prefix(server)
}

fn scripted_tool_call(name: &str, args: serde_json::Value) -> Arc<dyn ModelProvider> {
    Arc::new(ScriptedProvider {
        outputs: Mutex::new(
            vec![
                ModelOutput::ToolCalls(vec![ToolCall {
                    id: "mcp-call-1".to_string(),
                    name: name.to_string(),
                    arguments: args,
                }]),
                ModelOutput::Text("工具调用完成".to_string()),
            ]
            .into(),
        ),
    })
}

fn test_config() -> McpServerConfig {
    McpServerConfig::stdio(
        "test",
        env!("CARGO_BIN_EXE_owo-mcp-test-server"),
        Vec::new(),
    )
}

fn plugin_path(plugin: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins")
        .join(plugin)
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins")
                .join(plugin)
        })
        .to_string_lossy()
        .to_string()
}

#[tokio::test]
async fn mcp_stdio_connect_list_and_call() {
    let mut client = McpClient::connect(&test_config()).await.unwrap();
    let tools = client.tools();
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    assert!(tools.iter().any(|tool| tool.name == "add"));

    let echo = client
        .call_tool("echo", json!({ "text": "你好" }))
        .await
        .unwrap();
    assert_eq!(echo["text"], "你好");

    let add = client
        .call_tool("add", json!({ "a": 1, "b": 2 }))
        .await
        .unwrap();
    assert_eq!(add["text"], "3");

    let unknown = client.call_tool("nope", json!({})).await;
    assert!(unknown.is_err());
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn mcp_stdio_timeout_kills_and_reconnects() {
    let mut config = test_config();
    // 并行运行 workspace 测试时子进程启动可能超过 400ms；仍需明显短于
    // hang 工具的 5s，避免把启动阶段误判成请求超时。
    config.timeout_ms = Some(2_000);
    let mut client = McpClient::connect(&config).await.unwrap();

    // 挂起 5s 远超 400ms 超时：应返回明确超时错误而非永久阻塞。
    let error = client
        .call_tool("hang", json!({ "sleep_ms": 5_000 }))
        .await
        .unwrap_err();
    assert!(error.contains("超时"), "超时错误：{error}");

    // 自动重连后下一次调用应恢复正常。
    let echo = client
        .call_tool("echo", json!({ "text": "recovered" }))
        .await
        .unwrap();
    assert_eq!(echo["text"], "recovered");
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn mcp_tools_are_registered_and_callable() {
    let client = Arc::new(tokio::sync::Mutex::new(
        McpClient::connect(&test_config()).await.unwrap(),
    ));
    let tools = client.lock().await.tools();
    let workspace =
        std::env::temp_dir().join(format!("owo-mcp-registry-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        scripted_tool_call(&mcp_name("test", "echo"), json!({ "text": "ok" })),
        ToolRegistry::empty(),
        Policy::new(&workspace),
        AgentConfig::default(),
    );
    agent.register_mcp_tools("test", Arc::clone(&client), tools);
    assert!(agent
        .visible_tool_specs()
        .iter()
        .any(|spec| spec.name == mcp_name("test", "echo")));

    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let outcome = agent
        .run_turn(
            &mut session,
            "调用 MCP echo",
            &AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(outcome.final_text.as_deref(), Some("工具调用完成"));
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        owo_agent_core::TurnEvent::ToolResult { tool, ok: true, .. } if *tool == mcp_name("test", "echo")
    )));
    let audit = agent.audit_log();
    assert!(audit.lock().unwrap().entries.iter().any(|entry| {
        entry.tool.as_deref() == Some(mcp_name("test", "echo").as_str())
            && entry.approved == Some(true)
    }));
    let _ = agent.shutdown_all_mcp().await;
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn mcp_http_connect_list_and_call() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut server = tokio::process::Command::new(env!("CARGO_BIN_EXE_owo-mcp-http-test-server"))
        .arg(port.to_string())
        .spawn()
        .unwrap();

    let config = McpServerConfig::http("http-test", format!("http://127.0.0.1:{port}/mcp"));
    let mut client = None;
    for _attempt in 0..10 {
        match McpClient::connect(&config).await {
            Ok(connected) => {
                client = Some(connected);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let mut client = client.expect("HTTP MCP 服务器应可连接");

    let tools = client.tools();
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    let echo = client
        .call_tool("echo", json!({ "text": "http-ok" }))
        .await
        .unwrap();
    assert_eq!(echo["text"], "http-ok");
    client.shutdown().await.unwrap();
    let _ = server.kill().await;
}

#[tokio::test]
async fn official_translate_plugin_serves_tools() {
    let config = McpServerConfig::stdio(
        "owo-translate",
        "python",
        vec![plugin_path("owo-translate/server.py")],
    );
    let mut client = McpClient::connect(&config).await.unwrap();
    let tools = client.tools();
    assert!(tools.iter().any(|tool| tool.name == "translate"));

    let result = client
        .call_tool("translate", json!({ "text": "hello", "target": "zh" }))
        .await
        .unwrap();
    assert!(
        result["text"].as_str().unwrap_or("").contains("你好"),
        "翻译结果：{result}"
    );

    let fallback = client
        .call_tool("translate", json!({ "text": "自定义内容", "target": "zh" }))
        .await
        .unwrap();
    assert!(fallback["text"].as_str().unwrap_or("").contains("演示翻译"));

    assert!(client.call_tool("nope", json!({})).await.is_err());
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn official_clipboard_plugin_lists_and_reads() {
    let config = McpServerConfig::stdio(
        "owo-clipboard",
        "python",
        vec![plugin_path("owo-clipboard/server.py")],
    );
    let mut client = McpClient::connect(&config).await.unwrap();
    let tools = client.tools();
    assert!(tools.iter().any(|tool| tool.name == "clipboard_read"));
    assert!(tools.iter().any(|tool| tool.name == "clipboard_write"));

    // 读剪贴板是只读操作；非 Windows 或空剪贴板时返回明确字段而非崩溃。
    let result = client.call_tool("clipboard_read", json!({})).await.unwrap();
    assert!(result["text"].is_string(), "剪贴板读取应返回文本：{result}");
    client.shutdown().await.unwrap();
}

#[test]
fn official_example_plugins_discover_and_validate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
    let workspace = root.canonicalize().unwrap();
    let data = std::env::temp_dir().join(format!("owo-plugin-data-{}", uuid::Uuid::new_v4()));
    let plugins = owo_agent_core::discover_plugins(&workspace, &data);
    let translate = plugins
        .iter()
        .find(|(_, manifest)| manifest.id == "owo.plugin.translate")
        .expect("翻译示例插件应被发现");
    assert!(translate.1.mcp.is_some());
    assert_eq!(translate.1.mcp.as_ref().unwrap().command, "python");
    let clipboard = plugins
        .iter()
        .find(|(_, manifest)| manifest.id == "owo.plugin.clipboard")
        .expect("剪贴板示例插件应被发现");
    assert!(clipboard
        .1
        .permissions
        .contains(&"clipboard:write".to_string()));
    let _ = std::fs::remove_dir_all(&data);
}

/// v0.5.5：Agent 注册表 RwLock 改造后——MCP 服务器可热连接（Arc<Agent> 上无 &mut 注册），
/// 注册后模型可见工具包含新前缀，执行可命中；移除前缀后从可见集消失。
#[tokio::test]
async fn agent_hot_register_mcp_tools_after_construction() {
    use owo_agent_core::agent::Agent;
    use owo_agent_core::gateway::ModelProvider;

    struct FixedProvider;
    #[async_trait::async_trait]
    impl ModelProvider for FixedProvider {
        async fn complete(
            &self,
            _messages: &[owo_agent_core::ChatMessage],
            _tools: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            Ok(owo_agent_core::ModelOutput::Text("ok".to_string()))
        }
    }

    let workspace = std::env::temp_dir().join(format!("owo-agent-hotmcp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Arc::new(Agent::new(
        Arc::new(FixedProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    ));
    let before: Vec<String> = agent
        .visible_tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert!(!before
        .iter()
        .any(|name| name.starts_with(&mcp_prefix("test"))));

    // 构造后热注册（关键：&self，无需重建 Agent）。
    let client = Arc::new(tokio::sync::Mutex::new(
        McpClient::connect(&test_config()).await.unwrap(),
    ));
    let tools = client.lock().await.tools();
    agent.register_mcp_tools("test", Arc::clone(&client), tools);

    let after: Vec<String> = agent
        .visible_tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert!(after.iter().any(|name| *name == mcp_name("test", "echo")));

    // 前缀撤销（热卸载）：注册表移除 + 禁用前缀。
    let removed = agent.remove_tools_prefix(&mcp_prefix("test"));
    assert!(removed > 0);
    agent.set_tool_prefix_enabled(&mcp_prefix("test"), false);
    let final_specs: Vec<String> = agent
        .visible_tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert!(!final_specs
        .iter()
        .any(|name| name.starts_with(&mcp_prefix("test"))));
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn agent_process_kill_unloads_mcp_server() {
    // 进程级热卸载（M3 收尾）：connect → 注册工具 → shutdown 杀子进程 + 撤销工具。
    let config = McpServerConfig::stdio(
        "kill-test",
        env!("CARGO_BIN_EXE_owo-mcp-test-server"),
        Vec::new(),
    );
    let workspace =
        std::env::temp_dir().join(format!("owo-agent-kill-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = owo_agent_core::Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        owo_agent_core::AgentConfig::default(),
    );

    let tools = agent.connect_mcp_server(&config).await.unwrap();
    assert!(tools > 0, "应注册到工具");
    let visible: Vec<String> = agent
        .visible_tool_specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    assert!(visible
        .iter()
        .any(|name| name.starts_with(&mcp_prefix("kill-test"))));

    // 进程级卸载：子进程被 kill，工具从注册表移除。
    let killed = agent.shutdown_mcp_server("kill-test").await.unwrap();
    assert!(killed, "应报告进程被终止");
    assert!(!agent.mcp_clients().is_running("kill-test"));
    let after: Vec<String> = agent
        .visible_tool_specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    assert!(
        !after
            .iter()
            .any(|name| name.starts_with(&mcp_prefix("kill-test"))),
        "卸载后工具应从注册表移除"
    );

    // 幂等：再次卸载返回 false 且不报错。
    let again = agent.shutdown_mcp_server("kill-test").await.unwrap();
    assert!(!again);

    // 重新连接可再次注册（热恢复路径）。
    let tools2 = agent.connect_mcp_server(&config).await.unwrap();
    assert!(tools2 > 0);
    let killed2 = agent.shutdown_mcp_server("kill-test").await.unwrap();
    assert!(killed2);
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn mcp_registry_shutdown_all_kills_children() {
    let registry = owo_agent_core::McpRegistry::new();
    for name in ["reg-a", "reg-b"] {
        let config =
            McpServerConfig::stdio(name, env!("CARGO_BIN_EXE_owo-mcp-test-server"), Vec::new());
        let client = Arc::new(tokio::sync::Mutex::new(
            McpClient::connect(&config).await.unwrap(),
        ));
        registry.insert(name, Arc::clone(&client));
        assert!(registry.is_running(name));
    }
    let errors = registry.shutdown_all().await;
    assert!(errors.is_empty(), "关闭错误：{errors:?}");
    assert!(registry.names().is_empty());
    for name in ["reg-a", "reg-b"] {
        assert!(!registry.is_running(name));
    }
}

/// P1 延迟加载：超预算 schema 注册为压缩骨架（保留属性名+类型+required），
/// 完整 schema 保留在注册表供按需查询；调用仍可用压缩后的工具。
#[tokio::test]
async fn mcp_large_schema_is_compacted_and_full_schema_available() {
    use owo_agent_core::tools::compact_schema;

    let big_schema = json!({
        "type": "object",
        "required": ["query", "filters"],
        "properties": {
            "query": { "type": "string", "description": "搜索关键词" },
            "filters": {
                "type": "object",
                "description": "过滤条件",
                "properties": {
                    "a".repeat(40): { "type": "string", "description": "d".repeat(600) },
                    "b".repeat(40): { "type": "integer", "enum": [1, 2, 3], "description": "e".repeat(600) },
                    "c".repeat(40): { "type": "array", "items": { "type": "string" }, "description": "f".repeat(600) }
                }
            },
            "nested": { "type": "object", "properties": { "x": { "type": "string" } } }
        }
    });
    assert!(
        owo_agent_core::tools::schema_bytes(&big_schema) > 2048,
        "测试样例必须超过默认预算"
    );

    // 压缩只剥离说明性噪声，完整保留结构/类型/enum（安全契约见 compact_schema 文档）。
    let compact = compact_schema(&big_schema);
    let compact_bytes = owo_agent_core::tools::schema_bytes(&compact);
    assert!(
        compact_bytes < owo_agent_core::tools::schema_bytes(&big_schema) / 2,
        "剥离 description 后体积应显著缩小：{compact_bytes}"
    );
    assert_eq!(compact["type"], "object");
    assert_eq!(compact["required"], json!(["query", "filters"]));
    assert!(compact["properties"]["query"].get("type").is_some());
    assert!(compact["properties"]["nested"].get("type").is_some());
    assert!(compact["properties"]["query"].get("description").is_none());
    // 嵌套结构、字段名与校验关键字必须保留，否则模型会拿到错误参数契约。
    assert!(compact["properties"]["filters"].get("properties").is_some());
    assert_eq!(
        compact["properties"]["filters"]["properties"]["b".repeat(40)]["enum"],
        json!([1, 2, 3])
    );
    assert!(compact["properties"]["filters"]
        .get("description")
        .is_none());

    // 注册：大 schema 工具模型可见的是骨架，完整 schema 可从注册表按需取回。
    let client = Arc::new(tokio::sync::Mutex::new(
        McpClient::connect(&test_config()).await.unwrap(),
    ));
    let mut registry = ToolRegistry::new();
    let tools = vec![owo_agent_core::McpTool {
        name: "big_tool".into(),
        description: "大 schema 工具".into(),
        input_schema: big_schema,
        annotations: None,
    }];
    registry.register_mcp_tools("big-server", Arc::clone(&client), tools);
    let visible = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name == mcp_name("big-server", "big_tool"))
        .expect("工具应注册");
    assert!(
        owo_agent_core::tools::schema_bytes(&visible.input_schema) <= 2048,
        "模型可见 schema 应在预算内"
    );
    assert!(visible.description.contains("schema 已压缩"));
    let full = registry
        .full_schema(&mcp_name("big-server", "big_tool"))
        .expect("完整 schema 应保留");
    assert_eq!(full["properties"]["filters"]["description"], "过滤条件");
    let _ = client.lock().await.shutdown().await;
}

/// P1 延迟加载：预算内的小 schema 不压缩，且不重复存储完整副本。
#[tokio::test]
async fn mcp_small_schema_kept_as_is_and_no_full_copy() {
    let small = json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"]
    });
    let client = Arc::new(tokio::sync::Mutex::new(
        McpClient::connect(&test_config()).await.unwrap(),
    ));
    let mut registry = ToolRegistry::new();
    registry.register_mcp_tools(
        "small-server",
        Arc::clone(&client),
        vec![owo_agent_core::McpTool {
            name: "small_tool".into(),
            description: "小 schema".into(),
            input_schema: small.clone(),
            annotations: None,
        }],
    );
    let visible = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name == mcp_name("small-server", "small_tool"))
        .expect("工具应注册");
    assert_eq!(visible.input_schema, small);
    assert!(
        !visible.description.contains("schema 已压缩"),
        "小 schema 不应标记压缩"
    );
    assert!(registry
        .full_schema(&mcp_name("small-server", "small_tool"))
        .is_none());
    let _ = client.lock().await.shutdown().await;
}

/// P1 延迟加载：压缩后的 schema 仍能经 Agent 权限入口实际调用 MCP 工具。
#[tokio::test]
async fn mcp_compacted_tool_still_callable() {
    let big_schema = json!({
        "type": "object",
        "required": ["text"],
        "properties": {
            "text": {
                "type": "string",
                "description": "x".repeat(3000),
                "enum": ["a", "b"]
            }
        }
    });
    let client = Arc::new(tokio::sync::Mutex::new(
        McpClient::connect(&test_config()).await.unwrap(),
    ));
    let workspace = std::env::temp_dir().join(format!("owo-mcp-compact-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    // 压缩 schema 保留 enum 校验：调用参数必须满足 `enum: ["a","b"]`。
    let agent = Agent::new(
        scripted_tool_call(&mcp_name("comp-test", "echo"), json!({ "text": "a" })),
        ToolRegistry::empty(),
        Policy::new(&workspace),
        AgentConfig::default(),
    );
    agent.register_mcp_tools(
        "comp-test",
        Arc::clone(&client),
        vec![owo_agent_core::McpTool {
            name: "echo".into(),
            description: "回显".into(),
            input_schema: big_schema,
            annotations: None,
        }],
    );
    let visible = agent
        .visible_tool_specs()
        .into_iter()
        .find(|spec| spec.name == mcp_name("comp-test", "echo"))
        .expect("压缩后工具仍注册");
    assert!(visible.description.contains("schema 已压缩"));
    assert!(owo_agent_core::tools::schema_bytes(&visible.input_schema) <= 2048);

    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let outcome = agent
        .run_turn(
            &mut session,
            "调用压缩 schema MCP 工具",
            &AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    assert_eq!(outcome.final_text.as_deref(), Some("工具调用完成"));
    assert!(outcome.events.iter().any(|event| matches!(
        event,
        owo_agent_core::TurnEvent::ToolResult { tool, ok: true, .. } if *tool == mcp_name("comp-test", "echo")
    )));
    let audit = agent.audit_log();
    let receipt = audit
        .lock()
        .unwrap()
        .entries
        .iter()
        .find(|entry| {
            entry.event == "tool_receipt"
                && entry.tool.as_deref() == Some(mcp_name("comp-test", "echo").as_str())
        })
        .cloned()
        .expect("MCP 工具经 Agent 入口执行必须留下 tool_receipt");
    assert_eq!(receipt.approved, Some(true));
    assert!(receipt.detail.contains("args_sha256"));
    assert!(receipt.detail.contains("tool_version"));
    let _ = agent.shutdown_all_mcp().await;
    let _ = std::fs::remove_dir_all(&workspace);
}

/// A2-2：resources/prompts 能力协商、读取往返，以及泛化工具注册与调用。
#[tokio::test]
async fn mcp_resources_and_prompts_roundtrip_and_tools() {
    use owo_agent_core::agent::Agent;
    use owo_agent_core::mcp::{McpPrompt, McpResource};

    let mut client = McpClient::connect(&test_config()).await.unwrap();
    // 连接时按能力协商拉取（mock 声明了 resources/prompts）。
    let resources = client.resources();
    assert!(
        resources
            .iter()
            .any(|resource| resource.uri == "file:///readme.md"),
        "{resources:?}"
    );
    assert!(resources.iter().any(|resource| resource.name == "tips"));
    let read = client.read_resource("file:///readme.md").await.unwrap();
    assert!(read["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("欢迎"));
    let error = client
        .read_resource("file:///missing.md")
        .await
        .unwrap_err();
    assert!(error.contains("未知资源"), "{error}");

    let prompts = client.prompts();
    assert!(
        prompts.iter().any(|prompt| prompt.name == "summarize"),
        "{prompts:?}"
    );
    let prompt = client
        .get_prompt("summarize", Some(json!({ "topic": "构建" })))
        .await
        .unwrap();
    assert!(prompt["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("构建"));

    // 泛化工具注册：test_read_resource / test_get_prompt。
    let client = Arc::new(tokio::sync::Mutex::new(client));
    let mut registry = ToolRegistry::new();
    registry.register_mcp_extras(
        "test",
        Arc::clone(&client),
        vec![McpResource {
            uri: "file:///readme.md".into(),
            name: "readme".into(),
            description: String::new(),
            mime_type: None,
        }],
        vec![McpPrompt {
            name: "summarize".into(),
            description: String::new(),
            arguments: Vec::new(),
        }],
    );
    let specs = registry.specs();
    let resource_spec = specs
        .iter()
        .find(|spec| spec.name == mcp_name("test", "read_resource"))
        .expect("资源工具应注册");
    assert!(
        resource_spec.description.contains("file:///readme.md"),
        "目录应进描述：{}",
        resource_spec.description
    );
    assert!(specs
        .iter()
        .any(|spec| spec.name == mcp_name("test", "get_prompt")));

    // 经 Agent 回合实际调用资源工具（capability 由权限策略签发）。
    let workspace = std::env::temp_dir().join(format!("owo-mcp-extras-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let mut session = Session::new(&workspace, "mock", None);
    let agent = Agent::new(
        scripted_tool_call(
            &mcp_name("test", "read_resource"),
            json!({ "uri": "file:///readme.md" }),
        ),
        registry,
        Policy::new(&workspace),
        AgentConfig::default(),
    );
    let outcome = agent
        .run_turn(
            &mut session,
            "读资源",
            &AutoApprover { allow: true },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .await
        .unwrap();
    let called = outcome.events.iter().any(|event| {
        matches!(
            event,
            owo_agent_core::TurnEvent::ToolResult { tool, ok: true, preview: Some(preview), .. }
                if *tool == mcp_name("test", "read_resource") && preview.contains("欢迎")
        )
    });
    assert!(called, "资源工具结果应回传 contents：{:?}", outcome.events);
    let _ = std::fs::remove_dir_all(&workspace);
    let _ = client.lock().await.shutdown().await;
}

/// A2-2：Agent 热连接后可见工具包含 resources/prompts 泛化工具。
#[tokio::test]
async fn agent_connect_registers_mcp_extras_tools() {
    use owo_agent_core::agent::Agent;

    let workspace =
        std::env::temp_dir().join(format!("owo-mcp-extras-agent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        AgentConfig::default(),
    );
    let count = agent.connect_mcp_server(&test_config()).await.unwrap();
    assert!(count >= 3, "至少 echo/add/hang 三个工具");
    let visible: Vec<String> = agent
        .visible_tool_specs()
        .iter()
        .map(|spec| spec.name.clone())
        .collect();
    assert!(
        visible.iter().any(|name| *name == mcp_name("test", "echo")),
        "{visible:?}"
    );
    assert!(
        visible
            .iter()
            .any(|name| *name == mcp_name("test", "read_resource")),
        "{visible:?}"
    );
    assert!(
        visible
            .iter()
            .any(|name| *name == mcp_name("test", "get_prompt")),
        "{visible:?}"
    );
    agent.shutdown_mcp_server("test").await.unwrap();
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn timed_out_tool_side_effect_is_not_replayed_and_next_call_recovers() {
    let counter =
        std::env::temp_dir().join(format!("owo-mcp-sideeffect-{}.txt", uuid::Uuid::new_v4()));
    let mut config = test_config();
    config.timeout_ms = Some(2000);
    let mut client = McpClient::connect(&config).await.unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.call_tool("write_then_hang", json!({"path":counter.to_string_lossy()})),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.contains("mcp/outcome_unknown"), "{error}");
    assert!(error.contains("request_not_replayed"), "{error}");
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "1");
    let echo = client
        .call_tool("echo", json!({"text":"recovered"}))
        .await
        .unwrap();
    assert_eq!(echo["text"], "recovered");
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "1");
    client.shutdown().await.unwrap();
    std::fs::remove_file(counter).unwrap();
}
