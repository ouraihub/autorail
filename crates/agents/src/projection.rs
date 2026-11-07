//! The host projection of one engine conversation: a bounded transcript plus phase, model and
//! pending interactions. Every change bumps a process-wide revision and is published while the
//! state lock is held, so events reach the sink in revision order.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use thinkrail_core::{
    BackendKind, Capabilities, ChatId, ContentBlock, Interaction, Message, ModelInfo,
    NativeSessionKey, SessionError, SessionEvent, SessionPhase, SessionSink, SessionSnapshot,
    SlashCommand, ToolStatus,
};

const MAX_MESSAGES: usize = 2000;
const MAX_BYTES: usize = 8 * 1024 * 1024;

mod bound;
mod lifecycle;

#[cfg(test)]
use bound::TRUNCATED;
use bound::{MAX_MESSAGE_BYTES, bound, carry_tool_results, size_of};

static REVISION: AtomicU64 = AtomicU64::new(0);

fn next_revision() -> u64 {
    REVISION.fetch_add(1, Ordering::Relaxed) + 1
}

pub(crate) struct Identity {
    pub chat: ChatId,
    pub backend: BackendKind,
    pub cwd: PathBuf,
    pub capabilities: Capabilities,
}

pub(crate) struct Projection {
    snapshot: SessionSnapshot,
    sizes: Vec<usize>,
    total: usize,
    running: bool,
    /// When the current turn started; `None` while idle.
    turn_started: Option<std::time::Instant>,
    closed: bool,
    sink: Arc<dyn SessionSink>,
    trace: Option<Arc<crate::trace::SessionTrace>>,
    turn_span: Option<tracing::Span>,
}

impl Projection {
    pub(crate) fn new(identity: Identity, sink: Arc<dyn SessionSink>) -> Self {
        let snapshot = SessionSnapshot {
            id: identity.chat,
            backend: identity.backend,
            native_session_key: None,
            cwd: identity.cwd,
            phase: SessionPhase::Idle,
            model: None,
            messages: Vec::new(),
            interactions: Vec::new(),
            capabilities: identity.capabilities,
            output_truncated: false,
            last_error: None,
            commands: Vec::new(),
            session_name: None,
            revision: next_revision(),
        };
        Self {
            snapshot,
            sizes: Vec::new(),
            total: 0,
            running: false,
            turn_started: None,
            closed: false,
            sink,
            trace: None,
            turn_span: None,
        }
    }

    pub(crate) fn attach_trace(&mut self, trace: Arc<crate::trace::SessionTrace>) {
        self.trace = Some(trace);
    }

    pub(crate) fn snapshot(&self) -> SessionSnapshot {
        self.snapshot.clone()
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn has_interactions(&self) -> bool {
        !self.snapshot.interactions.is_empty()
    }

    pub(crate) fn set_key(&mut self, key: NativeSessionKey) {
        self.snapshot.native_session_key = Some(key);
    }

    pub(crate) fn model(&self) -> Option<&ModelInfo> {
        self.snapshot.model.as_ref()
    }

    pub(crate) fn set_model(&mut self, model: Option<ModelInfo>) {
        if self.snapshot.model != model {
            self.snapshot.model = model;
            self.publish();
        }
    }

    pub(crate) fn set_commands(&mut self, commands: Vec<SlashCommand>) {
        if self.snapshot.commands != commands {
            self.snapshot.commands = commands;
            self.publish();
        }
    }

    pub(crate) fn set_session_name(&mut self, name: Option<String>) {
        if self.snapshot.session_name != name {
            self.snapshot.session_name = name;
            self.publish();
        }
    }

    pub(crate) fn set_capabilities(&mut self, capabilities: Capabilities) {
        self.snapshot.capabilities = capabilities;
    }

    pub(crate) fn add_interaction(&mut self, interaction: Interaction) {
        self.snapshot
            .interactions
            .retain(|i| i.request_id != interaction.request_id);
        self.snapshot.interactions.push(interaction);
        self.publish();
    }

    /// Removes the interaction without publishing; callers publish once the engine has the answer.
    pub(crate) fn take_interaction(&mut self, request_id: &str) -> Option<Interaction> {
        let index = self
            .snapshot
            .interactions
            .iter()
            .position(|i| i.request_id == request_id)?;
        Some(self.snapshot.interactions.remove(index))
    }

    pub(crate) fn restore_interaction(&mut self, interaction: Interaction) {
        self.snapshot.interactions.push(interaction);
    }

    pub(crate) fn take_interactions(&mut self) -> Vec<Interaction> {
        std::mem::take(&mut self.snapshot.interactions)
    }

    pub(crate) fn error(&mut self, error: SessionError) {
        self.snapshot.last_error = Some(error.clone());
        let revision = self.bump();
        self.emit(SessionEvent::Error {
            chat: self.snapshot.id.clone(),
            revision,
            error,
        });
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.position(id).is_some()
    }

    /// Inserts or replaces a message by id. A replacement keeps tool results that arrived
    /// separately from the message that called the tool.
    pub(crate) fn upsert(&mut self, mut message: Message) {
        let existing = self.position(&message.id);
        if let Some(index) = existing {
            carry_tool_results(&self.snapshot.messages[index], &mut message);
        }
        if bound(&mut message) {
            self.snapshot.output_truncated = true;
        }
        let size = size_of(&message);
        match existing {
            Some(index) => {
                self.total = self.total - self.sizes[index] + size;
                self.sizes[index] = size;
                self.snapshot.messages[index] = message.clone();
            }
            None => {
                self.total += size;
                self.sizes.push(size);
                self.snapshot.messages.push(message.clone());
            }
        }
        self.finish_message(message);
    }

    /// Applies `change` to a message; `false` if no such message.
    pub(crate) fn update(&mut self, id: &str, change: impl FnOnce(&mut Message)) -> bool {
        let Some(index) = self.position(id) else {
            return false;
        };
        let mut message = self.snapshot.messages[index].clone();
        change(&mut message);
        self.upsert(message);
        true
    }

    /// Appends streamed text to a text or thinking block. Text beyond the per-message bound is
    /// dropped from the projection only (the engine keeps it).
    pub(crate) fn append(&mut self, id: &str, block: usize, delta: &str) {
        let Some(index) = self.position(id) else {
            return;
        };
        if self.sizes[index] + delta.len() > MAX_MESSAGE_BYTES {
            if !self.snapshot.output_truncated {
                self.snapshot.output_truncated = true;
                self.publish();
            }
            return;
        }
        let message = &mut self.snapshot.messages[index];
        let Some(ContentBlock::Text { text } | ContentBlock::Thinking { text }) =
            message.content.get_mut(block)
        else {
            return;
        };
        text.push_str(delta);
        self.sizes[index] += delta.len();
        self.total += delta.len();
        let revision = self.bump();
        let (chat, message_id) = (self.snapshot.id.clone(), id.to_owned());
        self.emit(SessionEvent::Delta {
            chat,
            revision,
            message_id,
            block,
            text: delta.to_owned(),
        });
        if self.total > MAX_BYTES {
            self.evict();
            self.publish();
        }
    }

    /// Id of the message holding the tool block for `tool_call_id`, newest first.
    pub(crate) fn tool_owner(&self, tool_call_id: &str) -> Option<String> {
        self.snapshot
            .messages
            .iter()
            .rev()
            .find(|m| m.content.iter().any(|b| matches!(b, ContentBlock::Tool { tool_call_id: id, .. } if id == tool_call_id)))
            .map(|m| m.id.clone())
    }

    /// Sets a tool block's status and (when given) replaces its output.
    pub(crate) fn set_tool(
        &mut self,
        tool_call_id: &str,
        status: ToolStatus,
        output: Option<String>,
    ) -> bool {
        let Some(owner) = self.tool_owner(tool_call_id) else {
            return false;
        };
        self.update(&owner, |message| {
            for block in &mut message.content {
                if let ContentBlock::Tool {
                    tool_call_id: id,
                    status: s,
                    output: o,
                    ..
                } = block
                    && id == tool_call_id
                {
                    *s = status;
                    if output.is_some() {
                        *o = output.clone();
                    }
                }
            }
        })
    }

    /// Tools that never reported a result (e.g. a replayed, interrupted turn) did not complete.
    pub(crate) fn fail_unfinished_tools(&mut self) {
        let mut changed = false;
        for message in &mut self.snapshot.messages {
            for block in &mut message.content {
                if let ContentBlock::Tool {
                    status: status @ ToolStatus::Running,
                    ..
                } = block
                {
                    *status = ToolStatus::Failed;
                    changed = true;
                }
            }
        }
        if changed {
            self.publish();
        }
    }

    /// Publishes the full snapshot with the current phase.
    pub(crate) fn publish(&mut self) {
        self.snapshot.phase = if self.closed {
            SessionPhase::Closed
        } else if !self.snapshot.interactions.is_empty() {
            SessionPhase::Waiting
        } else if self.running {
            SessionPhase::Running
        } else {
            SessionPhase::Idle
        };
        self.bump();
        self.emit(SessionEvent::Snapshot {
            snapshot: Box::new(self.snapshot.clone()),
        });
    }

    fn finish_message(&mut self, message: Message) {
        if self.snapshot.messages.len() > MAX_MESSAGES || self.total > MAX_BYTES {
            self.evict();
            self.publish();
            return;
        }
        let revision = self.bump();
        self.emit(SessionEvent::Message {
            chat: self.snapshot.id.clone(),
            revision,
            message,
        });
    }

    fn evict(&mut self) {
        while self.snapshot.messages.len() > 1
            && (self.snapshot.messages.len() > MAX_MESSAGES || self.total > MAX_BYTES)
        {
            self.snapshot.messages.remove(0);
            self.total -= self.sizes.remove(0);
            self.snapshot.output_truncated = true;
        }
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.snapshot.messages.iter().rposition(|m| m.id == id)
    }

    fn bump(&mut self) -> u64 {
        self.snapshot.revision = next_revision();
        self.snapshot.revision
    }

    fn emit(&self, event: SessionEvent) {
        self.sink.publish(event);
    }
}

#[cfg(test)]
pub(crate) mod tests;
