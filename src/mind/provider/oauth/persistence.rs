use super::storage::{self, Stage};
use super::*;
use crate::scratch::Scratch;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Set on a re-executed copy of this test binary, to make it act as a second process rather than
/// run the suite. `-` means the default location, which is what reads `XDG_DATA_HOME`.
const HELPER: &str = "MELCHIOR_OAUTH_HELPER_STORE";
const PROVIDER: &str = "MELCHIOR_OAUTH_HELPER_PROVIDER";
const READY: &str = "MELCHIOR_OAUTH_HELPER_READY";
const RENEW: &str = "MELCHIOR_OAUTH_HELPER_RENEW";
const NAME: &str = "mind::provider::oauth::persistence::a_second_process";

fn store() -> Store {
    let mut store = Store::default();
    store.put(
        "test",
        Tokens {
            access: "synthetic-access".into(),
            refresh: Some("synthetic-refresh".into()),
            expires_at: 1000,
        },
    );
    store
}

/// A scratch directory the credential code will accept: the store refuses to sit in a directory
/// others can write to, and the ambient umask is not the test's to rely on.
fn private(name: &str) -> Scratch {
    let dir = Scratch::new("oauth-persistence", name);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("mode");
    dir
}

fn tokens(provider: &str) -> Tokens {
    Tokens {
        access: format!("{provider}-access"),
        refresh: Some(format!("{provider}-refresh")),
        expires_at: 2000,
    }
}

fn second_process(path: &Path, provider: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().expect("this binary"));
    command
        .args([NAME, "--exact", "--quiet"])
        .env(HELPER, path)
        .env(PROVIDER, provider);
    command
}

fn waited_for(marker: &Path) {
    let until = Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        assert!(Instant::now() < until, "the second process never started");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Not a test of its own: the body a re-executed copy of this binary runs, because a store shared
/// between processes cannot be exercised by threads.
#[test]
fn a_second_process() {
    let Some(at) = std::env::var_os(HELPER) else {
        return;
    };
    let at = PathBuf::from(at);
    let provider = std::env::var(PROVIDER).expect("a provider");
    if let Some(ready) = std::env::var_os(READY) {
        let held = hold_within(&at, &provider, Duration::from_secs(5)).expect("a claim");
        std::fs::write(PathBuf::from(ready), "held").expect("a marker");
        if std::env::var_os(RENEW).is_some() {
            std::thread::sleep(Duration::from_millis(300));
            Store::amend_within(&at, Duration::from_secs(20), |store| {
                store.put(&provider, tokens(&provider));
            })
            .expect("amended");
        } else {
            std::thread::sleep(Duration::from_secs(30));
        }
        drop(held);
        return;
    }
    let change = |store: &mut Store| store.put(&provider, tokens(&provider));
    if at == Path::new("-") {
        Store::amend(change).expect("amended");
    } else {
        Store::amend_within(&at, Duration::from_secs(20), change).expect("amended");
    }
}

#[test]
fn credential_writes_refuse_symlinks_without_changing_the_target() {
    let dir = private("symlink");
    let target = dir.join("target");
    std::fs::write(&target, "untouched").expect("target");
    let path = dir.join("credentials.json");
    symlink(&target, &path).expect("link");
    assert!(store().save_to(&path).is_err());
    assert_eq!(
        std::fs::read_to_string(target).expect("target"),
        "untouched"
    );
}

#[test]
fn credential_writes_refuse_public_existing_files() {
    let dir = private("public");
    let path = dir.join("credentials.json");
    std::fs::write(&path, "untouched").expect("file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
    assert!(store().save_to(&path).is_err());
    assert_eq!(std::fs::read_to_string(path).expect("file"), "untouched");
}

#[test]
fn corrupt_credential_errors_never_echo_stored_values() {
    let dir = private("corrupt");
    let path = dir.join("credentials.json");
    std::fs::write(
        &path,
        r#"{"providers":{"test":{"access":"a","expires_at":"SYNTHETIC_CREDENTIAL_SECRET"}}}"#,
    )
    .expect("file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
    let error = Store::load_from(&path).expect_err("corruption").to_string();
    assert!(!error.contains("SYNTHETIC_CREDENTIAL_SECRET"), "{error}");
}

#[test]
fn a_refresh_without_a_replacement_keeps_the_stored_refresh_token() {
    let mut store = store();
    store.renew(
        "test",
        Tokens {
            access: "renewed".into(),
            refresh: None,
            expires_at: 2000,
        },
    );
    let held = store.get("test").expect("tokens");
    assert_eq!(held.access, "renewed");
    assert_eq!(held.refresh.as_deref(), Some("synthetic-refresh"));
    store.renew(
        "test",
        Tokens {
            access: "again".into(),
            refresh: Some("rotated".into()),
            expires_at: 3000,
        },
    );
    let held = store.get("test").expect("tokens");
    assert_eq!(held.refresh.as_deref(), Some("rotated"));
}

#[test]
fn an_interrupted_credential_write_leaves_the_previous_store_readable() {
    let dir = private("interrupted");
    let path = dir.join("credentials.json");
    store().save_to(&path).expect("a first write");
    let before = std::fs::read_to_string(&path).expect("stored");
    for stage in [Stage::Created, Stage::Synced] {
        let directory = storage::Directory::open(&path, true).expect("a directory");
        let mut changed = Store::default();
        changed.put("test", tokens("second"));
        let why = directory
            .write_observed(&changed, |reached| {
                if reached == stage {
                    return Err(Error::Refused("interrupted".into()));
                }
                Ok(())
            })
            .expect_err("an interrupted write");
        assert!(why.to_string().contains("interrupted"), "{why}");
        assert_eq!(std::fs::read_to_string(&path).expect("stored"), before);
        assert_eq!(Store::load_from(&path).expect("readable"), store());
    }
}

#[test]
fn two_processes_changing_different_providers_both_survive() {
    let dir = private("concurrent");
    let path = dir.join("credentials.json");
    let names = ["alpha", "beta", "gamma", "delta"];
    let mut running: Vec<_> = names
        .iter()
        .map(|provider| second_process(&path, provider).spawn().expect("a process"))
        .collect();
    for child in &mut running {
        assert!(child.wait().expect("reaped").success());
    }
    let store = Store::load_from(&path).expect("the store");
    for provider in names {
        assert_eq!(
            store.get(provider).expect(provider).access,
            tokens(provider).access
        );
    }
}

#[test]
fn a_claim_held_by_a_process_that_dies_is_released() {
    let dir = private("died");
    let path = dir.join("credentials.json");
    let marker = dir.join("ready");
    store().save_to(&path).expect("a first write");
    let mut child = second_process(&path, "test")
        .env(READY, &marker)
        .spawn()
        .expect("a process");
    waited_for(&marker);
    let why = hold_within(&path, "test", Duration::from_millis(200))
        .err()
        .expect("a claim another process holds");
    assert!(why.to_string().contains("another process"), "{why}");
    child.kill().expect("killed");
    child.wait().expect("reaped");
    hold_within(&path, "test", Duration::from_secs(5)).expect("released by the kernel");
}

#[test]
fn credentials_follow_a_custom_data_directory() {
    let dir = private("xdg");
    let mut child = second_process(Path::new("-"), "test")
        .env("XDG_DATA_HOME", dir.as_ref() as &Path)
        .spawn()
        .expect("a process");
    assert!(child.wait().expect("reaped").success());
    let store = Store::load_from(&dir.join("magi").join("credentials.json")).expect("the store");
    assert_eq!(store.get("test").expect("tokens").access, "test-access");
}

#[test]
fn credential_writes_refuse_world_writable_directories() {
    let dir = private("world");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("mode");
    let why = store()
        .save_to(&dir.join("credentials.json"))
        .expect_err("a directory anybody can write to");
    assert!(why.to_string().contains("unsafe"), "{why}");
}

#[test]
fn a_process_that_waited_for_a_claim_sees_what_the_holder_wrote() {
    let dir = private("waited");
    let path = dir.join("credentials.json");
    let marker = dir.join("ready");
    let mut child = second_process(&path, "test")
        .env(READY, &marker)
        .env(RENEW, "1")
        .spawn()
        .expect("a process");
    waited_for(&marker);
    let held = hold_within(&path, "test", Duration::from_secs(20)).expect("the released claim");
    let store = Store::load_from(&path).expect("the store");
    assert_eq!(
        store.get("test").expect("tokens").access,
        tokens("test").access
    );
    drop(held);
    assert!(child.wait().expect("reaped").success());
}
