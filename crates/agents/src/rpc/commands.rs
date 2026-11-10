//! Slash-command payloads of the pi-family engines: pi `get_commands` and OMP
//! `get_available_commands` / `available_commands_update`, plus the system note that reports
//! host-run command results.

use serde_json::Value;
use thinkrail_core::{
    CommandScope, CommandSource, ContentBlock, ErrorCode as _, Message, Result, Role, SlashCommand,
};
use tracing::{debug, warn};

use crate::{projection::Projection, session::new_id};

/// pi `get_commands` data: `{commands: [{name, description?, source, sourceInfo: {scope}}]}`.
pub(crate) fn pi_commands(data: &Value) -> Vec<SlashCommand> {
    parse(&data["commands"], |entry| {
        let source = match entry["source"].as_str()? {
            "extension" => CommandSource::Extension,
            "prompt" => CommandSource::Prompt,
            "skill" => CommandSource::Skill,
            _ => return None,
        };
        let scope = match entry["sourceInfo"]["scope"].as_str() {
            Some("user") => Some(CommandScope::User),
            Some("project") => Some(CommandScope::Project),
            Some("temporary") => Some(CommandScope::Temporary),
            _ => None,
        };
        Some((source, None, scope))
    })
}

/// OMP command array: `[{name, aliases?, description?, input?: {hint}, subcommands?, source}]`.
pub(crate) fn omp_commands(commands: &Value) -> Vec<SlashCommand> {
    parse(commands, |entry| {
        let source = match entry["source"].as_str()? {
            "builtin" => CommandSource::Builtin,
            "skill" => CommandSource::Skill,
            "extension" => CommandSource::Extension,
            "custom" => CommandSource::Custom,
            "mcp_prompt" => CommandSource::McpPrompt,
            "file" => CommandSource::File,
            _ => return None,
        };
        Some((source, non_empty(&entry["input"]["hint"]), None))
    })
}

type Fields = (CommandSource, Option<String>, Option<CommandScope>);

/// Keeps entries with a name and a known source, in engine order.
fn parse(commands: &Value, fields: impl Fn(&Value) -> Option<Fields>) -> Vec<SlashCommand> {
    let mut out = Vec::new();
    for entry in commands.as_array().into_iter().flatten() {
        let Some(name) = non_empty(&entry["name"]) else {
            debug!("slash command without a name skipped");
            continue;
        };
        let Some((source, hint, scope)) = fields(entry) else {
            debug!("agent.commands.unknown_source");
            continue;
        };
        out.push(SlashCommand {
            name,
            description: non_empty(&entry["description"]),
            hint,
            source,
            scope,
        });
    }
    out
}

fn non_empty(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// `get_state.sessionName` (pi and OMP).
pub(crate) fn session_name(state: &Value) -> Option<String> {
    non_empty(&state["sessionName"])
}

/// The command list of a handshake; an engine that cannot list them still starts.
pub(crate) fn or_empty(listed: Result<Vec<SlashCommand>>) -> Vec<SlashCommand> {
    listed.unwrap_or_else(|error| {
        warn!(code = error.code(), "slash commands unavailable");
        Vec::new()
    })
}

/// Appends a system note (a host-run command's result) under a fresh id.
pub(crate) fn note(p: &mut Projection, text: String) {
    p.upsert(Message {
        id: format!("note-{}", new_id()),
        role: Role::System,
        content: vec![ContentBlock::Text { text }],
    });
}

#[cfg(test)]
mod tests;
