//! Domain types, service contracts and the error model shared by every ThinkRail Native crate.
//!
//! This crate has no I/O and no UI: the service crates implement the traits, `thinkrail-native`
//! consumes them through trait objects.

mod agent;
mod app_prefs;
mod appearance;
mod commands;
mod engine_setup;
mod error;
mod host_tools;
mod ids;
mod mcp;
mod model_catalog;
mod plan_review;
mod preset;
mod profile;
mod provider;
mod provider_edit;
mod routing;
mod terminal;
mod usage;
mod workspace;

pub use agent::*;
pub use app_prefs::*;
pub use appearance::*;
pub use commands::*;
pub use engine_setup::*;
pub use error::{Error, ErrorCode, ErrorKind, IoContext, Result};
pub use host_tools::*;
pub use ids::*;
pub use mcp::*;
pub use model_catalog::*;
pub use plan_review::*;
pub use preset::*;
pub use profile::*;
pub use provider::*;
pub use provider_edit::*;
pub use routing::*;
pub use terminal::*;
pub use usage::*;
pub use workspace::*;
