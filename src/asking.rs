//! Reaching another instance: the other half of [`crate::serving`], and deliberately blocking.
//! [`Held`] stays open across calls — a client that closes after each one dies on its second with
//! a broken pipe. Every call names its caller, or the far end answers `verbs` and nothing else.

use crate::framing;
use crate::identity::Identity;
use crate::wire::{Call, Reply};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// How long to wait for a connection, and then for each answer from a healthy peer.
const PATIENCE: Duration = Duration::from_secs(10);

/// What answers a call made to this process's own socket, without the socket.
pub type Answerer = std::sync::Arc<dyn Fn(Call) -> Reply + Send + Sync>;

/// The sockets this process answers itself, and how.
static HERE: std::sync::Mutex<Vec<(std::path::PathBuf, Answerer)>> =
    std::sync::Mutex::new(Vec::new());

/// Answer calls this process makes to `path` with `answerer`, rather than by dialling it.
pub fn answer_here(path: &Path, answerer: Answerer) {
    if let Ok(mut here) = HERE.lock() {
        here.retain(|(held, _)| held != path);
        here.push((path.to_path_buf(), answerer));
    }
}

fn here(path: &Path) -> Option<Answerer> {
    let here = HERE.lock().ok()?;
    here.iter()
        .find(|(held, _)| held == path)
        .map(|(_, answerer)| std::sync::Arc::clone(answerer))
}

/// How a call reaches the far end.
enum Line {
    Socket(UnixStream),
    /// This process's own socket, answered in place.
    Local(Answerer),
}

/// An open connection to one instance.
pub struct Held {
    line: Line,
    /// Who this session is, put on every call.
    me: String,
    /// Whether a call went out whose reply was never read to the end. One reply per call and
    /// nothing in a reply that says which call it answers, so an abandoned one is still in the
    /// stream: the next call would read it as its own answer. See FAMILY.md.
    adrift: bool,
}

impl Held {
    /// Open a connection to whatever is listening at `path`, as long as it is this user. A path
    /// this process answers itself is answered in place.
    pub fn at(path: &Path, me: &Identity) -> std::io::Result<Self> {
        Self::within(path, me, PATIENCE)
    }

    /// The same, waiting at most `patience` to connect, to write a call and to read its reply.
    pub fn within(path: &Path, me: &Identity, patience: Duration) -> std::io::Result<Self> {
        let line = match here(path) {
            Some(answerer) => Line::Local(answerer),
            None => {
                let stream = dial(path, patience)?;
                stream.set_read_timeout(Some(patience))?;
                stream.set_write_timeout(Some(patience))?;
                Line::Socket(stream)
            }
        };
        Ok(Self {
            line,
            me: me.full(),
            adrift: false,
        })
    }

    /// Make one call and read its answer. A refusal comes back as a [`Reply`] with `ok: false`.
    pub fn call(&mut self, verb: &str, args: Vec<serde_json::Value>) -> std::io::Result<Reply> {
        self.ask(Call {
            call: verb.to_owned(),
            args,
            from: Some(self.me.clone()),
            token: None,
        })
    }

    /// The same, carrying the secret the far end was started with. Only `stop` needs one; kept
    /// separate so a verb cannot pick one up by accident.
    pub fn call_with(
        &mut self,
        verb: &str,
        args: Vec<serde_json::Value>,
        token: &str,
    ) -> std::io::Result<Reply> {
        self.ask(Call {
            call: verb.to_owned(),
            args,
            from: Some(self.me.clone()),
            token: Some(token.to_owned()),
        })
    }

    /// Write one call, read one reply. A call that did not get its whole answer takes the
    /// connection with it: what it is still owed arrives on this stream and nowhere else, and
    /// reading it means waiting for a call this side has already given up on.
    fn ask(&mut self, call: Call) -> std::io::Result<Reply> {
        if self.adrift {
            return Err(std::io::Error::other(
                "this connection is closed: a reply left on the wire would answer the next call",
            ));
        }
        let stream = match &self.line {
            Line::Local(answerer) => return Ok(answerer(call)),
            Line::Socket(stream) => stream,
        };
        // Set before the write, not after: a request half written is one the far end will finish
        // reading out of whatever is sent next.
        self.adrift = true;
        let answered: std::io::Result<Reply> = framing::write_to(&mut Writing(stream), &call)
            .and_then(|()| framing::read_from(&mut Reading(stream)));
        match answered {
            // Read whole, so the connection is in step; refused by name all the same.
            Ok(reply) if reply.family > crate::wire::FAMILY => {
                self.adrift = false;
                Err(std::io::Error::other(format!(
                    "the reply is written in family {}, newer than this melchior's {}",
                    reply.family,
                    crate::wire::FAMILY
                )))
            }
            Ok(reply) => {
                self.adrift = false;
                Ok(reply)
            }
            Err(why) => {
                // So the far end learns too, rather than answering into a socket nobody reads.
                let _ = stream.shutdown(std::net::Shutdown::Both);
                Err(why)
            }
        }
    }
}

/// Connect to `path` within `patience`, to a server running as this user and nobody else. A
/// listener whose backlog is full is retried until the deadline rather than waited on for good.
pub fn dial(path: &Path, patience: Duration) -> std::io::Result<UnixStream> {
    use rustix::io::Errno;
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    let fd = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )?;
    let address = SocketAddrUnix::new(path)?;
    let deadline = std::time::Instant::now() + patience;
    loop {
        match rustix::net::connect_unix(&fd, &address) {
            Ok(()) | Err(Errno::ISCONN) => break,
            Err(Errno::AGAIN | Errno::INPROGRESS | Errno::ALREADY | Errno::INTR)
                if std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(Errno::AGAIN | Errno::INPROGRESS | Errno::ALREADY) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("{} did not take the connection in time", path.display()),
                ));
            }
            Err(why) => return Err(why.into()),
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    let peer = rustix::net::sockopt::get_socket_peercred(&stream)?;
    if peer.uid.as_raw() != crate::places::uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "{} answers as uid {}, not as this user",
                path.display(),
                peer.uid.as_raw()
            ),
        ));
    }
    Ok(stream)
}

/// A `&UnixStream` writes; the borrow is what lets one connection do both halves.
struct Writing<'a>(&'a UnixStream);

impl Write for Writing<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// And reads.
struct Reading<'a>(&'a UnixStream);

impl Read for Reading<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

/// Whether anything is actually listening at `where_it_is`. A socket file outlives the process
/// that made it, so the directory only says who *was* here.
#[must_use]
pub fn answers(where_it_is: &Path, _me: &Identity) -> bool {
    alive(where_it_is)
}

/// Whether something may still be listening at `path`, asked with one connect that never waits:
/// it is gone only when nothing is bound there or the kernel refuses the connection. A full
/// backlog, an interrupted call or a process out of descriptors is a live peer having a bad moment.
#[must_use]
pub fn alive(path: &Path) -> bool {
    use rustix::io::Errno;
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    let Ok(fd) = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    ) else {
        return true;
    };
    let Ok(address) = SocketAddrUnix::new(path) else {
        return true;
    };
    match rustix::net::connect_unix(&fd, &address) {
        Err(Errno::CONNREFUSED | Errno::NOENT) => {
            crate::noted!("asking: nothing answers at {}", path.display());
            false
        }
        _ => true,
    }
}

/// The phase `them` reports off its `status`, or `None` when it does not answer or does not say —
/// so a reader like `crew` can show what each agent is doing, not only that it is there.
#[must_use]
pub fn phase(where_it_is: &Path, me: &Identity) -> Option<String> {
    let mut held = Held::at(where_it_is, me).ok()?;
    let reply = held.call("status", Vec::new()).ok()?;
    reply
        .result
        .first()?
        .get("phase")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Identity {
        Identity {
            project: "magi".to_owned(),
            role: "main".to_owned(),
            id: "alpha-rho".to_owned(),
        }
    }

    #[test]
    fn a_call_carries_the_caller_s_name() {
        let held = Held {
            // Any fd will do: nothing is written, and building the frame is what is under test.
            line: Line::Socket(UnixStream::pair().expect("a pair").0),
            adrift: false,
            me: me().full(),
        };
        let call = Call {
            call: "status".to_owned(),
            args: Vec::new(),
            from: Some(held.me.clone()),
            token: None,
        };
        assert_eq!(call.from.as_deref(), Some("magi/main/alpha-rho"));
        assert!(call.token.is_none(), "an ordinary call carries no secret");
    }

    #[test]
    fn a_round_trip_over_a_real_socket_pair_reads_back() {
        let (mine, theirs) = UnixStream::pair().expect("a pair");
        let mut held = Held {
            line: Line::Socket(mine),
            adrift: false,
            me: me().full(),
        };
        let answering = std::thread::spawn(move || {
            let call: Call = framing::read_from(&mut Reading(&theirs)).expect("reads");
            assert_eq!(call.call, "status");
            assert_eq!(call.from.as_deref(), Some("magi/main/alpha-rho"));
            framing::write_to(
                &mut Writing(&theirs),
                &Reply::of(serde_json::json!({"busy": true})),
            )
            .expect("writes");
        });
        let reply = held.call("status", Vec::new()).expect("answered");
        answering.join().expect("the far end finished");
        assert!(reply.ok);
        assert_eq!(reply.result[0]["busy"], true);
    }

    #[test]
    fn a_call_that_did_not_get_its_answer_takes_the_connection_with_it() {
        let (mine, theirs) = UnixStream::pair().expect("a pair");
        let mut held = Held {
            line: Line::Socket(mine),
            adrift: false,
            me: me().full(),
        };
        let (wrote, written) = std::sync::mpsc::channel::<()>();
        let (done, finish) = std::sync::mpsc::channel::<()>();
        let answering = std::thread::spawn(move || {
            let _: Call = framing::read_from(&mut Reading(&theirs)).expect("the first call");
            // A header past what this socket reads, with the connection still open on both
            // sides: the shape a timeout has, reachable in no time at all.
            (&theirs).write_all(&[0xff_u8; 4]).expect("a header");
            wrote.send(()).ok();
            // What that call was still owed, arriving after the caller has given up on it.
            let _ = framing::write_to(
                &mut Writing(&theirs),
                &Reply::of(serde_json::json!("the answer to the first call")),
            );
            wrote.send(()).ok();
            // Held open to the end, so a second call that went out would be answered rather than
            // failing on a socket that had simply gone.
            finish.recv().ok();
        });

        let gave_up = held
            .call("status", Vec::new())
            .expect_err("that reply cannot be read");
        written.recv().expect("the header is written");
        written.recv().expect("the abandoned reply is written");

        let told = held
            .call("status", Vec::new())
            .expect_err("the second call must not be answered by the first call's reply");
        assert!(
            told.to_string().contains("closed"),
            "left with `{told}` after `{gave_up}`"
        );
        done.send(()).ok();
        answering.join().expect("the far end finished");
    }

    #[test]
    fn a_reply_from_a_newer_family_is_refused_by_name_and_the_line_stays_in_step() {
        let (mine, theirs) = UnixStream::pair().expect("a pair");
        let mut held = Held {
            line: Line::Socket(mine),
            adrift: false,
            me: me().full(),
        };
        let answering = std::thread::spawn(move || {
            for family in [crate::wire::FAMILY + 1, crate::wire::FAMILY] {
                let _: Call = framing::read_from(&mut Reading(&theirs)).expect("a call");
                let mut reply = Reply::of(serde_json::json!("hello"));
                reply.family = family;
                framing::write_to(&mut Writing(&theirs), &reply).expect("writes");
            }
        });
        let newer = held.call("status", Vec::new()).expect_err("a newer family");
        assert!(newer.to_string().contains("newer"), "{newer}");
        let same = held
            .call("status", Vec::new())
            .expect("the next call is answered");
        assert_eq!(same.result[0], "hello");
        answering.join().expect("the far end finished");
    }

    #[test]
    fn a_server_of_this_user_is_dialled_and_its_answer_read() {
        let room = crate::scratch::Scratch::new("melchior-asking", "dial");
        let at = room.join("s");
        let listening = std::os::unix::net::UnixListener::bind(&at).expect("bound");
        let stream = dial(&at, Duration::from_secs(2)).expect("this user's own server");
        drop(stream);
        drop(listening);
        assert!(dial(&room.join("absent"), Duration::from_millis(50)).is_err());
    }

    #[test]
    fn a_peer_is_gone_only_when_nothing_is_bound_or_it_refuses() {
        use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
        let room = crate::scratch::Scratch::new("melchior-asking", "alive");
        assert!(!alive(&room.join("absent")), "nothing bound");
        std::fs::write(room.join("plain"), b"").expect("a file");
        assert!(!alive(&room.join("plain")), "not a socket");

        // A listener that accepts nobody, with its backlog full: alive, and not waited on.
        let at = room.join("full");
        let listener = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .expect("a socket");
        let address = SocketAddrUnix::new(&at).expect("an address");
        rustix::net::bind_unix(&listener, &address).expect("bound");
        rustix::net::listen(&listener, 0).expect("listening");
        let mut queued = Vec::new();
        while queued.len() < 64 {
            let Ok(waiting) = dial(&at, Duration::ZERO) else {
                break;
            };
            queued.push(waiting);
        }
        assert!(alive(&at), "a full backlog read as a corpse");
        let began = std::time::Instant::now();
        let refused = dial(&at, Duration::from_millis(100)).expect_err("nobody accepts");
        assert_eq!(refused.kind(), std::io::ErrorKind::TimedOut, "{refused}");
        assert!(began.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_call_to_this_process_s_own_socket_is_answered_in_place() {
        let at = std::path::Path::new("/nonexistent/melchior-asking/here");
        answer_here(
            at,
            std::sync::Arc::new(|call: Call| Reply::of(serde_json::json!(call.call))),
        );
        let mut held = Held::at(at, &me()).expect("no socket is dialled");
        let reply = held.call("status", Vec::new()).expect("answered");
        assert_eq!(reply.result[0], "status");
    }

    #[test]
    fn nothing_listening_is_an_error_rather_than_a_wait() {
        // A path, not a name: turning a name into a path is `directory::dial`'s job.
        assert!(!answers(std::path::Path::new("/no/such/socket"), &me()));
    }
}
