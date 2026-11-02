//! Query detection, menu merge/filter, built-in parsing and `/model` resolution.

use thinkrail_core::{
    BackendKind, Capabilities, CommandScope, CommandSource, ModelChoice, ModelInfo, ProjectedModel,
    ProviderId, SlashCommand, ThinkingLevel,
};

use super::{
    HostCommand, ModelChange, Row, Submit, catalog, filter, host_commands, parse, resolve_model,
    slash_query,
};
use crate::strings::chat as s;

fn command(name: &str, source: CommandSource, scope: Option<CommandScope>) -> SlashCommand {
    SlashCommand {
        name: name.into(),
        description: Some(format!("{name} desc")),
        hint: None,
        source,
        scope,
    }
}

fn caps(compact: bool, rename: bool, model_switch: bool) -> Capabilities {
    Capabilities {
        compact,
        rename,
        model_switch,
        ..Capabilities::default()
    }
}

fn choice(
    provider: &str,
    route: &str,
    id: &str,
    name: &str,
    efforts: &[ThinkingLevel],
) -> ModelChoice {
    ModelChoice {
        provider: ProviderId::from(provider),
        provider_name: provider.into(),
        route: route.into(),
        model: ProjectedModel {
            id: id.into(),
            name: name.into(),
            vision: false,
            reasoning: !efforts.is_empty(),
            efforts: efforts.to_vec(),
            context_window: 200_000,
            max_tokens: 32_000,
        },
    }
}

fn models() -> Vec<ModelChoice> {
    use ThinkingLevel::{High, Low, Medium};
    vec![
        choice(
            "Anthropic",
            "anthropic",
            "claude-opus-4",
            "Claude Opus 4",
            &[Low, Medium, High],
        ),
        choice(
            "Anthropic",
            "anthropic",
            "claude-sonnet-4",
            "Claude Sonnet 4",
            &[Low, High],
        ),
        choice("OpenAI", "openai", "gpt-5", "GPT-5", &[Low, Medium, High]),
        choice("OpenAI", "openai", "gpt-5-mini", "GPT-5 mini", &[]),
    ]
}

fn names(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

fn model_args(
    query: &[&str],
    effort: Option<ThinkingLevel>,
) -> (Vec<String>, Option<ThinkingLevel>) {
    (query.iter().map(|q| (*q).to_owned()).collect(), effort)
}

#[test]
fn query_needs_a_leading_slash_and_no_whitespace() {
    assert_eq!(slash_query("/"), Some(""));
    assert_eq!(slash_query("/com"), Some("com"));
    assert_eq!(slash_query("/skill:x"), Some("skill:x"));
    assert_eq!(slash_query("/compact "), None);
    assert_eq!(slash_query("/a\nb"), None);
    assert_eq!(slash_query("/a\tb"), None);
    assert_eq!(slash_query(" /a"), None);
    assert_eq!(slash_query("hi /a"), None);
    assert_eq!(slash_query(""), None);
}

#[test]
fn host_built_ins_follow_capabilities() {
    assert_eq!(
        host_commands(&caps(true, true, true)),
        [HostCommand::Compact, HostCommand::Name, HostCommand::Model]
    );
    assert_eq!(
        host_commands(&caps(false, false, true)),
        [HostCommand::Model]
    );
    assert!(host_commands(&caps(false, false, false)).is_empty());
}

#[test]
fn merge_puts_host_first_drops_shadowed_engine_commands_and_labels_sources() {
    let engine = [
        command("compact", CommandSource::Builtin, None),
        command(
            "review",
            CommandSource::Extension,
            Some(CommandScope::Temporary),
        ),
        command("skill:plan", CommandSource::Skill, None),
        command("rename", CommandSource::Builtin, None),
        command("name", CommandSource::Prompt, Some(CommandScope::User)),
    ];
    let host = host_commands(&caps(true, true, true));
    let rows = catalog(BackendKind::Pi, &host, &engine);
    assert_eq!(
        names(&rows),
        ["compact", "name", "model", "review", "skill:plan", "rename"]
    );
    let builtin = s::builtin_source("pi");
    assert_eq!(rows[0].source, builtin);
    assert_eq!(rows[0].description.as_deref(), Some(s::CMD_COMPACT));
    assert_eq!(rows[3].source, "extension/temporary");
    assert_eq!(rows[4].source, "skill");
    assert_eq!(rows[5].source, builtin);

    // OMP offers no host compact: its own /compact stays, labeled as built-in.
    let rows = catalog(
        BackendKind::Omp,
        &host_commands(&caps(false, false, true)),
        &engine,
    );
    assert_eq!(names(&rows)[..2], ["model", "compact"]);
    assert_eq!(rows[1].source, s::builtin_source("OMP"));
    assert_eq!(rows[1].description.as_deref(), Some("compact desc"));
}

#[test]
fn filter_is_case_insensitive_ordered_and_uncapped() {
    let engine: Vec<SlashCommand> = (0..12)
        .map(|i| command(&format!("Tool{i}"), CommandSource::Extension, None))
        .collect();
    let rows = catalog(
        BackendKind::Pi,
        &host_commands(&caps(true, true, true)),
        &engine,
    );
    let all = filter(rows.clone(), "");
    assert_eq!(all.len(), 15);
    assert_eq!(names(&all)[..4], ["compact", "name", "model", "Tool0"]);
    assert_eq!(names(&all)[14], "Tool11");
    let tools = filter(rows.clone(), "tOOL1");
    assert_eq!(names(&tools), ["Tool1", "Tool10", "Tool11"]);
    let middle = filter(rows.clone(), "m");
    assert_eq!(names(&middle), ["compact", "name", "model"]);
    assert!(filter(rows, "zzz").is_empty());
}

#[test]
fn parse_only_takes_offered_host_built_ins() {
    let all = host_commands(&caps(true, true, true));
    assert_eq!(
        parse("/compact", &all),
        Some(Ok(Submit::Compact { instructions: None }))
    );
    assert_eq!(
        parse("/compact keep the API notes", &all),
        Some(Ok(Submit::Compact {
            instructions: Some("keep the API notes".into())
        }))
    );
    assert_eq!(
        parse("/name  新标题 ", &all),
        Some(Ok(Submit::Name {
            title: "新标题".into()
        }))
    );
    assert_eq!(parse("/name", &all), Some(Err(s::NAME_EMPTY.to_owned())));
    assert_eq!(parse("/name   ", &all), Some(Err(s::NAME_EMPTY.to_owned())));
    assert_eq!(parse("/model", &all), Some(Err(s::MODEL_USAGE.to_owned())));
    assert_eq!(
        parse("/model opus HIGH", &all),
        Some(Ok(Submit::Model {
            query: vec!["opus".into()],
            effort: Some(ThinkingLevel::High)
        }))
    );
    assert_eq!(
        parse("/model xhigh", &all),
        Some(Ok(Submit::Model {
            query: Vec::new(),
            effort: Some(ThinkingLevel::Xhigh)
        }))
    );
    assert_eq!(parse("/compactor", &all), None);
    assert_eq!(parse("/review it", &all), None);
    assert_eq!(parse("hello /compact", &all), None);
    // Not offered → the engine gets the text as typed.
    let omp = host_commands(&caps(false, false, true));
    assert_eq!(parse("/compact", &omp), None);
    assert_eq!(parse("/name x", &omp), None);
}

#[test]
fn model_resolution_counts_matches() {
    let catalog = models();
    let current = ModelInfo {
        provider: Some("anthropic".into()),
        id: "claude-sonnet-4".into(),
        effort: Some(ThinkingLevel::Medium),
    };
    let resolve = |query: &[&str], effort| {
        let (query, effort) = model_args(query, effort);
        resolve_model(&query, effort, &catalog, Some(&current))
    };

    assert_eq!(resolve(&["llama"], None), Err(s::model_none("llama")));
    assert_eq!(
        resolve(&["claude"], None),
        Err(s::model_many(&[
            "Anthropic · Claude Opus 4".into(),
            "Anthropic · Claude Sonnet 4".into()
        ]))
    );
    // Multi-word: every token must match; current effort kept when supported.
    assert_eq!(
        resolve(&["anthropic", "OPUS"], None),
        Ok(ModelChange::Switch {
            choice: catalog[0].clone(),
            effort: Some(ThinkingLevel::Medium)
        })
    );
    // Current effort unsupported → first effort.
    assert_eq!(
        resolve(&["sonnet"], None),
        Ok(ModelChange::Switch {
            choice: catalog[1].clone(),
            effort: Some(ThinkingLevel::Low)
        })
    );
    // `gpt-5` is a substring of `gpt-5-mini` too, but the exact id wins.
    assert_eq!(
        resolve(&["GPT-5"], Some(ThinkingLevel::High)),
        Ok(ModelChange::Switch {
            choice: catalog[2].clone(),
            effort: Some(ThinkingLevel::High)
        })
    );
    assert_eq!(
        resolve(&["mini"], None),
        Ok(ModelChange::Switch {
            choice: catalog[3].clone(),
            effort: None
        })
    );
    assert_eq!(
        resolve(&["opus"], Some(ThinkingLevel::Max)),
        Err(s::effort_unsupported("max"))
    );
}

#[test]
fn effort_only_changes_the_current_model() {
    let catalog = models();
    let current = ModelInfo {
        provider: Some("anthropic".into()),
        id: "claude-sonnet-4".into(),
        effort: Some(ThinkingLevel::Low),
    };
    assert_eq!(
        resolve_model(&[], Some(ThinkingLevel::High), &catalog, Some(&current)),
        Ok(ModelChange::Effort(ThinkingLevel::High))
    );
    assert_eq!(
        resolve_model(&[], Some(ThinkingLevel::Medium), &catalog, Some(&current)),
        Err(s::effort_unsupported("medium"))
    );
    assert_eq!(
        resolve_model(&[], Some(ThinkingLevel::High), &catalog, None),
        Err(s::MODEL_UNKNOWN.to_owned())
    );
    assert_eq!(
        resolve_model(&[], None, &catalog, Some(&current)),
        Err(s::MODEL_USAGE.to_owned())
    );
}
