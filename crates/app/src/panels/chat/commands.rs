//! Slash commands of the composer (native-multi-agent 07), pure logic: when the `/` menu opens,
//! which rows it lists, and how a submitted host built-in (`/compact`, `/name`, `/model`) parses
//! and resolves against the chat's model catalog. Everything else goes to the engine as typed.

use thinkrail_core::{
    BackendKind, Capabilities, CommandSource, ModelChoice, ModelInfo, SlashCommand, ThinkingLevel,
};

use crate::strings::chat as s;

#[cfg(test)]
mod tests;

/// The effort words `/model` accepts as its last argument.
const EFFORTS: [(&str, ThinkingLevel); 7] = [
    ("off", ThinkingLevel::Off),
    ("minimal", ThinkingLevel::Minimal),
    ("low", ThinkingLevel::Low),
    ("medium", ThinkingLevel::Medium),
    ("high", ThinkingLevel::High),
    ("xhigh", ThinkingLevel::Xhigh),
    ("max", ThinkingLevel::Max),
];

/// Commands the host runs itself instead of sending them to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostCommand {
    Compact,
    Name,
    Model,
}

impl HostCommand {
    const ALL: [Self; 3] = [Self::Compact, Self::Name, Self::Model];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Name => "name",
            Self::Model => "model",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Compact => s::CMD_COMPACT,
            Self::Name => s::CMD_NAME,
            Self::Model => s::CMD_MODEL,
        }
    }

    fn offered(self, caps: &Capabilities) -> bool {
        match self {
            Self::Compact => caps.compact,
            Self::Name => caps.rename,
            Self::Model => caps.model_switch,
        }
    }
}

/// The host built-ins a chat with these capabilities offers, in menu order.
pub(super) fn host_commands(caps: &Capabilities) -> Vec<HostCommand> {
    HostCommand::ALL
        .into_iter()
        .filter(|command| command.offered(caps))
        .collect()
}

/// One row of the `/` menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
}

/// The text after `/` while the whole draft is one `/word` (no whitespace anywhere).
pub(super) fn slash_query(draft: &str) -> Option<&str> {
    draft
        .strip_prefix('/')
        .filter(|_| !draft.chars().any(char::is_whitespace))
}

/// Host built-ins first, then the engine's commands minus those a host built-in shadows (07 C6).
pub(super) fn catalog(
    backend: BackendKind,
    host: &[HostCommand],
    engine: &[SlashCommand],
) -> Vec<Row> {
    let builtin = s::builtin_source(backend.label());
    let hosted = host.iter().map(|command| Row {
        name: command.name().to_owned(),
        description: Some(command.description().to_owned()),
        source: builtin.clone(),
    });
    let engine = engine
        .iter()
        .filter(|command| !host.iter().any(|h| h.name() == command.name))
        .map(|command| Row {
            name: command.name.clone(),
            description: command.description.clone(),
            source: source_label(command, &builtin),
        });
    hosted.chain(engine).collect()
}

/// 07 C7: built-ins read `{engine} / 内置`, others `{source}/{scope}` or just `{source}`.
fn source_label(command: &SlashCommand, builtin: &str) -> String {
    match command.source {
        CommandSource::Builtin => builtin.to_owned(),
        CommandSource::Extension
        | CommandSource::Prompt
        | CommandSource::Skill
        | CommandSource::Custom
        | CommandSource::McpPrompt
        | CommandSource::File => match command.scope {
            Some(scope) => format!("{}/{}", command.source.as_str(), scope.as_str()),
            None => command.source.as_str().to_owned(),
        },
    }
}

/// Rows whose name contains `query` (case-insensitive), in catalog order. All of them: the menu
/// scrolls (a deliberate change from the original's 8-match cap, 07 C11).
pub(super) fn filter(rows: Vec<Row>, query: &str) -> Vec<Row> {
    let query = query.to_lowercase();
    rows.into_iter()
        .filter(|row| row.name.to_lowercase().contains(&query))
        .collect()
}

/// A submitted draft that names a host built-in this chat offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Submit {
    Compact {
        instructions: Option<String>,
    },
    Name {
        title: String,
    },
    Model {
        query: Vec<String>,
        effort: Option<ThinkingLevel>,
    },
}

/// `None` → the text is an ordinary prompt; `Err` → a host built-in with unusable arguments.
pub(super) fn parse(text: &str, host: &[HostCommand]) -> Option<Result<Submit, String>> {
    let rest = text.strip_prefix('/')?;
    let (head, args) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(head, args)| (head, args.trim()));
    let command = host.iter().copied().find(|c| c.name() == head)?;
    Some(match command {
        HostCommand::Compact => Ok(Submit::Compact {
            instructions: Some(args.to_owned()).filter(|a| !a.is_empty()),
        }),
        HostCommand::Name if args.is_empty() => Err(s::NAME_EMPTY.to_owned()),
        HostCommand::Name => Ok(Submit::Name {
            title: args.to_owned(),
        }),
        HostCommand::Model => {
            let mut query: Vec<String> = args.split_whitespace().map(str::to_owned).collect();
            let effort = query.last().and_then(|last| effort_of(last));
            if effort.is_some() {
                query.pop();
            }
            if query.is_empty() && effort.is_none() {
                Err(s::MODEL_USAGE.to_owned())
            } else {
                Ok(Submit::Model { query, effort })
            }
        }
    })
}

fn effort_of(token: &str) -> Option<ThinkingLevel> {
    EFFORTS
        .iter()
        .find(|(word, _)| word.eq_ignore_ascii_case(token))
        .map(|(_, level)| *level)
}

fn effort_word(level: ThinkingLevel) -> &'static str {
    EFFORTS
        .iter()
        .find(|(_, l)| *l == level)
        .map_or("", |(word, _)| word)
}

/// What a resolved `/model` changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModelChange {
    /// Switch to `choice` with `effort` (already one of its efforts, or `None` when it has none).
    Switch {
        choice: ModelChoice,
        effort: Option<ThinkingLevel>,
    },
    /// Keep the current model, change only its effort.
    Effort(ThinkingLevel),
}

/// Resolves `/model [query…] [effort]` over `catalog`; `current` is the session's model.
pub(super) fn resolve_model(
    query: &[String],
    effort: Option<ThinkingLevel>,
    catalog: &[ModelChoice],
    current: Option<&ModelInfo>,
) -> Result<ModelChange, String> {
    if query.is_empty() {
        let Some(effort) = effort else {
            return Err(s::MODEL_USAGE.to_owned());
        };
        let choice = current
            .and_then(|model| {
                catalog.iter().find(|c| {
                    c.route == model.provider.as_deref().unwrap_or_default()
                        && c.model.id == model.id
                })
            })
            .ok_or_else(|| s::MODEL_UNKNOWN.to_owned())?;
        return if choice.model.efforts.contains(&effort) {
            Ok(ModelChange::Effort(effort))
        } else {
            Err(s::effort_unsupported(effort_word(effort)))
        };
    }
    let choice = pick(query, catalog)?;
    let effort = match effort {
        Some(effort) if choice.model.efforts.contains(&effort) => Some(effort),
        Some(effort) => return Err(s::effort_unsupported(effort_word(effort))),
        // Same rule as the header dropdown: keep the current effort when supported.
        None => choice
            .model
            .efforts
            .iter()
            .copied()
            .find(|e| Some(*e) == current.and_then(|m| m.effort))
            .or_else(|| choice.model.efforts.first().copied()),
    };
    Ok(ModelChange::Switch {
        choice: choice.clone(),
        effort,
    })
}

/// Every token must occur in the choice's searchable text; an exact id match wins.
fn pick<'a>(query: &[String], catalog: &'a [ModelChoice]) -> Result<&'a ModelChoice, String> {
    let joined = query.join(" ");
    if let Some(exact) = catalog
        .iter()
        .find(|c| c.model.id.eq_ignore_ascii_case(&joined))
    {
        return Ok(exact);
    }
    let tokens: Vec<String> = query.iter().map(|t| t.to_lowercase()).collect();
    let matches: Vec<&ModelChoice> = catalog
        .iter()
        .filter(|c| {
            let haystack = format!(
                "{} {} {} {}",
                c.provider_name, c.model.name, c.model.id, c.route
            )
            .to_lowercase();
            tokens.iter().all(|t| haystack.contains(t.as_str()))
        })
        .collect();
    match matches.as_slice() {
        [] => Err(s::model_none(&joined)),
        [one] => Ok(one),
        many => Err(s::model_many(
            &many
                .iter()
                .take(5)
                .map(|c| format!("{} · {}", c.provider_name, c.model.name))
                .collect::<Vec<_>>(),
        )),
    }
}
