//! 查询串百分号编码（HTTP 查询参数里的中文/空格/保留字符）。
//!
//! 仓库没有引入 `urlencoding`/`percent-encoding` 依赖，而 `/symbols?query=…` 之类
//! 的查询串需要把符号名原样送达（中文符号名也要能用）。这里只做 RFC 3986 的
//! unreserved 白名单编码，行为可单测。

/// 按 RFC 3986 编码一个查询参数值（unreserved 字符保持原样，其余按 UTF-8 字节转 `%XX`）。
pub(crate) fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let character = *byte as char;
        let unreserved =
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~');
        if unreserved {
            encoded.push(character);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_unreserved_and_encodes_everything_else() {
        assert_eq!(encode_query_component("parse_config"), "parse_config");
        assert_eq!(encode_query_component("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode_query_component("with space"), "with%20space");
        assert_eq!(encode_query_component("a&b=c"), "a%26b%3Dc");
        assert_eq!(encode_query_component("50%"), "50%25");
        assert_eq!(encode_query_component("a/b"), "a%2Fb");
        assert_eq!(encode_query_component("a?b#c"), "a%3Fb%23c");
    }

    #[test]
    fn encodes_unicode_as_utf8_bytes() {
        // “符号”的 UTF-8：E7 AC A6 E5 8F B7
        assert_eq!(encode_query_component("符号"), "%E7%AC%A6%E5%8F%B7");
        assert_eq!(encode_query_component(""), "");
    }
}
