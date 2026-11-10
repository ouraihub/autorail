//! Manager writes preserve drafts on failure and support optional labels and label-only edits.

use gpui_kit::TestAppContext;

use super::*;
use crate::testing;

fn manager(
    cx: &mut TestAppContext,
) -> (testing::Fixture, Entity<ProvidersPage>, Entity<KeysDialog>) {
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    let dialog = cx
        .update_window(fixture.window, |_, window, cx| {
            let provider = page.read(cx).provider().unwrap().id.clone();
            cx.new(|cx| KeysDialog {
                page: page.clone(),
                provider,
                label: cx.new(|cx| InputState::new(window, cx)),
                value: cx.new(|cx| InputState::new(window, cx).masked(true)),
                editing: None,
                error: None,
                saving: false,
                generation: 0,
                _subscriptions: Vec::new(),
            })
        })
        .unwrap();
    (fixture, page, dialog)
}

#[gpui_kit::test]
fn manager_add_clears_only_after_success_and_label_only_edit_keeps_secret(cx: &mut TestAppContext) {
    let (fixture, page, dialog) = manager(cx);
    cx.update_window(fixture.window, |_, window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog
                .value
                .update(cx, |i, cx| i.set_value("manager-secret", window, cx));
            dialog.submit(window, cx);
            dialog.submit(window, cx);
            assert!(dialog.saving);
            assert_eq!(dialog.value.read(cx).value().as_str(), "manager-secret");
        });
    })
    .unwrap();
    cx.run_until_parked();
    let key = cx.update(|cx| {
        let dialog = dialog.read(cx);
        assert!(!dialog.saving);
        assert!(dialog.error.is_none());
        assert!(dialog.value.read(cx).value().is_empty());
        let keys = dialog.keys(cx);
        assert_eq!(keys.len(), 1);
        assert!(keys[0].label.is_none());
        keys[0].id.clone()
    });
    cx.update_window(fixture.window, |_, window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.editing = Some(key.clone());
            dialog
                .label
                .update(cx, |i, cx| i.set_value("Renamed only", window, cx));
            dialog.submit(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let dialog = dialog.read(cx);
        assert!(dialog.error.is_none());
        assert!(dialog.editing.is_none());
        assert!(dialog.label.read(cx).value().is_empty());
        assert_eq!(dialog.keys(cx)[0].label.as_deref(), Some("Renamed only"));
        assert_eq!(
            page.read(cx).service(cx).reveal_key(&key).unwrap().expose(),
            "manager-secret"
        );
    });
}

#[gpui_kit::test]
fn manager_failure_retains_label_secret_and_edit_target(cx: &mut TestAppContext) {
    let (fixture, page, dialog) = manager(cx);
    let key = cx.update(|cx| {
        let page = page.read(cx);
        let provider = page.provider().unwrap();
        let stored = page
            .service(cx)
            .add_key(
                &provider.id,
                ProviderKeyInput {
                    label: Some("Original".into()),
                    key: Secret::new("original-secret"),
                    enabled: true,
                },
            )
            .unwrap();
        let key = stored.keys[0].id.clone();
        page.service(cx).delete_key(&stored.id, &key).unwrap();
        key
    });
    cx.update_window(fixture.window, |_, window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.editing = Some(key.clone());
            dialog
                .label
                .update(cx, |i, cx| i.set_value("Retained label", window, cx));
            dialog
                .value
                .update(cx, |i, cx| i.set_value("retained-secret", window, cx));
            dialog.submit(window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let dialog = dialog.read(cx);
        assert!(!dialog.saving);
        assert!(dialog.error.is_some());
        assert_eq!(dialog.editing.as_ref(), Some(&key));
        assert_eq!(dialog.label.read(cx).value().as_str(), "Retained label");
        assert_eq!(dialog.value.read(cx).value().as_str(), "retained-secret");
    });
}
