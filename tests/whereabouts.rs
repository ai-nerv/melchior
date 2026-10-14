//! Where the shipped Lua client looks for a session: the uid only from the host, the runtime
//! directory only when the host's `fs.stat` says it is this uid's and closed, and
//! `<tmp>/melchior-<uid>` otherwise.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use melchior::mind::lua::engine::Engine;

/// What `script` evaluates to, run after the client is loaded as `client` over a transport that
/// records the path it was asked to dial in `asked`, with `env` as the environment and `host` as
/// the `hexe` global.
fn probe(env: &str, host: &str, script: &str) -> String {
    let mut engine = Engine::new();
    let chunk = format!(
        "local said = {{ {env} }}\n\
         os.getenv = function(name) return said[name] end\n\
         hexe = {host}\n\
         local asked = ''\n\
         local fake = {{ connect = function(path) asked = path; return nil, 'no' end }}\n\
         local client = load([=====[\n{source}]=====])(fake)\n\
         melchior.answer = tostring({script})\n",
        source = melchior::CLIENT,
    );
    engine.run(&chunk, "where.lua").expect("the client loads");
    engine.harvest();
    engine
        .config()
        .string("answer")
        .unwrap_or("<unrecorded>")
        .to_owned()
}

/// The socket the client dials for `demo/main/x`; empty when it dials nothing.
fn dialled(env: &str, host: &str) -> String {
    probe(
        env,
        host,
        "(function() client.connect('demo/main/x') return asked end)()",
    )
}

/// What a host's `fs.stat` answering one record for every path looks like.
fn stat(kind: &str, uid: u32, mode: u32) -> String {
    format!("function() return {{ type = {kind:?}, uid = {uid}, mode = {mode} }} end")
}

#[test]
fn the_uid_is_the_hosts_and_never_the_environment() {
    let env = r#"UID = "4242", TMPDIR = "/scratch""#;
    assert_eq!(dialled(env, "nil"), "");
    assert_eq!(
        dialled(env, "{ uid = 1000 }"),
        "/scratch/melchior-1000/demo/x"
    );
    assert_eq!(
        dialled(env, "{ uid = function() return 7 end }"),
        "/scratch/melchior-7/demo/x"
    );
    for said in [r#"{ uid = "1000" }"#, "{ uid = 1.5 }", "{ uid = -1 }"] {
        assert_eq!(dialled(env, said), "", "{said} was taken for a uid");
    }
}

#[test]
fn the_runtime_dir_is_used_only_when_the_host_says_it_is_private() {
    let at = |runtime: &str, said: &str| {
        dialled(
            &format!("XDG_RUNTIME_DIR = {runtime:?}, TMPDIR = \"/scratch\""),
            &format!("{{ uid = 1000, fs = {{ stat = {said} }} }}"),
        )
    };
    assert_eq!(
        at("/w/run/1000/", &stat("directory", 1000, 0o700)),
        "/w/run/1000/melchior/demo/x"
    );
    for (runtime, said) in [
        ("/w/run/1000", stat("directory", 1000, 0o755)),
        ("/w/run/1000", stat("directory", 1000, 0o710)),
        ("/w/run/1000", stat("directory", 1001, 0o700)),
        ("/w/run/1000", stat("symlink", 1000, 0o700)),
        ("/w/run/1000", "nil".to_owned()),
        (
            "/w/run/1000",
            "function() error('no such directory') end".to_owned(),
        ),
        ("w/run/1000", stat("directory", 1000, 0o700)),
        ("", stat("directory", 1000, 0o700)),
    ] {
        assert_eq!(
            at(runtime, &said),
            "/scratch/melchior-1000/demo/x",
            "{runtime:?} trusted though stat said {said}"
        );
    }
}

#[test]
fn the_fallback_is_under_an_absolute_tmpdir_only() {
    assert_eq!(
        dialled(r#"TMPDIR = "/scratch/""#, "{ uid = 1000 }"),
        "/scratch/melchior-1000/demo/x"
    );
    for env in [r#"TMPDIR = "scratch""#, r#"TMPDIR = """#, ""] {
        assert_eq!(
            dialled(env, "{ uid = 1000 }"),
            "/tmp/melchior-1000/demo/x",
            "{env:?}"
        );
    }
}

#[test]
fn a_host_with_no_uid_gets_no_directory_and_nothing_is_run() {
    let host = "{ run = function() ran = true return { ok = true } end, \
                fs = { ls = function() listed = true return {} end } }";
    let env = r#"XDG_RUNTIME_DIR = "/w/run/1000", TMPDIR = "/scratch""#;
    let said = probe(
        env,
        host,
        "(function()
            local session, why = client.connect('demo/main/x')
            local found, missing = client.instances('demo')
            return table.concat({ tostring(session), asked, tostring(found), tostring(missing),
                                  tostring(client.answers('demo/main/x')), tostring(ran),
                                  tostring(listed) }, '|')
        end)()",
    );
    let parts: Vec<&str> = said.split('|').collect();
    assert_eq!(parts[0], "nil", "{said}");
    assert_eq!(parts[1], "", "it dialled something: {said}");
    assert_eq!(parts[2], "nil", "{said}");
    assert!(parts[3].contains("uid"), "{said}");
    assert_eq!(parts[4], "false", "{said}");
    assert_eq!(
        parts[5..],
        ["nil", "nil"],
        "it ran or listed something: {said}"
    );
}

#[test]
fn the_client_reads_no_uid_from_the_environment_and_runs_no_descriptor() {
    assert!(
        !melchior::CLIENT.contains("\"UID\""),
        "the client names $UID"
    );
    assert!(
        !melchior::CLIENT.contains(".tool\""),
        "the client reads a .tool descriptor"
    );
}
