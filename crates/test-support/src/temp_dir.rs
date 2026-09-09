//! Unique temporary directories cleaned on drop.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp location, removed recursively when
/// dropped (best effort; leaks are reported by OS temp cleanup).
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// # Errors
    /// Fails only if the system temp directory itself is unusable.
    pub fn new(tag: &str) -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("silk-{tag}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_cleans_up() {
        let dir = TempDir::new("basic").expect("temp dir");
        assert!(dir.path().exists());
        let marker = dir.path().join("marker.txt");
        std::fs::write(&marker, b"x").expect("write");
        let path_text = dir.path().to_path_buf();
        drop(dir);
        assert!(!path_text.exists());
    }

    #[test]
    fn unique_paths() {
        let a = TempDir::new("uniq").expect("a");
        let b = TempDir::new("uniq").expect("b");
        assert_ne!(a.path(), b.path());
    }
}
