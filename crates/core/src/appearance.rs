//! Appearance preferences (native-multi-agent 08): theme choice and the font settings, one JSON
//! value in `app_prefs`. Ported from the original's `AppConfig` theme and font fields; every field
//! is validated on its own and an invalid one falls back to its default without touching the rest.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The configured default theme (original `DEFAULT_CONFIG.theme`).
pub const DEFAULT_THEME: &str = "dark";

/// `MIN`–`MAX` px; the editor and diffs render `EDITOR_DELTA` px smaller.
#[derive(Debug)]
pub struct CodeFontSize;
impl CodeFontSize {
    pub const MIN: u8 = 10;
    pub const MAX: u8 = 24;
    pub const DEFAULT: u8 = 13;
    pub const EDITOR_DELTA: u8 = 2;
}

/// Added to every text weight except the brand face's.
#[derive(Debug)]
pub struct FontWeightOffset;
impl FontWeightOffset {
    pub const MIN: i16 = -70;
    pub const MAX: i16 = 130;
    pub const STEP: i16 = 10;
    pub const DEFAULT: i16 = 0;
}

/// File and diff wrap column in characters (original `LINE_WIDTH_COLUMNS`).
#[derive(Debug)]
pub struct LineWidth;
impl LineWidth {
    pub const MIN: u16 = 40;
    pub const MAX: u16 = 240;
    pub const DEFAULT: u16 = 120;

    pub fn is_valid(value: u16) -> bool {
        (Self::MIN..=Self::MAX).contains(&value)
    }
}

pub const FONT_FALLBACKS_MAX: usize = 8;
pub const FONT_FAMILY_NAME_MAX: usize = 128;
/// A bundled manifest id; a user theme's catalog id is `user.<id>`.
const THEME_ID_MAX: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// `theme` everywhere.
    #[default]
    Fixed,
    /// The pair slot matching the operating system's light/dark setting.
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemThemePair {
    pub light: String,
    pub dark: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontRole {
    /// Menus, chat and documents.
    Interface,
    /// The editor, diffs, code blocks and the terminal.
    Code,
}

/// A bundled face named by the role it ships for, or a family name the text system resolves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontFamily {
    Builtin(FontRole),
    Name(String),
}

/// One role's font: `family: None` is the role's bundled face; fallbacks draw what it lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontChoice {
    pub family: Option<FontFamily>,
    pub fallbacks: Vec<FontFamily>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearancePrefs {
    pub theme: String,
    pub theme_mode: ThemeMode,
    /// Kept after leaving system mode, so re-entering it restores the user's pair.
    pub system_theme_pair: Option<SystemThemePair>,
    pub interface_font: FontChoice,
    pub code_font: FontChoice,
    /// Terminal and code blocks, in px.
    pub code_font_size: u8,
    /// The terminal never draws ligatures.
    pub code_ligatures: bool,
    pub font_weight_offset: i16,
    /// The file and diff viewer wraps at this many characters.
    pub file_line_width: u16,
    /// Wraps sooner when the pane is narrower than `file_line_width` (else it scrolls).
    pub file_line_width_bounded: bool,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            theme: DEFAULT_THEME.to_owned(),
            theme_mode: ThemeMode::Fixed,
            system_theme_pair: None,
            interface_font: FontChoice::default(),
            code_font: FontChoice::default(),
            code_font_size: CodeFontSize::DEFAULT,
            code_ligatures: false,
            font_weight_offset: FontWeightOffset::DEFAULT,
            file_line_width: LineWidth::DEFAULT,
            file_line_width_bounded: true,
        }
    }
}

impl AppearancePrefs {
    /// Reads a stored value field by field (original `normalizeThemePreference` and
    /// `normalizeFontSettings`): an invalid field takes its default, the others are kept.
    pub fn normalize(value: &Value) -> Self {
        let default = Self::default();
        let pair = value
            .get("systemThemePair")
            .and_then(|pair| serde_json::from_value::<SystemThemePair>(pair.clone()).ok())
            .filter(|pair| is_theme_id(&pair.light) && is_theme_id(&pair.dark));
        let theme_mode = match value.get("themeMode").and_then(Value::as_str) {
            Some("fixed") => ThemeMode::Fixed,
            Some("system") if pair.is_some() => ThemeMode::System,
            _ => default.theme_mode,
        };
        Self {
            theme: value
                .get("theme")
                .and_then(Value::as_str)
                .filter(|id| is_theme_id(id))
                .map_or(default.theme, str::to_owned),
            theme_mode,
            system_theme_pair: pair,
            interface_font: font_choice(value.get("interfaceFont")),
            code_font: font_choice(value.get("codeFont")),
            code_font_size: value
                .get("codeFontSize")
                .and_then(Value::as_u64)
                .and_then(|size| u8::try_from(size).ok())
                .filter(|size| (CodeFontSize::MIN..=CodeFontSize::MAX).contains(size))
                .unwrap_or(default.code_font_size),
            code_ligatures: value
                .get("codeLigatures")
                .and_then(Value::as_bool)
                .unwrap_or(default.code_ligatures),
            font_weight_offset: value
                .get("fontWeightOffset")
                .and_then(Value::as_i64)
                .and_then(|offset| i16::try_from(offset).ok())
                .filter(|offset| is_weight_offset(*offset))
                .unwrap_or(default.font_weight_offset),
            file_line_width: value
                .get("fileLineWidth")
                .and_then(Value::as_u64)
                .and_then(|width| u16::try_from(width).ok())
                .filter(|width| LineWidth::is_valid(*width))
                .unwrap_or(default.file_line_width),
            file_line_width_bounded: value
                .get("fileLineWidthBounded")
                .and_then(Value::as_bool)
                .unwrap_or(default.file_line_width_bounded),
        }
    }

    pub fn font(&self, role: FontRole) -> &FontChoice {
        match role {
            FontRole::Interface => &self.interface_font,
            FontRole::Code => &self.code_font,
        }
    }

    pub fn font_mut(&mut self, role: FontRole) -> &mut FontChoice {
        match role {
            FontRole::Interface => &mut self.interface_font,
            FontRole::Code => &mut self.code_font,
        }
    }
}

pub fn is_weight_offset(offset: i16) -> bool {
    (FontWeightOffset::MIN..=FontWeightOffset::MAX).contains(&offset)
        && offset % FontWeightOffset::STEP == 0
}

/// A theme catalog id: a lowercase slug (original `THEME_ID`), optionally in the `user.`
/// namespace.
pub fn is_theme_id(id: &str) -> bool {
    let slug = id.strip_prefix("user.").unwrap_or(id);
    let mut chars = slug.chars();
    slug.len() <= THEME_ID_MAX
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// A family name the host can store and hand to the text system (original `isFontFamilyName`).
pub fn is_font_family_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= FONT_FAMILY_NAME_MAX
        && name.trim() == name
        && !name.contains(['"', '\\', ';', '{', '}'])
        && !name.chars().any(char::is_control)
}

fn font_family(value: &Value) -> Option<FontFamily> {
    let family: FontFamily = serde_json::from_value(value.clone()).ok()?;
    match &family {
        FontFamily::Builtin(_) => Some(family),
        FontFamily::Name(name) => is_font_family_name(name).then_some(family),
    }
}

/// The whole choice or the default (original `isFontChoice`): at most eight distinct valid
/// fallbacks.
fn font_choice(value: Option<&Value>) -> FontChoice {
    let parse = || {
        let value = value?;
        let family = match value.get("family")? {
            Value::Null => None,
            family => Some(font_family(family)?),
        };
        let raw = value.get("fallbacks")?.as_array()?;
        if raw.len() > FONT_FALLBACKS_MAX {
            return None;
        }
        let mut fallbacks = Vec::with_capacity(raw.len());
        for item in raw {
            let fallback = font_family(item)?;
            if fallbacks.contains(&fallback) {
                return None;
            }
            fallbacks.push(fallback);
        }
        Some(FontChoice { family, fallbacks })
    };
    parse().unwrap_or_default()
}

#[cfg(test)]
mod tests;
