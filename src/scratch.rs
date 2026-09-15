//! Directories a test owns, and that go when it does, on the unwind as well as on the return.
//! [`Scratch`] is a directory under `$TMPDIR`; [`Project`] is one under
//! `$XDG_RUNTIME_DIR/melchior`, which is where the code under test keeps sockets and notes.

use std::path::{Path, PathBuf};

/// Distinguishes two scratches made in one process: the pid alone is not enough, because two
/// tests in one binary run on two threads and the name is the caller's to choose.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A directory under the temporary directory, removed when this is dropped. Derefs to [`Path`];
/// a caller that wants to own the path wants [`Scratch::leak`] or `.to_path_buf()`.
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// A fresh directory, named after `prefix` and `name`.
    #[must_use]
    pub fn new(prefix: &str, name: &str) -> Self {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}-{name}", std::process::id()));
        // A pid is reused, and a run that was killed rather than unwound left its directory.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self { path }
    }

    /// Keep the directory, and stop owning it: whoever calls this owns the cleanup.
    #[must_use]
    pub fn leak(self) -> PathBuf {
        let path = self.path.clone();
        std::mem::forget(self);
        path
    }
}

/// A path *inside* a scratch directory, where the directory is what is removed. Derefs to the
/// file.
#[derive(Debug)]
pub struct ScratchFile {
    /// Kept for its `Drop`.
    _dir: Scratch,
    path: PathBuf,
}

impl Scratch {
    /// A named file inside a fresh scratch directory.
    #[must_use]
    pub fn file(prefix: &str, name: &str, file: &str) -> ScratchFile {
        let dir = Scratch::new(prefix, name);
        let path = dir.join(file);
        ScratchFile { _dir: dir, path }
    }
}

impl std::ops::Deref for ScratchFile {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for ScratchFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Ignored: a cleanup that panicked during an unwind would abort the process.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A project directory under `$XDG_RUNTIME_DIR/melchior`, removed when this is dropped: a test
/// about the directory cannot use a temporary one, because the code under test looks there and
/// nowhere else.
#[derive(Debug)]
pub struct Project {
    name: String,
}

impl Project {
    /// A project nothing else is in, named after `prefix` and `name`.
    #[must_use]
    pub fn new(prefix: &str, name: &str) -> Self {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let it = Self {
            name: format!("{prefix}-{}-{n}-{name}", std::process::id()),
        };
        let _ = std::fs::remove_dir_all(it.home());
        std::fs::create_dir_all(it.home()).expect("a project directory");
        it
    }

    /// The directory this project's sockets and notes live in.
    #[must_use]
    pub fn home(&self) -> PathBuf {
        crate::directory::home(&self.name)
    }
}

impl std::ops::Deref for Project {
    type Target = str;

    fn deref(&self) -> &str {
        &self.name
    }
}

impl AsRef<str> for Project {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

/// So a test can write `format!("{it}/main/alpha-rho")`, which is how a full name is spelled.
impl std::fmt::Display for Project {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.name)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.home());
    }
}

#[cfg(test)]
mod tests {
    use super::Scratch;

    #[test]
    fn a_scratch_removes_itself() {
        let path = {
            let dir = Scratch::new("melchior-scratch", "gone");
            std::fs::write(dir.join("f"), "x").expect("write");
            dir.to_path_buf()
        };
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn a_scratch_removes_itself_when_a_test_panics() {
        let path = std::panic::catch_unwind(|| {
            let dir = Scratch::new("melchior-scratch", "panicked");
            let path = dir.to_path_buf();
            std::fs::write(dir.join("f"), "x").expect("write");
            std::panic::panic_any(path);
        })
        .expect_err("the closure panics");
        let path = path.downcast::<std::path::PathBuf>().expect("the path");
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn two_scratches_of_one_name_are_two_directories() {
        let a = Scratch::new("melchior-scratch", "same");
        let b = Scratch::new("melchior-scratch", "same");
        assert_ne!(a.to_path_buf(), b.to_path_buf());
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn a_leaked_scratch_outlives_the_guard() {
        let path = Scratch::new("melchior-scratch", "leaked").leak();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&path);
    }
}

#[cfg(test)]
mod projects {
    use super::Project;

    #[test]
    fn a_project_removes_its_directory_when_a_test_panics() {
        let home = std::panic::catch_unwind(|| {
            let it = Project::new("melchior-project", "panicked");
            let home = it.home();
            std::fs::write(it.home().join("alpha-rho"), "x").expect("write");
            std::panic::panic_any(home);
        })
        .expect_err("the closure panics");
        let home = home.downcast::<std::path::PathBuf>().expect("the path");
        assert!(!home.exists(), "{}", home.display());
    }

    #[test]
    fn two_projects_of_one_name_do_not_share_a_directory() {
        let a = Project::new("melchior-project", "same");
        let b = Project::new("melchior-project", "same");
        assert_ne!(a.home(), b.home());
        assert!(a.home().exists() && b.home().exists());
    }

    #[test]
    fn a_project_is_under_the_runtime_directory_the_code_reads() {
        let it = Project::new("melchior-project", "where");
        assert_eq!(it.home(), crate::directory::home(&it));
    }
}
