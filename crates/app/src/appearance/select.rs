//! What the theme controls do to the preferences, ported from the original's
//! `AppearanceSettings.tsx:232-329`: mode, fixed theme, system slot and contrast. Pure over the
//! catalog, the stored preferences and the system's appearance, so every rule is unit-tested.

use std::sync::Arc;

use thinkrail_core::{AppearancePrefs, SystemThemePair, ThemeMode};
use thinkrail_ui::{
    Appearance, Contrast, ThemeCatalog, ThinkrailTheme,
    theme::{Resolution, SystemThemes},
};

/// The mode control's three choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    System,
    Light,
    Dark,
}

fn appearance_of(choice: ModeChoice) -> Option<Appearance> {
    match choice {
        ModeChoice::System => None,
        ModeChoice::Light => Some(Appearance::Light),
        ModeChoice::Dark => Some(Appearance::Dark),
    }
}

/// The resolved state the page shows and the controls act on.
#[derive(Debug, Clone)]
pub struct ThemeView {
    /// `theme` resolved as a fixed choice (what the fixed list marks active).
    pub fixed: Resolution,
    /// The pair system mode uses: the stored one, else derived from `theme`.
    pub pair: SystemThemePair,
    pub light: Resolution,
    pub dark: Resolution,
    /// The theme on screen now.
    pub current: Resolution,
    pub system_mode: bool,
}

impl ThemeView {
    pub fn new(catalog: &ThemeCatalog, prefs: &AppearancePrefs, system: Appearance) -> Self {
        let fixed = catalog.resolve(&prefs.theme, None);
        let pair = prefs.system_theme_pair.clone().unwrap_or_else(|| {
            let derived = catalog.derive_pair(&prefs.theme);
            SystemThemePair {
                light: derived.light,
                dark: derived.dark,
            }
        });
        let slot = |appearance| {
            catalog.resolve(
                &prefs.theme,
                Some(SystemThemes {
                    light: &pair.light,
                    dark: &pair.dark,
                    appearance,
                }),
            )
        };
        let (light, dark) = (slot(Appearance::Light), slot(Appearance::Dark));
        let system_mode = prefs.theme_mode == ThemeMode::System;
        let current = if !system_mode {
            fixed.clone()
        } else if system == Appearance::Light {
            light.clone()
        } else {
            dark.clone()
        };
        Self {
            fixed,
            pair,
            light,
            dark,
            current,
            system_mode,
        }
    }

    pub fn mode(&self) -> ModeChoice {
        if self.system_mode {
            return ModeChoice::System;
        }
        match self.fixed.theme.appearance {
            Appearance::Light => ModeChoice::Light,
            Appearance::Dark => ModeChoice::Dark,
        }
    }

    pub fn contrast(&self) -> Contrast {
        self.current.theme.contrast
    }

    /// The theme a light/dark choice switches to; `None` disables that option.
    pub fn mode_target<'a>(
        &'a self,
        catalog: &'a ThemeCatalog,
        appearance: Appearance,
    ) -> Option<&'a Arc<ThinkrailTheme>> {
        if self.system_mode {
            return Some(match appearance {
                Appearance::Light => &self.light.theme,
                Appearance::Dark => &self.dark.theme,
            });
        }
        catalog.appearance_target(&self.fixed.theme, appearance)
    }
}

/// The preferences after choosing a mode; `None` when nothing changes.
pub fn select_mode(
    catalog: &ThemeCatalog,
    prefs: &AppearancePrefs,
    system: Appearance,
    choice: ModeChoice,
) -> Option<AppearancePrefs> {
    let view = ThemeView::new(catalog, prefs, system);
    if view.mode() == choice {
        return None;
    }
    let mut next = prefs.clone();
    match appearance_of(choice) {
        None => {
            next.theme_mode = ThemeMode::System;
            next.system_theme_pair = Some(view.pair);
        }
        Some(appearance) => {
            let target = view.mode_target(catalog, appearance)?;
            next.theme = target.id.clone();
            next.theme_mode = ThemeMode::Fixed;
        }
    }
    Some(next)
}

/// The preferences after picking a theme from the fixed list.
pub fn select_fixed(prefs: &AppearancePrefs, id: &str) -> Option<AppearancePrefs> {
    if prefs.theme == id && prefs.theme_mode == ThemeMode::Fixed {
        return None;
    }
    let mut next = prefs.clone();
    next.theme = id.to_owned();
    next.theme_mode = ThemeMode::Fixed;
    Some(next)
}

/// The preferences after picking the light or dark slot of the system pair.
pub fn select_system(
    catalog: &ThemeCatalog,
    prefs: &AppearancePrefs,
    system: Appearance,
    appearance: Appearance,
    id: &str,
) -> Option<AppearancePrefs> {
    let mut pair = ThemeView::new(catalog, prefs, system).pair;
    let slot = match appearance {
        Appearance::Light => &mut pair.light,
        Appearance::Dark => &mut pair.dark,
    };
    if slot == id {
        return None;
    }
    id.clone_into(slot);
    let mut next = prefs.clone();
    next.system_theme_pair = Some(pair);
    Some(next)
}

/// The preferences after choosing a contrast: the counterpart of the fixed theme, or of both
/// resolved system slots (a slot without one keeps its theme).
pub fn select_contrast(
    catalog: &ThemeCatalog,
    prefs: &AppearancePrefs,
    system: Appearance,
    contrast: Contrast,
) -> Option<AppearancePrefs> {
    let view = ThemeView::new(catalog, prefs, system);
    if view.contrast() == contrast {
        return None;
    }
    let mut next = prefs.clone();
    if !view.system_mode {
        let fixed = &view.fixed.theme;
        let target = catalog.counterpart(fixed, fixed.appearance, contrast)?;
        next.theme = target.id.clone();
        next.theme_mode = ThemeMode::Fixed;
        return Some(next);
    }
    let pick = |resolution: &Resolution, appearance, current: &str| {
        catalog
            .counterpart(&resolution.theme, appearance, contrast)
            .map_or_else(|| current.to_owned(), |t| t.id.clone())
    };
    let pair = SystemThemePair {
        light: pick(&view.light, Appearance::Light, &view.pair.light),
        dark: pick(&view.dark, Appearance::Dark, &view.pair.dark),
    };
    if pair == view.pair {
        return None;
    }
    next.system_theme_pair = Some(pair);
    Some(next)
}

#[cfg(test)]
mod tests;
