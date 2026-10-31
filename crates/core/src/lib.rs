//! Domain types, service contracts and the error model shared by every ThinkRail Native crate.
//!
//! This crate has no I/O and no UI: the service crates implement the traits, `thinkrail-native`
//! consumes them through trait objects.

mod agent;
mod error;
mod host_tools;
mod ids;
mod mcp;
mod model_catalog;
mod plan_review;
mod profile;
mod preset;
mod provider;
mod provider_edit;
mod routing;
mod terminal;
mod workspace;

pub use agent::*;
pub use error::{Error, ErrorCode, ErrorKind, IoContext, Result};
pub use host_tools::*;
pub use ids::*;
pub use mcp::*;
pub use model_catalog::*;
pub use plan_review::*;
pub use profile::*;
pub use preset::*;
pub use provider::*;
pub use provider_edit::*;
pub use routing::*;
pub use terminal::*;
pub use workspace::*;
