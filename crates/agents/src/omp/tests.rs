use serde_json::{Value, json};

use super::stopped_request;

fn user(id: &str, parent: &str) -> Value {
    json!({"type": "message", "id": id, "parentId": parent, "message": {"role": "user", "content": [{"type": "text", "text": "q"}]}})
}

fn assistant(id: &str, parent: &str, stop: &str, content: Value) -> Value {
    json!({"type": "message", "id": id, "parentId": parent, "message": {"role": "assistant", "stopReason": stop, "content": content}})
}

fn thinking() -> Value {
    json!([{"type": "thinking", "thinking": "plan"}])
}

#[test]
fn a_request_stopped_before_any_answer_is_withdrawn() {
    let entries = json!([
        user("u1", "root"),
        assistant("a1", "u1", "stop", json!([{"type": "text", "text": "done"}])),
        user("u2", "a1"),
        assistant("a2", "u2", "aborted", thinking()),
        {"type": "custom", "id": "c", "parentId": "a2"},
    ]);
    assert_eq!(stopped_request(&entries, Some("c")).as_deref(), Some("u2"));
}

#[test]
fn stacked_stopped_requests_are_all_withdrawn() {
    let entries = json!([
        user("u0", "root"),
        assistant("a0", "u0", "stop", json!([{"type": "text", "text": "done"}])),
        user("u1", "a0"),
        assistant("a1", "u1", "aborted", thinking()),
        user("u2", "a1"),
        assistant("a2", "u2", "aborted", json!([])),
    ]);
    assert_eq!(stopped_request(&entries, Some("a2")).as_deref(), Some("u1"));
}

#[test]
fn a_partial_answer_or_tool_call_keeps_the_request() {
    let text = json!([{"type": "thinking", "thinking": "p"}, {"type": "text", "text": "half an article"}]);
    let tool = json!([{"type": "toolCall", "id": "t", "name": "write", "arguments": {}}]);
    for content in [text, tool] {
        let entries = json!([user("u1", "root"), assistant("a1", "u1", "aborted", content)]);
        assert_eq!(stopped_request(&entries, Some("a1")), None);
    }
}

#[test]
fn finished_errored_or_unanswered_turns_are_left_alone() {
    let done = json!([user("u1", "root"), assistant("a1", "u1", "stop", thinking())]);
    assert_eq!(stopped_request(&done, Some("a1")), None);
    let failed = json!([user("u1", "root"), assistant("a1", "u1", "error", thinking())]);
    assert_eq!(stopped_request(&failed, Some("a1")), None);
    let pending = json!([user("u1", "root")]);
    assert_eq!(stopped_request(&pending, Some("u1")), None);
    assert_eq!(stopped_request(&done, None), None);
}
