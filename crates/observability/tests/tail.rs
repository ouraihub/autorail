//! The log page's read-back: which file, how much of it, and how a line splits.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration test helpers fail loudly"
)]

use std::fs;

use thinkrail_observability::tail;

const OP: &str = "2026-11-01T15:26:55.669+08:00  INFO op{id=42 at=panels/chat.rs:312}: thinkrail_agents::pi: agent session started chat=c1";

#[test]
fn a_line_inside_an_operation_keeps_its_span_target_and_fields_apart() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        format!("{OP}\n"),
    )
    .unwrap();
    let [entry] = tail(dir.path(), 10).unwrap().try_into().unwrap();
    assert_eq!(entry.time, "2026-11-01T15:26:55.669+08:00");
    assert_eq!(entry.level, "INFO");
    assert_eq!(entry.spans, "op{id=42 at=panels/chat.rs:312}");
    assert_eq!(entry.target, "thinkrail_agents::pi");
    assert_eq!(entry.message, "agent session started chat=c1");
    assert_eq!(entry.raw, OP);
}

#[test]
fn lines_outside_operations_and_unparsable_lines_still_show() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("thinkrail-native.2026-11-01.log"),
        "2026-11-01T15:00:00+08:00 ERROR : 无效的窗口句柄。 (0x80070578)\n\
         2026-11-01T15:00:01+08:00  WARN thinkrail_app::state: operation failed code=NOT_FOUND\n\
         panicked at somewhere\n",
    )
    .unwrap();
    let entries = tail(dir.path(), 10).unwrap();
    assert_eq!(
        (
            entries[0].level.as_str(),
            entries[0].target.as_str(),
            entries[0].message.as_str()
        ),
        ("ERROR", "", "无效的窗口句柄。 (0x80070578)")
    );
    assert_eq!(
        (entries[1].spans.as_str(), entries[1].target.as_str()),
        ("", "thinkrail_app::state")
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
