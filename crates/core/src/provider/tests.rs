use super::*;

#[test]
fn secret_debug_hides_value() {
    let secret = Secret::new("sk-test-a");
    assert_eq!(format!("{secret:?}"), "Secret(***)");
    assert_eq!(secret.expose(), "sk-test-a");

    let input = ProviderKeyInput {
        label: Some("main".into()),
        key: secret.clone(),
        enabled: true,
    };
    let update = ProviderKeyUpdate {
        label: None,
        key: Some(secret.clone()),
        enabled: false,
    };
    let selected = SelectedKey {
        id: ProviderKeyId::from("k1"),
        label: None,
        value: secret,
    };
    for text in [
        format!("{input:?}"),
        format!("{update:?}"),
        format!("{selected:?}"),
    ] {
        assert!(!text.contains("sk-test-a"), "{text}");
    }
}

#[test]
fn clamp_effort_takes_the_highest_declared_level_not_above_the_saved_one() {
    use ThinkingLevel::*;
    let declared = [Low, Medium, High];
    assert_eq!(clamp_effort(None, &declared), None);
    assert_eq!(clamp_effort(Some(High), &[]), None);
    assert_eq!(clamp_effort(Some(Medium), &declared), Some(Medium));
    assert_eq!(clamp_effort(Some(Max), &declared), Some(High));
    assert_eq!(clamp_effort(Some(Minimal), &declared), None);
    assert_eq!(clamp_effort(Some(Off), &[Off, High]), Some(Off));
}
