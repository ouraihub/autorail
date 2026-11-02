use serde::{Deserialize, Serialize};

use crate::{Result, TerminalRuntimeId, WorkspaceId};

/// Explicit shell choice; a missing shell is an error, never a silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    PowerShell,
    Pwsh,
    Cmd,
    Bash,
}

/// A persisted terminal tab. The PTY behind it is not persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalTab {
    pub workspace: WorkspaceId,
    pub tab_key: String,
    pub title: String,
    pub runtime: Option<TerminalRuntimeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub workspace: WorkspaceId,
    pub tab_key: String,
    pub shell: Shell,
    pub size: GridSize,
}

/// Replay is captured under the same lock that assigns byte offsets, so a client applies live
/// chunks only when `byte_offset >= replay_end_offset` and never duplicates output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub runtime: TerminalRuntimeId,
    pub created: bool,
    pub replay: String,
    pub replay_truncated: bool,
    pub replay_end_offset: u64,
    pub size: GridSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Data {
        runtime: TerminalRuntimeId,
        byte_offset: u64,
        data: String,
    },
    Exit {
        runtime: TerminalRuntimeId,
        exit_code: Option<u32>,
    },
}

/// Receives PTY output. May block briefly: a full queue is the backpressure on the reader.
pub trait TerminalSink: Send + Sync {
    fn publish(&self, event: TerminalEvent);
}

pub trait TerminalService: Send + Sync {
    fn tabs(&self, workspace: &WorkspaceId) -> Result<Vec<TerminalTab>>;
    fn reserve(&self, workspace: &WorkspaceId, tab_key: &str, title: &str) -> Result<TerminalTab>;
    /// Reuses a live PTY for the tab or starts one in the workspace root.
    fn attach(&self, request: AttachRequest) -> Result<Attached>;
    fn write(&self, runtime: &TerminalRuntimeId, data: &str) -> Result<()>;
    fn resize(&self, runtime: &TerminalRuntimeId, size: GridSize) -> Result<()>;
    /// Ends the shell and forgets the tab.
    fn close(&self, workspace: &WorkspaceId, tab_key: &str) -> Result<()>;
    /// Ends every shell; used on app exit.
    fn shutdown(&self);
}
