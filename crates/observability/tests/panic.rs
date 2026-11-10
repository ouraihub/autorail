//! A subprocess proves crash diagnostics survive abort without dropping the async logger.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "isolated crash fixture"
)]

use std::{path::PathBuf, process::Command};
use thinkrail_observability::{TraceContext, init, tail};

#[test]
fn panic_child() {
    let Some(dir) = std::env::var_os("THINKRAIL_PANIC_TEST_DIR").map(PathBuf::from) else {
        return;
    };
    if std::env::var("THINKRAIL_PANIC_TEST_MODE").as_deref() == Ok("invalid-filter") {
        assert!(init(&dir).is_err());
        return;
    }
    let _log = init(&dir).unwrap();
    let mode = std::env::var("THINKRAIL_PANIC_TEST_MODE").unwrap();
    let work = move || {
        if mode == "entity" {
            let (_, root) = TraceContext::root("test.panic", "ui");
            let _entered = root.span().clone().entered();
            panic!("cannot read SecretProvider while it is already being updated");
        }
        panic!("Authorization: Bearer private-fake-key | user prompt /private/path\nsecret");
    };
    let _ = std::panic::catch_unwind(work);
    // No WorkerGuard drop: the GUI's non-unwinding FFI boundary terminates the same way.
    std::process::abort();
}

#[test]
fn panic_is_durable_redacted_correlated_and_not_filtered_out() {
    for mode in ["entity", "secret"] {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panic_child", "--nocapture"])
            .env("THINKRAIL_PANIC_TEST_DIR", dir.path())
            .env("THINKRAIL_PANIC_TEST_MODE", mode)
            .env("THINKRAIL_LOG", "off")
            .output()
            .unwrap();
        assert!(!child.status.success());
        assert!(!String::from_utf8_lossy(&child.stderr).contains("private-fake-key"));
        assert!(!String::from_utf8_lossy(&child.stdout).contains("private-fake-key"));
        let entries = tail(&dir.path().join("logs"), 100).unwrap();
        let panics: Vec<_> = entries
            .iter()
            .filter(|line| line.message == "app.panic")
            .collect();
        assert_eq!(panics.len(), 1, "{entries:?}");
        let line = panics[0];
        assert_eq!(line.level, "ERROR");
        assert!(line.raw.contains("code:RUST_PANIC"));
        assert!(line.raw.contains("summary:redacted_string_payload"));
        assert!(line.raw.contains("panic_file:") && line.raw.contains("panic.rs"));
        assert!(line.raw.contains("panic_line:"));
        assert!(line.raw.contains("backtrace:"));
        assert!(!line.raw.contains("disabled backtrace"));
        assert!(
            line.raw.contains("0:"),
            "captured stack must contain a frame even without release symbols"
        );
        assert!(!line.raw.contains("private-fake-key"));
        assert!(!line.raw.contains("user prompt"));
        assert!(!line.raw.contains("/private/path"));
        assert!(!line.raw.contains("SecretProvider"));
        if mode == "entity" {
            assert_eq!(line.operation.as_deref(), Some("test.panic"));
            assert_eq!(line.trace_id.as_ref().map(String::len), Some(32));
        } else {
            assert!(line.raw.contains("summary:redacted_string_payload"));
            assert_eq!(line.trace_id, None);
        }
    }
}

#[test]
fn invalid_filter_does_not_silently_enable_a_default_level() {
    let dir = tempfile::tempdir().unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "panic_child", "--nocapture"])
        .env("THINKRAIL_PANIC_TEST_DIR", dir.path())
        .env("THINKRAIL_PANIC_TEST_MODE", "invalid-filter")
        .env("THINKRAIL_LOG", "[invalid")
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
}
