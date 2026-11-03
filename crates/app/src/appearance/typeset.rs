//! Font preferences to the `Typography` the text system can actually draw (08 A9, A18): a chosen
//! family the device lacks is replaced by the role's bundled face and reported, because a missing
//! primary family never falls back to `Font.fallbacks` on DirectWrite; fallbacks the device
//! lacks are dropped.

use std::collections::HashSet;

use gpui_kit::px;
use thinkrail_core::{AppearancePrefs, FontChoice, FontFamily, FontRole};
use thinkrail_ui::{Typography, fonts};

/// The family names this device can draw, compared case-insensitively like DirectWrite.
#[derive(Debug, Default)]
pub struct Families {
    /// Installed system families.
    installed: HashSet<String>,
    /// Families of registered custom font files.
    custom: HashSet<String>,
}

impl Families {
    pub fn new<'a>(
        installed: impl IntoIterator<Item = &'a str>,
        custom: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            installed: installed.into_iter().map(str::to_lowercase).collect(),
            custom: custom.into_iter().map(str::to_lowercase).collect(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.installed.contains(&key) || self.custom.contains(&key)
    }

    /// DirectWrite searches only installed fonts for fallbacks (08 A18).
    fn can_fall_back_to(&self, name: &str) -> bool {
        self.installed.contains(&name.to_lowercase())
    }
}

pub fn bundled_face(role: FontRole) -> &'static str {
    match role {
        FontRole::Interface => fonts::INTERFACE,
        FontRole::Code => fonts::CODE,
    }
}

/// The family name a reference stands for.
pub fn family_name(family: &FontFamily) -> &str {
    match family {
        FontFamily::Builtin(role) => bundled_face(*role),
        FontFamily::Name(name) => name,
    }
}

/// A role's typography with the chosen family the device lacks, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleFont {
    pub family: String,
    pub fallbacks: Vec<String>,
    /// The chosen family, when this device cannot draw it.
    pub unavailable: Option<String>,
}

pub fn role_font(choice: &FontChoice, role: FontRole, families: &Families) -> RoleFont {
    let bundled = bundled_face(role);
    let (family, unavailable) = match &choice.family {
        None => (bundled.to_owned(), None),
        Some(chosen) => {
            let name = family_name(chosen);
            match chosen {
                FontFamily::Builtin(_) => (name.to_owned(), None),
                FontFamily::Name(_) if families.has(name) => (name.to_owned(), None),
                FontFamily::Name(_) => (bundled.to_owned(), Some(name.to_owned())),
            }
        }
    };
    let fallbacks = choice
        .fallbacks
        .iter()
        .map(family_name)
        .filter(|name| *name != family && families.can_fall_back_to(name))
        .map(str::to_owned)
        .collect();
    RoleFont {
        family,
        fallbacks,
        unavailable,
    }
}

pub fn typography(prefs: &AppearancePrefs, families: &Families) -> Typography {
    let interface = role_font(&prefs.interface_font, FontRole::Interface, families);
    let code = role_font(&prefs.code_font, FontRole::Code, families);
    Typography {
        interface_family: interface.family,
        interface_fallbacks: interface.fallbacks,
        code_family: code.family,
        code_fallbacks: code.fallbacks,
        code_size: px(f32::from(prefs.code_font_size)),
        ligatures: prefs.code_ligatures,
        weight_offset: f32::from(prefs.font_weight_offset),
    }
}

#[cfg(test)]
mod tests;
