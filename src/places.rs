//! Where the family keeps its sockets and notes, by the rule FAMILY.md writes down: the runtime
//! directory only when it is safe, else `<tmp>/<program>-<uid>`, and every directory made here
//! made `0700` and checked with `lstat` before anything is put in it.

use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The programs whose sockets a family member may dial, by the name each keeps its own under.
pub const FAMILY: &[&str] = &["magi", "casper", "melchior", "memory", "balthasar"];

/// This process's user.
#[must_use]
pub fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// `$XDG_RUNTIME_DIR`, when it is absolute, a directory, this user's, and closed to everyone else.
/// Empty is unset.
#[must_use]
pub fn runtime() -> Option<PathBuf> {
    let said = std::env::var_os("XDG_RUNTIME_DIR").filter(|said| !said.is_empty())?;
    trusted(&PathBuf::from(said), uid())
}

/// The same check, on a path and an owner handed in.
#[must_use]
pub fn trusted(dir: &Path, owner: u32) -> Option<PathBuf> {
    if !dir.is_absolute() {
        return None;
    }
    let meta = std::fs::metadata(dir).ok()?;
    (meta.is_dir() && meta.uid() == owner && meta.mode() & 0o077 == 0).then(|| dir.to_path_buf())
}

/// The system temporary directory: `$TMPDIR` when it is absolute, else `/tmp`.
#[must_use]
pub fn tmp() -> PathBuf {
    tmp_from(std::env::var_os("TMPDIR"))
}

/// The same, from what the environment said.
#[must_use]
pub fn tmp_from(said: Option<std::ffi::OsString>) -> PathBuf {
    said.map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Where `program` keeps its sockets: `<runtime>/<program>`, or `<tmp>/<program>-<uid>`.
#[must_use]
pub fn base(program: &str) -> PathBuf {
    runtime().map_or_else(
        || tmp().join(format!("{program}-{}", uid())),
        |runtime| runtime.join(program),
    )
}

/// Every directory a family socket may sit under, canonical, for the ones that exist.
#[must_use]
pub fn family_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for program in FAMILY {
        if let Some(runtime) = runtime() {
            out.push(runtime.join(program));
        }
        out.push(tmp().join(format!("{program}-{}", uid())));
    }
    out.into_iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect()
}

/// Whether `path` is a socket of this user's, inside one of `roots`: the parent directory resolved
/// and under a root, and the path itself a socket by `lstat`.
#[must_use]
pub fn family_socket(path: &Path, roots: &[PathBuf], owner: u32) -> Option<PathBuf> {
    use std::os::unix::fs::FileTypeExt;
    let name = path.file_name()?;
    let parent = std::fs::canonicalize(path.parent()?).ok()?;
    if !roots.iter().any(|root| parent.starts_with(root)) {
        return None;
    }
    let real = parent.join(name);
    let meta = std::fs::symlink_metadata(&real).ok()?;
    (meta.file_type().is_socket() && meta.uid() == owner).then_some(real)
}

/// Make `dir` and anything missing above it `0700`, then require it to be a real directory of this
/// user's that nobody else can enter. One of ours left wider by an older build is closed.
///
/// # Errors
/// A refusal naming the path, when it is a link, somebody else's, or not a directory.
pub fn ensure(dir: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|why| format!("{}: {why}", dir.display()))?;
    held(dir, uid())
}

/// The check [`ensure`] makes, on an owner handed in.
///
/// # Errors
/// As [`ensure`].
pub fn held(dir: &Path, owner: u32) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|why| format!("{}: {why}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a directory of its own, so nothing is kept in it",
            dir.display()
        ));
    }
    if meta.uid() != owner {
        return Err(format!(
            "{} belongs to uid {}, not to this user, so nothing is kept in it",
            dir.display(),
            meta.uid()
        ));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|why| format!("{} is open to others and stays so: {why}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_runtime_directory_open_to_others_or_relative_is_not_trusted() {
        let dir = Scratch::new("melchior-places", "open");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert_eq!(trusted(&dir, uid()), None, "a 0755 directory was trusted");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert_eq!(trusted(&dir, uid()).as_deref(), Some(&*dir.to_path_buf()));
        assert_eq!(trusted(Path::new("relative"), uid()), None);
        assert_eq!(trusted(Path::new(""), uid()), None, "empty is unset");
        assert_eq!(
            trusted(&dir, uid() + 1),
            None,
            "somebody else's was trusted"
        );
    }

    #[test]
    fn the_fallback_is_under_an_absolute_tmpdir_only() {
        let said = |value: &str| tmp_from(Some(std::ffi::OsString::from(value)));
        assert_eq!(said("/scratch"), Path::new("/scratch"));
        for value in ["scratch", "./scratch", ""] {
            assert_eq!(said(value), Path::new("/tmp"), "{value:?}");
        }
        assert_eq!(tmp_from(None), Path::new("/tmp"));
    }

    #[test]
    fn a_directory_somebody_else_owns_or_a_link_is_refused_by_name() {
        let dir = Scratch::new("melchior-places", "owned");
        let why = held(&dir, uid() + 1).expect_err("another owner");
        assert!(why.contains(&dir.display().to_string()), "{why}");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&*dir, &link).expect("a link");
        assert!(
            held(&link, uid()).is_err(),
            "a symlink was taken for a directory"
        );
    }

    #[test]
    fn a_directory_made_here_is_closed_to_everyone_else() {
        let dir = Scratch::new("melchior-places", "made");
        let deep = dir.join("a").join("b");
        ensure(&deep).expect("made");
        let mode = std::fs::metadata(&deep).expect("there").mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o775)).expect("chmod");
        ensure(&deep).expect("an older build's directory is taken back");
        assert_eq!(
            std::fs::metadata(&deep).expect("there").mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn only_a_socket_under_a_family_root_is_dialable() {
        let dir = Scratch::new("melchior-places", "dial");
        let root = dir.join(format!("melchior-{}", uid()));
        std::fs::create_dir_all(root.join("demo")).expect("a root");
        let roots = vec![std::fs::canonicalize(&root).expect("canonical")];
        let inside = root.join("demo").join("alpha-rho");
        let _held = std::os::unix::net::UnixListener::bind(&inside).expect("bound");
        assert!(
            family_socket(&inside, &roots, uid()).is_some(),
            "the family's own"
        );

        let bus = dir.join("bus");
        let _bus = std::os::unix::net::UnixListener::bind(&bus).expect("bound");
        assert!(
            family_socket(&bus, &roots, uid()).is_none(),
            "the session bus"
        );

        let climbing = root.join("demo").join("..").join("..").join("bus");
        assert!(
            family_socket(&climbing, &roots, uid()).is_none(),
            "climbed out"
        );

        let file = root.join("demo").join("plain");
        std::fs::write(&file, b"").expect("a file");
        assert!(
            family_socket(&file, &roots, uid()).is_none(),
            "not a socket"
        );

        let link = root.join("demo").join("link");
        std::os::unix::fs::symlink(&bus, &link).expect("a link");
        assert!(family_socket(&link, &roots, uid()).is_none(), "a link out");
        assert!(
            family_socket(&inside, &roots, uid() + 1).is_none(),
            "another owner"
        );
    }
}
