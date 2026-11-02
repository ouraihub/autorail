//! Real-engine smokes of the slash-command list and pi's session rename (no model requests).

use std::{fs, sync::Arc};

use thinkrail_core::{
    AgentRuntime, BackendKind, CommandSource, ContentBlock, Role, SessionPhase, SlashCommand,
};

use super::{fixture, request};
use crate::{OmpRuntime, PiRuntime, projection::tests::Events};

fn print(commands: &[SlashCommand]) {
    for c in commands {
        println!("/{} [{}] {:?}", c.name, c.source.as_str(), c.scope);
    }
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_lists_commands_and_renames_the_session() {
    let f = fixture();
    let session = PiRuntime::new()
        .start(
            request(BackendKind::Pi, "THINKRAIL_SMOKE_PI", &f),
            Arc::new(Events::default()),
        )
        .unwrap_or_else(|e| panic!("start: {e}"));
    let commands = session.snapshot().commands;
    print(&commands);
    assert!(
        commands
            .iter()
            .any(|c| c.name.starts_with("skill:") && c.source == CommandSource::Skill),
        "the staged bundled skills are listed"
    );
    session
        .rename("smoke-name")
        .unwrap_or_else(|e| panic!("rename: {e}"));
    assert_eq!(
        session.snapshot().session_name.as_deref(),
        Some("smoke-name")
    );
    // An empty session has nothing to compact: pi refuses at once (no model call), which must
    // be a transcript note, not an error.
    session
        .compact(None)
        .unwrap_or_else(|e| panic!("compact: {e}"));
    let snapshot = session.snapshot();
    let note = snapshot
        .messages
        .last()
        .and_then(|m| match m.content.first() {
            Some(ContentBlock::Text { text }) if m.role == Role::System => Some(text.clone()),
            _ => None,
        });
    println!("compact note: {note:?}");
    assert!(note.is_some_and(|t| t.starts_with("压缩失败。")));
    assert_eq!(snapshot.phase, SessionPhase::Idle);
    session.dispose().unwrap();
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_lists_its_builtin_commands() {
    let f = fixture();
    // A keyless local placeholder model: OMP refuses to start without a default model.
    fs::write(
        f.config.join("models.yml"),
        "providers:\n  smoke:\n    baseUrl: http://127.0.0.1:9/v1\n    api: openai-completions\n    auth: none\n    models:\n      - id: smoke-model\n        name: Smoke\n",
    )
    .unwrap();
    let session = OmpRuntime::new()
        .start(
            request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f),
            Arc::new(Events::default()),
        )
        .unwrap_or_else(|e| panic!("start: {e}"));
    let commands = session.snapshot().commands;
    print(&commands);
    assert!(
        commands
            .iter()
            .any(|c| c.name == "compact" && c.source == CommandSource::Builtin),
        "OMP lists its text-mode built-ins (an empty list means this OMP lacks get_available_commands)"
    );
    session.dispose().unwrap();
}
