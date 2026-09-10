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

/// An open connection to one instance.
pub struct Held {
    stream: UnixStream,
    /// Who this session is, put on every call.
    me: String,
    /// Whether a call went out whose reply was never read to the end. One reply per call and
    /// nothing in a reply that says which call it answers, so an abandoned one is still in the
    /// stream: the next call would read it as its own answer, and every answer after that would
    /// belong to the call before it. See FAMILY.md.
    adrift: bool,
}

impl Held {
    /// Open a connection to whatever is listening at `path`.
    pub fn at(path: &Path, me: &Identity) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        // Both directions: a peer that accepted and never answered would hold this open, and the
        // tool call with it.
        stream.set_read_timeout(Some(PATIENCE))?;
        stream.set_write_timeout(Some(PATIENCE))?;
        Ok(Self {
            stream,
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
        // Set before the write, not after: a request half written is one the far end will finish
        // reading out of whatever is sent next.
        self.adrift = true;
        let answered = framing::write_to(&mut Writing(&self.stream), &call)
            .and_then(|()| framing::read_from(&mut Reading(&self.stream)));
        match answered {
            Ok(reply) => {
                self.adrift = false;
                Ok(reply)
            }
            Err(why) => {
                // So the far end learns too, rather than answering into a socket nobody reads.
                let _ = self.stream.shutdown(std::net::Shutdown::Both);
                Err(why)
            }
        }
    }
}

/// A `&UnixStream` writes; the borrow is what lets one connection do both halves.
struct Writing<'a>(&'a UnixStream);

impl Write for Writing<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

/// And reads.
struct Reading<'a>(&'a UnixStream);

impl Read for Reading<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// Whether anything is actually listening as `them`. A socket file outlives the process that made
/// it, so the directory only says who *was* here.
#[must_use]
pub fn answers(where_it_is: &Path, me: &Identity) -> bool {
    match Held::at(where_it_is, me) {
        Ok(_) => true,
        Err(why) => {
            // The only place the reason survives: the caller wants a yes or a no.
            crate::noted!(
                "asking: nothing answers at {}: {why}",
                where_it_is.display()
            );
            false
        }
    }
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
            stream: UnixStream::pair().expect("a pair").0,
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
            stream: mine,
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
            stream: mine,
            adrift: false,
            me: me().full(),
        };
        let (wrote, written) = std::sync::mpsc::channel::<()>();
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
        answering.join().expect("the far end finished");
    }

    #[test]
    fn nothing_listening_is_an_error_rather_than_a_wait() {
        // A path, not a name: turning a name into a path is `directory::dial`'s job.
        assert!(!answers(std::path::Path::new("/no/such/socket"), &me()));
    }
}
