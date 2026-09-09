//! The mind: which models there are, and asking one a question. The protocols, the credentials
//! and the HTTP live here; a harness only hands over an `Ask` and writes down what comes back.

pub mod acknowledged;
pub mod catalog;
pub mod discovering;
pub mod lua;
pub mod model;
pub mod plugins;
pub mod provider;
pub mod running;
pub mod setup;
pub mod signing;
pub mod speaking;
pub mod wire;
