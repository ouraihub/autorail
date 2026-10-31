//! ThinkRail visual tokens (taken from the original themes) and stateless GPUI components.
//!
//! `theme` owns colors, `metrics` sizes and type styles, `fonts` the bundled faces, `assets` the
//! icon source; `components` builds on all four and never touches services.

mod assets;
pub mod components;
pub mod fonts;
pub mod metrics;
pub mod theme;
mod theme_map;

pub use assets::UiAssets;
pub use theme::{Appearance, Roles, ThinkrailTheme};
