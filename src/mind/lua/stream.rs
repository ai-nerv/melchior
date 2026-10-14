//! The socket primitive the family's Lua clients need, with the framing done here: a client hands
//! over a request table and gets the decoded reply back, so no client writes a length prefix or
//! reads a frame by hand.
//!
//! ```lua
//! local h = melchior.stream.connect(path, timeout_ms)
//! local reply, why = h:call(request_table, { deadline_ms = N })
//! h:close()
//! ```

use crate::framing::{self, Wire};
use crate::mind::lua::convert::{json_from_lua, lua_from_json};
use luna::{Callback, CallbackReturn, Context, Table, Value};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A connected socket, shared between the handle's methods.
type Handle = Rc<RefCell<Option<UnixStream>>>;

/// What the next call on a closed handle meets.
const CLOSED: &str = "this connection is closed";

/// The socket a config may dial at `path`, resolved: one of the family's own, in its runtime
/// directory, and nothing else there — not the session bus, the agents or systemd beside them.
fn dialable(path: &std::path::Path) -> Option<std::path::PathBuf> {
    crate::places::family_socket(path, &crate::places::family_roots(), crate::places::uid())
}

/// A whole number of milliseconds above zero, as a duration.
fn millis(value: Value) -> Option<Duration> {
    match value {
        Value::Integer(ms) if ms > 0 => Some(Duration::from_millis(ms.unsigned_abs())),
        Value::Number(ms) if ms > 0.0 && ms.is_finite() => Some(Duration::from_millis(ms as u64)),
        _ => None,
    }
}

pub fn table<'gc>(ctx: Context<'gc>) -> Table<'gc> {
    let stream = Table::new(&ctx);
    let connect = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let (path, timeout_ms): (Value, Value) = stack.consume(ctx)?;
        let Value::String(path) = path else {
            stack.replace(ctx, (Value::Nil, "connect needs a path"));
            return Ok(CallbackReturn::Return);
        };
        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();

        // Refused as `nil` and a reason rather than a raise, so a config probing for an absent
        // sibling carries on without it.
        let Some(path) = dialable(std::path::Path::new(&path)) else {
            stack.replace(
                ctx,
                (
                    Value::Nil,
                    "only a family program's own socket is this VM's to open",
                ),
            );
            return Ok(CallbackReturn::Return);
        };

        // A default rather than a wait forever: a stale socket left by a killed peer accepts and
        // never answers.
        let timeout = millis(timeout_ms).unwrap_or(Duration::from_secs(5));

        match UnixStream::connect(&path) {
            Ok(socket) => {
                let handle = handle_table(ctx, Rc::new(RefCell::new(Some(socket))), timeout);
                stack.replace(ctx, handle);
            }
            Err(e) => {
                stack.replace(ctx, (Value::Nil, e.to_string()));
            }
        }
        Ok(CallbackReturn::Return)
    });
    stream.set(ctx, "connect", connect).ok();
    stream
}

/// A handle, as the client expects: `call` and `close`, called with `:`.
fn handle_table<'gc>(ctx: Context<'gc>, socket: Handle, patience: Duration) -> Table<'gc> {
    let handle = Table::new(&ctx);

    let held = Rc::clone(&socket);
    let call = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // Called as `h:call(request, options)`, so the handle itself is the first argument.
        let (_self, request, options): (Value, Value, Value) = stack.consume(ctx)?;
        let deadline = match options {
            Value::Table(options) => millis(options.get_value(ctx, "deadline_ms")),
            _ => None,
        }
        .unwrap_or(patience);
        match called(ctx, &held, request, deadline) {
            Ok(reply) => stack.replace(ctx, reply),
            Err(why) => stack.replace(ctx, (Value::Nil, why)),
        }
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "call", call).ok();

    let held = Rc::clone(&socket);
    let close = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // Taking the stream closes it and makes a second close a no-op, which client cleanup
        // paths rely on.
        held.borrow_mut().take();
        stack.replace(ctx, true);
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "close", close).ok();

    handle
}

/// One call on `held`: `request` framed as JSON and sent, one reply frame read back and decoded,
/// the whole of it within `deadline`.
///
/// A call refused before a byte is sent leaves the connection as it was. Any failure after that
/// closes it, because a reply left on the wire would answer the next call.
fn called<'gc>(
    ctx: Context<'gc>,
    held: &Handle,
    request: Value<'gc>,
    deadline: Duration,
) -> Result<Value<'gc>, String> {
    let mut slot = held.borrow_mut();
    let Some(socket) = slot.as_mut() else {
        return Err(CLOSED.to_owned());
    };
    if !matches!(request, Value::Table(_)) {
        return Err("call needs a request table".to_owned());
    }
    let request = json_from_lua(ctx, request)
        .map_err(|why| format!("the request cannot be written as JSON: {why}"))?;
    let frame =
        framing::framed(Wire::Json, &request).map_err(|why| format!("{why}; nothing was sent"))?;

    let mut line = Within {
        socket,
        until: Instant::now() + deadline,
    };
    let answered = line
        .write_all(&frame)
        .and_then(|()| line.flush())
        .map_err(|why| failed(&why, deadline, "sending the call"))
        .and_then(|()| {
            framing::read_body(&mut line).map_err(|why| failed(&why, deadline, "reading the reply"))
        })
        .and_then(|body| {
            Wire::read_any::<serde_json::Value>(&body).map_err(|why| format!("bad reply: {why}"))
        })
        .and_then(|reply| {
            if reply.is_object() {
                Ok(reply)
            } else {
                Err("a reply that is not an object".to_owned())
            }
        });
    match answered {
        Ok(reply) => Ok(lua_from_json(ctx, &reply)),
        Err(why) => {
            slot.take();
            Err(format!(
                "{why} — {CLOSED}, because a reply left on the wire would answer the next call"
            ))
        }
    }
}

/// What went wrong while `doing`, in words a caller can act on.
fn failed(why: &std::io::Error, deadline: Duration, doing: &str) -> String {
    use std::io::ErrorKind;
    if framing::too_large(why) {
        return why.to_string();
    }
    match why.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
            format!("{doing} took longer than {} ms", deadline.as_millis())
        }
        ErrorKind::UnexpectedEof => format!("the session closed the connection while {doing}"),
        _ => format!("{doing} failed: {why}"),
    }
}

/// A socket read and written against one deadline for the whole call, not one per syscall.
struct Within<'a> {
    socket: &'a mut UnixStream,
    until: Instant,
}

impl Within<'_> {
    /// How long is left, or `TimedOut` once nothing is.
    fn left(&self) -> std::io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the deadline passed",
            ));
        }
        Ok(left)
    }
}

impl Read for Within<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        self.socket.set_read_timeout(Some(left))?;
        self.socket.read(buf)
    }
}

impl Write for Within<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.left()?;
        self.socket.set_write_timeout(Some(left))?;
        self.socket.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}

/// A config reaches the family's sockets and nothing else that listens beside them.
#[cfg(test)]
mod tests {
    use crate::mind::lua::engine::Engine;
    use crate::scratch::Scratch;

    #[test]
    fn a_socket_outside_the_family_is_refused_by_the_vm() {
        let dir = Scratch::new("melchior-stream", "bus");
        let bus = dir.join("bus");
        let _held = std::os::unix::net::UnixListener::bind(&bus).expect("bound");
        let mut engine = Engine::new();
        engine
            .run(
                &format!(
                    "local h, why = melchior.stream.connect({:?}, 500)\n\
                     melchior.opened = h ~= nil\n\
                     melchior.why = tostring(why)",
                    bus.display().to_string()
                ),
                "dial.lua",
            )
            .expect("runs");
        engine.harvest();
        assert_eq!(engine.config().boolean("opened"), Some(false));
        let why = engine.config().string("why").unwrap_or_default().to_owned();
        assert!(why.contains("family"), "{why}");
    }
}

/// `call` frames, sends, reads one reply and decodes it, and gives the connection up on anything
/// that leaves the two ends out of step.
#[cfg(test)]
mod calling {
    use crate::framing::{self, MOST};
    use crate::mind::lua::engine::Engine;
    use crate::scratch::Project;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    const OK: &str = r#"{"ok":true,"family":1,"n":1,"result":["x"]}"#;

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(body.len())
            .expect("fits")
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(body);
        out
    }

    /// A socket in a family directory answering each frame it reads with the next of `replies`,
    /// then holding the line until the caller lets go, or hanging up at once unless `hold`;
    /// joining it gives the frames it read.
    fn fake(
        name: &str,
        replies: Vec<Vec<u8>>,
        hold: bool,
    ) -> (Project, String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let project = Project::new("mls-t", name);
        let at = project.home().join("s");
        let listening = UnixListener::bind(&at).expect("bind");
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listening.accept().expect("accept");
            sock.set_read_timeout(Some(Duration::from_secs(10))).ok();
            let mut read = Vec::new();
            for reply in replies {
                let Ok(frame) = framing::read_body(&mut sock) else {
                    break;
                };
                read.push(frame);
                let _ = sock.write_all(&reply);
            }
            if hold {
                let _ = std::io::Read::read(&mut sock, &mut [0_u8; 1]);
            }
            read
        });
        (project, at.display().to_string(), server)
    }

    /// Two calls on one connection, `first` with `options`: each one's reply `ok` and reason.
    fn twice(at: &str, first: &str, options: &str) -> [String; 4] {
        let mut engine = Engine::new();
        engine
            .run(
                &format!(
                    "local h = assert(melchior.stream.connect({at:?}, 2000))\n\
                     local a, why = h:call({first}, {options})\n\
                     melchior.a = tostring(a and a.ok)\n\
                     melchior.why = tostring(why)\n\
                     local b, after = h:call({{ call = 'verbs' }})\n\
                     melchior.b = tostring(b and b.result and b.result[1])\n\
                     melchior.after = tostring(after)\n\
                     h:close()"
                ),
                "call.lua",
            )
            .expect("runs");
        engine.harvest();
        let config = engine.config();
        ["a", "why", "b", "after"].map(|key| config.string(key).unwrap_or_default().to_owned())
    }

    #[test]
    fn a_request_goes_out_as_one_json_frame_and_the_reply_comes_back_a_table() {
        let (_project, at, server) = fake("trip", vec![framed(OK.as_bytes()); 2], true);
        let [a, why, b, after] = twice(&at, "{ call = 'status', args = { 'x' } }", "nil");
        assert_eq!([a.as_str(), b.as_str()], ["true", "x"], "{why} {after}");
        let read = server.join().expect("the fake ends");
        let sent: serde_json::Value = serde_json::from_slice(&read[0]).expect("JSON");
        assert_eq!(sent, serde_json::json!({ "call": "status", "args": ["x"] }));
    }

    #[test]
    fn a_call_one_byte_over_the_cap_is_refused_before_a_byte_is_sent() {
        let around = serde_json::to_vec(&serde_json::json!({ "call": "tell", "args": [""] }))
            .expect("JSON")
            .len();
        let (_project, at, server) = fake("cap", vec![framed(OK.as_bytes())], true);
        let first = format!(
            "{{ call = 'tell', args = {{ string.rep('z', {}) }} }}",
            MOST + 1 - around
        );
        let [a, why, b, after] = twice(&at, &first, "nil");
        assert_eq!(a, "nil");
        assert!(why.contains(&(MOST + 1).to_string()), "{why}");
        assert!(why.contains("nothing was sent"), "{why}");
        assert_eq!(
            b, "x",
            "the line stays open after a refusal at home: {after}"
        );
        let read = server.join().expect("the fake ends");
        assert_eq!(read.len(), 1, "the oversized call reached the far end");
        assert!(read[0].starts_with(br#"{"call":"verbs""#));
    }

    #[test]
    fn a_short_read_takes_the_connection_with_it() {
        let mut short = 100_u32.to_be_bytes().to_vec();
        short.extend_from_slice(br#"{"ok":true"#);
        let (_project, at, server) = fake("short", vec![short], false);
        let [a, why, b, after] = twice(&at, "{ call = 'status' }", "nil");
        assert_eq!(a, "nil");
        assert!(why.contains("closed the connection"), "{why}");
        assert!(why.contains("this connection is closed"), "{why}");
        assert_eq!(
            [b.as_str(), after.as_str()],
            ["nil", "this connection is closed"]
        );
        server.join().expect("the fake ends");
    }

    #[test]
    fn a_reply_that_does_not_finish_in_time_takes_the_connection_with_it() {
        let half = 100_u32.to_be_bytes().to_vec();
        let (_project, at, server) = fake("late", vec![half], true);
        let [a, why, b, after] = twice(&at, "{ call = 'status' }", "{ deadline_ms = 150 }");
        assert_eq!(a, "nil");
        assert!(why.contains("150 ms"), "{why}");
        assert_eq!(
            [b.as_str(), after.as_str()],
            ["nil", "this connection is closed"]
        );
        server.join().expect("the fake ends");
    }

    #[test]
    fn a_reply_over_the_cap_is_refused_before_its_body_is_read() {
        let over = u32::try_from(MOST + 1)
            .expect("fits")
            .to_be_bytes()
            .to_vec();
        let (_project, at, server) = fake("over", vec![over], true);
        let [a, why, b, after] = twice(&at, "{ call = 'status' }", "nil");
        assert_eq!(a, "nil");
        assert!(why.contains(&(MOST + 1).to_string()), "{why}");
        assert!(why.contains("over the family's cap"), "{why}");
        assert_eq!(
            [b.as_str(), after.as_str()],
            ["nil", "this connection is closed"]
        );
        server.join().expect("the fake ends");
    }
}
