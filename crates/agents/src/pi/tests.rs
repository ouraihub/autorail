use std::{ffi::OsString, fs};

use thinkrail_core::{BackendKind, EngineRouting};

use super::launch_spec;
use crate::{
    process::LaunchSpec,
    projection_engine,
    testing::{Fixture, routing_vars, sample_routing, secret_routing},
};

fn launch(
    f: &Fixture,
    resume: Option<&str>,
    host_tools: bool,
    routing: EngineRouting,
) -> (Vec<String>, Vec<(String, OsString)>) {
    let request = f.request(BackendKind::Pi, resume, host_tools, routing);
    let LaunchSpec {
        host_args,
        host_env,
        cwd,
        config_var,
        ..
    } = launch_spec(&request).unwrap();
    assert_eq!((cwd, config_var), (f.cwd(), "PI_CODING_AGENT_DIR"));
    (host_args, host_env)
}

fn host_vars(f: &Fixture) -> Vec<(String, OsString)> {
    vec![
        ("THINKRAIL_HOST_EXE".into(), f.exe().into_os_string()),
        ("THINKRAIL_HOST_PIPE".into(), "pipe-1".into()),
        ("THINKRAIL_HOST_TOKEN".into(), "token-1".into()),
    ]
}

/// Every launch loads the workflow (rules in the extension, bundled skills via `--skill`); host
/// tools add only the MCP server and its variables.
#[test]
fn every_launch_loads_the_workflow_extension_and_skills() {
    let f = Fixture::new();
    let (extension, skills) = (f.support_arg("pi-thinkrail.js"), f.support_arg("skills"));
    let (args, env) = launch(&f, Some("s.jsonl"), false, EngineRouting::default());
    assert_eq!(
        args,
        [
            "--mode",
            "rpc",
            "-e",
            extension.as_str(),
            "--skill",
            skills.as_str(),
            "--session",
            "s.jsonl"
        ]
    );
    assert!(env.is_empty());
    let text = fs::read_to_string(&extension).unwrap();
    assert!(
        text.contains(r#"sections["pi-thinkrail-workflow"]"#)
            && !text.contains("registerMcpServer")
    );
    assert!(
        f.support()
            .join("skills")
            .join("setting-up-a-project")
            .join("SKILL.md")
            .is_file()
    );

    let (_, env) = launch(&f, Some("s.jsonl"), true, EngineRouting::default());
    assert_eq!(env, host_vars(&f));
    assert!(
        fs::read_to_string(&extension)
            .unwrap()
            .contains(r#"pi.registerMcpServer("thinkrail", {"#)
    );
}

#[test]
fn routing_env_follows_the_host_variables_and_never_reaches_files() {
    let f = Fixture::new();
    let routing = secret_routing();
    let (args, env) = launch(&f, None, true, routing.clone());
    assert_eq!(args[..3], ["--mode", "rpc", "-e"]);
    let mut expected = host_vars(&f);
    expected.extend(routing_vars(&routing));
    assert_eq!(env, expected);
    assert_eq!(
        launch(&f, None, false, routing.clone()).1,
        routing_vars(&routing)
    );
    f.assert_no_secret_written();
}

#[test]
fn routing_adds_the_extension_and_the_default_model_for_new_sessions_only() {
    let f = Fixture::new();
    let routing = sample_routing();
    let (extension, skills) = (f.support_arg("pi-thinkrail.js"), f.support_arg("skills"));
    let (args, env) = launch(&f, None, false, routing.clone());
    assert_eq!(
        args,
        [
            "--mode",
            "rpc",
            "-e",
            extension.as_str(),
            "--skill",
            skills.as_str(),
            "--model",
            "thinkrail-1a2b3c4d-am/claude-x",
            "--thinking",
            "high"
        ]
    );
    assert_eq!(env, routing_vars(&routing));
    assert_eq!(
        fs::read_to_string(&extension).unwrap(),
        projection_engine::pi_extension(false, &routing)
    );

    let (args, _) = launch(&f, Some("s.jsonl"), true, routing.clone());
    assert_eq!(
        args,
        [
            "--mode",
            "rpc",
            "-e",
            extension.as_str(),
            "--skill",
            skills.as_str(),
            "--session",
            "s.jsonl"
        ]
    );
    assert_eq!(
        fs::read_to_string(&extension).unwrap(),
        projection_engine::pi_extension(true, &routing)
    );
    f.assert_no_secret_written();
}
