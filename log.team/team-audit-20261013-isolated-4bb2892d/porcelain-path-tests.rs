pub fn parse_porcelain_path(line: &str) -> Option<String> {
    let rest = line.get(3..)?; // "XY " 前缀：X、Y 各 1 字符 + 1 空格
    let rest = rest.trim();
    let target = match rest.find(" -> ") {
        Some(index) => rest[index + 4..].trim(),
        None => rest,
    };
    let unquoted = target.trim_matches('"');
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}
#[test] fn ascii_control(){assert_eq!(parse_porcelain_path(" M plain.txt"),Some("plain.txt".into()));}
#[test] fn unicode_git_quoted_path(){assert_eq!(parse_porcelain_path(r#"?? "\346\226\207\346\241\243.txt""#),Some("文档.txt".into()));}
#[test] fn arrow_in_plain_filename(){assert_eq!(parse_porcelain_path(r#"?? "report -> final.txt""#),Some("report -> final.txt".into()));}
