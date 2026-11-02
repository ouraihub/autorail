//! ThinkRail colors: the original bundled palettes (`themes/*.theme.json`, copied verbatim) and the
//! semantic roles the original derives from them (`apps/web/src/styles/colors.json`). Views paint
//! with [`Roles`] only; a palette value is never used directly.

use std::sync::LazyLock;

use gpui_kit::{App, Global, Rgba, Window, component::Theme as KitTheme};
use serde::Deserialize;

const DARK: &str = include_str!("../themes/dark.theme.json");
const LIGHT: &str = include_str!("../themes/light.theme.json");

/// The `colors.json` alpha ladder, as `color-mix(in srgb, X n%, transparent)` = alpha × n%.
mod alpha {
    pub const SUBTLE: f32 = 0.10;
    pub const WASH: f32 = 0.12;
    pub const SOFT: f32 = 0.20;
    pub const MUTED: f32 = 0.40;
    pub const STRONG: f32 = 0.60;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Dark,
    Light,
}

/// A bundled theme's `colors` block.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_solid: Rgba,
    pub on_accent: Rgba,
    pub bubble_accent: Rgba,
    pub background: Rgba,
    pub header: Rgba,
    pub content: Rgba,
    pub sidebar: Rgba,
    pub input: Rgba,
    pub elevated: Rgba,
    pub hover: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub hint: Rgba,
    pub selection: Rgba,
    pub selection_foreground: Option<Rgba>,
    pub editor_selection: Rgba,
    pub editor_selection_foreground: Option<Rgba>,
    pub info: Rgba,
    pub success: Rgba,
    pub danger: Rgba,
    pub danger_hover: Rgba,
    pub on_danger: Rgba,
    pub warning: Rgba,
}

/// A bundled theme's `ansi` block, in SGR order 0–15.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ansi {
    pub black: Rgba,
    pub red: Rgba,
    pub green: Rgba,
    pub yellow: Rgba,
    pub blue: Rgba,
    pub magenta: Rgba,
    pub cyan: Rgba,
    pub white: Rgba,
    pub bright_black: Rgba,
    pub bright_red: Rgba,
    pub bright_green: Rgba,
    pub bright_yellow: Rgba,
    pub bright_blue: Rgba,
    pub bright_magenta: Rgba,
    pub bright_cyan: Rgba,
    pub bright_white: Rgba,
}

impl Ansi {
    pub fn colors(&self) -> [Rgba; 16] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
            self.bright_black,
            self.bright_red,
            self.bright_green,
            self.bright_yellow,
            self.bright_blue,
            self.bright_magenta,
            self.bright_cyan,
            self.bright_white,
        ]
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    id: String,
    appearance: Appearance,
    colors: Palette,
    ansi: Ansi,
}

/// The semantic roles of `colors.json`, resolved for one palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roles {
    pub text_default: Rgba,
    pub text_muted: Rgba,
    pub text_subtle: Rgba,
    pub text_disabled: Rgba,
    pub text_on_primary: Rgba,
    pub text_link: Rgba,
    pub container_workspace_bg: Rgba,
    pub container_sidebar_bg: Rgba,
    pub container_terminal_bg: Rgba,
    pub container_header_bg: Rgba,
    pub container_content_bg: Rgba,
    pub container_elevated_bg: Rgba,
    pub control_bg: Rgba,
    pub control_bg_hovered: Rgba,
    pub control_bg_selected: Rgba,
    pub control_primary_bg: Rgba,
    pub control_primary_bg_hovered: Rgba,
    pub control_primary_text: Rgba,
    pub control_border_default: Rgba,
    pub control_border_active: Rgba,
    pub control_disabled_bg: Rgba,
    pub control_disabled_text: Rgba,
    pub control_disabled_border: Rgba,
    pub control_primary_disabled_bg: Rgba,
    pub control_primary_disabled_text: Rgba,
    pub control_destructive_bg: Rgba,
    pub control_soft_primary_bg_hovered: Rgba,
    pub control_destructive_bg_hovered: Rgba,
    pub control_destructive_text: Rgba,
    pub control_destructive_disabled_bg: Rgba,
    pub control_destructive_disabled_text: Rgba,
    pub border_default: Rgba,
    pub border_muted: Rgba,
    pub primary: Rgba,
    pub primary_subtle: Rgba,
    pub primary_soft: Rgba,
    pub primary_muted: Rgba,
    pub on_primary_soft: Rgba,
    pub feedback_info: Rgba,
    pub feedback_info_subtle: Rgba,
    pub feedback_info_muted: Rgba,
    pub feedback_success: Rgba,
    pub feedback_success_subtle: Rgba,
    pub feedback_success_muted: Rgba,
    pub feedback_error: Rgba,
    pub feedback_error_subtle: Rgba,
    pub feedback_error_muted: Rgba,
    pub feedback_warning: Rgba,
    pub feedback_warning_subtle: Rgba,
    pub feedback_warning_muted: Rgba,
    pub bubble_user_bg: Rgba,
    pub bubble_user_border: Rgba,
    /// `None` keeps the text's own color (`inherit`).
    pub selection_text: Option<Rgba>,
    pub selection_bg: Rgba,
    pub selection_bg_muted: Rgba,
    pub editor_selection_bg: Rgba,
    pub editor_selection_text: Option<Rgba>,
    pub editor_selection_highlight_bg: Rgba,
    pub editor_find_match_bg: Rgba,
    pub widget_shadow: Rgba,
    pub sunken: Rgba,
    pub overlay: Rgba,
}

impl Roles {
    pub fn derive(p: &Palette, appearance: Appearance) -> Self {
        let mix = |c: Rgba, f: f32| c.opacity(f);
        let black = |a: f32| Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a,
        };
        Self {
            text_default: p.text,
            text_muted: p.muted,
            text_subtle: p.hint,
            text_disabled: mix(p.text, alpha::STRONG),
            text_on_primary: p.on_accent,
            text_link: p.info,
            container_workspace_bg: p.background,
            container_sidebar_bg: p.sidebar,
            container_terminal_bg: p.sidebar,
            container_header_bg: p.header,
            container_content_bg: p.content,
            container_elevated_bg: p.elevated,
            control_bg: p.input,
            control_bg_hovered: p.hover,
            control_bg_selected: p.hover,
            control_primary_bg: p.accent_solid,
            control_primary_bg_hovered: p.accent_hover,
            control_primary_text: p.on_accent,
            control_border_default: p.border,
            control_border_active: p.border_strong,
            control_disabled_bg: mix(p.input, alpha::STRONG),
            control_disabled_text: mix(p.text, alpha::STRONG),
            control_disabled_border: mix(p.border, alpha::STRONG),
            control_primary_disabled_bg: mix(p.accent, alpha::STRONG),
            control_primary_disabled_text: mix(p.on_accent, alpha::STRONG),
            control_destructive_bg: p.danger,
            control_soft_primary_bg_hovered: mix(p.accent, alpha::WASH),
            control_destructive_bg_hovered: p.danger_hover,
            control_destructive_text: p.on_danger,
            control_destructive_disabled_bg: mix(p.danger, alpha::STRONG),
            control_destructive_disabled_text: mix(p.on_danger, alpha::STRONG),
            border_default: p.border_strong,
            border_muted: p.border,
            primary: p.accent,
            primary_subtle: mix(p.accent, alpha::SUBTLE),
            primary_soft: mix(p.accent, alpha::SOFT),
            primary_muted: mix(p.accent, alpha::MUTED),
            on_primary_soft: mix(p.on_accent, alpha::SOFT),
            feedback_info: p.info,
            feedback_info_subtle: mix(p.info, alpha::WASH),
            feedback_info_muted: mix(p.info, alpha::MUTED),
            feedback_success: p.success,
            feedback_success_subtle: mix(p.success, alpha::WASH),
            feedback_success_muted: mix(p.success, alpha::MUTED),
            feedback_error: p.danger,
            feedback_error_subtle: mix(p.danger, alpha::WASH),
            feedback_error_muted: mix(p.danger, alpha::MUTED),
            feedback_warning: p.warning,
            feedback_warning_subtle: mix(p.warning, alpha::WASH),
            feedback_warning_muted: mix(p.warning, alpha::MUTED),
            bubble_user_bg: mix(p.bubble_accent, alpha::SUBTLE),
            bubble_user_border: mix(p.bubble_accent, alpha::MUTED),
            selection_text: p.selection_foreground,
            selection_bg: p.selection,
            selection_bg_muted: mix(p.selection, alpha::MUTED),
            editor_selection_bg: p.editor_selection,
            editor_selection_text: p.editor_selection_foreground,
            editor_selection_highlight_bg: mix(p.editor_selection, alpha::SOFT),
            editor_find_match_bg: mix(p.warning, alpha::MUTED),
            widget_shadow: mix(p.text, alpha::SOFT),
            sunken: black(if appearance == Appearance::Dark {
                0.12
            } else {
                0.05
            }),
            overlay: black(0.5),
        }
    }
}

/// One loaded bundled theme.
#[derive(Debug, Clone)]
pub struct ThinkrailTheme {
    pub id: String,
    pub appearance: Appearance,
    pub palette: Palette,
    pub ansi: Ansi,
    pub roles: Roles,
}

impl Global for ThinkrailTheme {}

impl ThinkrailTheme {
    fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let manifest: Manifest = serde_json::from_str(json)?;
        let roles = Roles::derive(&manifest.colors, manifest.appearance);
        Ok(Self {
            id: manifest.id,
            appearance: manifest.appearance,
            palette: manifest.colors,
            ansi: manifest.ansi,
            roles,
        })
    }

    /// The bundled dark or light theme. The manifests are compiled in and pinned by tests.
    pub fn bundled(appearance: Appearance) -> Result<&'static Self, &'static str> {
        type Loaded = Result<ThinkrailTheme, String>;
        static THEMES: LazyLock<[Loaded; 2]> = LazyLock::new(|| {
            let load =
                |json| ThinkrailTheme::parse(json).map_err(|e| format!("内置主题无法解析：{e}"));
            [load(DARK), load(LIGHT)]
        });
        let index = match appearance {
            Appearance::Dark => 0,
            Appearance::Light => 1,
        };
        THEMES[index].as_ref().map_err(String::as_str)
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Installs the theme as the app global and maps it onto the component library's colors.
    pub fn install(
        appearance: Appearance,
        window: Option<&mut Window>,
        cx: &mut App,
    ) -> Result<(), &'static str> {
        let theme = Self::bundled(appearance)?.clone();
        let mode = match appearance {
            Appearance::Dark => gpui_kit::component::ThemeMode::Dark,
            Appearance::Light => gpui_kit::component::ThemeMode::Light,
        };
        KitTheme::change(mode, window, cx);
        let roles = theme.roles;
        KitTheme::update(cx, |kit| {
            crate::theme_map::apply(&mut kit.colors, &roles);
            kit.font_family = crate::fonts::INTERFACE.into();
            kit.mono_font_family = crate::fonts::CODE.into();
            kit.font_size = crate::metrics::font::UI;
            kit.mono_font_size = crate::metrics::font::CODE;
            kit.radius = crate::metrics::radius::SM;
            kit.radius_lg = crate::metrics::radius::LG;
        });
        cx.set_global(theme);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
