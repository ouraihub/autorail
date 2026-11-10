//! Narrowing the log list by event and layer, together with 只看问题 and the text filter.

use thinkrail_observability::LogEntry;

use super::{Narrow, layers};

fn entry(level: &str, layer: Option<&str>, event: Option<&str>, raw: &str) -> LogEntry {
    LogEntry {
        seq: 0,
        time: String::new(),
        level: level.into(),
        trace_id: None,
        operation: None,
        event: event.map(Into::into),
        duration_ms: None,
        layer: layer.map(Into::into),
        component: String::new(),
        message: String::new(),
        raw: raw.into(),
    }
}

#[test]
fn event_and_layer_choices_combine_with_problems_and_text() {
    let entries = [
        entry("INFO", Some("ui"), Some("ENTRY"), "a"),
        entry("ERROR", Some("ui"), Some("ERROR"), "b"),
        entry("ERROR", Some("review"), Some("ERROR"), "c"),
        entry("WARN", None, None, "plain warning c"),
    ];
    let kept = |narrow: &Narrow, query: &str| -> Vec<&str> {
        entries
            .iter()
            .filter(|e| narrow.keeps(e, query))
            .map(|e| e.raw.as_str())
            .collect()
    };
    let all = Narrow::default();
    assert_eq!(kept(&all, ""), ["a", "b", "c", "plain warning c"]);

    let errors = Narrow {
        event: Some("ERROR"),
        ..Narrow::default()
    };
    assert_eq!(
        kept(&errors, ""),
        ["b", "c"],
        "lines without EV never match an event"
    );

    let ui_errors = Narrow {
        layer: Some("ui".into()),
        ..errors.clone()
    };
    assert_eq!(kept(&ui_errors, ""), ["b"]);
    assert_eq!(kept(&errors, "c"), ["c"], "text narrows further");

    let problems = Narrow {
        only_problems: true,
        ..Narrow::default()
    };
    assert_eq!(kept(&problems, ""), ["b", "c", "plain warning c"]);
}

#[test]
fn layer_menu_lists_each_present_layer_once_sorted() {
    let entries = [
        entry("INFO", Some("ui"), None, ""),
        entry("INFO", Some("review"), None, ""),
        entry("INFO", None, None, ""),
        entry("INFO", Some("ui"), None, ""),
    ];
    assert_eq!(layers(&entries), ["review", "ui"]);
}
