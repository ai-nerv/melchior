use super::{Error, Store};
use rustix::fs::{self, AtFlags, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LIMIT: u64 = 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Directory {
    pub(super) file: File,
    name: OsString,
    path: PathBuf,
}

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> Result<Self, Error> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let name = path
            .file_name()
            .ok_or(Error::UnsafePath("missing filename"))?
            .to_owned();
        let parent = path
            .parent()
            .ok_or(Error::UnsafePath("missing directory"))?;
        let mut file = File::open("/")?;
        for component in parent.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                _ => return Err(Error::UnsafePath("parent traversal is not supported")),
            };
            trusted_directory(&file, false)?;
            if create {
                match fs::mkdirat(&file, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => file.sync_all()?,
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(e) => return Err(Error::Io(e.into())),
                }
            }
            file = open(&file, name, OFlags::RDONLY | OFlags::DIRECTORY)?;
        }
        trusted_directory(&file, true)?;
        Ok(Self { file, name, path })
    }

    pub(super) fn read(&self) -> Result<Store, Error> {
        let file = match self.credential_file() {
            Ok(file) => file,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Store::default());
            }
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::Refused(
                "credentials exceed the 1 MiB storage limit".into(),
            ));
        }
        serde_json::from_slice(&bytes).map_err(|error| Error::Corrupt {
            path: self.path.clone(),
            detail: format!(
                "invalid JSON at line {}, column {}",
                error.line(),
                error.column()
            ),
        })
    }

    fn credential_file(&self) -> Result<File, Error> {
        let file = open(&self.file, &self.name, OFlags::RDONLY)?;
        private_file(&file)?;
        Ok(file)
    }

    pub(super) fn write(&self, store: &Store) -> Result<(), Error> {
        self.write_observed(store, |_| Ok(()))
    }

    pub(super) fn write_observed(
        &self,
        store: &Store,
        mut checkpoint: impl FnMut(Stage) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.read()?;
        let bytes = serde_json::to_vec_pretty(store)?;
        if bytes.len() as u64 > LIMIT {
            return Err(Error::Refused(
                "credentials exceed the 1 MiB storage limit".into(),
            ));
        }
        let mut temp = self.temporary()?;
        checkpoint(Stage::Created)?;
        temp.file.write_all(&bytes)?;
        temp.file.sync_all()?;
        checkpoint(Stage::Synced)?;
        fs::renameat(&self.file, &temp.name, &self.file, &self.name)
            .map_err(std::io::Error::from)?;
        checkpoint(Stage::Replaced)?;
        self.file.sync_all()?;
        checkpoint(Stage::DirectorySynced)?;
        Ok(())
    }

    fn temporary(&self) -> Result<Temporary<'_>, Error> {
        let hash = format!("{:x}", Sha256::digest(self.name.as_bytes()));
        for _ in 0..100 {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = OsString::from(format!(
                ".credentials-{hash}.tmp-{}-{sequence}",
                std::process::id()
            ));
            match open(
                &self.file,
                &name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            ) {
                Ok(file) => {
                    return Ok(Temporary {
                        directory: &self.file,
                        name,
                        file,
                    });
                }
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(Error::Refused(
            "could not allocate a credential temporary file".into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Created,
    Synced,
    Replaced,
    DirectorySynced,
}

struct Temporary<'a> {
    directory: &'a File,
    name: OsString,
    file: File,
}

impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        let _ = fs::unlinkat(self.directory, &self.name, AtFlags::empty());
    }
}

fn open(directory: &File, name: &OsStr, flags: OFlags) -> Result<File, Error> {
    fs::openat(
        directory,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::from_raw_mode(0o600),
    )
    .map(File::from)
    .map_err(|error| Error::Io(error.into()))
}

fn trusted_directory(file: &File, final_component: bool) -> Result<(), Error> {
    let meta = file.metadata()?;
    let owner = rustix::process::geteuid().as_raw();
    // Writable by others, not merely by a group: a private group is how most systems arrange a
    // person's own directories, and refusing those locks them out of their own credentials.
    let writable = meta.mode() & 0o002 != 0;
    let trusted_sticky = meta.uid() == 0 && meta.mode() & 0o1000 != 0;
    if !meta.is_dir()
        || (meta.uid() != owner && (final_component || meta.uid() != 0))
        || (writable && (final_component || !trusted_sticky))
    {
        return Err(Error::UnsafePath(
            "directory ownership or permissions are unsafe",
        ));
    }
    Ok(())
}

fn private_file(file: &File) -> Result<(), Error> {
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.nlink() != 1
        || meta.mode() & 0o7177 != 0
    {
        return Err(Error::UnsafePath(
            "expected an owner-only regular file with one link",
        ));
    }
    Ok(())
}

/// How often a waiting caller looks again at a lock another process holds.
const POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// An exclusive claim on the store, or on one provider within it. Released when dropped, and by
/// the kernel when the process holding it exits, so a holder that dies leaves nothing stale.
pub(super) struct Lock {
    _file: File,
}

impl Directory {
    pub(super) fn lock(&self, scope: &str, patience: std::time::Duration) -> Result<Lock, Error> {
        let name = self.beside(scope);
        let file = open(&self.file, &name, OFlags::WRONLY | OFlags::CREATE)?;
        let until = std::time::Instant::now() + patience;
        loop {
            match fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => return Ok(Lock { _file: file }),
                Err(rustix::io::Errno::WOULDBLOCK) => {}
                Err(error) => return Err(Error::Io(error.into())),
            }
            if std::time::Instant::now() >= until {
                return Err(Error::Refused(
                    "another process is still writing the credentials".into(),
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    /// A lock file named for the store it guards and the scope within it, so that a provider's
    /// claim and the whole store's are different files and neither is the store.
    fn beside(&self, scope: &str) -> OsString {
        let store = format!("{:x}", Sha256::digest(self.name.as_bytes()));
        let scope = format!("{:x}", Sha256::digest(scope.as_bytes()));
        OsString::from(format!(
            ".credentials-{}.lock-{}",
            &store[..16],
            &scope[..16]
        ))
    }
}
