//! SSE（Server-Sent Events）增量解析器——W3C EventSource 语义子集。
//!
//! agent-server 的 turn 端点是 `text/event-stream`（`owo-agent-server::to_event`：
//! `Event::default().event(name).data(json)`）。本解析器处理：
//! - 字节流跨 chunk 的 UTF-8 边界与行边界；
//! - `event:` / `data:` 字段（多行 data 以 `\n` 连接）；
//! - 注释行（`:` 开头，含 keep-alive）与其他字段忽略；
//! - 空行派发帧（`data` 为空不派发）。

/// 一个完整的 SSE 帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    /// 事件名（缺省 `message`）。
    pub event: String,
    /// data 字段拼接结果。
    pub data: String,
}

/// 增量解析器。
#[derive(Debug, Default)]
pub struct SseParser {
    pending_bytes: Vec<u8>,
    line_buffer: String,
    event_type: String,
    data: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推入一段字节，返回本次可派发的完整帧。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        self.pending_bytes.extend_from_slice(chunk);

        // UTF-8 安全：只消费完整序列；不完整的尾部留待下一块。
        loop {
            match std::str::from_utf8(&self.pending_bytes) {
                Ok(text) => {
                    let text = text.to_string();
                    self.pending_bytes.clear();
                    self.consume_text(&text, &mut frames);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        let text = std::str::from_utf8(&self.pending_bytes[..valid])
                            .expect("valid_up_to 保证前缀合法")
                            .to_string();
                        self.pending_bytes.drain(..valid);
                        self.consume_text(&text, &mut frames);
                    }
                    match error.error_len() {
                        // 真正非法字节：跳过并继续（替换为忽略）。
                        Some(len) => {
                            self.pending_bytes.drain(..len);
                        }
                        // 不完整序列：等待下一块。
                        None => break,
                    }
                }
            }
        }
        frames
    }

    /// 流结束时冲刷未派发的尾部帧（无尾随空行的场景）。
    pub fn finish(&mut self) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        let line = std::mem::take(&mut self.line_buffer);
        if !line.is_empty() {
            self.handle_line(line.trim_end_matches('\r'), &mut frames);
        }
        self.dispatch(&mut frames);
        frames
    }

    fn consume_text(&mut self, text: &str, frames: &mut Vec<SseFrame>) {
        self.line_buffer.push_str(text);
        while let Some(position) = self.line_buffer.find('\n') {
            let line = self.line_buffer[..position]
                .trim_end_matches('\r')
                .to_string();
            self.line_buffer.drain(..=position);
            self.handle_line(&line, frames);
        }
    }

    fn handle_line(&mut self, line: &str, frames: &mut Vec<SseFrame>) {
        if line.is_empty() {
            self.dispatch(frames);
            return;
        }
        if line.starts_with(':') {
            return; // 注释 / keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event_type = value.to_string(),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {} // id / retry 等忽略
        }
    }

    fn dispatch(&mut self, frames: &mut Vec<SseFrame>) {
        if self.data.is_empty() {
            self.event_type.clear();
            return;
        }
        let event = if self.event_type.is_empty() {
            "message".to_string()
        } else {
            std::mem::take(&mut self.event_type)
        };
        frames.push(SseFrame {
            event,
            data: std::mem::take(&mut self.data),
        });
        self.event_type.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_frame() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"event: token_delta\ndata: {\"delta\":\"hi\"}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "token_delta");
        assert_eq!(frames[0].data, "{\"delta\":\"hi\"}");
    }

    #[test]
    fn splits_across_chunks() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"event: tok").is_empty());
        assert!(parser.push(b"en_delta\ndata: {\"del").is_empty());
        let frames = parser.push(b"ta\":\"hi\"}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "token_delta");
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut parser = SseParser::new();
        let payload = "event: final\ndata: {\"text\":\"你好\"}\n\n".as_bytes();
        let (head, tail) = payload.split_at(16); // 切在多字节字符中间
        let mut frames = parser.push(head);
        frames.extend(parser.push(tail));
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.contains("你好"));
    }

    #[test]
    fn ignores_comments_and_keepalive() {
        let mut parser = SseParser::new();
        let frames = parser.push(b": keep-alive\n\nevent: progress\ndata: {}\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "progress");
    }

    #[test]
    fn multi_line_data_joined() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"data: line1\ndata: line2\n\n");
        assert_eq!(frames[0].data, "line1\nline2");
    }

    #[test]
    fn crlf_line_endings() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"event: final\r\ndata: {}\r\n\r\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "final");
    }

    #[test]
    fn finish_flushes_without_trailing_blank() {
        let mut parser = SseParser::new();
        assert!(parser
            .push(b"event: final\ndata: {\"text\":\"ok\"}\n")
            .is_empty());
        let frames = parser.finish();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "{\"text\":\"ok\"}");
    }

    #[test]
    fn empty_data_not_dispatched() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"event: ping\n\n");
        assert!(frames.is_empty());
    }

    #[test]
    fn default_event_name_is_message() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"data: hello\n\n");
        assert_eq!(frames[0].event, "message");
    }
}
