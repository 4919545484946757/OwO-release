//! TUI 思考通道指示器：正文流不被打断，但状态栏必须看得见"模型在思考"。
//!
//! 行式 REPL 是暗色实时流式输出 `reasoning_content`；全屏 TUI 不能那么做
//! （会打散回答流的版式），但也**不能像以前那样完全丢弃**——用户会以为卡死。
//! 这里把思考增量折叠成一行状态栏文本：字符数 + 尾部预览，正文开始时收起并
//! 留下一句小结。纯状态机，可离线单测。

/// 状态栏预览保留的字符数（尾部，体现"正在想什么"的最新片段）。
pub const PREVIEW_CHARS: usize = 48;

/// 思考通道指示器。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReasoningIndicator {
    total_chars: usize,
    /// 最近 `PREVIEW_CHARS` 个字符（已压平空白）。
    preview: String,
}

impl ReasoningIndicator {
    /// 累积一段思考增量，返回应显示在状态栏的文本（空串增量不改状态）。
    pub fn push(&mut self, delta: &str) -> String {
        // 只统计"有内容的字符"（不含空白）：状态栏上的字数应是真实思考量。
        let content_chars = delta
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        let flattened: String = delta
            .chars()
            .map(|character| {
                if character.is_whitespace() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        if flattened.is_empty() {
            return self.text();
        }
        self.total_chars = self.total_chars.saturating_add(content_chars);
        let mut combined = self.preview.clone();
        combined.push_str(&flattened);
        // 折叠连续空格：预览里不需要成串空白。
        let mut normalized = String::with_capacity(combined.len());
        let mut last_space = false;
        for character in combined.chars() {
            if character == ' ' {
                if last_space {
                    continue;
                }
                last_space = true;
            } else {
                last_space = false;
            }
            normalized.push(character);
        }
        let count = normalized.chars().count();
        self.preview = if count > PREVIEW_CHARS {
            normalized
                .chars()
                .skip(count - PREVIEW_CHARS)
                .collect::<String>()
        } else {
            normalized
        };
        self.text()
    }

    /// 是否处于"正在思考"状态。
    pub fn is_active(&self) -> bool {
        self.total_chars > 0
    }

    /// 当前状态栏文本（未在思考时为空串）。
    pub fn text(&self) -> String {
        if !self.is_active() {
            return String::new();
        }
        let preview = self.preview.trim_start();
        let truncated = self.total_chars > preview.chars().count();
        format!(
            "🧠 思考中… {} 字 · {}{}",
            self.total_chars,
            if truncated { "…" } else { "" },
            preview
        )
    }

    /// 正文开始/回合结束：收起指示器，返回一句话小结（仅当确实思考过）。
    pub fn settle(&mut self) -> Option<String> {
        if !self.is_active() {
            self.preview.clear();
            return None;
        }
        let summary = format!("  🧠 思考 {} 字（已折叠）", self.total_chars);
        self.reset();
        Some(summary)
    }

    /// 复位（回合切换/取消）。
    pub fn reset(&mut self) {
        self.total_chars = 0;
        self.preview.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_chars_and_flattens_whitespace_into_one_line() {
        let mut indicator = ReasoningIndicator::default();
        assert_eq!(indicator.push("先看\n文件"), "🧠 思考中… 4 字 · 先看 文件");
        assert!(indicator.is_active());
        let text = indicator.push("\n\n再跑测试");
        assert!(!text.contains('\n'), "状态栏必须单行：{text}");
        assert!(text.contains("8 字"), "{text}");
        assert!(text.contains("再跑测试"), "{text}");
    }

    #[test]
    fn preview_keeps_only_the_tail_and_marks_truncation() {
        let mut indicator = ReasoningIndicator::default();
        let long = "啊".repeat(PREVIEW_CHARS * 3);
        let text = indicator.push(&long);
        assert!(
            text.contains(&format!("{} 字", PREVIEW_CHARS * 3)),
            "{text}"
        );
        assert!(text.contains('…'), "被截断时要有省略号：{text}");
        // 预览长度有界（不在状态栏堆长文本）。
        let preview_chars = text
            .split("·")
            .nth(1)
            .map(|tail| tail.trim().chars().count())
            .unwrap_or_default();
        assert!(preview_chars <= PREVIEW_CHARS + 1, "{preview_chars}");
    }

    #[test]
    fn empty_deltas_do_not_activate_the_indicator() {
        let mut indicator = ReasoningIndicator::default();
        assert_eq!(indicator.push(""), "");
        assert_eq!(indicator.push("\n\t "), "");
        indicator.reset();
        assert!(!indicator.is_active());
        assert_eq!(indicator.text(), "");
    }

    #[test]
    fn settle_reports_once_then_resets() {
        let mut indicator = ReasoningIndicator::default();
        indicator.push("权衡方案");
        let summary = indicator.settle().expect("应给出小结");
        assert!(summary.contains("思考 4 字"), "{summary}");
        assert!(!indicator.is_active());
        assert!(indicator.settle().is_none(), "复位后不应再报告");
        assert_eq!(indicator.text(), "");
    }

    #[test]
    fn long_reasoning_counts_are_saturating_and_bounded() {
        let mut indicator = ReasoningIndicator::default();
        for _ in 0..100 {
            indicator.push(&"字".repeat(100));
        }
        assert!(indicator.total_chars >= 10_000);
        let text = indicator.push("尾");
        assert!(text.starts_with("🧠 思考中… 10001 字"), "{text}");
        // 预览仍然有界。
        assert!(text.chars().count() < 120, "{}", text.chars().count());
    }
}
