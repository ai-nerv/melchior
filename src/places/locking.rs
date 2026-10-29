use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) const WAIT: Duration = Duration::from_secs(2);

pub(crate) struct Directory {
    file: File,
    path: PathBuf,
}

impl Directory {
    pub(crate) fn take(path: &Path, create: bool, wait: Duration) -> io::Result<Self> {
        let until = Instant::now() + wait;
        loop {
            match Self::once(path, create) {
                Ok(held) => return Ok(held),
                Err(why)
                    if (why.kind() == io::ErrorKind::WouldBlock
                        || (create && why.kind() == io::ErrorKind::NotFound))
                        && Instant::now() < until =>
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(why) => return Err(why),
            }
        }
    }

    fn once(path: &Path, create: bool) -> io::Result<Self> {
        if create {
            super::ensure(path).map_err(io::Error::other)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::NONBLOCK)
                    .bits()
                    .cast_signed(),
            )
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_dir() || meta.uid() != super::uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unowned runtime directory",
            ));
        }
        if meta.mode() & 0o077 != 0 {
            file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        }
        exclusive(&file)?;
        let held = Self {
            file,
            path: path.to_owned(),
        };
        if !same(&held.file, path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "runtime directory changed",
            ));
        }
        Ok(held)
    }

    pub(crate) fn lock(&self, path: &Path, create: bool) -> io::Result<File> {
        if path.parent() != Some(self.path.as_path()) || !same(&self.file, &self.path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "runtime directory changed",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .mode(0o600)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK)
                    .bits()
                    .cast_signed(),
            )
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != super::uid()
            || meta.nlink() != 1
            || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unsafe socket lock",
            ));
        }
        exclusive(&file)?;
        if !self.matches(path, &file) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "socket lock changed",
            ));
        }
        Ok(file)
    }

    pub(crate) fn matches(&self, path: &Path, file: &File) -> bool {
        path.parent() == Some(self.path.as_path())
            && same(&self.file, &self.path)
            && same(file, path)
    }
}

fn exclusive(file: &File) -> io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(Into::into)
}

fn same(file: &File, path: &Path) -> bool {
    let (Ok(held), Ok(named)) = (file.metadata(), std::fs::symlink_metadata(path)) else {
        return false;
    };
    held.nlink() > 0 && held.dev() == named.dev() && held.ino() == named.ino()
}

pub(crate) fn socket_lock(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_busy_directory_has_a_bounded_wait_and_can_be_taken_after_release() {
        let room = Scratch::new("melchior-locking", "busy");
        let held = Directory::take(&room, false, Duration::ZERO).expect("held");
        let start = Instant::now();
        let why = Directory::take(&room, false, Duration::from_millis(20))
            .err()
            .expect("busy");
        assert_eq!(why.kind(), io::ErrorKind::WouldBlock);
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(held);
        assert!(Directory::take(&room, false, Duration::ZERO).is_ok());
    }

    #[test]
    fn a_removed_directory_cannot_authorize_a_lock_in_its_replacement() {
        let room = Scratch::new("melchior-locking", "replace");
        let path = room.join("project");
        let old = Directory::take(&path, true, WAIT).expect("old directory");
        std::fs::remove_dir(&path).expect("remove");
        let current = Directory::take(&path, true, WAIT).expect("new directory");
        let at = path.join("s.lock");
        let lock = current.lock(&at, true).expect("current lock");
        assert!(!old.matches(&at, &lock));
        assert_eq!(
            old.lock(&at, true).expect_err("old domain").kind(),
            io::ErrorKind::NotFound
        );
        assert!(current.matches(&at, &lock));
    }

    #[test]
    fn links_shared_inodes_and_open_permissions_are_not_socket_locks() {
        let room = Scratch::new("melchior-locking", "unsafe");
        let directory = Directory::take(&room, false, WAIT).expect("directory");
        let target = room.join("target");
        std::fs::write(&target, b"untouched").expect("target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let link = room.join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert!(directory.lock(&link, true).is_err());
        let hard = room.join("hard");
        std::fs::hard_link(&target, &hard).expect("hardlink");
        assert!(directory.lock(&hard, true).is_err());
        std::fs::remove_file(&hard).expect("unlink");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(directory.lock(&target, true).is_err());
        assert_eq!(std::fs::read(&target).expect("read"), b"untouched");
    }
}
