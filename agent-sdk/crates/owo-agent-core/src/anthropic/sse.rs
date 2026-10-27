//! Bounded byte-oriented SSE framing. A partial UTF-8 character in the next
//! event must not delay a complete preceding event.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct SseDecoder {
    line: Vec<u8>,
    data: String,
    has_data: bool,
    frame_bytes: usize,
    skip_lf: bool,
    saw_first_line: bool,
}

impl SseDecoder {
    /// Returns true when the consumer accepts a terminal event. No bytes after
    /// that event are read and the HTTP body can be dropped immediately.
    pub(super) fn feed(
        &mut self,
        mut chunk: &[u8],
        consume: &mut impl FnMut(&str) -> Result<bool, String>,
    ) -> Result<bool, String> {
        while !chunk.is_empty() {
            if self.skip_lf {
                self.skip_lf = false;
                if chunk[0] == b'\n' {
                    chunk = &chunk[1..];
                    continue;
                }
            }
            let end = chunk.iter().position(|b| matches!(b, b'\n' | b'\r'));
            let length = end.unwrap_or(chunk.len());
            self.frame_bytes = self
                .frame_bytes
                .saturating_add(length + usize::from(end.is_some()));
            if self.frame_bytes > MAX_FRAME_BYTES {
                return Err(
                    "provider/stream_frame_too_large: Anthropic SSE event exceeds 1 MiB".into(),
                );
            }
            self.line.extend_from_slice(&chunk[..length]);
            let Some(end) = end else { break };
            self.skip_lf = chunk[end] == b'\r';
            chunk = &chunk[end + 1..];

            let line = std::mem::take(&mut self.line);
            let line = std::str::from_utf8(&line).map_err(|_| {
                "provider/stream_invalid_utf8: invalid Anthropic SSE text".to_string()
            })?;
            let line = if self.saw_first_line {
                line
            } else {
                self.saw_first_line = true;
                line.strip_prefix('\u{feff}').unwrap_or(line)
            };
            if line.is_empty() {
                self.frame_bytes = 0;
                if self.has_data {
                    self.has_data = false;
                    let payload = std::mem::take(&mut self.data);
                    if consume(&payload)? {
                        return Ok(true);
                    }
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // event/id/retry/comment and future fields are deliberately ignored.
        }
        Ok(false)
    }
}
