//! The shipped Lua client, driven against a session that answers a call after the caller has
//! given up on it.
//!
//! The real client, the real socket primitive and a real unix socket, because the thing that has
//! to be true is only true at that level: a reply says nothing about which call it answers, so a
//! client that keeps its connection after a failed read hands the abandoned reply to whoever
//! calls next. See FAMILY.md, "A caller that abandons a call abandons the connection".

use melchior::mind::lua::engine::Engine;
use melchior::scratch::Scratch;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// How long the client waits for a reply. Short on purpose: the first call is meant to give up.
const PATIENCE_MS: u64 = 150;

/// The reply to the first call, which arrives after that call has been abandoned.
const STRANDED: &str = r#"{"ok":true,"result":["the answer to the first call"],"n":1}"#;

/// What the second call must not be told.
const STALE: &str = "the answer to the first call";

/// The reply to whatever the client sends next, so a stale answer is not the only thing it could
/// have read.
const FRESH: &str = r#"{"ok":true,"result":["the answer to the second call"],"n":1}"#;

/// One length-prefixed frame, as the family writes them.
fn framed(body: &str) -> Vec<u8> {
    let mut out = (body.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Read one whole frame, or `None` if the peer hung up first.
fn read_frame(sock: &mut UnixStream) -> Option<Vec<u8>> {
    let mut head = [0_u8; 4];
    sock.read_exact(&mut head).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
    sock.read_exact(&mut body).ok()?;
    Some(body)
}

/// A socket in a directory that goes when the test does.
struct Fixture {
    _dir: Scratch,
    at: std::path::PathBuf,
    listening: Option<UnixListener>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = Scratch::new("mlc-t", name);
        // Short: a unix socket path is capped at `SUN_LEN`, and this one is bound for real.
        let at = dir.join("s");
        let listening = UnixListener::bind(&at).expect("bind");
        Self {
            _dir: dir,
            at,
            listening: Some(listening),
        }
    }
}

/// The two halves of the handshake between the test and its fixture: the test says when the
/// caller has given up, and the fixture says when the abandoned reply is on the wire.
struct Cue {
    give_up: Sender<()>,
    landed: Receiver<()>,
}

/// Answer one call late, in `written` bytes, and answer whatever comes next with [`FRESH`].
///
/// The reply is held until the test releases it, so the first call gives up for certain rather
/// than for a sleep long enough to hope for.
fn session(
    fixture: &mut Fixture,
    prompt: Vec<u8>,
    written: Vec<u8>,
) -> (Cue, std::thread::JoinHandle<()>) {
    let listening = fixture.listening.take().expect("one session per fixture");
    let (give_up, released) = channel::<()>();
    let (report, landed) = channel::<()>();

    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listening.accept().expect("accept");
        sock.set_read_timeout(Some(Duration::from_secs(10))).ok();
        read_frame(&mut sock).expect("the first call arrives");
        let _ = sock.write_all(&prompt);
        released.recv().expect("the test releases the reply");
        // Ignored: with the fix the caller has already gone, which is the point.
        let _ = sock.write_all(&written);
        report.send(()).ok();
        if read_frame(&mut sock).is_some() {
            let _ = sock.write_all(&framed(FRESH));
        }
    });

    (Cue { give_up, landed }, handle)
}

/// A VM holding the shipped client, dialled at `at` and ready to be called through.
///
/// The source goes in as a long string and is `load`ed, which is how a host with no filesystem
/// reach takes this library — and means the file under test is the file that ships.
fn dialled(at: &std::path::Path) -> Engine {
    let mut engine = Engine::new();
    let chunk = format!(
        "local source = [=====[\n{source}]=====]\n\
         local it, why = load(source)(melchior.stream).connect({{ path = {at:?}, timeout_ms = {PATIENCE_MS} }})\n\
         assert(it, tostring(why))\n\
         them = it\n",
        source = melchior::CLIENT,
        at = at.display().to_string(),
    );
    engine.run(&chunk, "dial.lua").expect("the client loads");
    engine
}

/// Call `status` through the held connection, recording both return values under `name`.
fn call(engine: &mut Engine, name: &str) {
    let chunk = format!(
        "local got, why = them:call(\"status\")\n\
         melchior.{name} = tostring(got)\n\
         melchior.{name}_why = tostring(why)\n"
    );
    engine.run(&chunk, "call.lua").expect("the call returns");
}

/// What a recorded call said, as `(value, reason)`.
fn said(engine: &mut Engine, name: &str) -> (String, String) {
    engine.harvest();
    let config = engine.config();
    let value = config.string(name).unwrap_or("<unrecorded>").to_owned();
    let why = config
        .string(&format!("{name}_why"))
        .unwrap_or("<unrecorded>")
        .to_owned();
    (value, why)
}

#[test]
fn a_reply_the_caller_gave_up_on_does_not_answer_the_next_call() {
    let mut fixture = Fixture::new("late");
    let (cue, ended) = session(&mut fixture, Vec::new(), framed(STRANDED));

    let mut engine = dialled(&fixture.at);
    call(&mut engine, "first");
    let (first, gave_up) = said(&mut engine, "first");
    assert_eq!(
        first, "nil",
        "the first call was answered in time: {gave_up}"
    );

    cue.give_up.send(()).expect("the fixture is listening");
    cue.landed.recv().expect("the abandoned reply is written");

    call(&mut engine, "second");
    let (second, why) = said(&mut engine, "second");
    assert_ne!(
        second, STALE,
        "the second call was answered by the first call's reply"
    );
    assert_eq!(second, "nil", "and it was told nothing else either");
    assert!(
        why.contains("closed"),
        "the caller must be told the connection is gone, not left guessing: {why}"
    );

    drop(engine);
    ended.join().expect("the fixture ends");
}

#[test]
fn a_frame_read_in_half_takes_the_connection_with_it() {
    // The worse half of the same bug: the header is already consumed, so what is left on the
    // wire is a body the next reader would take for a frame of its own. Here it is one — a whole
    // reply, wrapped inside the body of the abandoned frame, so a client that carries on reads a
    // perfectly well-formed answer to a call it never made.
    let inner = framed(STRANDED);
    let head = (inner.len() as u32).to_be_bytes().to_vec();

    let mut fixture = Fixture::new("half");
    let (cue, ended) = session(&mut fixture, head, inner);

    let mut engine = dialled(&fixture.at);
    call(&mut engine, "first");
    let (first, gave_up) = said(&mut engine, "first");
    assert_eq!(first, "nil", "the first call read a whole frame: {gave_up}");

    cue.give_up.send(()).expect("the fixture is listening");
    cue.landed.recv().expect("the rest of the frame is written");

    call(&mut engine, "second");
    let (second, why) = said(&mut engine, "second");
    assert_ne!(
        second, STALE,
        "the second call read the tail of the abandoned frame as its own reply"
    );
    assert_eq!(second, "nil");
    assert!(why.contains("closed"), "{why}");

    drop(engine);
    ended.join().expect("the fixture ends");
}
