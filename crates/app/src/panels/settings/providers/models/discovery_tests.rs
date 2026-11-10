//! Discovery opens the real picker; an upstream failure leaves synchronization retryable.

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    thread,
};

use gpui_kit::{AppContext as _, ElementId, TestAppContext, test::TestWindowExt as _};
use thinkrail_core::{Endpoint, Protocol};

use super::super::ProvidersPage;
use crate::testing;

fn server(responses: Vec<(&'static str, &'static str)>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/v1", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
            }
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (address, worker)
}

fn discover(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ProvidersPage>,
) {
    // Mount the real component Root plugin before asking it to host a dialog.
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.sync_models(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    // Presence mounts the dialog on its first frame, then exposes its animated contents.
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn configure(cx: &mut TestAppContext, page: &gpui_kit::Entity<ProvidersPage>, address: String) {
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let mut input = page.provider().unwrap().input.clone();
            input.name = "Discovery fixture".into();
            input.endpoints = vec![Endpoint {
                protocol: Protocol::OpenaiChat,
                base_url: address,
            }];
            input.default_protocol = Protocol::OpenaiChat;
            input.models.clear();
            let provider = page.service(cx).create(input).unwrap();
            page.providers.push(provider.clone());
            page.select(provider.id, cx);
        });
    });
}

#[gpui_kit::test]
fn synchronization_opens_picker_and_picked_model_is_persisted(cx: &mut TestAppContext) {
    let (address, worker) = server(vec![("200 OK", r#"{"data":[{"id":"discovered-model"}]}"#)]);
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    configure(cx, &page, address);
    discover(cx, fixture.window, &page);
    worker.join().unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(ElementId::from("pick-discovered-model"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let page = page.read(cx);
        let provider = page
            .service(cx)
            .provider(page.selected.as_ref().unwrap())
            .unwrap();
        assert!(
            provider
                .input
                .models
                .iter()
                .any(|model| model.model_id == "discovered-model"
                    && model.protocols.contains(&Protocol::OpenaiChat))
        );
        assert!(!page.models.syncing);
        assert!(page.models.sync_error.is_none());
    });
}

#[gpui_kit::test]
fn synchronization_failure_retains_error_and_retry_opens_picker(cx: &mut TestAppContext) {
    let (address, worker) = server(vec![
        (
            "503 Service Unavailable",
            r#"{"error":{"message":"fixture unavailable"}}"#,
        ),
        ("200 OK", r#"{"data":[{"id":"retry-model"}]}"#),
    ]);
    let (fixture, page) = testing::open(cx, ProvidersPage::new);
    cx.run_until_parked();
    configure(cx, &page, address);
    discover(cx, fixture.window, &page);
    cx.update(|cx| {
        let page = page.read(cx);
        assert!(!page.models.syncing);
        assert!(page.models.sync_error.is_some());
        assert!(
            page.state
                .read(cx)
                .notice
                .as_ref()
                .unwrap()
                .trace_id
                .is_some()
        );
    });
    discover(cx, fixture.window, &page);
    worker.join().unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .try_find(ElementId::from("pick-retry-model"))
                .is_some()
        );
    })
    .unwrap();
    cx.update(|cx| {
        assert!(!page.read(cx).models.syncing);
        assert!(page.read(cx).models.sync_error.is_none());
    });
}
