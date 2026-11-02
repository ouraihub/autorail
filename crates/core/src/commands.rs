//! Slash commands an engine offers in the chat composer (native-multi-agent 07).

use serde::{Deserialize, Serialize};

/// What registered a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandSource {
    /// Part of the engine itself (OMP's text-mode built-ins).
    Builtin,
    /// Registered by an engine extension.
    Extension,
    /// A prompt template file.
    Prompt,
    /// A skill; pi and OMP name it `skill:<name>`, DSH `<name>`.
    Skill,
    /// An OMP custom (TypeScript) command.
    Custom,
    /// A prompt exposed by an MCP server (OMP).
    McpPrompt,
    /// An OMP Markdown slash-command file.
    File,
}

impl CommandSource {
    /// The engine's own spelling, shown as the row's source label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Extension => "extension",
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Custom => "custom",
            Self::McpPrompt => "mcp_prompt",
            Self::File => "file",
        }
    }
}

/// Where the resource behind a command lives (pi `sourceInfo.scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandScope {
    User,
    Project,
    /// Loaded for this run only (CLI flags, the host's generated extension).
    Temporary,
}

impl CommandScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Temporary => "temporary",
        }
    }
}

/// One command the engine runs when the prompt starts with `/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// Without the leading `/`.
    pub name: String,
    pub description: Option<String>,
    /// What to type after the name (OMP `input.hint`).
    pub hint: Option<String>,
    pub source: CommandSource,
    /// `None` when the engine does not report it (OMP, DSH).
    pub scope: Option<CommandScope>,
}
