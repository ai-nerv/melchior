//! One agent talking to another.
//!
//! Naming, finding, reaching and refusing: everything about a session's relationship to the
//! other sessions on the machine, and nothing about what a session *is*. There are no turns
//! here, no transcript, no model and no screen — those belong to whatever harness is using
//! this, and melchior is deliberately ignorant of all of them.
//!
//! It began inside magi and was lifted out whole. The rule that made that possible is the one
//! worth keeping:
//!
//! > **Nothing here knows what a harness is.**
//!
//! No dependency on one, and no type from one. The vocabulary a model can call goes out as
//! *data* — [`verbs::described`] hands over a name, a description and a JSON schema — and the
//! harness turns that into whatever a tool looks like on its side. Implementing somebody's tool
//! trait is how a library stops being liftable, and this one has been lifted once already.
//!
//! # Where sessions live
//!
//! ```text
//! $XDG_RUNTIME_DIR/magi/
//!   myproject/                 <- one directory per project
//!     alpha-rho                <- a socket, named by the id and nothing else
//!     iota-mu
//!     iota-mu.parent           <- "alpha-rho": who started it
//! ```
//!
//! There is no server for the layer as a whole. Every session binds its own socket and answers
//! for itself, so the directory *is* the registry: a process that died did not get to remove
//! itself from a list, and a socket file nobody answers is discovered on the first call rather
//! than trusted forever.
//!
//! # The wire is the family's
//!
//! Four-byte big-endian length, then a JSON body. `{"call":…,"args":[…]}` in,
//! `{"ok":true,"n":N,"result":[…]}` out, where **`result` is a list** and `n` is its length.
//! A refusal is a reply, not a dropped connection. A connection serves more than one call, and
//! hanging up is not a mistake. `verbs` is answered from the first version, before any
//! permission check, and `client` beside it hands over the library that speaks all this.
//!
//! Settled before anything shipped, because two tools in one family disagreeing about the reply
//! shape fail *silently* at each other: a client that unpacks a list reads a bare-value server
//! as having returned nothing at all, and an empty answer looks like an empty session.

pub mod answering;
pub mod asking;
pub mod briefing;
pub mod directory;
pub mod framing;
pub mod identity;
pub mod inherited;
pub mod mind;
pub mod noted;
pub mod policy;
pub mod scratch;
pub mod serving;
pub mod verbs;
pub mod wire;

pub use directory::Address;
pub use identity::Identity;
pub use policy::{Reach, Relation, Talk, Whom};

/// What this crate is called, wherever a name is printed.
pub const NAME: &str = "melchior";

/// The Lua client library, as source.
///
/// Shipped here and copied by whoever wants to talk to melchior — that is the family's arrangement,
/// and it is why there is one implementation of the framing rather than one per sibling. It is
/// handed out three ways, because a host that can do one of them often cannot do the others: as
/// this constant to anything linking the crate, on stdout as `melchior lua-api` for anything that
/// can shell out, and over the wire as the `client` verb for a sandboxed VM that can do neither.
pub const CLIENT: &str = include_str!("../lua/melchior.lua");
