//! One agent talking to another: naming, finding, reaching and refusing. Nothing here knows what
//! a harness is — no dependency on one, no type from one, and the vocabulary goes out as data.

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

pub const NAME: &str = "melchior";

/// The Lua client library as source, also served by `melchior lua-api` and the `client` verb.
pub const CLIENT: &str = include_str!("../lua/melchior.lua");
