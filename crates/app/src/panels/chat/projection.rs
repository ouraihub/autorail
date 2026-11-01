//! The UI copy of one chat's `SessionSnapshot`, kept current from `SessionEvent`s. Revisions only
//! move forward: a late event never overwrites a newer state.

use thinkrail_core::{ContentBlock, SessionEvent, SessionSnapshot};

/// Applies `event` to the chat's projection. Returns whether anything changed.
pub fn apply(current: &mut Option<SessionSnapshot>, event: SessionEvent) -> bool {
    match event {
        SessionEvent::Snapshot { snapshot } => {
            if current.as_ref().is_some_and(|c| c.revision > snapshot.revision) {
                return false;
            }
            *current = Some(*snapshot);
            true
        }
        SessionEvent::Message { revision, message, .. } => {
            let Some(snapshot) = fresh(current, revision) else { return false };
            match snapshot.messages.iter_mut().find(|m| m.id == message.id) {
                Some(existing) => *existing = message,
                None => snapshot.messages.push(message),
            }
            true
        }
        SessionEvent::Delta { revision, message_id, block, text, .. } => {
            let Some(snapshot) = fresh(current, revision) else { return false };
            let target = snapshot.messages.iter_mut().find(|m| m.id == message_id).and_then(|m| m.content.get_mut(block));
            match target {
                Some(ContentBlock::Text { text: existing } | ContentBlock::Thinking { text: existing }) => {
                    existing.push_str(&text);
                    true
                }
                _ => false,
            }
        }
        SessionEvent::Error { revision, error, .. } => {
            let Some(snapshot) = fresh(current, revision) else { return false };
            snapshot.last_error = Some(error);
            true
        }
    }
}

/// The projection to update when `revision` is newer; bumps it.
fn fresh(current: &mut Option<SessionSnapshot>, revision: u64) -> Option<&mut SessionSnapshot> {
    let snapshot = current.as_mut()?;
    if revision < snapshot.revision {
        return None;
    }
    snapshot.revision = revision;
    Some(snapshot)
}

#[cfg(test)]
mod tests {
    use thinkrail_core::{
        BackendKind, Capabilities, ChatId, Message, Role, SessionError, SessionPhase,
    };

    use super::*;

    fn snapshot(revision: u64, messages: Vec<Message>) -> SessionSnapshot {
        SessionSnapshot {
            id: "c".into(),
            backend: BackendKind::Pi,
            native_session_key: None,
            cwd: "C:/w".into(),
            phase: SessionPhase::Idle,
            model: None,
            messages,
            interactions: Vec::new(),
            capabilities: Capabilities::default(),
            output_truncated: false,
            last_error: None,
            revision,
        }
    }

    fn assistant(id: &str, content: Vec<ContentBlock>) -> Message {
        Message { id: id.into(), role: Role::Assistant, content }
    }

    fn chat() -> ChatId {
        "c".into()
    }

    #[test]
    fn older_snapshots_never_replace_newer_state() {
        let mut current = None;
        assert!(apply(&mut current, SessionEvent::Snapshot { snapshot: Box::new(snapshot(5, Vec::new())) }));
        assert!(!apply(&mut current, SessionEvent::Snapshot { snapshot: Box::new(snapshot(4, Vec::new())) }));
        assert_eq!(current.as_ref().map(|s| s.revision), Some(5));
        assert!(apply(&mut current, SessionEvent::Snapshot { snapshot: Box::new(snapshot(9, Vec::new())) }));
        assert_eq!(current.map(|s| s.revision), Some(9));
    }

    #[test]
    fn deltas_append_to_the_named_text_or_thinking_block() {
        let blocks = vec![ContentBlock::Thinking { text: "pl".into() }, ContentBlock::Text { text: "He".into() }];
        let mut current = Some(snapshot(1, vec![assistant("m", blocks)]));
        let delta = |block, text: &str, revision| SessionEvent::Delta {
            chat: chat(),
            revision,
            message_id: "m".into(),
            block,
            text: text.into(),
        };
        assert!(apply(&mut current, delta(0, "an", 2)));
        assert!(apply(&mut current, delta(1, "llo", 3)));
        assert!(!apply(&mut current, delta(1, "late", 2)), "older revisions are dropped");
        assert!(!apply(&mut current, delta(7, "x", 4)), "missing blocks are ignored");
        let content = &current.as_ref().map(|s| s.messages[0].content.clone()).unwrap_or_default();
        assert_eq!(content[0], ContentBlock::Thinking { text: "plan".into() });
        assert_eq!(content[1], ContentBlock::Text { text: "Hello".into() });
    }

    #[test]
    fn messages_upsert_by_id_and_errors_are_recorded() {
        let mut current = Some(snapshot(1, Vec::new()));
        let message = |text: &str, revision| SessionEvent::Message {
            chat: chat(),
            revision,
            message: assistant("m", vec![ContentBlock::Text { text: text.into() }]),
        };
        assert!(apply(&mut current, message("a", 2)));
        assert!(apply(&mut current, message("b", 3)));
        let error = SessionError { code: "EXTERNAL".into(), message: "boom".into() };
        assert!(apply(&mut current, SessionEvent::Error { chat: chat(), revision: 4, error }));
        let snapshot = current.unwrap_or_else(|| snapshot(0, Vec::new()));
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].content, vec![ContentBlock::Text { text: "b".into() }]);
        assert_eq!(snapshot.last_error.map(|e| e.code), Some("EXTERNAL".into()));
    }

    #[test]
    fn events_before_the_first_snapshot_are_ignored() {
        let mut current = None;
        let message = SessionEvent::Message { chat: chat(), revision: 1, message: assistant("m", Vec::new()) };
        assert!(!apply(&mut current, message));
        assert!(current.is_none());
    }
}
