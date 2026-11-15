use super::{DeliveryError, DeliveryResult};
use owo_agent_core::CasStore;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) const LIMIT: usize = 256 * 1024;

pub(super) fn target_path(workspace: &Path, relative: &str) -> DeliveryResult<PathBuf> {
    let mut path = workspace.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.ends_with(['.', ' '])
            || reserved_device_name(segment)
            || segment
                .chars()
                .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
            || [".git", ".owo", ".owo-agent"]
                .iter()
                .any(|name| segment.eq_ignore_ascii_case(name))
        {
            return Err(DeliveryError::Invalid("invalid_project_path"));
        }
        path.push(segment);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if redirected(&metadata) => {
                return Err(DeliveryError::Invalid("redirected_project_path"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let safe = owo_agent_core::permissions::resolve_within(workspace, relative)
        .map_err(|_| DeliveryError::Invalid("invalid_project_path"))?;
    if !safe.parent().is_some_and(Path::is_dir) {
        return Err(DeliveryError::Invalid("output_directory_missing"));
    }
    Ok(safe)
}

fn reserved_device_name(segment: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let name = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            name.strip_prefix(*prefix).is_some_and(|suffix| {
                matches!(suffix.as_bytes(), [b'0'..=b'9'])
                    || matches!(suffix, "\u{00b9}" | "\u{00b2}" | "\u{00b3}")
            })
        })
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn snapshot(path: &Path) -> DeliveryResult<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if redirected(&metadata) || !metadata.is_file() || metadata.len() > LIMIT as u64 {
        return Err(DeliveryError::Invalid("invalid_project_file"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT || std::str::from_utf8(&bytes).is_err() {
        return Err(DeliveryError::Invalid("expected_bounded_text_file"));
    }
    Ok(Some(bytes))
}

pub(super) fn load_content(cas: &CasStore, hash: &str) -> DeliveryResult<Vec<u8>> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DeliveryError::Unavailable);
    }
    cas.get(hash)
        .filter(|bytes| CasStore::hash_of(bytes) == hash)
        .ok_or(DeliveryError::Unavailable)
}

/// Keep the old entry intact if writing fails. New workspace files use an
/// atomic no-clobber link, so a file created during apply is never replaced.
pub(super) fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let temp = parent.join(format!(".owobot-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| {
        if replace {
            fs::rename(&temp, path)
        } else {
            fs::hard_link(&temp, path)
        }
    });
    let _ = fs::remove_file(&temp);
    result
}
