use thinkrail_core::{AppearancePrefs, SystemThemePair, ThemeMode};
use thinkrail_ui::{Appearance, Contrast, ThemeCatalog};

use super::*;

fn catalog() -> &'static ThemeCatalog {
    ThemeCatalog::bundled().unwrap()
}

fn fixed(id: &str) -> AppearancePrefs {
    AppearancePrefs {
        theme: id.into(),
        ..AppearancePrefs::default()
    }
}

fn pair(light: &str, dark: &str) -> SystemThemePair {
    SystemThemePair {
        light: light.into(),
        dark: dark.into(),
    }
}

#[test]
fn light_and_dark_stay_in_the_theme_family() {
    let to_light = select_mode(
        catalog(),
        &fixed("minimal-dark"),
        Appearance::Dark,
        ModeChoice::Light,
    )
    .unwrap();
    assert_eq!(to_light.theme, "minimal-light");
    assert_eq!(to_light.theme_mode, ThemeMode::Fixed);
    let to_dark = select_mode(
        catalog(),
        &fixed("high-contrast-light"),
        Appearance::Light,
        ModeChoice::Dark,
    )
    .unwrap();
    assert_eq!(to_dark.theme, "high-contrast-dark");
    assert!(
        select_mode(
            catalog(),
            &fixed("dark"),
            Appearance::Dark,
            ModeChoice::Dark
        )
        .is_none()
    );
}

#[test]
fn entering_system_mode_derives_the_pair_once_and_reuses_it() {
    let system = select_mode(
        catalog(),
        &fixed("minimal-dark"),
        Appearance::Light,
        ModeChoice::System,
    )
    .unwrap();
    assert_eq!(system.theme_mode, ThemeMode::System);
    assert_eq!(
        system.system_theme_pair,
        Some(pair("light", "minimal-dark"))
    );
    let view = ThemeView::new(catalog(), &system, Appearance::Light);
    assert_eq!(view.current.theme.id, "light");
    assert_eq!(view.mode(), ModeChoice::System);

    let back = select_mode(catalog(), &system, Appearance::Light, ModeChoice::Dark).unwrap();
    assert_eq!(
        back.theme, "minimal-dark",
        "dark picks the pair's dark slot"
    );
    assert_eq!(
        back.system_theme_pair,
        Some(pair("light", "minimal-dark")),
        "pair is kept"
    );
    let again = select_mode(catalog(), &back, Appearance::Dark, ModeChoice::System).unwrap();
    assert_eq!(again.system_theme_pair, Some(pair("light", "minimal-dark")));
}

#[test]
fn high_contrast_derives_a_high_contrast_pair() {
    let system = select_mode(
        catalog(),
        &fixed("high-contrast-dark"),
        Appearance::Dark,
        ModeChoice::System,
    )
    .unwrap();
    assert_eq!(
        system.system_theme_pair,
        Some(pair("high-contrast-light", "high-contrast-dark"))
    );
}

#[test]
fn contrast_switches_the_fixed_theme_or_both_slots() {
    let high =
        select_contrast(catalog(), &fixed("light"), Appearance::Dark, Contrast::High).unwrap();
    assert_eq!(high.theme, "high-contrast-light");
    let normal = select_contrast(
        catalog(),
        &fixed("high-contrast-dark"),
        Appearance::Dark,
        Contrast::Normal,
    )
    .unwrap();
    assert_eq!(normal.theme, "dark", "a family-less counterpart");
    assert!(
        select_contrast(
            catalog(),
            &fixed("dark"),
            Appearance::Dark,
            Contrast::Normal
        )
        .is_none()
    );

    let system = AppearancePrefs {
        theme_mode: ThemeMode::System,
        system_theme_pair: Some(pair("minimal-light", "dark")),
        ..fixed("dark")
    };
    let high = select_contrast(catalog(), &system, Appearance::Dark, Contrast::High).unwrap();
    assert_eq!(
        high.system_theme_pair,
        Some(pair("high-contrast-light", "high-contrast-dark"))
    );
    assert_eq!(high.theme_mode, ThemeMode::System);
}

#[test]
fn a_system_slot_is_set_on_its_own() {
    let system = AppearancePrefs {
        theme_mode: ThemeMode::System,
        system_theme_pair: Some(pair("light", "dark")),
        ..fixed("dark")
    };
    let next = select_system(
        catalog(),
        &system,
        Appearance::Dark,
        Appearance::Dark,
        "minimal-dark",
    )
    .unwrap();
    assert_eq!(next.system_theme_pair, Some(pair("light", "minimal-dark")));
    assert!(
        select_system(
            catalog(),
            &system,
            Appearance::Dark,
            Appearance::Light,
            "light"
        )
        .is_none()
    );
}

#[test]
fn a_missing_theme_is_reported_as_a_fallback() {
    let view = ThemeView::new(catalog(), &fixed("user.gone"), Appearance::Dark);
    assert!(view.fixed.fallback);
    assert_eq!(view.current.theme.id, "dark");
    let system = AppearancePrefs {
        theme_mode: ThemeMode::System,
        system_theme_pair: Some(pair("dark", "dark")),
        ..fixed("dark")
    };
    let view = ThemeView::new(catalog(), &system, Appearance::Light);
    assert!(
        view.light.fallback,
        "a dark theme in the light slot falls back"
    );
    assert_eq!(view.current.theme.id, "light");
}

#[test]
fn fixed_selection_leaves_system_mode() {
    let system = AppearancePrefs {
        theme_mode: ThemeMode::System,
        system_theme_pair: Some(pair("light", "dark")),
        ..fixed("dark")
    };
    let next = select_fixed(&system, "minimal-light").unwrap();
    assert_eq!(next.theme, "minimal-light");
    assert_eq!(next.theme_mode, ThemeMode::Fixed);
    assert!(select_fixed(&next, "minimal-light").is_none());
}
