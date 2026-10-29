//! Taking a socket path, and giving it back: under an exclusive `flock` on `<socket>.lock` for as
//! long as the listener lives, clearing only a socket of this user's that nothing answers, and on
//! the way out unlinking only the inode this process bound.

use crate::places::locking::{Directory, WAIT};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

/// What holds a bound path: the lock, and which inode was bound.
pub struct Lease {
    path: PathBuf,
    lock: std::fs::File,
    socket: std::fs::File,
    released: std::sync::atomic::AtomicBool,
}

impl Lease {
    /// Give the path back: the socket, if it is still the one bound here, and the lock beside it.
    pub fn release(&self) {
        if self
            .released
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        let Ok(directory) = Directory::take(parent, false, WAIT) else {
            return;
        };
        if !directory.matches(&locked_at(&self.path), &self.lock) {
            return;
        }
        if directory.matches(&self.path, &self.socket) {
            let _ = std::fs::remove_file(&self.path);
        }
        let _ = std::fs::remove_file(locked_at(&self.path));
    }
}

/// Where the lock for `path` is.
#[must_use]
pub fn locked_at(path: &Path) -> PathBuf {
    crate::places::locking::socket_lock(path)
}

/// Bind `path`, or say why not.
pub(super) fn bind(path: &Path) -> std::io::Result<(tokio::net::UnixListener, Lease)> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("socket has no parent"))?;
    let directory = Directory::take(parent, true, WAIT)?;
    let lock = directory.lock(&locked_at(path), true).map_err(|why| {
        if why.kind() == std::io::ErrorKind::WouldBlock {
            std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("{} is held by another session", path.display()),
            )
        } else {
            why
        }
    })?;
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_socket()
        && meta.uid() == crate::places::uid()
        && !crate::asking::alive(path)
    {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    let socket = std::fs::File::from(rustix::fs::open(
        path,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?);
    if !socket.metadata()?.file_type().is_socket() || !directory.matches(path, &socket) {
        return Err(std::io::Error::other("socket changed during binding"));
    }
    Ok((
        listener,
        Lease {
            path: path.to_path_buf(),
            lock,
            socket,
            released: std::sync::atomic::AtomicBool::new(false),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[tokio::test]
    async fn directory_sweeps_cannot_unlink_a_socket_or_lock_under_a_held_binding_lease() {
        let project = crate::scratch::Project::new("melchior-bind", "sweep");
        let name = project.to_string();
        let path = crate::directory::socket(&name, "alpha-rho");
        let (listener, lease) = bind(&path).expect("bound");
        let notes = ["parent", "session", "role", "sent", "given", "adopting"];
        for suffix in notes {
            std::fs::write(project.home().join(format!("alpha-rho.{suffix}")), suffix)
                .expect("note");
        }
        let socket_inode = std::fs::symlink_metadata(&path).expect("socket").ino();
        let lock_inode = std::fs::symlink_metadata(locked_at(&path))
            .expect("lock")
            .ino();
        drop(listener);
        assert!(crate::directory::listening(&name).is_empty());
        assert_eq!(
            std::fs::symlink_metadata(&path)
                .expect("leased socket survived sweep")
                .ino(),
            socket_inode
        );
        assert_eq!(
            std::fs::symlink_metadata(locked_at(&path))
                .expect("leased lock survived sweep")
                .ino(),
            lock_inode
        );
        for suffix in notes {
            assert_eq!(
                std::fs::read_to_string(project.home().join(format!("alpha-rho.{suffix}")))
                    .expect("retained note"),
                suffix
            );
        }
        lease.release();
    }

    #[tokio::test]
    async fn a_repeated_release_cannot_remove_a_successors_socket_or_lock() {
        let room = Scratch::new("melchior-binding", "successor");
        let at = room.join("s");
        let (listener, old) = bind(&at).expect("old");
        drop(listener);
        old.release();
        let (_listener, current) = bind(&at).expect("successor");
        old.release();
        let directory = Directory::take(&room, false, WAIT).expect("directory");
        assert!(directory.matches(&at, &current.socket));
        assert!(directory.matches(&locked_at(&at), &current.lock));
        drop(directory);
        current.release();
    }

    #[tokio::test]
    async fn replacing_the_lock_cannot_make_an_old_lease_unlink_current_paths() {
        let room = Scratch::new("melchior-binding", "new-lock");
        let at = room.join("s");
        let (listener, old) = bind(&at).expect("old");
        drop(listener);
        std::fs::remove_file(locked_at(&at)).expect("unlink lock");
        let (_listener, current) = bind(&at).expect("replacement");
        old.release();
        let directory = Directory::take(&room, false, WAIT).expect("directory");
        assert!(directory.matches(&at, &current.socket));
        assert!(directory.matches(&locked_at(&at), &current.lock));
        drop(directory);
        current.release();
    }

    #[test]
    fn a_sibling_sweep_between_bind_and_listen_preserves_the_socket() {
        use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
        let project = crate::scratch::Project::new("melchior-bind", "window");
        let at = crate::directory::socket(&project, "alpha-rho");
        let directory = Directory::take(&project.home(), false, WAIT).expect("directory");
        let lease = directory.lock(&locked_at(&at), true).expect("lease");
        let socket = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .expect("socket");
        rustix::net::bind_unix(&socket, &SocketAddrUnix::new(&at).expect("address")).expect("bind");
        assert!(!crate::directory::answers(&at), "not listening yet");
        std::thread::scope(|scope| {
            scope
                .spawn(|| assert!(crate::directory::listening(&project).is_empty()))
                .join()
                .expect("sibling sweep");
        });
        assert!(at.exists());
        assert!(directory.matches(&locked_at(&at), &lease));
        drop(directory);
        assert!(crate::directory::listening(&project).is_empty());
        assert!(at.exists(), "lifetime lock protects bind/listen window too");
        rustix::net::listen(&socket, 8).expect("listen");
        assert!(crate::directory::answers(&at));
        drop(socket);
        drop(lease);
        assert!(crate::directory::listening(&project).is_empty());
        assert!(!at.exists());
        assert!(!locked_at(&at).exists());
    }

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
