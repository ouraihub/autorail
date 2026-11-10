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
    let (first, first_root) = TraceContext::root("test.first", "ui");
    let (second, second_root) = TraceContext::root("test.second", "ui");
    assert_ne!(first.trace_id(), second.trace_id());
    let (sender, receiver) = mpsc::channel();

    first_root.span().in_scope(|| {
        tracing::info!(target: "thinkrail_test", "first root");
        operation(&first, "test.channel", "service").fail("TEST_FAILURE");
        let worker_context = first.clone();
        let worker_span = first_root.span().clone();
        let sender = sender.clone();
        thread::spawn(move || {
            let _entered = worker_span.enter();
            operation(&worker_context, "test.thread", "worker").cancel("TEST_CANCEL");
            sender.send(()).unwrap();
        });
    });
    receiver.recv().unwrap();
    first_root.finish("SUCCESS");
    second_root.span().in_scope(|| {
        tracing::debug!(target: "thinkrail_test", "second root");
    });
    second_root.finish("SUCCESS");
    handle.set_level(LogLevel::Info).unwrap();
    tracing::debug!(target: "thinkrail_test", "hidden at info");
    let (info_trace, info_root) = TraceContext::root("test.info_failure", "ui");
    info_root.span().in_scope(|| {
        operation(&info_trace, "test.hidden_child", "service").finish("SUCCESS");
    });
    info_root.fail("TEST_FINAL_FAILURE");
    let dir = handle.dir().to_owned();
    drop(handle);

    let entries = thinkrail_observability::tail(&dir, 100).unwrap();
    let of = |trace: &TraceContext| -> Vec<_> {
        entries
            .iter()
            .filter(|entry| entry.trace_id.as_deref() == Some(trace.trace_id()))
            .collect()
    };
    let lifecycle = |list: &[&thinkrail_observability::LogEntry], op: &str| -> Vec<String> {
        list.iter()
            .filter(|entry| entry.operation.as_deref() == Some(op))
            .filter_map(|entry| entry.event.clone())
            .collect()
    };
    let first_entries = of(&first);
    let second_entries = of(&second);
    // Business events inside the root and its worker thread keep the root's TraceId.
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.message == "first root" && entry.component == "thinkrail_test")
    );
    assert!(
        second_entries
            .iter()
            .any(|entry| entry.message == "second root")
    );
    // Each operation writes ENTRY and exactly one terminal, on the right trace.
    assert_eq!(lifecycle(&first_entries, "test.first"), ["ENTRY", "EXIT"]);
    assert_eq!(
        lifecycle(&first_entries, "test.channel"),
        ["ENTRY", "ERROR"]
    );
    assert_eq!(
        lifecycle(&first_entries, "test.thread"),
        ["ENTRY", "CANCEL"]
    );
    assert_eq!(lifecycle(&second_entries, "test.second"), ["ENTRY", "EXIT"]);
    assert!(lifecycle(&second_entries, "test.channel").is_empty());
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.raw.contains("|Error:TEST_FAILURE"))
    );
    assert!(
        first_entries
            .iter()
            .any(|entry| entry.raw.contains("|Reason:TEST_CANCEL"))
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.message == "hidden at info")
    );
    let info_entries = of(&info_trace);
    assert_eq!(
        lifecycle(&info_entries, "test.info_failure"),
        ["ENTRY", "ERROR"]
    );
    assert!(lifecycle(&info_entries, "test.hidden_child").is_empty());
    assert_eq!(
        info_entries
            .iter()
            .filter(|entry| entry.level == "ERROR")
            .count(),
        1
    );
    assert_eq!(info_entries[0].level, "INFO");
}
