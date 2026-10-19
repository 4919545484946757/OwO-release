//! MCP 服务器命名规则（CLI `owo-agent mcp` 与服务端 `/mcp/add` 共用唯一实现）。
//!
//! 名字直接进工具命名空间 `mcp__{server}__{tool}`（审查 N5）。因此必须拒绝：
//! - 空名/超长名；
//! - 分隔符 `__`（会让 `mcp__a__b__c` 归属歧义，命名空间无法反解）；
//! - 空白与控制字符、路径分隔符与 `..`（同一名字还会出现在配置/日志/审计里）。

/// MCP 服务器名长度上限（命名空间可读性与日志预算）。
pub const MAX_MCP_SERVER_NAME_CHARS: usize = 64;

/// 校验并归一（trim）MCP 服务器名；返回可用的名字或拒绝原因。
pub fn validate_mcp_server_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("MCP 服务器名不能为空".to_string());
    }
    if trimmed.chars().count() > MAX_MCP_SERVER_NAME_CHARS {
        return Err(format!(
            "MCP 服务器名过长（>{MAX_MCP_SERVER_NAME_CHARS} 字符）：{}",
            trimmed.chars().count()
        ));
    }
    if trimmed.contains("__") {
        return Err(format!(
            "MCP 服务器名不得包含 `__`（工具命名空间 mcp__{{server}}__{{tool}} 需要唯一解析）：{trimmed}"
        ));
    }
    for character in trimmed.chars() {
        if character.is_control() || character.is_whitespace() {
            return Err(format!("MCP 服务器名不得包含空白或控制字符：{trimmed}"));
        }
        if r#"/\:*?"<>|"#.contains(character) {
            return Err(format!(
                "MCP 服务器名不得包含路径/保留字符 `{character}`：{trimmed}"
            ));
        }
    }
    if trimmed.contains("..") {
        return Err(format!("MCP 服务器名不得包含 `..`：{trimmed}"));
    }
    // 首字符必须是字母/数字/下划线之外的可见字符也可，但不能是 `-`（易与参数混淆）。
    if trimmed.starts_with('-') {
        return Err(format!("MCP 服务器名不得以 `-` 开头：{trimmed}"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_server_names_and_trims() {
        assert_eq!(validate_mcp_server_name("translate").unwrap(), "translate");
        assert_eq!(
            validate_mcp_server_name("  owo-translate ").unwrap(),
            "owo-translate"
        );
        assert_eq!(
            validate_mcp_server_name("clipboard.v2").unwrap(),
            "clipboard.v2"
        );
        assert_eq!(validate_mcp_server_name("翻译").unwrap(), "翻译");
    }

    #[test]
    fn rejects_names_that_break_the_tool_namespace_or_paths() {
        for bad in [
            "",
            "   ",
            "a__b",   // 命名空间歧义
            "mcp__x", // 自带前缀会与命名空间前缀撞车
            "with space",
            "with\ttab",
            "with\nnewline",
            "a/b",
            "a\\b",
            "..",
            "../escape",
            "-leading-dash",
            "C:evil",
        ] {
            assert!(
                validate_mcp_server_name(bad).is_err(),
                "应拒绝 MCP 服务器名：{bad:?}"
            );
        }
        let long = "x".repeat(MAX_MCP_SERVER_NAME_CHARS + 1);
        assert!(validate_mcp_server_name(&long).is_err());
        let max = "x".repeat(MAX_MCP_SERVER_NAME_CHARS);
        assert!(validate_mcp_server_name(&max).is_ok());
    }
}
