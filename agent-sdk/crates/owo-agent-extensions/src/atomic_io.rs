//! 原子文件写入：先写同目录临时文件再 rename 覆盖。
//!
//! 背景（鲁棒性）：任务队列、自动化任务等状态文件此前直接 `fs::write`，
//! 崩溃/断电/并发写可能留下半截 JSON，重启后无法判定真实状态。这里统一
//! 「临时文件 + 同卷 rename」，崩溃任一点只会看到旧完整文件或新完整文件。

use std::path::Path;

/// 原子写入：同目录临时文件 + rename 覆盖；失败时清理临时文件。
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    // 唯一后缀避免同进程并发写同名临时文件互相覆盖。
    let tmp = parent.join(format!(".{stem}.tmp-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::atomic_write;

    #[test]
    fn atomic_write_replaces_without_temp_residue() {
        let dir = std::env::temp_dir().join(format!("owo-atomic-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.json");
        atomic_write(&path, b"{\"v\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"v\":1}");
        atomic_write(&path, b"{\"v\":2}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"v\":2}");
        let residue: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(residue.is_empty(), "不应残留临时文件：{residue:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
