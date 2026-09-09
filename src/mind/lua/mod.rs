//! The Lua a protocol is written in: `apis.lua` says how to build a request and how to fold one
//! server-sent event into a delta, and this runs it. A protocol script sees `api` to register
//! itself with and `json` to encode with, and nothing else — no filesystem, shell, sockets or
//! tools.

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
    #[error("{file}: {message}")]
    Syntax { file: String, message: String },

    #[error("{file}: {message}")]
    Runtime { file: String, message: String },

    /// A declaration was the wrong shape.
    #[error("{what}: {message}")]
    Shape { what: String, message: String },

    #[error("reading {file}: {source}")]
    Io {
        file: String,
        source: std::io::Error,
    },
}
