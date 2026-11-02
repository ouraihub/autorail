use serde_json::json;
use thinkrail_core::{CommandScope, CommandSource};

use super::*;

#[test]
fn pi_commands_keep_known_entries_in_order() {
    let data = json!({"commands": [
        {"name": "skill:review", "description": "Review", "source": "skill",
         "sourceInfo": {"scope": "temporary", "path": "x"}},
        {"name": "", "source": "skill"},
        {"name": "odd", "source": "builtin"},
        {"source": "prompt"},
        {"name": "fix", "source": "prompt", "sourceInfo": {"scope": "project"}},
        {"name": "ext", "description": "", "source": "extension"}
    ]});
    let commands = pi_commands(&data);
    let names: Vec<_> = commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["skill:review", "fix", "ext"]);
    assert_eq!(commands[0].source, CommandSource::Skill);
    assert_eq!(commands[0].scope, Some(CommandScope::Temporary));
    assert_eq!(commands[0].description.as_deref(), Some("Review"));
    assert_eq!(commands[1].scope, Some(CommandScope::Project));
    assert_eq!(commands[2].description, None);
    assert_eq!(commands[2].scope, None);
    assert!(commands.iter().all(|c| c.hint.is_none()));
    assert!(pi_commands(&json!({})).is_empty());
}

#[test]
fn omp_commands_map_every_source_and_hint() {
    let data = json!([
        {"name": "compact", "aliases": ["c"], "description": "Compact", "source": "builtin",
         "input": {"hint": "[instructions]"}, "subcommands": []},
        {"name": "skill:x", "source": "skill"},
        {"name": "e", "source": "extension"},
        {"name": "c", "source": "custom"},
        {"name": "m", "source": "mcp_prompt"},
        {"name": "f", "source": "file"},
        {"name": "p", "source": "prompt"},
        {"name": "n"}
    ]);
    let commands = omp_commands(&data);
    let sources: Vec<_> = commands.iter().map(|c| c.source).collect();
    assert_eq!(
        sources,
        [
            CommandSource::Builtin,
            CommandSource::Skill,
            CommandSource::Extension,
            CommandSource::Custom,
            CommandSource::McpPrompt,
            CommandSource::File,
        ]
    );
    assert_eq!(commands[0].hint.as_deref(), Some("[instructions]"));
    assert!(commands.iter().all(|c| c.scope.is_none()));
}
