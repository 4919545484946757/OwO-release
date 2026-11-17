//! Agent Plugins 1.0.0 兼容层契约测试。
//!
//! 覆盖规范里**有约束力**的边界：清单闭模式、名称规则、固定发现位置、窄失败边界、
//! `mcp.json` 传输变体与路径/占位符规则。测试只碰临时目录，不联网、不执行子进程。

use owo_agent_plugins::agent_plugins::{
    is_agent_plugin_dir, load_agent_plugin, validate_mcp_url, validate_plugin_name,
    AgentMcpTransport, DiagnosticLevel, AGENT_PLUGINS_MANIFEST_SCHEMA, AGENT_PLUGINS_MCP_SCHEMA,
};
use owo_agent_plugins::{discover_plugins_with_report, plugin_mcp_configs, PluginDiagnostic};
use std::path::{Path, PathBuf};

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "owo-agent-plugins-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建父目录");
    }
    std::fs::write(path, content).expect("写文件");
}

fn manifest_json(name: &str, extra: &str) -> String {
    let mut text = String::from("{\"$schema\":\"");
    text.push_str(AGENT_PLUGINS_MANIFEST_SCHEMA);
    text.push_str("\",\"name\":\"");
    text.push_str(name);
    text.push('"');
    text.push_str(extra);
    text.push('}');
    text
}

fn codes(diagnostics: &[PluginDiagnostic]) -> Vec<&'static str> {
    diagnostics.iter().map(|item| item.code).collect()
}

fn has_code(diagnostics: &[PluginDiagnostic], code: &str) -> bool {
    diagnostics.iter().any(|item| item.code == code)
}

// ---------------------------------------------------------------------------
// 清单
// ---------------------------------------------------------------------------

#[test]
fn minimal_manifest_loads_without_components() {
    let root = scratch("minimal");
    write(
        &root.join("plugin.json"),
        &manifest_json("minimal-plugin", ""),
    );
    let loaded = load_agent_plugin(&root, None).expect("最小清单应可加载");
    assert_eq!(loaded.id(), "minimal-plugin");
    assert!(loaded.skills.is_empty());
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "plugin_loaded"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unknown_top_level_field_is_reported_and_ignored() {
    let root = scratch("unknown-field");
    write(
        &root.join("plugin.json"),
        &manifest_json("noisy-plugin", r#","futureField":{"a":1}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("未知顶层字段不应致命");
    assert!(has_code(&loaded.diagnostics, "manifest_unknown_field"));
    let diagnostic = loaded
        .diagnostics
        .iter()
        .find(|item| item.code == "manifest_unknown_field")
        .expect("应有诊断");
    assert_eq!(diagnostic.level, DiagnosticLevel::Warning);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unsupported_schema_rejects_plugin() {
    let root = scratch("bad-schema");
    write(
        &root.join("plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/2.0.0/plugin.schema.json","name":"x"}"#,
    );
    let error = load_agent_plugin(&root, None).expect_err("不支持的版本必须拒绝");
    assert_eq!(error.code, "manifest_unsupported_schema");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_manifest_rejects_plugin() {
    let root = scratch("no-manifest");
    let error = load_agent_plugin(&root, None).expect_err("缺清单必须拒绝");
    assert_eq!(error.code, "manifest_missing");
    assert!(!is_agent_plugin_dir(&root));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn plugin_name_constraints_are_enforced() {
    for valid in ["a", "my-plugin", "acme.tools", "lint3r", "a.b-c9"] {
        assert!(validate_plugin_name(valid).is_ok(), "{valid} 应合法");
    }
    for invalid in [
        "",
        "My-Plugin",
        "-start",
        "end-",
        "has--double",
        "too..many",
        "with space",
        "under_score",
        &"a".repeat(65),
    ] {
        assert!(validate_plugin_name(invalid).is_err(), "{invalid} 应被拒绝");
    }
    // 64 字符是上限，恰好 64 合法。
    assert!(validate_plugin_name(&"a".repeat(64)).is_ok());
}

#[test]
fn invalid_name_in_manifest_rejects_plugin() {
    let root = scratch("bad-name");
    write(&root.join("plugin.json"), &manifest_json("Bad_Name", ""));
    let error = load_agent_plugin(&root, None).expect_err("非法插件名必须拒绝");
    assert_eq!(error.code, "plugin_name_invalid");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn author_object_is_closed() {
    let root = scratch("author");
    write(
        &root.join("plugin.json"),
        &manifest_json(
            "author-plugin",
            r#","author":{"name":"A","email":"a@example.com"}"#,
        ),
    );
    load_agent_plugin(&root, None).expect("合法 author 应通过");

    write(
        &root.join("plugin.json"),
        &manifest_json("author-plugin", r#","author":{"name":"A","company":"B"}"#),
    );
    let error = load_agent_plugin(&root, None).expect_err("author 未知字段必须拒绝");
    assert_eq!(error.code, "manifest_author_unknown_field");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn non_object_extensions_warns_and_continues() {
    let root = scratch("extensions");
    write(
        &root.join("plugin.json"),
        &manifest_json("ext-plugin", r#","extensions":"nope""#),
    );
    let loaded = load_agent_plugin(&root, None).expect("extensions 非对象不应致命");
    assert!(loaded.manifest.extensions.is_empty());
    assert!(has_code(
        &loaded.diagnostics,
        "manifest_extensions_not_object"
    ));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extension_namespaces_are_preserved_verbatim() {
    let root = scratch("extensions-object");
    write(
        &root.join("plugin.json"),
        &manifest_json(
            "ext-plugin",
            r#","extensions":{"com.example.client":{"setting":true}}"#,
        ),
    );
    let loaded = load_agent_plugin(&root, None).expect("合法 extensions 应通过");
    let namespace = loaded
        .manifest
        .extensions
        .get("com.example.client")
        .expect("命名空间应保留");
    assert_eq!(namespace.get("setting"), Some(&serde_json::json!(true)));
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 技能
// ---------------------------------------------------------------------------

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
}

#[test]
fn skills_are_discovered_only_in_direct_child_directories() {
    let root = scratch("skills");
    write(
        &root.join("plugin.json"),
        &manifest_json("skill-plugin", ""),
    );
    write(
        &root.join("skills/deploy/SKILL.md"),
        &skill_md("deploy", "发布流程与回滚手册"),
    );
    write(
        &root.join("skills/review/SKILL.md"),
        &skill_md("review", "代码评审清单"),
    );
    // 缺少 description：按规范跳过该技能，不影响其它技能。
    write(
        &root.join("skills/broken/SKILL.md"),
        "---\nname: broken\n---\n",
    );
    // 更深一层不是技能（§7.1 明确不递归）。
    write(
        &root.join("skills/nested/inner/SKILL.md"),
        &skill_md("inner", "不该被发现"),
    );

    let loaded = load_agent_plugin(&root, None).expect("技能包应加载");
    let names: Vec<&str> = loaded
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names, vec!["deploy", "review"]);
    assert!(has_code(&loaded.diagnostics, "skill_invalid"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn invalid_skill_name_is_skipped() {
    let root = scratch("skill-name");
    write(
        &root.join("plugin.json"),
        &manifest_json("skill-plugin", ""),
    );
    write(
        &root.join("skills/Upper/SKILL.md"),
        &skill_md("Upper", "大写名不符合 Agent Skills 规范"),
    );
    let loaded = load_agent_plugin(&root, None).expect("包本身仍应加载");
    assert!(loaded.skills.is_empty());
    assert!(has_code(&loaded.diagnostics, "skill_invalid"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn skill_block_scalar_description_is_supported() {
    let root = scratch("skill-block");
    write(
        &root.join("plugin.json"),
        &manifest_json("skill-plugin", ""),
    );
    write(
        &root.join("skills/block/SKILL.md"),
        "---\nname: block\ndescription: >\n  第一行说明\n  第二行说明\n---\n",
    );
    let loaded = load_agent_plugin(&root, None).expect("块标量应可解析");
    assert_eq!(loaded.skills.len(), 1);
    assert!(loaded.skills[0].description.contains("第一行说明"));
    assert!(loaded.skills[0].description.contains("第二行说明"));
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// MCP：stdio
// ---------------------------------------------------------------------------

fn mcp_json(servers: &str) -> String {
    let mut text = String::from("{\"$schema\":\"");
    text.push_str(AGENT_PLUGINS_MCP_SCHEMA);
    text.push_str("\",\"mcpServers\":{");
    text.push_str(servers);
    text.push_str("}}");
    text
}

#[test]
fn stdio_server_maps_command_args_env_and_cwd() {
    let root = scratch("mcp-stdio");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(
            r#""validator":{"type":"stdio","command":"./bin/validator","args":["--data","${PLUGIN_DATA}/validator"],"env":{"CONFIG":"${PLUGIN_ROOT}/config.json"},"cwd":"./work"}"#,
        ),
    );

    let loaded = load_agent_plugin(&root, None).expect("stdio 服务器应加载");
    assert_eq!(loaded.mcp_servers.len(), 1);
    let server = &loaded.mcp_servers[0];
    assert_eq!(server.key, "validator");
    assert_eq!(server.transport, AgentMcpTransport::Stdio);
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    assert_eq!(
        server.config.command,
        canonical_root.join("bin/validator").to_string_lossy()
    );
    assert_eq!(server.config.args[0], "--data");
    // 占位符是字符串替换，保留调用方写的 `/`（Windows 路径两种分隔符都接受）。
    let expanded_argument = server.config.args[1].replace('\\', "/");
    assert!(
        expanded_argument.ends_with("/.plugin-data/validator"),
        "{expanded_argument}"
    );
    let expanded_env = server
        .config
        .env
        .get("CONFIG")
        .expect("env 应展开")
        .replace('\\', "/");
    assert!(expanded_env.ends_with("/config.json"), "{expanded_env}");
    assert_eq!(
        server.config.cwd.as_deref(),
        Some(canonical_root.join("work").to_string_lossy().as_ref())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stdio_cwd_defaults_to_plugin_root() {
    let root = scratch("mcp-cwd");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""s":{"type":"stdio","command":"python"}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("缺省 cwd 应落插件根");
    let canonical_root = std::fs::canonicalize(&root).unwrap();
    assert_eq!(
        loaded.mcp_servers[0].config.cwd.as_deref(),
        Some(canonical_root.to_string_lossy().as_ref())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn invalid_server_is_skipped_without_losing_siblings() {
    let root = scratch("mcp-partial");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(
            r#""good":{"type":"stdio","command":"python"},"bad":{"type":"websocket","url":"https://example.com"},"escaping":{"type":"stdio","command":"../outside.exe"}"#,
        ),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    let keys: Vec<&str> = loaded
        .mcp_servers
        .iter()
        .map(|server| server.key.as_str())
        .collect();
    assert_eq!(keys, vec!["good"]);
    assert_eq!(
        loaded
            .diagnostics
            .iter()
            .filter(|item| item.code == "mcp_server_skipped")
            .count(),
        2
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reserved_env_variables_are_rejected() {
    let root = scratch("mcp-env");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""s":{"type":"stdio","command":"python","env":{"PLUGIN_ROOT":"/evil"}}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_server_skipped"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cwd_escaping_plugin_root_is_rejected() {
    let root = scratch("mcp-cwd-escape");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""s":{"type":"stdio","command":"python","cwd":"../escape"}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// MCP：HTTP 与配置级边界
// ---------------------------------------------------------------------------

#[test]
fn url_rules_match_the_specification() {
    assert_eq!(
        validate_mcp_url("https://deploy.example.com/mcp").unwrap(),
        "deploy.example.com"
    );
    // 回环可用明文 HTTP。
    assert!(validate_mcp_url("http://127.0.0.1:8080/mcp").is_ok());
    assert!(validate_mcp_url("http://localhost:3000").is_ok());
    assert!(validate_mcp_url("http://[::1]:9000/mcp").is_ok());
    // 非回环必须 HTTPS。
    assert!(validate_mcp_url("http://deploy.example.com/mcp").is_err());
    // 不得含 userinfo / fragment。
    assert!(validate_mcp_url("https://user:pass@example.com/mcp").is_err());
    assert!(validate_mcp_url("https://example.com/mcp#frag").is_err());
    assert!(validate_mcp_url("ftp://example.com/mcp").is_err());
    assert!(validate_mcp_url("example.com/mcp").is_err());
}

#[test]
fn http_server_maps_to_runtime_config_with_headers() {
    let root = scratch("mcp-http");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(
            r#""deploy":{"type":"streamable-http","url":"https://deploy.example.com/mcp","headers":{"X-Tenant":"public"}}"#,
        ),
    );
    let loaded = load_agent_plugin(&root, None).expect("HTTP 服务器应加载");
    let server = &loaded.mcp_servers[0];
    assert_eq!(server.transport, AgentMcpTransport::StreamableHttp);
    assert_eq!(server.config.transport, "http");
    assert_eq!(
        server.config.url.as_deref(),
        Some("https://deploy.example.com/mcp")
    );
    assert_eq!(
        server.config.headers.get("X-Tenant").map(String::as_str),
        Some("public")
    );
    // R9：声明 host 进白名单，既有 allowlist 检查有据可依。
    assert_eq!(server.config.network_allowlist, vec!["deploy.example.com"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn duplicate_headers_are_rejected_case_insensitively() {
    let root = scratch("mcp-headers");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(
            r#""deploy":{"type":"streamable-http","url":"https://example.com/mcp","headers":{"X-Tenant":"a","x-tenant":"b"}}"#,
        ),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_server_skipped"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn legacy_sse_transport_is_skipped_not_misconnected() {
    let root = scratch("mcp-sse");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""legacy":{"type":"sse","url":"https://example.com/sse"}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_server_skipped"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn malformed_cwd_placeholder_form_is_rejected() {
    let root = scratch("mcp-cwd-form");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    // 规范只允许 `${PLUGIN_ROOT}` 或 `${PLUGIN_ROOT}/...`，不接受拼接写法。
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""s":{"type":"stdio","command":"python","cwd":"${PLUGIN_ROOT}work"}"#),
    );
    let loaded = load_agent_plugin(&root, None).expect("包仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_server_skipped"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unknown_top_level_field_disables_the_whole_mcp_component() {
    let root = scratch("mcp-top-level");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        &format!(
            r#"{{"$schema":"{AGENT_PLUGINS_MCP_SCHEMA}","mcpServers":{{"s":{{"type":"stdio","command":"python"}}}},"servers":{{}}}}"#
        ),
    );
    let loaded = load_agent_plugin(&root, None).expect("插件本身仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_disabled"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn mcp_schema_must_match_the_manifest_version() {
    let root = scratch("mcp-schema");
    write(&root.join("plugin.json"), &manifest_json("mcp-plugin", ""));
    write(
        &root.join("mcp.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/2.0.0/mcp.schema.json","mcpServers":{}}"#,
    );
    let loaded = load_agent_plugin(&root, None).expect("插件本身仍应加载");
    assert!(loaded.mcp_servers.is_empty());
    assert!(has_code(&loaded.diagnostics, "mcp_disabled"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_component_locations_are_not_errors() {
    let root = scratch("no-components");
    write(&root.join("plugin.json"), &manifest_json("bare-plugin", ""));
    let loaded = load_agent_plugin(&root, None).expect("缺组件不是错误");
    assert!(!has_code(&loaded.diagnostics, "skills_not_directory"));
    assert!(!has_code(&loaded.diagnostics, "mcp_disabled"));
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 与宿主发现/装配的衔接
// ---------------------------------------------------------------------------

#[test]
fn discovery_picks_up_spec_packages_alongside_legacy_manifests() {
    let workspace = scratch("discovery-workspace");
    let data = scratch("discovery-data");
    // 规范包。
    write(
        &workspace.join("plugins/spec-one/plugin.json"),
        &manifest_json("spec-one", ""),
    );
    write(
        &workspace.join("plugins/spec-one/mcp.json"),
        &mcp_json(r#""server":{"type":"stdio","command":"python","args":["server.py"]}"#),
    );
    // 历史格式包。
    write(
        &data.join("plugins/legacy-one/manifest.json"),
        r#"{"id":"legacy-one","name":"Legacy","version":"1.0.0"}"#,
    );

    let (plugins, diagnostics) = discover_plugins_with_report(&workspace, &data);
    assert_eq!(plugins.len(), 2, "两类包都应被发现：{diagnostics:?}");
    let spec = plugins
        .iter()
        .find(|(_, manifest)| manifest.id == "spec-one")
        .expect("规范包应在结果里");
    assert_eq!(spec.1.mcp_servers.len(), 1);
    // 服务器名带上插件名，避免跨插件同名顶替。
    assert_eq!(spec.1.mcp_servers[0].name, "spec-one/server");
    assert!(spec.1.mcp.is_none());

    let legacy = plugins
        .iter()
        .find(|(_, manifest)| manifest.id == "legacy-one")
        .expect("历史包应在结果里");
    assert!(legacy.1.mcp_servers.is_empty());

    // 装配层取全部服务器。
    assert_eq!(plugin_mcp_configs(&spec.0, &spec.1).len(), 1);
    assert!(plugin_mcp_configs(&legacy.0, &legacy.1).is_empty());
    let _ = std::fs::remove_dir_all(&workspace);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn rejected_spec_package_is_reported_without_aborting_discovery() {
    let workspace = scratch("discovery-reject");
    let data = scratch("discovery-reject-data");
    write(
        &workspace.join("plugins/broken/plugin.json"),
        r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"Bad_Name"}"#,
    );
    write(
        &workspace.join("plugins/fine/plugin.json"),
        &manifest_json("fine", ""),
    );
    let (plugins, diagnostics) = discover_plugins_with_report(&workspace, &data);
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].1.id, "fine");
    assert!(diagnostics
        .iter()
        .any(|item| item.level == DiagnosticLevel::Error && item.code == "plugin_name_invalid"));
    let _ = std::fs::remove_dir_all(&workspace);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn spec_package_wins_over_legacy_manifest_in_the_same_directory() {
    let workspace = scratch("discovery-precedence");
    let data = scratch("discovery-precedence-data");
    let dir = workspace.join("plugins/both");
    write(&dir.join("plugin.json"), &manifest_json("both-spec", ""));
    write(
        &dir.join("manifest.json"),
        r#"{"id":"both-legacy","name":"Legacy","version":"1.0.0"}"#,
    );
    let (plugins, _) = discover_plugins_with_report(&workspace, &data);
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].1.id, "both-spec");
    let _ = std::fs::remove_dir_all(&workspace);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn shipped_demo_package_stays_conformant() {
    // 随包示例必须始终是合法规范包：它同时是文档里的验证样本。
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/owo-demo-kit")
        .canonicalize()
        .expect("示例包应存在");
    let loaded = load_agent_plugin(&root, None).expect("示例包必须通过规范校验");
    assert_eq!(loaded.id(), "owo-demo-kit");
    assert_eq!(loaded.skills.len(), 1);
    assert_eq!(loaded.skills[0].name, "demo-kit");
    assert_eq!(loaded.mcp_servers.len(), 1);
    let server = &loaded.mcp_servers[0];
    assert_eq!(server.key, "demo");
    assert_eq!(server.config.command, "python");
    assert_eq!(server.config.args, vec!["./servers/demo_server.py"]);
    // cwd 缺省/显式都落插件根；env 里的 ${PLUGIN_ROOT} 已展开。
    assert_eq!(
        server.config.cwd.as_deref(),
        Some(root.to_string_lossy().as_ref())
    );
    let plugin_root_env = server
        .config
        .env
        .get("OWO_DEMO_PLUGIN_ROOT")
        .expect("env 应展开");
    assert_eq!(plugin_root_env.as_str(), root.to_string_lossy());
    // 示例包不应产生任何 warning 级诊断。
    assert!(
        !loaded
            .diagnostics
            .iter()
            .any(|item| item.level == DiagnosticLevel::Warning),
        "示例包不应有降级诊断：{:?}",
        loaded.diagnostics
    );
}

#[test]
fn risk_scan_ignores_manifest_metadata_but_flags_code() {
    use owo_agent_plugins::agent_plugins::scan_agent_plugin_for_risks;

    let root = scratch("risk-scan");
    write(&root.join("plugin.json"), &manifest_json("risk-plugin", ""));
    write(
        &root.join("servers/clean.py"),
        "import json\nprint(json.dumps({}))\n",
    );
    // 清单里的 $schema / homepage 是元数据，不能被当成网络请求。
    assert!(
        scan_agent_plugin_for_risks(&root, &[]).is_empty(),
        "元数据 URL 不应产生风险项"
    );

    write(
        &root.join("servers/risky.py"),
        "import subprocess\nsubprocess.run(['cmd'])\n",
    );
    let risks = scan_agent_plugin_for_risks(&root, &[]);
    assert!(
        risks.iter().any(|risk| risk.contains("subprocess")),
        "可执行内容里的危险 API 必须被发现：{risks:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// 客户端扩展：hooks 贡献点
// ---------------------------------------------------------------------------

fn hook_manifest(name: &str, hooks_json: &str) -> String {
    let mut extra = String::from(",\"extensions\":{\"com.owo.agent\":{\"hooks\":");
    extra.push_str(hooks_json);
    extra.push_str("}}");
    manifest_json(name, &extra)
}

fn hook_plugin(label: &str, hooks_json: &str) -> PathBuf {
    let root = scratch(label);
    write(
        &root.join("plugin.json"),
        &hook_manifest("hook-plugin", hooks_json),
    );
    write(&root.join("hooks/guard.cmd"), "@exit /b 0\r\n");
    root
}

#[test]
fn hooks_are_loaded_from_the_extension_namespace() {
    let root = hook_plugin(
        "hooks",
        r#"[{"event":"pre_tool_use","matcher":"desktop_*","command":"./hooks/guard.cmd","args":["--strict"]}]"#,
    );
    let loaded = load_agent_plugin(&root, None).expect("hooks 包应加载");
    assert_eq!(loaded.hooks.len(), 1);
    let hook = &loaded.hooks[0];
    assert_eq!(hook.event, "pre_tool_use");
    assert_eq!(hook.matcher.as_deref(), Some("desktop_*"));
    assert_eq!(hook.args, vec!["--strict"]);
    let canonical = std::fs::canonicalize(root.join("hooks/guard.cmd")).unwrap();
    assert_eq!(hook.script, canonical);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hooks_from_the_extension_directory_are_merged() {
    let root = hook_plugin("hooks-dir", "[]");
    write(
        &root.join("com.owo.agent/hooks.json"),
        r#"[{"event":"stop","command":"${PLUGIN_ROOT}/hooks/guard.cmd"}]"#,
    );
    let loaded = load_agent_plugin(&root, None).expect("扩展目录 hooks 应加载");
    assert_eq!(loaded.hooks.len(), 1);
    assert_eq!(loaded.hooks[0].event, "stop");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bare_or_escaping_hook_commands_are_rejected() {
    let root = hook_plugin(
        "hooks-bad-command",
        r#"[{"event":"stop","command":"python evil.py"},{"event":"stop","command":"../outside.cmd"},{"event":"stop","command":"./hooks/missing.cmd"}]"#,
    );
    let loaded = load_agent_plugin(&root, None).expect("包本身仍应加载");
    assert!(loaded.hooks.is_empty());
    assert_eq!(
        loaded
            .diagnostics
            .iter()
            .filter(|item| item.code == "plugin_hook_skipped")
            .count(),
        3
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unknown_hook_event_and_bad_args_are_rejected() {
    let root = hook_plugin(
        "hooks-bad-fields",
        r#"[{"event":"before_everything","command":"./hooks/guard.cmd"},{"event":"stop","command":"./hooks/guard.cmd","args":["a\"b"]},{"event":"stop","command":"./hooks/guard.cmd","fail_mode":"sometimes"}]"#,
    );
    let loaded = load_agent_plugin(&root, None).expect("包本身仍应加载");
    assert!(loaded.hooks.is_empty());
    assert_eq!(
        loaded
            .diagnostics
            .iter()
            .filter(|item| item.code == "plugin_hook_skipped")
            .count(),
        3
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hook_count_is_capped() {
    let entries: Vec<String> = (0..(owo_agent_plugins::MAX_PLUGIN_HOOKS + 3))
        .map(|_| r#"{"event":"stop","command":"./hooks/guard.cmd"}"#.to_string())
        .collect();
    let root = hook_plugin("hooks-cap", &format!("[{}]", entries.join(",")));
    let loaded = load_agent_plugin(&root, None).expect("包本身仍应加载");
    assert_eq!(loaded.hooks.len(), owo_agent_plugins::MAX_PLUGIN_HOOKS);
    assert!(has_code(&loaded.diagnostics, "plugin_hooks_truncated"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diagnostic_codes_stay_stable_per_component() {
    let root = scratch("codes");
    write(&root.join("plugin.json"), &manifest_json("codes", ""));
    write(
        &root.join("skills/ok/SKILL.md"),
        &skill_md("ok", "合法技能"),
    );
    write(&root.join("skills/bad/SKILL.md"), "---\nname: bad\n---\n");
    write(
        &root.join("mcp.json"),
        &mcp_json(r#""a":{"type":"stdio","command":"python"},"b":{"type":"bogus"}"#),
    );
    let loaded = load_agent_plugin(&root, None).unwrap();
    let found = codes(&loaded.diagnostics);
    assert!(found.contains(&"skill_invalid"));
    assert!(found.contains(&"mcp_server_skipped"));
    assert!(found.contains(&"plugin_loaded"));
    let _ = std::fs::remove_dir_all(&root);
}
