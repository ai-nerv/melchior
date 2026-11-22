use rustix::fs::{self, AtFlags, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const BYTES: usize = 8 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn path(purpose: &str, fields: &[&[u8]]) -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(
        root.join("melchior/models/v2")
            .join(format!("{}.json", digest(purpose, fields))),
    )
}

fn digest(purpose: &str, fields: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for field in std::iter::once(purpose.as_bytes()).chain(fields.iter().copied()) {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field);
    }
    format!("{:x}", hash.finalize())
}

pub(crate) struct Held {
    pub raw: String,
    pub fresh: bool,
}

pub(crate) fn read(path: &Path) -> Option<Held> {
    let directory = directory(path, false)?;
    let file = open(&directory, path.file_name()?, OFlags::RDONLY)?;
    let metadata = file.metadata().ok()?;
    if !private(&metadata) || metadata.len() > BYTES as u64 {
        return None;
    }
    let fresh = metadata
        .modified()
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age < super::FRESH);
    let mut raw = String::new();
    file.take(BYTES as u64 + 1).read_to_string(&mut raw).ok()?;
    (raw.len() <= BYTES).then_some(Held { raw, fresh })
}

pub(crate) fn write(path: &Path, raw: &[u8]) -> Option<()> {
    if raw.len() > BYTES {
        return None;
    }
    let directory = directory(path, true)?;
    let name = path.file_name()?;
    match fs::statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => {
            if !private(&open(&directory, name, OFlags::RDONLY)?.metadata().ok()?) {
                return None;
            }
        }
        Err(rustix::io::Errno::NOENT) => {}
        Err(_) => return None,
    }
    for _ in 0..32 {
        let name = format!(
            ".cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let Some(mut file) = open(
            &directory,
            std::ffi::OsStr::new(&name),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        ) else {
            continue;
        };
        let temporary = Temporary {
            directory: &directory,
            name,
        };
        file.write_all(raw).ok()?;
        file.sync_all().ok()?;
        fs::renameat(&directory, &temporary.name, &directory, path.file_name()?).ok()?;
        directory.sync_all().ok()?;
        return Some(());
    }
    None
}

struct Temporary<'a> {
    directory: &'a File,
    name: String,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        let _ = fs::unlinkat(self.directory, &self.name, AtFlags::empty());
    }
}

fn open(directory: &File, name: &std::ffi::OsStr, flags: OFlags) -> Option<File> {
    fs::openat(
        directory,
        name,
        flags | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .ok()
    .map(File::from)
}

fn private(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file()
        && metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.nlink() == 1
        && metadata.mode() & 0o077 == 0
}

fn directory(path: &Path, create: bool) -> Option<File> {
    let path = std::path::absolute(path).ok()?;
    let mut file = File::open("/").ok()?;
    for component in path.parent()?.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => return None,
        };
        if !trusted(&file, false) {
            return None;
        }
        if create {
            match fs::mkdirat(&file, name, Mode::from_raw_mode(0o700)) {
                Ok(()) => file.sync_all().ok()?,
                Err(rustix::io::Errno::EXIST) => {}
                Err(_) => return None,
            }
        }
        file = open(&file, name, OFlags::RDONLY | OFlags::DIRECTORY)?;
    }
    trusted(&file, true).then_some(file)
}

fn trusted(file: &File, leaf: bool) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let owner = rustix::process::geteuid().as_raw();
    metadata.is_dir()
        && if leaf {
            metadata.uid() == owner && metadata.mode() & 0o077 == 0
        } else {
            (metadata.uid() == owner || metadata.uid() == 0)
                && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0)
        }
}

#[cfg(test)]
mod tests;
