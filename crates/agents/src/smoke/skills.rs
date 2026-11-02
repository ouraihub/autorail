//! Real-engine smokes: the bundled setup skill expands into the engine's first model request
//! (sent to a local capture provider that answers 500, so no model cost).

use std::{fs, sync::Arc};

use thinkrail_core::{AgentRuntime, BackendKind, StartSession};

use super::{Fixture, fixture, placeholder_routing, request};
use crate::{DshRuntime, OmpRuntime, PiRuntime, projection::tests::Events};

/// A loopback "provider" that records each request body under `dir/request-<n>.json` and answers
/// 500, so the turn fails right after the engine built its first model request (no model cost).
fn capture_provider(dir: &std::path::Path) -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = dir.to_owned();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let _ = fs::write(dir.join(format!("request-{n}.json")), &body);
            let reply = r#"{"error":{"message":"smoke capture","type":"server_error"}}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

/// Sends the setup skill command and returns the first model request the engine built.
fn skill_round(
    runtime: &dyn AgentRuntime,
    request: StartSession,
    f: &Fixture,
    command: &str,
) -> String {
    let dir = f.config.parent().unwrap().to_owned();
    let mut routing = placeholder_routing(&dir);
    routing.mcp.clear();
    routing.providers[0].headers.clear();
    routing.providers[0].base_url = capture_provider(&dir);
    let request = StartSession {
        routing,
        support_dir: dir.join("support"),
        ..request
    };
    let session = runtime
        .start(request, Arc::new(Events::default()))
        .unwrap_or_else(|e| panic!("start: {e}"));
    session
        .prompt(&format!("{command}天气 app"))
        .unwrap_or_else(|e| panic!("prompt: {e}"));
    // Engines may send side requests first (OMP summarizes skill descriptions); the turn's
    // request is the one carrying the user's words.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let body = loop {
        let turn = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("request-"))
            })
            .filter_map(|p| fs::read_to_string(p).ok())
            .find(|body| body.contains("天气 app"));
        if let Some(body) = turn {
            break body;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the engine never sent the turn's model request"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let _ = session.dispose();
    body
}

/// The engine expands the bundled setup skill and carries the workflow and spec rules.
fn assert_setup_skill_expanded(body: &str) {
    assert!(
        body.contains("Setting up a project"),
        "the skill body reached the model"
    );
    assert!(
        body.contains("天气 app"),
        "the user's words follow the skill"
    );
    assert!(
        body.contains("read the choosing-a-workflow skill"),
        "WORKFLOW_RULE is in the system prompt"
    );
    assert!(
        body.contains("Specs are this project's ground truth."),
        "SPEC_RULE is in the system prompt"
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_expands_the_bundled_setup_skill() {
    let f = fixture();
    let request = request(BackendKind::Pi, "THINKRAIL_SMOKE_PI", &f);
    assert_setup_skill_expanded(&skill_round(
        &PiRuntime::new(),
        request,
        &f,
        &crate::skill_command(BackendKind::Pi, "setting-up-a-project"),
    ));
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_expands_the_bundled_setup_skill() {
    let f = fixture();
    let request = request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f);
    assert_setup_skill_expanded(&skill_round(
        &OmpRuntime::new(),
        request,
        &f,
        &crate::skill_command(BackendKind::Omp, "setting-up-a-project"),
    ));
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_DSH"]
fn dsh_expands_the_bundled_setup_skill() {
    let f = fixture();
    let request = request(BackendKind::Dsh, "THINKRAIL_SMOKE_DSH", &f);
    assert_setup_skill_expanded(&skill_round(
        &DshRuntime::new(),
        request,
        &f,
        &crate::skill_command(BackendKind::Dsh, "setting-up-a-project"),
    ));
}
