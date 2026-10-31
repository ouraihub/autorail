use super::*;

#[test]
fn secret_debug_hides_value() {
    let secret = Secret::new("sk-test-a");
    assert_eq!(format!("{secret:?}"), "Secret(***)");
    assert_eq!(secret.expose(), "sk-test-a");

    let input = ProviderKeyInput { label: Some("main".into()), key: secret.clone(), enabled: true };
    let update = ProviderKeyUpdate { label: None, key: Some(secret.clone()), enabled: false };
    let selected = SelectedKey { id: ProviderKeyId::from("k1"), label: None, value: secret };
    for text in [format!("{input:?}"), format!("{update:?}"), format!("{selected:?}")] {
        assert!(!text.contains("sk-test-a"), "{text}");
    }
}
