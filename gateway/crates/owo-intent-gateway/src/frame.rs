//! 管道帧协议：4 字节无符号小端长度 + UTF-8 JSON 载荷。
//!
//! 语义对齐官方参考实现 `OwO-release/apps/agent_mock/main.cpp:71-102`：
//! - 读取时长度 0 或超过 [`MAXIMUM_AGENT_PAYLOAD_BYTES`] 一律拒绝（不分配内存）；
//! - 写入时载荷为空或超限一律拒绝。

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::protocol::MAXIMUM_AGENT_PAYLOAD_BYTES;

/// 帧编解码错误。
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("管道 IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("帧长度非法：{size}（合法范围 1..={max} 字节）")]
    InvalidLength { size: usize, max: usize },
}

/// 读取一帧载荷。长度非法时立即返回错误且不读取后续字节。
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, FrameError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header).await?;
    let size = u32::from_le_bytes(header) as usize;
    if size == 0 || size > MAXIMUM_AGENT_PAYLOAD_BYTES {
        return Err(FrameError::InvalidLength {
            size,
            max: MAXIMUM_AGENT_PAYLOAD_BYTES,
        });
    }
    let mut payload = vec![0_u8; size];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// 写入一帧载荷并 flush（对应 mock 的 `FlushFileBuffers`）。
pub async fn write_frame<W>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    if payload.is_empty() || payload.len() > MAXIMUM_AGENT_PAYLOAD_BYTES {
        return Err(FrameError::InvalidLength {
            size: payload.len(),
            max: MAXIMUM_AGENT_PAYLOAD_BYTES,
        });
    }
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"{\"ok\":true}").await.unwrap();
        assert_eq!(&buffer[..4], &[11, 0, 0, 0], "小端长度头");
        let mut reader = std::io::Cursor::new(buffer);
        let payload = read_frame(&mut reader).await.unwrap();
        assert_eq!(payload, b"{\"ok\":true}");
    }

    #[tokio::test]
    async fn zero_length_rejected() {
        let mut reader = std::io::Cursor::new(vec![0_u8, 0, 0, 0]);
        let error = read_frame(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::InvalidLength { size: 0, .. }));
    }

    #[tokio::test]
    async fn oversized_length_rejected_without_allocation() {
        let bogus = ((MAXIMUM_AGENT_PAYLOAD_BYTES + 1) as u32).to_le_bytes();
        let mut reader = std::io::Cursor::new(bogus.to_vec());
        let error = read_frame(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::InvalidLength { .. }));
    }

    #[tokio::test]
    async fn write_empty_rejected() {
        let mut buffer = Vec::new();
        let error = write_frame(&mut buffer, b"").await.unwrap_err();
        assert!(matches!(error, FrameError::InvalidLength { size: 0, .. }));
    }

    #[tokio::test]
    async fn truncated_payload_fails() {
        let mut buffer = (10_u32).to_le_bytes().to_vec();
        buffer.extend_from_slice(b"short");
        let mut reader = std::io::Cursor::new(buffer);
        let error = read_frame(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::Io(_)));
    }
}
