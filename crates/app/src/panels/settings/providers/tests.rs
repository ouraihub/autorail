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

#[gpui_kit::test]
fn pasted_key_is_saved_before_check_with_either_blur_order(cx: &mut TestAppContext) {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };
    use thinkrail_core::{Endpoint, Protocol};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}/v1", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for _ in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "no authenticated probe received");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(
                request
                    .lines()
                    .any(|line| line == "authorization: bearer fixture-key")
            );
            let body = r#"{"data":[{"id":"fixture-model"}]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let mut input = page.provider().unwrap().input.clone();
            input.name = "Key check fixture".into();
            input.endpoints = vec![Endpoint {
                protocol: Protocol::OpenaiChat,
                base_url: address,
            }];
            input.default_protocol = Protocol::OpenaiChat;
            let provider = page.service(cx).create(input).unwrap();
            page.providers.push(provider.clone());
            page.select(provider.id, cx);
            page.reset_key = false;
        })
    });
    for blur_first in [false, true] {
        cx.update_window(fixture.window, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.key_input
                    .update(cx, |input, cx| input.set_value("fixture-key", window, cx));
                if blur_first {
                    page.add_first_key(window, cx);
                }
                page.check(window, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                let checks = page.checks.as_ref().unwrap().as_ref().unwrap();
                assert_eq!(checks[0].models, Some(1));
                assert!(checks[0].error.is_none());
                assert!(page.key_input.read(cx).value().is_empty());
                let provider = page
                    .service(cx)
                    .provider(page.selected.as_ref().unwrap())
                    .unwrap();
                assert_eq!(provider.keys.len(), 1);
                let provider = page
                    .service(cx)
                    .delete_key(&provider.id, &provider.keys[0].id)
                    .unwrap();
                page.stored(provider, cx);
                assert!(
                    page.checks.is_none(),
                    "deleted key must invalidate its check"
                );
            })
        });
        cx.run_until_parked();
    }
    worker.join().unwrap();
}

#[gpui_kit::test]
fn check_save_failure_keeps_pasted_key_and_does_not_probe(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    cx.update_window(fixture.window, |_, window, cx| {
        page.update(cx, |page, cx| {
            let mut input = page.provider().unwrap().input.clone();
            input.name = "Deleted check fixture".into();
            let provider = page.service(cx).create(input).unwrap();
            page.providers.push(provider.clone());
            page.select(provider.id.clone(), cx);
            page.reset_key = false;
            page.service(cx)
                .delete(&provider.id, provider.revision)
                .unwrap();
            page.key_input
                .update(cx, |input, cx| input.set_value("retry-key", window, cx));
            page.check(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        assert_eq!(page.key_input.read(cx).value().as_str(), "retry-key");
        assert!(page.checks.is_none());
        assert!(!page.key_saving);
        assert!(!page.check_after_key_save);
        assert!(page.state.read(cx).notice.is_some());
    });
}
