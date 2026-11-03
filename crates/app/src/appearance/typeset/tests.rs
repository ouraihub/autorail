use gpui_kit::px;
use thinkrail_core::{AppearancePrefs, FontChoice, FontFamily, FontRole};
use thinkrail_ui::{Typography, fonts};

use super::*;

fn families() -> Families {
    Families::new(["Segoe UI", "Consolas", "Segoe UI Symbol"], ["Fira Code"])
}

#[test]
fn defaults_are_the_bundled_typography() {
    assert_eq!(
        typography(&AppearancePrefs::default(), &families()),
        Typography::default()
    );
}

#[test]
fn chosen_families_map_case_insensitively_and_missing_ones_fall_back() {
    let prefs = AppearancePrefs {
        interface_font: FontChoice {
            family: Some(FontFamily::Name("segoe ui".into())),
            fallbacks: vec![
                FontFamily::Name("Segoe UI Symbol".into()),
                FontFamily::Name("Fira Code".into()),
                FontFamily::Name("Nowhere Sans".into()),
            ],
        },
        code_font: FontChoice {
            family: Some(FontFamily::Name("Nowhere Mono".into())),
            fallbacks: vec![FontFamily::Builtin(FontRole::Code)],
        },
        code_font_size: 16,
        code_ligatures: true,
        font_weight_offset: 40,
        ..AppearancePrefs::default()
    };
    let t = typography(&prefs, &families());
    assert_eq!(t.interface_family, "segoe ui");
    assert_eq!(
        t.interface_fallbacks,
        ["Segoe UI Symbol"],
        "custom and missing fonts cannot be fallbacks"
    );
    assert_eq!(
        t.code_family,
        fonts::CODE,
        "a missing family uses the bundled face"
    );
    assert!(t.code_fallbacks.is_empty(), "the primary is not repeated");
    assert_eq!(t.code_size, px(16.));
    assert!(t.ligatures);
    assert!((t.weight_offset - 40.).abs() < f32::EPSILON);
    let code = role_font(&prefs.code_font, FontRole::Code, &families());
    assert_eq!(code.unavailable.as_deref(), Some("Nowhere Mono"));
}

#[test]
fn a_custom_family_is_drawable_as_the_primary() {
    let choice = FontChoice {
        family: Some(FontFamily::Name("Fira Code".into())),
        fallbacks: Vec::new(),
    };
    let code = role_font(&choice, FontRole::Code, &families());
    assert_eq!(code.family, "Fira Code");
    assert_eq!(code.unavailable, None);
    let builtin = FontChoice {
        family: Some(FontFamily::Builtin(FontRole::Code)),
        fallbacks: Vec::new(),
    };
    assert_eq!(
        role_font(&builtin, FontRole::Interface, &families()).family,
        fonts::CODE,
        "the other role's bundled face"
    );
}
