//! The real subscriber's lines read back as the log page sees them. One test per binary: the
//! global subscriber can be installed once per process.

use std::panic::Location;

use thinkrail_core::LogLevel;
use thinkrail_observability::{init, op};

#[test]
fn an_operation_s_lines_carry_its_span_in_beijing_time_and_the_level_switches() {
    let data = tempfile::tempdir().unwrap();
    let handle = init(data.path()).unwrap();
    let at = Location::caller();
    let span = op(at);
    span.in_scope(|| tracing::info!(chat = "c1", "inside"));
    tracing::debug!(target: "thinkrail_agents::pi", "hidden at info");
    handle.set_level(LogLevel::Debug).unwrap();
    tracing::debug!(target: "thinkrail_agents::pi", "shown at debug");
    tracing::debug!(target: "turso_core::storage", "dependency internals stay hidden");
    let dir = handle.dir().to_owned();
    drop(handle); // flushes the non-blocking writer

    let entries = thinkrail_observability::tail(&dir, 100).unwrap();
    let inside = entries.iter().find(|e| e.message.starts_with("inside")).unwrap();
    assert!(inside.time.ends_with("+08:00"), "{}", inside.time);
    assert!(inside.spans.starts_with("op{id="), "{}", inside.spans);
    assert!(inside.spans.contains(&format!("tests/init.rs:{}}}", at.line())), "{}", inside.spans);
    assert_eq!(inside.target, "init");
    assert_eq!(inside.message, "inside chat=\"c1\"");
    assert!(!entries.iter().any(|e| e.message == "hidden at info"));
    assert!(entries.iter().any(|e| e.message == "shown at debug"));
    assert!(!entries.iter().any(|e| e.target.starts_with("turso")), "debug raises only thinkrail crates");
}
