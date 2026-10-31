use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ChatId, NativeSessionKey, ProfileId, Result, WorkspaceId};

/// The external engine family a profile launches. Each has its own wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Official pi CLI, `--mode rpc` JSONL.
    Pi,
    /// Official OMP CLI, `--mode rpc-ui` JSONL.
    Omp,
    /// DSH ACP JSON-RPC over stdio.
    Dsh,
}

impl BackendKind {
    pub const ALL: [Self; 3] = [Self::Pi, Self::Omp, Self::Dsh];

    pub fn label(self) -> &'static str {
        match self { Self::Pi => "pi", Self::Omp => "OMP", Self::Dsh => "DSH" }
    }
}

/// How the engine process is started. The app never bundles or downloads an engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Launch {
    /// An absolute executable (e.g. `node.exe`) plus argv (e.g. the official CLI entry).
    External { executable: PathBuf, args: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvSource {
    /// Copy the named variable from this app's own launch environment.
    Launch,
}

/// A reference to a variable, never its value: profiles hold no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRef {
    pub name: String,
    pub source: EnvSource,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// User-editable profile fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileInput {
    pub backend_id: BackendKind,
    pub label: String,
    pub enabled: bool,
    /// Engine config dir (`PI_CODING_AGENT_DIR` / `DSH_HOME`); `None` uses the engine default.
    pub config_root: Option<PathBuf>,
    pub launch: Launch,
    pub env_refs: Vec<EnvRef>,
}

/// One immutable stored revision of a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: ProfileId,
    pub config_revision: u64,
    #[serde(flatten)]
    pub input: ProfileInput,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCatalog {
    /// Latest revision of every profile, in creation order.
    pub profiles: Vec<Profile>,
    pub default_profile_id: Option<ProfileId>,
}

/// A chat is pinned to the exact profile revision it was created with; later edits or a new default
/// never rebind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatBinding {
    pub chat_id: ChatId,
    pub workspace_id: WorkspaceId,
    pub profile_id: ProfileId,
    pub config_revision: u64,
    pub native_session_key: NativeSessionKey,
}

/// Durable, revisioned Agent profiles and chat bindings.
///
/// Writes are optimistic: an update names the revision it was based on and fails with
/// [`crate::Error::Conflict`] if another writer moved it. Damaged storage is reported as
/// [`crate::Error::Corrupt`] and is never overwritten.
pub trait ProfileService: Send + Sync {
    fn catalog(&self) -> Result<ProfileCatalog>;
    fn create(&self, input: ProfileInput) -> Result<Profile>;
    fn update(&self, id: &ProfileId, expect_revision: u64, input: ProfileInput) -> Result<Profile>;
    fn delete(&self, id: &ProfileId, expect_revision: u64) -> Result<()>;
    fn set_default(&self, id: Option<&ProfileId>) -> Result<ProfileCatalog>;
    /// Exact stored revision, used to resume a chat with its original configuration.
    fn revision(&self, id: &ProfileId, revision: u64) -> Result<Profile>;

    fn bind(&self, binding: ChatBinding) -> Result<()>;
    fn binding(&self, chat: &ChatId) -> Result<Option<ChatBinding>>;
    fn bindings(&self, workspace: &WorkspaceId) -> Result<Vec<ChatBinding>>;
    /// Removes a closed chat's binding; the engine's native history is left untouched.
    /// Removing an absent binding succeeds.
    fn unbind(&self, chat: &ChatId) -> Result<()>;
}
