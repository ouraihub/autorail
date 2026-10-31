//! Projects, worktree workspaces, contained file reads and Git: implements `WorkspaceService`.

mod git;
mod paths;
mod registry;
mod service;
#[cfg(test)]
mod tests;

pub use service::GitWorkspaceService;
