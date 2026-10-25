//! Test-only scratch directories.
//!
//! The tests here write real files, so each one needs somewhere of its own.
//! Naming those directories after the process id alone is not enough: a run that
//! is killed part-way leaves its directories behind, and the next run with a
//! recycled pid inherits them, which shows up later as a test that fails once
//! and then passes. Every path here is unique per call and wiped before use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch directory that removes itself when it goes out of scope.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);

        let dir = std::env::temp_dir().join(format!(
            "cartridge-test-{}-{n}-{nanos}-{tag}",
            std::process::id()
        ));
        // Start from nothing, whatever happened on a previous run.
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create scratch directory");
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Write a file, creating parent directories as needed.
    pub fn write(&self, rel: &str, contents: &[u8]) -> PathBuf {
        let path = self.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }
        std::fs::write(&path, contents).expect("write scratch file");
        path
    }

    /// A path under the scratch directory, written with `/` between its parts.
    ///
    /// The separator is not cosmetic. Windows opens a file just as happily
    /// through `a/b` as through `a\b`, so a test that only reads and writes
    /// cannot tell them apart — but a test comparing against a path the code
    /// built with `Path::join` gets a backslash, and `a/b != a\b` as strings.
    /// Splitting the parts here means one spelling works on both platforms.
    pub fn join(&self, rel: &str) -> PathBuf {
        self.0.join(
            rel.split('/')
                .filter(|part| !part.is_empty())
                .fold(PathBuf::new(), |path, part| path.join(part)),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scratch_is_its_own_directory() {
        let a = Scratch::new("same");
        let b = Scratch::new("same");
        assert_ne!(a.path(), b.path(), "same tag must not collide");
        assert!(a.path().is_dir() && b.path().is_dir());
    }

    #[test]
    fn it_cleans_up_after_itself() {
        let path = {
            let s = Scratch::new("gone");
            s.write("a/b/c.txt", b"x");
            assert!(s.join("a/b/c.txt").is_file());
            s.path().to_path_buf()
        };
        assert!(!path.exists(), "scratch should be removed on drop");
    }

    #[test]
    fn a_joined_path_uses_this_platforms_separator() {
        // Otherwise every expectation spelled with a `/` fails on Windows only,
        // where the code under test produced a `\`.
        let s = Scratch::new("separators");
        assert_eq!(s.join("a/b"), s.path().join("a").join("b"));
    }
}
