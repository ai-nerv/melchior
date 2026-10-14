//! Taking a socket path, and giving it back: under an exclusive `flock` on `<socket>.lock` for as
//! long as the listener lives, clearing only a socket of this user's that nothing answers, and on
//! the way out unlinking only the inode this process bound.

use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// What holds a bound path: the lock, and which inode was bound.
pub struct Lease {
    path: PathBuf,
    _lock: std::fs::File,
    inode: u64,
}

impl Lease {
    /// Give the path back: the socket, if it is still the one bound here, and the lock beside it.
    pub fn release(&self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.ino() == self.inode) {
            let _ = std::fs::remove_file(&self.path);
        }
        let _ = std::fs::remove_file(locked_at(&self.path));
    }
}

/// Where the lock for `path` is.
#[must_use]
pub fn locked_at(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Bind `path`, or say why not.
pub(super) fn bind(path: &Path) -> std::io::Result<(tokio::net::UnixListener, Lease)> {
    if let Some(parent) = path.parent() {
        crate::places::ensure(parent).map_err(std::io::Error::other)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits().cast_signed())
        .open(locked_at(path))?;
    match rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(rustix::io::Errno::WOULDBLOCK) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("{} is held by another session", path.display()),
            ));
        }
        Err(why) => return Err(why.into()),
    }
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_socket()
        && meta.uid() == crate::places::uid()
        && !crate::asking::alive(path)
    {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    let inode = std::fs::symlink_metadata(path)?.ino();
    Ok((
        listener,
        Lease {
            path: path.to_path_buf(),
            _lock: lock,
            inode,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[tokio::test]
    async fn a_corpse_is_cleared_and_a_live_socket_or_a_held_lock_is_not() {
        let room = Scratch::new("melchior-binding", "corpse");
        let at = room.join("s");
        drop(std::os::unix::net::UnixListener::bind(&at).expect("a corpse"));
        let (_listener, lease) = bind(&at).expect("the corpse was cleared");

        let again = bind(&at)
            .err()
            .expect("a second bind while the first holds the lock");
        assert_eq!(again.kind(), std::io::ErrorKind::AddrInUse, "{again}");

        lease.release();
        assert!(!at.exists() && !locked_at(&at).exists());
    }

    #[tokio::test]
    async fn a_path_that_is_not_a_socket_is_never_removed() {
        let room = Scratch::new("melchior-binding", "file");
        let at = room.join("s");
        std::fs::write(&at, b"somebody's file").expect("a file");
        assert!(bind(&at).is_err(), "it bound over a file");
        assert_eq!(std::fs::read(&at).expect("still there"), b"somebody's file");
    }

    #[tokio::test]
    async fn giving_back_a_path_somebody_rebound_leaves_their_socket() {
        let room = Scratch::new("melchior-binding", "rebound");
        let at = room.join("s");
        let (listener, lease) = bind(&at).expect("bound");
        drop(listener);
        std::fs::remove_file(&at).expect("unlinked");
        let _theirs = std::os::unix::net::UnixListener::bind(&at).expect("rebound");
        lease.release();
        assert!(at.exists(), "another process's socket was unlinked");
    }
}
