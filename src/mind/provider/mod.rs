pub mod api;
pub mod client;
pub mod compat;
mod control;
pub mod copilot;
pub mod endpoint;
mod http;
mod identity;
pub mod model;
pub mod oauth;
pub mod retry;
pub(crate) mod routing;
pub mod sse;
pub mod thinking;

pub(crate) mod strict;
mod strict_schema;

#[cfg(test)]
mod testing;

pub(crate) use self::http::usable_url;
pub use self::http::{Builder, Http, Request, Unanswered, http};
