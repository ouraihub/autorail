use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{
    BackendKind, ChatId, EngineRouting, Error, HostToolAccess, Launch, NativeSessionKey, Result,
    SlashCommand, ThinkingLevel, WorkspaceId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    /// Images are announced by type only; the engine owns the bytes.
    Image {
        mime_type: String,
    },
    Tool {
        tool_call_id: String,
        name: String,
        status: ToolStatus,
        /// Raw engine arguments, rendered as code, never interpreted.
        input: Option<serde_json::Value>,
        output: Option<String>,
    },
}

/// A host projection of one engine message. `id` is stable across streaming updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    /// Ready for a prompt.
    Idle,
    /// A turn is in flight; only settle frames (not `agent_end`) return it to idle.
    Running,
    /// The engine is waiting on an [`Interaction`].
    Waiting,
    /// The engine process is gone; a new prompt resumes it from its native locator.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionKind {
    Permission,
    Confirm,
    Select,
    Input,
    Editor,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionOption {
    pub id: String,
    pub label: String,
}

/// A question the engine is blocked on. Nothing auto-approves it; dismissing is a cancel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub request_id: String,
    pub kind: InteractionKind,
    pub title: String,
    pub message: String,
    pub options: Vec<InteractionOption>,
    /// Engine-specific request (ACP `toolCall`, OMP `questions`) for richer rendering.
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum InteractionAnswer {
    Cancel,
    Confirm {
        confirmed: bool,
    },
    /// An option id from [`Interaction::options`].
    Select {
        option_id: String,
    },
    Text {
        text: String,
    },
    /// OMP structured questionnaire answers, passed through verbatim.
    Answers {
        answers: serde_json::Value,
    },
}

/// What a backend can actually do. The UI offers only what is true here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub resume: bool,
    /// Old turns can be re-read from the engine after a restart.
    pub history_replay: bool,
    /// Assistant text streams as deltas (else it arrives committed per message).
    pub token_deltas: bool,
    pub questions: bool,
    /// The engine asks before every tool call (not just some).
    pub tool_permissions: bool,
    /// The model can be switched inside a running session.
    pub model_switch: bool,
    /// The host runs `/compact` through [`AgentSession::compact`].
    pub compact: bool,
    /// The host runs `/name` through [`AgentSession::rename`].
    pub rename: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub provider: Option<String>,
    pub id: String,
    pub effort: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionError {
    pub code: String,
    pub message: String,
}

/// The authoritative state of one chat. `revision` increases on every change so a late snapshot
/// can never overwrite a newer event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: ChatId,
    pub backend: BackendKind,
    pub native_session_key: Option<NativeSessionKey>,
    pub cwd: PathBuf,
    pub phase: SessionPhase,
    pub model: Option<ModelInfo>,
    pub messages: Vec<Message>,
    pub interactions: Vec<Interaction>,
    pub capabilities: Capabilities,
    /// Older messages were dropped from this projection; the engine still has them.
    pub output_truncated: bool,
    pub last_error: Option<SessionError>,
    /// What the composer's `/` menu offers from the engine (host built-ins are added by the UI).
    pub commands: Vec<SlashCommand>,
    /// The engine's display name of this session, when one was set.
    pub session_name: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SessionEvent {
    Snapshot {
        snapshot: Box<SessionSnapshot>,
    },
    /// Insert or replace a message by id.
    Message {
        chat: ChatId,
        revision: u64,
        message: Message,
    },
    /// Append streamed text to `content[block]` of a message (a text or thinking block).
    Delta {
        chat: ChatId,
        revision: u64,
        message_id: String,
        block: usize,
        text: String,
    },
    Error {
        chat: ChatId,
        revision: u64,
        error: SessionError,
    },
}

impl SessionEvent {
    pub fn chat(&self) -> &ChatId {
        match self {
            Self::Snapshot { snapshot } => &snapshot.id,
            Self::Message { chat, .. } | Self::Delta { chat, .. } | Self::Error { chat, .. } => {
                chat
            }
        }
    }
}

/// Receives session events from engine reader threads. Must not block.
pub trait SessionSink: Send + Sync {
    fn publish(&self, event: SessionEvent);
}

#[derive(Debug, Clone)]
pub struct StartSession {
    pub chat: ChatId,
    /// Plans, reviews and host tools of this chat are scoped to `(workspace, chat)`.
    pub workspace: WorkspaceId,
    pub backend: BackendKind,
    /// The install 编码搭档 found for `backend`.
    pub launch: Launch,
    /// The engine's config dir (`PI_CODING_AGENT_DIR` / `DSH_HOME`). The app passes `None`, the
    /// engine's own default; tests pass an isolated directory.
    pub config_root: Option<PathBuf>,
    pub cwd: PathBuf,
    /// Resume this native session instead of creating one.
    pub resume: Option<NativeSessionKey>,
    /// `None` starts the engine without the host's TODO / review tools.
    pub host_tools: Option<HostToolAccess>,
    /// Providers, default model and MCP servers to project; empty keeps the engine's own config.
    pub routing: EngineRouting,
    /// This launch's engine support directory for generated files (pi extension, DSH patch, …).
    pub support_dir: PathBuf,
}

/// One live engine conversation. All methods return once the engine has *accepted* the command;
/// completion is reported through the [`SessionSink`].
pub trait AgentSession: Send + Sync {
    fn snapshot(&self) -> SessionSnapshot;
    fn prompt(&self, text: &str) -> Result<()>;
    fn stop(&self) -> Result<()>;
    fn respond(&self, request_id: &str, answer: InteractionAnswer) -> Result<()>;
    /// Graceful shutdown; the native transcript is kept.
    fn dispose(&self) -> Result<()>;
    /// Applies to this session only; never written back to engine config. `provider` is
    /// `ProjectedProvider.route`.
    fn set_model(
        &self,
        provider: &str,
        model_id: &str,
        effort: Option<ThinkingLevel>,
    ) -> Result<()>;
    /// Compacts the context and returns once the engine has finished. Only meaningful when
    /// `capabilities.compact`.
    ///
    /// # Errors
    ///
    /// `INVALID` when this engine has no compaction command; the engine's error otherwise.
    fn compact(&self, instructions: Option<&str>) -> Result<()> {
        let _ = instructions;
        Err(Error::invalid(format!(
            "{} 不支持 /compact",
            self.snapshot().backend.label()
        )))
    }
    /// Sets the session's display name. Only meaningful when `capabilities.rename`.
    ///
    /// # Errors
    ///
    /// `INVALID` when this engine cannot rename a session; the engine's error otherwise.
    fn rename(&self, name: &str) -> Result<()> {
        let _ = name;
        Err(Error::invalid(format!(
            "{} 不支持 /name",
            self.snapshot().backend.label()
        )))
    }
}

/// An engine family's process protocol (pi RPC, OMP RPC, DSH ACP).
///
/// `start` blocks only for the bounded handshake that produces a real native locator, so a chat is
/// never bound to a session that does not exist.
pub trait AgentRuntime: Send + Sync {
    fn backend(&self) -> BackendKind;
    fn capabilities(&self) -> Capabilities;
    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>>;
}
