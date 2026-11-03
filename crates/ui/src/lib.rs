//! ThinkRail visual tokens (taken from the original themes) and stateless GPUI components.
//!
//! `theme` owns colors and the theme catalog, `metrics` sizes and type styles, `fonts` the
//! bundled faces and font-file inspection, `typography` the user's font settings, `assets` the
//! icon source; `components` builds on all of them and never touches services or files.

mod assets;
pub mod components;
pub mod fonts;
pub mod metrics;
pub mod theme;
mod theme_map;
pub mod typography;

pub use assets::UiAssets;
pub use theme::{Appearance, Contrast, Roles, ThemeCatalog, ThinkrailTheme};
pub use typography::{TerminalFont, Typography};
