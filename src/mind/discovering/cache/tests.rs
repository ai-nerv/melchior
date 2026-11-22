use super::*;
use crate::scratch::Scratch;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn cache_names_preserve_identity_boundaries_without_values() {
    let a = digest(
        "models",
        &[b"a/b", b"https://one.invalid", b"private-sentinel"],
    );
    for fields in [
        vec![
            b"a-b".as_slice(),
            b"https://one.invalid",
            b"private-sentinel",
        ],
        vec![
            b"a/b".as_slice(),
            b"https://two.invalid",
            b"private-sentinel",
        ],
        vec![b"a/b".as_slice(), b"https://one.invalid", b"changed"],
    ] {
        assert_ne!(a, digest("models", &fields));
    }
    assert_ne!(
        digest("models", &[b"ab", b"c"]),
        digest("models", &[b"a", b"bc"])
    );
    assert_ne!(
        digest("models", &[b"same"]),
        digest("description", &[b"same"])
    );
    assert_eq!(a.len(), 64);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn private_atomic_cache_replaces_data_and_refuses_links_and_oversize() {
    let dir = Scratch::new("melchior-cache", "private");
    let path = dir.join("models/v2/fixture.json");
    assert!(write(&path, b"[]").is_some());
    assert!(read(&path).expect("read").fresh);
    assert_eq!(
        std::fs::metadata(&path).expect("file").permissions().mode() & 0o777,
        0o600
    );
    assert!(write(&path, b"[1]").is_some());
    assert_eq!(read(&path).expect("replaced").raw, "[1]");
    assert!(write(&path, &vec![b'x'; BYTES + 1]).is_none());
    assert_eq!(read(&path).expect("preserved").raw, "[1]");
    let other = dir.join("other");
    std::fs::write(&other, b"untouched").expect("other");
    std::fs::remove_file(&path).expect("remove");
    symlink(&other, &path).expect("link");
    assert!(read(&path).is_none());
    assert!(write(&path, b"rejected").is_none());
    assert_eq!(std::fs::read(&other).expect("other"), b"untouched");
    std::fs::remove_file(&path).expect("unlink");
    std::fs::hard_link(&other, &path).expect("hard link");
    assert!(read(&path).is_none());
    assert!(write(&path, b"rejected").is_none());
    assert_eq!(
        std::fs::read_dir(path.parent().expect("parent"))
            .expect("entries")
            .count(),
        1
    );
}

#[test]
fn cache_refuses_nonregular_files_and_unsafe_directories_without_waiting() {
    let dir = Scratch::new("melchior-cache", "nonregular");
    let path = dir.join("models/v2/fixture.json");
    assert!(write(&path, b"[]").is_some());
    std::fs::remove_file(&path).expect("remove");
    fs::mknodat(
        fs::CWD,
        &path,
        fs::FileType::Fifo,
        Mode::from_raw_mode(0o600),
        0,
    )
    .expect("fifo");
    let started = std::time::Instant::now();
    assert!(read(&path).is_none());
    assert!(write(&path, b"refused").is_none());
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    std::fs::remove_file(&path).expect("unlink");
    crate::places::ensure(&path).expect("directory");
    assert!(read(&path).is_none());
    assert!(write(&path, b"refused").is_none());
    std::fs::remove_dir(&path).expect("remove directory");
    assert!(write(&path, b"[]").is_some());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
    assert!(read(&path).is_none());
    assert!(write(&path, b"refused").is_none());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    let leaf = path.parent().expect("leaf");
    std::fs::set_permissions(leaf, std::fs::Permissions::from_mode(0o755)).expect("mode");
    assert!(read(&path).is_none());
    assert!(write(&path, b"refused").is_none());
    std::fs::set_permissions(leaf, std::fs::Permissions::from_mode(0o700)).expect("mode");
    let linked = dir.join("linked");
    symlink(path.parent().expect("leaf"), &linked).expect("linked ancestor");
    assert!(read(&linked.join("fixture.json")).is_none());
    assert!(write(&linked.join("fixture.json"), b"refused").is_none());
    assert_eq!(read(&path).expect("unchanged").raw, "[]");
}

#[test]
fn concurrent_cache_replacement_never_exposes_partial_json() {
    let dir = Scratch::new("melchior-cache", "atomic");
    let path = dir.join("models/v2/fixture.json");
    let short = serde_json::to_string(&vec!["short"; 8]).expect("json");
    let long = serde_json::to_string(&vec!["long"; 8192]).expect("json");
    assert!(write(&path, short.as_bytes()).is_some());
    std::thread::scope(|threads| {
        for body in [&short, &long] {
            let path = &path;
            threads.spawn(move || {
                for _ in 0..32 {
                    assert!(write(path, body.as_bytes()).is_some());
                }
            });
        }
        for _ in 0..128 {
            if let Some(held) = read(&path) {
                assert!(held.raw == short || held.raw == long);
                assert!(serde_json::from_str::<serde_json::Value>(&held.raw).is_ok());
            }
        }
    });
    assert!(read(&path).is_some());
    assert_eq!(
        std::fs::read_dir(path.parent().expect("parent"))
            .expect("entries")
            .count(),
        1
    );
}
