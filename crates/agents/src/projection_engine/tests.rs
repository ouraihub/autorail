use serde_json::json;
use thinkrail_core::{ErrorKind, ProjectedModel, ThinkingLevel};

use super::*;
use crate::testing::{SECRETS, dsh_routing, sample_routing};


fn assert_no_secret(text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "generated text contains {secret}");
    }
}

#[test]
fn generated_engine_files_do_not_contain_secret_values() {
    let routing = sample_routing();
    let skills = std::path::Path::new("skills");
    assert_no_secret(&pi_extension(true, &routing));
    assert_no_secret(&omp_extension(&routing));
    assert_no_secret(&omp_overlay(&routing, skills));
    assert_no_secret(&dsh_patch(true, &dsh_routing(), skills).unwrap());
}

#[test]
fn dsh_patch_refuses_header_bindings() {
    let error = dsh_patch(false, &sample_routing(), std::path::Path::new("skills")).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(!error.to_string().contains(SECRETS[1]));
}

#[test]
fn acp_mcp_servers_carry_values_from_the_routing_env() {
    let routing = sample_routing();
    let servers = acp_mcp_servers(&routing).unwrap();
    assert_eq!(
        servers,
        [
            json!({"name": "thinkrail-echo", "command": "node", "args": ["echo.js"], "env": [{"name": "ECHO_TOKEN", "value": "sk-test-c"}]}),
            json!({"type": "http", "name": "thinkrail-docs", "url": "https://mcp.example.com/mcp",
                   "headers": [{"name": "Authorization", "value": "Bearer sk-test-a"}]}),
        ]
    );
    let mut missing = routing;
    missing.env.retain(|(name, _)| name != "THINKRAIL_MCP_5E6F7A8B_E0");
    let error = acp_mcp_servers(&missing).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid));
    assert!(error.to_string().contains("THINKRAIL_MCP_5E6F7A8B_E0"));
}

fn model(reasoning: bool, efforts: Vec<ThinkingLevel>) -> ProjectedModel {
    ProjectedModel {
        id: "claude-x".into(),
        name: "Claude X".into(),
        vision: true,
        reasoning,
        efforts,
        context_window: 200_000,
        max_tokens: 16_384,
    }
}


#[test]
fn pi_map_lists_every_level_and_nulls_the_undeclared() {
    let declared = model(true, vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High]);
    assert_eq!(
        pi_thinking_level_map(&declared),
        Some(vec![
            ("off", Some("off")),
            ("minimal", None),
            ("low", Some("low")),
            ("medium", None),
            ("high", Some("high")),
            ("xhigh", None),
            ("max", None),
        ])
    );
    assert_eq!(pi_thinking_level_map(&model(false, Vec::new())), None);
}

#[test]
fn dsh_efforts_list_only_declared_levels_with_off_as_null() {
    let declared = model(true, vec![ThinkingLevel::High, ThinkingLevel::Off, ThinkingLevel::Low]);
    assert_eq!(dsh_reasoning_efforts(&declared), Some(vec![("off", None), ("low", Some("low")), ("high", Some("high"))]));
    assert_eq!(dsh_reasoning_efforts(&model(true, Vec::new())), Some(Vec::new()));
    assert_eq!(dsh_reasoning_efforts(&model(false, vec![ThinkingLevel::High])), None);
}
