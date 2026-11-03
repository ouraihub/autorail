//! ThinkRail colors: theme files in the original's v2 format (the six bundled ones in
//! `themes/*.theme.json`, copied verbatim, and the user's), the catalog that turns the configured
//! theme into one to install, and the semantic roles the original derives from a palette. Views
//! paint with [`Roles`] only; a palette value is never used directly.

mod catalog;
mod manifest;
mod palette;
mod roles;

use gpui_kit::{App, Global, Window, component::Theme as KitTheme};

pub use catalog::{
    DEFAULT_THEME, FileIssue, Resolution, SystemThemes, ThemeCatalog, ThemePair, UserThemeIssue,
    UserThemeIssueKind,
};
pub use manifest::{MAX_NESTING, ManifestError, ManifestErrorKind};
pub use palette::{Ansi, Palette, Syntax};
pub use roles::Roles;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contrast {
    Normal,
    High,
}

/// One parsed theme file. A user theme's `id` carries the catalog's `user.` prefix.
#[derive(Debug, Clone)]
pub struct ThinkrailTheme {
    pub id: String,
    pub label: String,
    pub order: i64,
    pub appearance: Appearance,
    pub contrast: Contrast,
    pub palette: Palette,
    pub ansi: Ansi,
    pub syntax: Syntax,
    pub roles: Roles,
}

impl Global for ThinkrailTheme {}

impl ThinkrailTheme {
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Installs the theme as the app global and maps it onto the component library's colors,
    /// fonts (the active [`crate::Typography`]) and radii.
    pub fn install(&self, window: Option<&mut Window>, cx: &mut App) {
        let mode = match self.appearance {
            Appearance::Dark => gpui_kit::component::ThemeMode::Dark,
            Appearance::Light => gpui_kit::component::ThemeMode::Light,
        };
        KitTheme::change(mode, window, cx);
        let roles = self.roles;
        KitTheme::update(cx, |kit| {
            crate::theme_map::apply(&mut kit.colors, &roles);
            crate::typography::apply_to_kit(kit);
            kit.radius = crate::metrics::radius::SM;
            kit.radius_lg = crate::metrics::radius::LG;
        });
        cx.set_global(self.clone());
    }
}

#[cfg(test)]
mod tests;
