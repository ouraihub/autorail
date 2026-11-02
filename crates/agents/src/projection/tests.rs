use std::sync::Arc;

use parking_lot::Mutex;
use thinkrail_core::{
    BackendKind, Capabilities, ContentBlock, Interaction, InteractionKind, Message, Role,
    SessionError, SessionEvent, SessionPhase, SessionSink, ToolStatus,
};

use super::*;

#[derive(Default)]
pub(crate) struct Events(pub(crate) Mutex<Vec<SessionEvent>>);

impl SessionSink for Events {
    fn publish(&self, event: SessionEvent) {
        self.0.lock().push(event);
    }
}

pub(crate) fn projection() -> (Projection, Arc<Events>) {
    let events = Arc::new(Events::default());
    let identity = Identity {
        chat: "chat".into(),
        backend: BackendKind::Pi,
        cwd: "C:/work".into(),
        capabilities: Capabilities::default(),
    };
    (Projection::new(identity, events.clone()), events)
}

fn text(id: &str, role: Role, text: &str) -> Message {
    Message {
        id: id.into(),
        role,
        content: vec![ContentBlock::Text { text: text.into() }],
    }
}

fn revision(event: &SessionEvent) -> u64 {
    match event {
        SessionEvent::Snapshot { snapshot } => snapshot.revision,
        SessionEvent::Message { revision, .. }
        | SessionEvent::Delta { revision, .. }
        | SessionEvent::Error { revision, .. } => *revision,
    }
}

#[test]
fn every_event_has_a_strictly_newer_revision_even_across_sessions() {
    let (mut first, events) = projection();
    first.upsert(text("a", Role::User, "hi"));
    first.upsert(Message {
        id: "b".into(),
        role: Role::Assistant,
        content: vec![ContentBlock::Text {
            text: String::new(),
        }],
    });
    first.append("b", 0, "he");
    first.append("b", 0, "llo");
    first.set_running(true);
    let (mut second, later) = projection();
    second.publish();

    let mut revisions: Vec<u64> = events.0.lock().iter().map(revision).collect();
    revisions.extend(later.0.lock().iter().map(revision));
    assert!(revisions.windows(2).all(|w| w[0] < w[1]), "{revisions:?}");
    assert_eq!(
        first.snapshot().messages[1].content,
        vec![ContentBlock::Text {
            text: "hello".into()
        }]
    );
    assert!(
        matches!(&events.0.lock()[2], SessionEvent::Delta { block: 0, text, .. } if text == "he")
    );
}

#[test]
fn phase_follows_interactions_running_and_close() {
    let (mut p, _) = projection();
    p.publish();
    assert_eq!(p.snapshot().phase, SessionPhase::Idle);
    p.set_running(true);
    assert_eq!(p.snapshot().phase, SessionPhase::Running);
    let question = Interaction {
        request_id: "q".into(),
        kind: InteractionKind::Confirm,
        title: "t".into(),
        message: String::new(),
        options: Vec::new(),
        detail: serde_json::Value::Null,
    };
    p.add_interaction(question);
    assert_eq!(p.snapshot().phase, SessionPhase::Waiting);
    p.close(Some(SessionError {
        code: "SESSION_CLOSED".into(),
        message: "gone".into(),
    }));
    let snapshot = p.snapshot();
    assert_eq!(snapshot.phase, SessionPhase::Closed);
    assert!(
        snapshot.interactions.is_empty(),
        "open questions die with the engine"
    );
    assert_eq!(
        snapshot.last_error.map(|e| e.code),
        Some("SESSION_CLOSED".into())
    );
}

#[test]
fn replacing_a_message_keeps_tool_results_that_arrived_separately() {
    let (mut p, _) = projection();
    let call = |status| Message {
        id: "m".into(),
        role: Role::Assistant,
        content: vec![ContentBlock::Tool {
            tool_call_id: "t1".into(),
            name: "read".into(),
            status,
            input: None,
            output: None,
        }],
    };
    p.upsert(call(ToolStatus::Running));
    assert!(p.set_tool("t1", ToolStatus::Completed, Some("file body".into())));
    p.upsert(call(ToolStatus::Running));
    assert!(matches!(
        &p.snapshot().messages[0].content[0],
        ContentBlock::Tool { status: ToolStatus::Completed, output: Some(o), .. } if o == "file body"
    ));
    assert!(!p.set_tool("missing", ToolStatus::Failed, None));
}

#[test]
fn unfinished_tools_fail_when_a_turn_settles() {
    let (mut p, _) = projection();
    let tool = |id: &str, status| ContentBlock::Tool {
        tool_call_id: id.into(),
        name: "x".into(),
        status,
        input: None,
        output: None,
    };
    p.upsert(Message {
        id: "m".into(),
        role: Role::Assistant,
        content: vec![
            tool("done", ToolStatus::Completed),
            tool("cut", ToolStatus::Running),
        ],
    });
    p.fail_unfinished_tools();
    let content = &p.snapshot().messages[0].content;
    assert!(matches!(
        content[0],
        ContentBlock::Tool {
            status: ToolStatus::Completed,
            ..
        }
    ));
    assert!(matches!(
        content[1],
        ContentBlock::Tool {
            status: ToolStatus::Failed,
            ..
        }
    ));
}

#[test]
fn oversized_messages_are_truncated_and_old_messages_evicted() {
    let (mut p, _) = projection();
    p.upsert(text(
        "big",
        Role::Assistant,
        &"中".repeat(MAX_MESSAGE_BYTES),
    ));
    let snapshot = p.snapshot();
    let ContentBlock::Text { text: kept } = &snapshot.messages[0].content[0] else {
        panic!("text block")
    };
    assert!(kept.len() <= MAX_MESSAGE_BYTES && kept.ends_with(TRUNCATED));
    assert!(snapshot.output_truncated);

    let (mut p, _) = projection();
    for index in 0..=MAX_MESSAGES {
        p.upsert(text(&index.to_string(), Role::User, "x"));
    }
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), MAX_MESSAGES);
    assert_eq!(snapshot.messages[0].id, "1");
    assert!(snapshot.output_truncated);
}

#[test]
fn streamed_text_past_the_message_bound_is_dropped_from_the_projection_only() {
    let (mut p, _) = projection();
    p.upsert(Message {
        id: "m".into(),
        role: Role::Assistant,
        content: vec![ContentBlock::Text {
            text: String::new(),
        }],
    });
    let chunk = "y".repeat(64 * 1024);
    for _ in 0..8 {
        p.append("m", 0, &chunk);
    }
    let snapshot = p.snapshot();
    let ContentBlock::Text { text } = &snapshot.messages[0].content[0] else {
        panic!("text block")
    };
    assert!(text.len() < MAX_MESSAGE_BYTES);
    assert!(snapshot.output_truncated);
}

#[test]
fn command_and_name_setters_publish_only_on_change() {
    let (mut p, events) = projection();
    let commands = vec![thinkrail_core::SlashCommand {
        name: "skill:review".into(),
        description: None,
        hint: None,
        source: thinkrail_core::CommandSource::Skill,
        scope: None,
    }];
    p.set_commands(commands.clone());
    p.set_commands(commands.clone());
    p.set_session_name(Some("名字".into()));
    p.set_session_name(Some("名字".into()));
    assert_eq!(events.0.lock().len(), 2);
    let snapshot = p.snapshot();
    assert_eq!(snapshot.commands, commands);
    assert_eq!(snapshot.session_name.as_deref(), Some("名字"));
}
