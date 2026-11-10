//! Observable key draft and independent-enable regressions over isolated real services.

use gpui_kit::{AppContext as _, TestAppContext};

use super::ProvidersPage;
use crate::testing;

#[gpui_kit::test]
fn first_key_failure_retains_draft_and_retry_can_submit(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    cx.update_window(fixture.window, |_, window, cx| {
        page.update(cx, |page, cx| {
            let mut input = page.provider().unwrap().input.clone();
            input.name = "Failure fixture".into();
            let provider = page.service(cx).create(input).unwrap();
            page.providers.push(provider.clone());
            page.select(provider.id.clone(), cx);
            page.reset_key = false;
            page.service(cx)
                .delete(&provider.id, provider.revision)
                .unwrap();
            page.key_input
                .update(cx, |input, cx| input.set_value("draft-key", window, cx));
            page.add_first_key(window, cx);
            page.add_first_key(window, cx);
            assert!(page.key_saving);
            assert_eq!(page.key_input.read(cx).value().as_str(), "draft-key");
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!page.read(cx).key_saving);
        assert_eq!(
            page.read(cx).key_input.read(cx).value().as_str(),
            "draft-key"
        );
        assert!(page.read(cx).state.read(cx).notice.is_some());
    });
}

#[gpui_kit::test]
fn first_key_captures_provider_and_never_clears_later_input(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    let (original, next) = cx.update(|cx| {
        let page = page.read(cx);
        (
            page.provider().unwrap().id.clone(),
            page.providers
                .iter()
                .find(|p| Some(&p.id) != page.selected.as_ref())
                .unwrap()
                .id
                .clone(),
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.key_input
                .update(cx, |input, cx| input.set_value("original-key", window, cx));
            page.add_first_key(window, cx);
            page.add_first_key(window, cx);
            page.select(next.clone(), cx);
            page.reset_key = false;
            page.key_input
                .update(cx, |input, cx| input.set_value("later-key", window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        let service = page.service(cx);
        assert_eq!(
            service.provider(&original).unwrap().keys.len(),
            1,
            "Enter plus blur must not duplicate the request"
        );
        assert!(service.provider(&next).unwrap().keys.is_empty());
        assert_eq!(page.key_input.read(cx).value().as_str(), "later-key");
    });
}

#[gpui_kit::test]
fn independent_enable_never_saves_url_or_model_draft(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    let saved = cx.update(|cx| page.read(cx).provider().unwrap().clone());
    cx.update_window(fixture.window, |_, _, cx| {
        page.update(cx, |page, cx| {
            let draft = page.draft.as_mut().unwrap();
            draft.endpoints[0].base_url = "https://unsaved.example/v1".into();
            draft.models.clear();
            page.dirty = true;
            page.set_enabled(!saved.input.enabled, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        let stored = page.service(cx).provider(&saved.id).unwrap();
        assert_eq!(stored.input.enabled, !saved.input.enabled);
        assert_eq!(stored.input.endpoints, saved.input.endpoints);
        assert_eq!(stored.input.models, saved.input.models);
        assert_eq!(
            page.draft.as_ref().unwrap().endpoints[0].base_url,
            "https://unsaved.example/v1"
        );
    });
}
