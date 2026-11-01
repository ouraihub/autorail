//! Chat bindings (agent-settings 07 §4): which engine a chat runs and its native session.

use serde::{Deserialize, Serialize};

use crate::{BackendKind, ChatId, NativeSessionKey, Result, WorkspaceId};

/// A chat is pinned to the backend it was created with; a new default Agent never rebinds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatBinding {
    pub chat_id: ChatId,
    pub workspace_id: WorkspaceId,
    pub backend: BackendKind,
    pub native_session_key: NativeSessionKey,
}

/// Durable chat bindings. Damaged storage is reported as [`crate::Error::Corrupt`].
pub trait ChatBindingService: Send + Sync {
    /// Inserts or replaces the binding of `binding.chat_id`.
    fn bind(&self, binding: ChatBinding) -> Result<()>;
    /// This workspace's bindings, oldest first (tab order).
    fn bindings(&self, workspace: &WorkspaceId) -> Result<Vec<ChatBinding>>;
    /// Removes a closed chat's binding; the engine's native history is left untouched.
    /// Removing an absent binding succeeds.
    fn unbind(&self, chat: &ChatId) -> Result<()>;
}
