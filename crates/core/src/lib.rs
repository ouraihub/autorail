//! Domain types, service contracts and the error model shared by every ThinkRail Native crate.
//!
//! This crate has no I/O and no UI: the service crates implement the traits, `thinkrail-native`
//! consumes them through trait objects.

mod agent;
mod error;
mod host_tools;
mod ids;
mod plan_review;
mod profile;
mod terminal;
mod workspace;

pub use agent::*;
pub use error::{Error, ErrorCode, ErrorKind, IoContext, Result};
pub use host_tools::*;
pub use ids::*;
pub use plan_review::*;
pub use profile::*;
pub use terminal::*;
pub use workspace::*;
