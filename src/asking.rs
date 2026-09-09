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

    /// Write one call, read one reply.
    fn ask(&mut self, call: Call) -> std::io::Result<Reply> {
        framing::write_to(&mut Writing(&self.stream), &call)?;
        framing::read_from(&mut Reading(&self.stream))
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
    fn nothing_listening_is_an_error_rather_than_a_wait() {
        // A path, not a name: turning a name into a path is `directory::dial`'s job.
        assert!(!answers(std::path::Path::new("/no/such/socket"), &me()));
    }
}
