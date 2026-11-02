//! Real-engine smokes: switching the model of a running session without persisting it.

use std::{fs, sync::Arc};

use thinkrail_core::{AgentRuntime, BackendKind, ErrorKind, StartSession, ThinkingLevel};

use super::{fixture, request};
use crate::{OmpRuntime, PiRuntime, projection::tests::Events};

/// Starts a session on the config's first model, switches to its second keyless placeholder model
/// at `high` and checks the engine's `get_state` (via the snapshot) reports it and that nothing in
/// the engine config (outside the model list and session transcripts) records it. No prompt is sent.
fn switch_round(runtime: &dyn AgentRuntime, request: StartSession) {
    let config = request
        .config_root
        .clone()
        .expect("an isolated config root");
    let events = Arc::new(Events::default());
    let session = runtime
        .start(request, events)
        .unwrap_or_else(|e| panic!("start: {e}"));
    assert!(session.snapshot().capabilities.model_switch);
    let before = session.snapshot().model.expect("a starting model");
    assert_eq!(
        (before.provider.as_deref(), before.id.as_str()),
        (Some("smoke"), "smoke-a")
    );
    session
        .set_model("smoke", "smoke-b", Some(ThinkingLevel::High))
        .unwrap_or_else(|e| panic!("set_model: {e}"));
    let after = session.snapshot().model.expect("a model after the switch");
    assert_eq!(
        (after.provider.as_deref(), after.id.as_str(), after.effort),
        (Some("smoke"), "smoke-b", Some(ThinkingLevel::High))
    );
    let missing = session
        .set_model("smoke", "no-such-model", None)
        .unwrap_err();
    assert_eq!(missing.kind(), Some(ErrorKind::External), "{missing}");
    assert_eq!(
        session.snapshot().model,
        Some(after),
        "a refused switch keeps the model"
    );
    session.dispose().unwrap();
    assert_not_persisted(&config, "smoke-b");
}

/// Fails if a config file mentions `needle`, except the model list, session transcripts and OMP's
/// `agent.db` (its `set_model` records model usage there for "recent models"; the default model
/// role is only written when `persist` is sent, which the adapter never does).
fn assert_not_persisted(dir: &std::path::Path, needle: &str) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if name != "sessions" {
                assert_not_persisted(&path, needle);
            }
        } else if !name.starts_with("models.") && !name.starts_with("agent.db") {
            let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
            assert!(
                !text.contains(needle),
                "{} records the switched model",
                path.display()
            );
        }
    }
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_switches_model_in_a_running_session() {
    let f = fixture();
    fs::write(
        f.config.join("models.json"),
        r#"{"providers":{"smoke":{"baseUrl":"http://127.0.0.1:9/v1","api":"openai-completions","apiKey":"placeholder",
            "models":[{"id":"smoke-a","reasoning":true},{"id":"smoke-b","reasoning":true}]}}}"#,
    )
    .unwrap();
    fs::write(
        f.config.join("settings.json"),
        r#"{"defaultProvider":"smoke","defaultModel":"smoke-a"}"#,
    )
    .unwrap();
    switch_round(
        &PiRuntime::new(),
        request(BackendKind::Pi, "THINKRAIL_SMOKE_PI", &f),
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_switches_model_in_a_running_session() {
    let f = fixture();
    fs::write(
        f.config.join("models.yml"),
        "providers:\n  smoke:\n    baseUrl: http://127.0.0.1:9/v1\n    api: openai-completions\n    auth: none\n    models:\n      - id: smoke-a\n        name: Smoke A\n        reasoning: true\n      - id: smoke-b\n        name: Smoke B\n        reasoning: true\n",
    )
    .unwrap();
    switch_round(
        &OmpRuntime::new(),
        request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f),
    );
}
