//! The Lua a protocol is written in.
//!
//! A wire protocol is a description, not code melchior ships: `apis.lua` says how to build a
//! request and how to fold one server-sent event into a delta, and this runs it. Trimmed from
//! the harness this came out of — an adapter needs `api` to register itself and `json` to encode
//! with, and nothing else. No filesystem, no shell, no sockets, no tools.

pub mod adapter;
pub mod convert;
pub mod engine;
pub mod fs;
pub mod json;
pub mod sandbox;
pub mod stream;

/// Anything that can go wrong loading a config.
#[derive(Debug, thiserror::Error)]
pub enum LuaError {
    /// The chunk would not compile.
    ///
    /// Fatal, and it names the file and line: a config that does not parse has not expressed an
    /// intention, so guessing at one is worse than stopping.
    #[error("{file}: {message}")]
    Syntax {
        /// The file that would not compile.
        file: String,
        /// What the parser said.
        message: String,
    },

    /// The chunk compiled and raised while running.
    #[error("{file}: {message}")]
    Runtime {
        /// The file that raised.
        file: String,
        /// What was raised.
        message: String,
    },

    /// A declaration was the wrong shape.
    #[error("{what}: {message}")]
    Shape {
        /// What was being read.
        what: String,
        /// Why it did not fit.
        message: String,
    },

    /// The file could not be read.
    #[error("reading {file}: {source}")]
    Io {
        /// The file that failed.
        file: String,
        /// Why.
        source: std::io::Error,
    },
}
