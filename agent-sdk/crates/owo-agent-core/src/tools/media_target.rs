//! Host-owned media destinations. No filesystem mutation occurs during preflight.
//! Worker scopes are canonical absolute paths from the task/role lease calculation.
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub(super) struct MediaTarget {
    workspace: PathBuf,
    path: PathBuf,
    relative: String,
}

fn normalized(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points must also be refused.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

impl MediaTarget {
    pub(super) fn prepare(
        workspace: &Path,
        relative: &Path,
        allowed: Option<&[PathBuf]>,
    ) -> Result<Self, String> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("media/output_path: expected a workspace-relative path".into());
        }
        let workspace = normalized(&workspace.canonicalize().map_err(|e| e.to_string())?);
        let path = workspace.join(relative);
        if let Some(allowed) = allowed {
            if !allowed
                .iter()
                .any(|scope| path.starts_with(super::registry_support::canonical_candidate(scope)))
            {
                return Err("media/write_scope: output is outside the task write scope".into());
            }
        }
        let target = Self {
            workspace,
            path,
            relative: relative.to_string_lossy().replace('\\', "/"),
        };
        target.check_parents(false)?;
        match std::fs::symlink_metadata(&target.path) {
            Ok(_) => return Err("media/output_exists: choose a new output path".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(target)
    }

    fn check_parents(&self, create: bool) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or("media/output_path: missing parent")?;
        let relative = parent
            .strip_prefix(&self.workspace)
            .map_err(|e| e.to_string())?;
        let mut current = self.workspace.clone();
        for part in relative.components() {
            current.push(part);
            match std::fs::symlink_metadata(&current) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
                    match std::fs::create_dir(&current) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(e.to_string()),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
                Ok(_) => {}
            }
            let meta = std::fs::symlink_metadata(&current).map_err(|e| e.to_string())?;
            if is_link(&meta) || !meta.is_dir() {
                return Err("media/output_path: linked or non-directory parent".into());
            }
            let canonical = normalized(&current.canonicalize().map_err(|e| e.to_string())?);
            if !canonical.starts_with(&self.workspace) || canonical != current {
                return Err("media/output_path: parent changed or escaped workspace".into());
            }
        }
        Ok(())
    }

    pub(super) fn write_new(&self, bytes: &[u8]) -> Result<String, String> {
        use std::io::Write;
        self.check_parents(true)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(|e| format!("media/output_create: {e}"))?;
        file.write_all(bytes)
            .map_err(|e| format!("media/output_write: {e}"))?;
        file.flush()
            .map_err(|e| format!("media/output_flush: {e}"))?;
        Ok(self.relative.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "owo-media-target-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(normalized(&path.canonicalize().unwrap()))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn scopes_reject_outputs_before_creating_directories() {
        let f = Fixture::new();
        let allowed = vec![f.0.join("images")];
        assert!(MediaTarget::prepare(&f.0, Path::new("outputs/a.png"), Some(&allowed)).is_err());
        assert!(!f.0.join("outputs").exists());
        assert!(MediaTarget::prepare(&f.0, Path::new("images/a.png"), Some(&[])).is_err());
        assert!(MediaTarget::prepare(&f.0, Path::new("../a.png"), None).is_err());
        assert!(
            MediaTarget::prepare(&f.0, Path::new("images-sibling/a.png"), Some(&allowed)).is_err()
        );
        let target =
            MediaTarget::prepare(&f.0, Path::new("images/nested/a.png"), Some(&allowed)).unwrap();
        assert!(!f.0.join("images").exists());
        assert_eq!(target.write_new(b"image").unwrap(), "images/nested/a.png");
        assert_eq!(
            std::fs::read(f.0.join("images/nested/a.png")).unwrap(),
            b"image"
        );
    }
    #[test]
    fn collision_never_overwrites_existing_output() {
        let f = Fixture::new();
        let a = MediaTarget::prepare(&f.0, Path::new("images/a.png"), None).unwrap();
        let b = MediaTarget::prepare(&f.0, Path::new("images/a.png"), None).unwrap();
        a.write_new(b"first").unwrap();
        assert!(b.write_new(b"second").is_err());
        assert_eq!(std::fs::read(f.0.join("images/a.png")).unwrap(), b"first");
        assert!(MediaTarget::prepare(&f.0, Path::new("images/a.png"), None).is_err());
    }
    #[test]
    fn concurrent_writers_have_one_winner() {
        let f = Fixture::new();
        let a = MediaTarget::prepare(&f.0, Path::new("images/a.png"), None).unwrap();
        let b = MediaTarget::prepare(&f.0, Path::new("images/a.png"), None).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let other = barrier.clone();
        let first = std::thread::spawn(move || {
            other.wait();
            a.write_new(b"first")
        });
        let second = std::thread::spawn(move || {
            barrier.wait();
            b.write_new(b"second")
        });
        assert_eq!(
            usize::from(first.join().unwrap().is_ok())
                + usize::from(second.join().unwrap().is_ok()),
            1
        );
        let bytes = std::fs::read(f.0.join("images/a.png")).unwrap();
        assert!(bytes == b"first" || bytes == b"second");
    }
    #[test]
    fn task_scope_can_narrow_role_scope() {
        let f = Fixture::new();
        let task = vec![f.0.join("images/task")];
        assert!(MediaTarget::prepare(&f.0, Path::new("images/other/a.png"), Some(&task)).is_err());
        assert!(MediaTarget::prepare(&f.0, Path::new("images/task/a.png"), Some(&task)).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_parent_cannot_create_outside_workspace() {
        let f = Fixture::new();
        let outside = Fixture::new();
        std::os::unix::fs::symlink(&outside.0, f.0.join("images")).unwrap();
        assert!(MediaTarget::prepare(&f.0, Path::new("images/nested/a.png"), None).is_err());
        assert!(!outside.0.join("nested").exists());
    }
    #[cfg(windows)]
    #[test]
    fn junction_parent_cannot_create_outside_workspace() {
        let f = Fixture::new();
        let outside = Fixture::new();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(f.0.join("images"))
            .arg(&outside.0)
            .output()
            .unwrap();
        assert!(status.status.success(), "junction fixture creation failed");
        assert!(MediaTarget::prepare(&f.0, Path::new("images/nested/a.png"), None).is_err());
        assert!(!outside.0.join("nested").exists());
        std::fs::remove_dir(f.0.join("images")).unwrap();
    }
}
