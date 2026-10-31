//! Real engine handshakes (`#[ignore]`): official CLIs, isolated config dirs, no credentials and no
//! model requests. Point them at installed engines:
//!
//! - `THINKRAIL_SMOKE_PI` = `<node.exe>|<pi dist/bundle/cli.js>`
//! - `THINKRAIL_SMOKE_OMP` = `<omp.exe>` or `<bun.exe>|<omp src/cli.ts>`
//! - `THINKRAIL_SMOKE_DSH` = `<node.exe ≥ 24.9>|<dsh lib/bin.js>`

use std::{fs, path::PathBuf, sync::Arc};

use tempfile::TempDir;
use thinkrail_core::{
    AgentRuntime, BackendKind, ErrorKind, Launch, Profile, ProfileInput, SessionPhase, StartSession,
};

use crate::{DshRuntime, OmpRuntime, PiRuntime, projection::tests::Events};

fn launch(var: &str) -> Launch {
    let value = std::env::var(var).unwrap_or_else(|_| panic!("set {var} to run this smoke test"));
    let mut parts = value.split('|').map(PathBuf::from);
    let executable = parts.next().unwrap_or_default();
    let args = parts.map(|p| p.to_string_lossy().into_owned()).collect();
    Launch::External { executable, args }
}

fn profile(backend: BackendKind, launch: Launch, config_root: PathBuf) -> Profile {
    Profile {
        id: "smoke".into(),
        config_revision: 1,
        input: ProfileInput {
            backend_id: backend,
            label: "smoke".into(),
            enabled: true,
            config_root: Some(config_root),
            launch,
            env_refs: Vec::new(),
        },
    }
}

struct Fixture {
    _root: TempDir,
    config: PathBuf,
    cwd: PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let cwd = root.path().join("工作区 smoke");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    Fixture { _root: root, config, cwd }
}

/// Start → real locator → idle → dispose → closed; then resume the same native session.
fn round_trip(runtime: &dyn AgentRuntime, profile: Profile, cwd: PathBuf, resume: bool) {
    let events = Arc::new(Events::default());
    let request = StartSession { chat: "smoke".into(), workspace: "w".into(), profile: profile.clone(), cwd: cwd.clone(), resume: None, host_tools: None };
    let session = runtime.start(request, events.clone()).unwrap_or_else(|e| panic!("start: {e}"));
    let snapshot = session.snapshot();
    let key = snapshot.native_session_key.clone().expect("a real native locator");
    assert!(!key.as_str().is_empty());
    assert_eq!(snapshot.phase, SessionPhase::Idle);
    session.dispose().unwrap();
    assert_eq!(session.snapshot().phase, SessionPhase::Closed);
    assert!(session.snapshot().last_error.is_none(), "graceful dispose is not an error");
    assert_eq!(session.prompt("x").unwrap_err().kind(), Some(ErrorKind::SessionClosed));
    if resume {
        let request = StartSession { chat: "smoke".into(), workspace: "w".into(), profile, cwd, resume: Some(key.clone()), host_tools: None };
        let resumed = runtime.start(request, events).unwrap_or_else(|e| panic!("resume: {e}"));
        assert_eq!(resumed.snapshot().native_session_key, Some(key));
        resumed.dispose().unwrap();
    }
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_rpc_handshake() {
    let f = fixture();
    round_trip(&PiRuntime::new(), profile(BackendKind::Pi, launch("THINKRAIL_SMOKE_PI"), f.config.clone()), f.cwd.clone(), false);
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_without_a_model_reports_a_readable_start_error() {
    let f = fixture();
    let profile = profile(BackendKind::Omp, launch("THINKRAIL_SMOKE_OMP"), f.config.clone());
    let request = StartSession { chat: "smoke".into(), workspace: "w".into(), profile, cwd: f.cwd.clone(), resume: None, host_tools: None };
    let error = OmpRuntime::new().start(request, Arc::new(Events::default())).err().expect("start must fail");
    assert_eq!(error.kind(), Some(ErrorKind::BackendUnavailable));
    assert!(error.to_string().contains("models.yml"), "{error}");
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_rpc_ui_v2_handshake() {
    let f = fixture();
    // A keyless local placeholder model: OMP refuses to start without a default model.
    fs::write(
        f.config.join("models.yml"),
        "providers:\n  smoke:\n    baseUrl: http://127.0.0.1:9/v1\n    api: openai-completions\n    auth: none\n    models:\n      - id: smoke-model\n        name: Smoke\n",
    )
    .unwrap();
    round_trip(&OmpRuntime::new(), profile(BackendKind::Omp, launch("THINKRAIL_SMOKE_OMP"), f.config.clone()), f.cwd.clone(), false);
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_DSH"]
fn dsh_acp_handshake_and_resume() {
    let f = fixture();
    round_trip(&DshRuntime::new(), profile(BackendKind::Dsh, launch("THINKRAIL_SMOKE_DSH"), f.config.clone()), f.cwd.clone(), true);
}
