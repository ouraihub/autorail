//! The log page's read-back: which file, how much of it, and how a line splits.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test helpers fail loudly"
)]

use std::fs;

use thinkrail_observability::tail;

const OP: &str = "2026-11-01T15:26:55.669+08:00  INFO TraceId:0123456789abcdef0123456789abcdef|SpanId:2|ParentSpanId:1|Layer:agents|ME:agent.session|Target:thinkrail_agents::pi|chat_id:c1|Msg:agent session started";

#[test]
fn a_business_line_inside_an_operation_keeps_trace_target_and_message_apart() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        format!("{OP}\n"),
    )
    .unwrap();
    let [entry] = tail(dir.path(), 10).unwrap().try_into().unwrap();
    assert_eq!(entry.time, "2026-11-01T15:26:55.669+08:00");
    assert_eq!(entry.level, "INFO");
    assert_eq!(
        entry.trace_id.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(entry.operation.as_deref(), Some("agent.session"));
    assert_eq!(entry.event, None);
    assert_eq!(entry.component, "thinkrail_agents::pi");
    assert_eq!(entry.message, "agent session started");
    assert_eq!(entry.raw, OP);
}

#[test]
fn a_lifecycle_line_shows_its_layer_event_and_duration() {
    let dir = tempfile::tempdir().unwrap();
    let line = "2026-11-01T15:26:55.669+08:00 ERROR TraceId:0123456789abcdef0123456789abcdef|SpanId:3|ParentSpanId:2|Layer:host_tool|ME:host_tool.call|EV:ERROR|RT:42|Result:FAILED|Error:NOT_FOUND";
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        format!("{line}\n"),
    )
    .unwrap();
    let [entry] = tail(dir.path(), 10).unwrap().try_into().unwrap();
    assert_eq!(entry.operation.as_deref(), Some("host_tool.call"));
    assert_eq!(entry.event.as_deref(), Some("ERROR"));
    assert_eq!(entry.duration_ms, Some(42));
    assert_eq!(entry.component, "host_tool");
}

#[test]
fn plain_text_does_not_fabricate_trace_fields() {
    let dir = tempfile::tempdir().unwrap();
    let line = "2026-11-01T15:26:55.669+08:00  INFO Target:thinkrail_test|Msg:startup TraceId:not-structured";
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        format!("{line}\n"),
    )
    .unwrap();
    let [entry] = tail(dir.path(), 10).unwrap().try_into().unwrap();
    assert_eq!(entry.trace_id, None);
    assert_eq!(entry.operation, None);
    assert_eq!(entry.message, "startup TraceId:not-structured");
    assert_eq!(entry.raw, line);
}

#[test]
fn lines_outside_operations_and_unparsable_lines_still_show() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        "2026-11-01T15:00:00+08:00 ERROR Msg:无效的窗口句柄。 (0x80070578)\n\
  2026-11-01T15:00:01+08:00  WARN Target:thinkrail_app::state|Msg:operation failed\n\
  panicked at somewhere\n",
    )
    .unwrap();
    let entries = tail(dir.path(), 10).unwrap();
    assert_eq!(
        (
            entries[0].level.as_str(),
            entries[0].component.as_str(),
            entries[0].message.as_str()
        ),
        ("ERROR", "", "无效的窗口句柄。 (0x80070578)")
    );
    assert_eq!(
        (
            entries[1].trace_id.as_deref(),
            entries[1].component.as_str()
        ),
        (None, "thinkrail_app::state")
    );
    assert_eq!(
        (
            entries[2].time.as_str(),
            entries[2].level.as_str(),
            entries[2].message.as_str()
        ),
        ("", "INFO", "panicked at somewhere")
    );
}

#[test]
fn reads_only_the_newest_file_and_its_last_lines() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("thinkrail-native.2026-10-31.log"),
        format!("{OP} old\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("other.2026-12-01.log"),
        format!("{OP} foreign\n"),
    )
    .unwrap();
    let lines: String = (0..5).map(|i| format!("{OP} n={i}\n")).collect();
    fs::write(dir.path().join("thinkrail-native.2026-11-01.log"), lines).unwrap();
    let entries = tail(dir.path(), 2).unwrap();
    let messages: Vec<_> = entries
        .iter()
        .map(|e| e.message.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(messages, ["n=3", "n=4"], "newest two, oldest first");
    assert_eq!(entries.iter().map(|e| e.seq).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn a_large_file_drops_the_cut_first_line_and_an_unfinished_last_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut text = "x".repeat(300 * 1024);
    text.push('\n');
    text.push_str(&format!("{OP} done\n"));
    text.push_str("2026-11-01T15:26:56+08:00  INFO still writ");
    fs::write(dir.path().join("thinkrail-native.2026-11-01.log"), text).unwrap();
    let entries = tail(dir.path(), 2000).unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert!(entries[0].message.ends_with("done"));
}

#[test]
fn no_log_directory_is_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    assert!(tail(&dir.path().join("logs"), 10).unwrap().is_empty());
}
