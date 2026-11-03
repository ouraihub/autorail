use serde_json::json;

use super::*;

#[test]
fn a_stored_value_round_trips() {
    let prefs = AppearancePrefs {
        theme: "user.ocean".into(),
        theme_mode: ThemeMode::System,
        system_theme_pair: Some(SystemThemePair {
            light: "light".into(),
            dark: "user.ocean".into(),
        }),
        interface_font: FontChoice {
            family: Some(FontFamily::Name("Segoe UI".into())),
            fallbacks: vec![FontFamily::Builtin(FontRole::Interface)],
        },
        code_font: FontChoice {
            family: Some(FontFamily::Builtin(FontRole::Code)),
            fallbacks: vec![FontFamily::Name("Symbols Nerd Font".into())],
        },
        code_font_size: 16,
        code_ligatures: true,
        font_weight_offset: -30,
    };
    let stored = serde_json::to_value(&prefs).unwrap();
    assert_eq!(AppearancePrefs::normalize(&stored), prefs);
}

#[test]
fn every_invalid_field_falls_back_alone() {
    let stored = json!({
        "theme": "Not A Slug",
        "themeMode": "system",
        "interfaceFont": {"family": {"name": " padded"}, "fallbacks": []},
        "codeFont": {"family": null, "fallbacks": [{"name": "A"}, {"name": "A"}]},
        "codeFontSize": 25,
        "codeLigatures": "yes",
        "fontWeightOffset": 15,
    });
    let prefs = AppearancePrefs::normalize(&stored);
    assert_eq!(prefs, AppearancePrefs::default());

    let partly = json!({"theme": "light", "codeFontSize": 9, "fontWeightOffset": 130});
    let prefs = AppearancePrefs::normalize(&partly);
    assert_eq!(prefs.theme, "light");
    assert_eq!(prefs.code_font_size, CodeFontSize::DEFAULT);
    assert_eq!(prefs.font_weight_offset, 130);
}

#[test]
fn system_mode_needs_a_valid_pair() {
    let without = json!({"theme": "light", "themeMode": "system"});
    assert_eq!(
        AppearancePrefs::normalize(&without).theme_mode,
        ThemeMode::Fixed
    );
    let bad_pair =
        json!({"themeMode": "system", "systemThemePair": {"light": "light", "dark": "BAD"}});
    let prefs = AppearancePrefs::normalize(&bad_pair);
    assert_eq!(prefs.theme_mode, ThemeMode::Fixed);
    assert_eq!(prefs.system_theme_pair, None);
}

#[test]
fn bounds_follow_the_original_contract() {
    assert!(is_theme_id("high-contrast-dark"));
    assert!(is_theme_id("user.my_theme.v2"));
    assert!(!is_theme_id(""));
    assert!(!is_theme_id("-dark"));
    assert!(!is_theme_id(&"a".repeat(65)));
    assert!(is_weight_offset(-70) && is_weight_offset(0) && is_weight_offset(130));
    assert!(!is_weight_offset(-80) && !is_weight_offset(140) && !is_weight_offset(5));
    assert!(is_font_family_name("JetBrains Mono"));
    assert!(!is_font_family_name("a;b") && !is_font_family_name("tab\there"));
    assert!(!is_font_family_name(&"x".repeat(FONT_FAMILY_NAME_MAX + 1)));
    let nine: Vec<_> = (0..9).map(|i| json!({"name": format!("F{i}")})).collect();
    let too_many = json!({"codeFont": {"family": null, "fallbacks": nine}});
    assert_eq!(
        AppearancePrefs::normalize(&too_many).code_font,
        FontChoice::default()
    );
}
