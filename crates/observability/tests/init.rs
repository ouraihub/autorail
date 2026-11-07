//! The real subscriber's lines read back as the log page sees them. One test per binary: the
//! global subscriber can be installed once per process.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test helpers fail loudly"
)]

use std::{sync::mpsc, thread};

use thinkrail_core::LogLevel;
use thinkrail_observability::{TraceContext, init, operation};

#[test]
fn initialized_subscriber_correlates_threads_and_lifecycle_fields() {
    let data = tempfile::tempdir().unwrap();
    let handle = init(data.path()).unwrap();
    handle.set_level(LogLevel::Debug).unwrap();
    let (first, first_span) = TraceContext::root("test.first").unwrap();
    let (second, second_span) = TraceContext::root("test.second").unwrap();
    assert_ne!(first.trace_id(), second.trace_id());
    let (sender, receiver) = mpsc::channel();

    first_span.in_scope(|| {
        tracing::info!(target: "thinkrail_test", "first root");
        let guard = operation(&first, "test.channel", "thinkrail_test");
        guard.fail("TEST_FAILURE");
        let worker_context = first.clone();
        let worker_span = first_span.clone();
        let sender = sender.clone();
        thread::spawn(move || {
            let _entered = worker_span.enter();
            let child = operation(&worker_context, "test.thread", "thinkrail_test");
            tracing::info!(target: "thinkrail_test", trace_id = %worker_context.trace_id(), "thread first");
            child.cancel("TEST_CANCEL");
            sender.send(()).unwrap();
        });
    });
    second_span.in_scope(|| {
        let guard = operation(&second, "test.second", "thinkrail_test");
        guard.finish("ok");
        tracing::info!(target: "thinkrail_test", "second root");
    });
    receiver.recv().unwrap();
    let dir = handle.dir().to_owned();
    drop(handle);

    let entries = thinkrail_observability::tail(&dir, 100).unwrap();
    let first_entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry.trace_id.as_deref() == Some(first.trace_id()))
        .collect();
    let second_entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry.trace_id.as_deref() == Some(second.trace_id()))
        .collect();
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.target == "thinkrail_test")
    );
    assert!(
        second_entries
            .iter()
            .any(|entry| entry.target == "thinkrail_test")
    );
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.operation.as_deref() == Some("test.channel"))
    );
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.operation.as_deref() == Some("test.thread"))
    );
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.raw.contains("result=\"failed\""))
    );
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.raw.contains("result=\"cancelled\""))
    );
    assert!(
        second_entries
            .iter()
            .any(|entry| entry.operation.as_deref() == Some("test.second"))
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.message == "hidden at info")
    );
}
