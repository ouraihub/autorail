use serde_json::json;
use thinkrail_core::{
    ContentBlock, EngineRouting, ErrorKind, Interaction, InteractionAnswer, InteractionKind, InteractionOption, Role,
    ThinkingLevel, ToolStatus,
};

use super::*;
use crate::{
    projection::tests::projection,
    testing::{Fixture, dsh_routing, routing_vars, secret_routing},
};

#[test]
fn committed_chunks_of_one_message_append_blocks_in_order() {
    let (mut p, _) = projection();
    let chunk = |kind: &str, text: &str| {
        json!({"sessionUpdate": kind, "messageId": "m", "content": {"type": "text", "text": text}})
    };
    apply_update(&mut p, &chunk("agent_thought_chunk", "think"));
    apply_update(&mut p, &chunk("agent_message_chunk", "answer"));
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].role, Role::Assistant);
    assert_eq!(
        snapshot.messages[0].content,
        vec![ContentBlock::Thinking { text: "think".into() }, ContentBlock::Text { text: "answer".into() }]
    );
}

#[test]
fn tool_lifecycle_and_model_option() {
    let (mut p, _) = projection();
    apply_update(&mut p, &json!({"sessionUpdate":"tool_call","toolCallId":"c1","title":"read","status":"in_progress","rawInput":{"path":"a"}}));
    apply_update(&mut p, &json!({"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"failed",
        "content":[{"type":"content","content":{"type":"text","text":"denied"}}]}));
    apply_update(&mut p, &json!({"sessionUpdate":"config_option_update","configOptions":[
        {"id":"model","category":"model","currentValue":"[\"deepseek\",\"deepseek-v4\"]"}]}));
    let snapshot = p.snapshot();
    assert!(matches!(&snapshot.messages[0].content[0],
        ContentBlock::Tool { status: ToolStatus::Failed, output: Some(o), .. } if o == "denied"));
    let model = snapshot.model.unwrap();
    assert_eq!((model.provider.as_deref(), model.id.as_str()), (Some("deepseek"), "deepseek-v4"));
}

/// The `configOptions` DSH returns for a reasoning model (`model-control.ts`).
fn reasoning_options(current: &str) -> Value {
    json!([
        {"id":"model","category":"model","type":"select","currentValue":"[\"thinkrail-1a2b3c4d-am\",\"claude-x\"]"},
        {"id":"reasoning_effort","category":"thought_level","type":"select","currentValue":current,
         "options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}
    ])
}

#[test]
fn model_switch_sets_the_model_option_then_the_thought_level_option() {
    assert_eq!(
        model_option("s1", "thinkrail-1a2b3c4d-am", "claude-x"),
        json!({"sessionId":"s1","configId":"model","value":"[\"thinkrail-1a2b3c4d-am\",\"claude-x\"]"})
    );
    assert_eq!(
        effort_option("s1", &reasoning_options(""), ThinkingLevel::High).unwrap(),
        json!({"sessionId":"s1","configId":"reasoning_effort","value":"high"})
    );
    let plain = json!([{"id":"model","category":"model","currentValue":"[\"p\",\"m\"]"}]);
    let error = effort_option("s1", &plain, ThinkingLevel::High).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid), "a model without reasoning has no level to set");
}

#[test]
fn effort_is_the_thought_level_current_value() {
    let info = model(&reasoning_options("high")).unwrap();
    assert_eq!((info.provider.as_deref(), info.id.as_str(), info.effort), (Some("thinkrail-1a2b3c4d-am"), "claude-x", Some(ThinkingLevel::High)));
    assert_eq!(model(&reasoning_options("")).unwrap().effort, None, "provider default is not a host level");
    let (mut p, _) = projection();
    apply_update(&mut p, &json!({"sessionUpdate":"config_option_update","configOptions":reasoning_options("low")}));
    assert_eq!(p.snapshot().model.unwrap().effort, Some(ThinkingLevel::Low));
}

#[test]
fn permission_answers_must_name_an_offered_option() {
    let interaction = Interaction {
        request_id: "r".into(),
        kind: InteractionKind::Permission,
        title: String::new(),
        message: String::new(),
        options: vec![InteractionOption { id: "allow-once".into(), label: "Allow once".into() }],
        detail: Value::Null,
    };
    let allow = InteractionAnswer::Select { option_id: "allow-once".into() };
    assert_eq!(permission_outcome(&interaction, &allow).unwrap(), json!({"outcome":"selected","optionId":"allow-once"}));
    assert_eq!(permission_outcome(&interaction, &InteractionAnswer::Cancel).unwrap(), json!({"outcome":"cancelled"}));
    let forged = InteractionAnswer::Select { option_id: "allow-always".into() };
    assert_eq!(permission_outcome(&interaction, &forged).unwrap_err().kind(), Some(ErrorKind::Invalid));
    let wrong = InteractionAnswer::Confirm { confirmed: true };
    assert_eq!(permission_outcome(&interaction, &wrong).unwrap_err().kind(), Some(ErrorKind::Invalid));
}

#[test]
fn a_failed_turn_is_visible_in_the_transcript() {
    let (mut p, _) = projection();
    turn_failed(&mut p, "user-1-error".into(), &engine_error("turn failed: 401 bad api_key=sk-123"));
    let snapshot = p.snapshot();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].role, Role::System);
    let ContentBlock::Text { text } = &snapshot.messages[0].content[0] else { panic!("text note") };
    assert!(text.contains("401") && !text.contains("sk-123"), "{text}");
    assert_eq!(snapshot.last_error.map(|e| e.code), Some("EXTERNAL".into()));
}

/// Expected values recorded from the adapter before routing existed; only the patch file name
/// changed (`dsh-host.patch.yml` → `dsh-thinkrail.patch.yml`, 04 §6.4).
#[test]
fn empty_routing_keeps_the_pre_routing_launch() {
    let f = Fixture::new();
    let request = f.request(BackendKind::Dsh, None, false, EngineRouting::default());
    let spec = launch_spec(&request).unwrap();
    assert_eq!(spec.host_args, ["--profile", "acp"]);
    assert!(spec.host_env.is_empty());
    assert_eq!((spec.cwd, spec.config_var), (f.config(), "DSH_HOME"));
    assert!(!f.support().exists(), "no host tools and no routing must not generate files");

    let request = f.request(BackendKind::Dsh, None, true, EngineRouting::default());
    let spec = launch_spec(&request).unwrap();
    let patch = f.support_arg("dsh-thinkrail.patch.yml");
    assert_eq!(spec.host_args, ["--profile", "acp", "--patch", patch.as_str()]);
    assert!(spec.host_env.is_empty(), "DSH gets the host pipe and token through ACP, not its environment");
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), "- id: tool-todo\n  disabled: true\n");
}

#[test]
fn routing_env_is_the_whole_launch_environment() {
    let f = Fixture::new();
    let routing = secret_routing();
    let request = f.request(BackendKind::Dsh, None, true, routing.clone());
    let spec = launch_spec(&request).unwrap();
    assert_eq!(spec.host_args, ["--profile", "acp", "--patch", f.support_arg("dsh-thinkrail.patch.yml").as_str()]);
    assert_eq!(spec.host_env, routing_vars(&routing));
    f.assert_no_secret_written();
}

#[test]
fn routing_writes_the_provider_patch() {
    let f = Fixture::new();
    let routing = dsh_routing();
    let request = f.request(BackendKind::Dsh, None, false, routing.clone());
    let spec = launch_spec(&request).unwrap();
    let patch = f.support_arg("dsh-thinkrail.patch.yml");
    assert_eq!(spec.host_args, ["--profile", "acp", "--patch", patch.as_str()]);
    assert_eq!(spec.host_env, routing_vars(&routing));
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), projection_engine::dsh_patch(false, &routing).unwrap().unwrap());
    f.assert_no_secret_written();
}

#[test]
fn session_parameters_append_the_projected_mcp_servers() {
    let f = Fixture::new();
    let mut routing = dsh_routing();
    let executable = std::env::current_exe().unwrap();
    if let thinkrail_core::ProjectedMcpTransport::Stdio { command, .. } = &mut routing.mcp[0].transport {
        *command = executable.display().to_string();
    }
    let cwd = std::path::PathBuf::from("w");
    let params = session_params(&f.request(BackendKind::Dsh, None, true, routing.clone()), &cwd).unwrap();
    let names: Vec<&str> = params["mcpServers"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["thinkrail", "thinkrail-echo", "thinkrail-docs"]);
    assert_eq!(params["mcpServers"][1]["env"], json!([{"name": "ECHO_TOKEN", "value": "sk-test-c"}]));
    assert_eq!(params["mcpServers"][1]["command"], json!(process::plain_path(&executable)));
    assert_eq!(params["mcpServers"][2]["type"], "http");
    assert!(params.get("sessionId").is_none());

    let resumed = session_params(&f.request(BackendKind::Dsh, Some("s-1"), false, routing), &cwd).unwrap();
    assert_eq!(resumed["sessionId"], "s-1");
    assert_eq!(resumed["mcpServers"].as_array().unwrap().len(), 2);
    let empty = session_params(&f.request(BackendKind::Dsh, None, false, EngineRouting::default()), &cwd).unwrap();
    assert_eq!(empty, json!({"cwd": "w", "mcpServers": []}));
}

#[test]
fn a_failed_session_with_mcp_points_at_the_settings() {
    let f = Fixture::new();
    let with_mcp = f.request(BackendKind::Dsh, None, false, dsh_routing());
    let error = mcp_hint(Error::external("MCP server failed (-32603)"), &with_mcp);
    assert_eq!(error.kind(), Some(ErrorKind::External));
    assert!(error.to_string().contains("设置 → MCP 服务器") && error.to_string().contains("thinkrail-echo"), "{error}");
    let without = f.request(BackendKind::Dsh, None, false, EngineRouting::default());
    assert!(!mcp_hint(Error::external("x"), &without).to_string().contains("MCP"));
}

#[test]
fn mcp_command_must_resolve_to_an_existing_absolute_file() {
    let executable = std::env::current_exe().unwrap();
    assert_eq!(mcp_executable(executable.to_str().unwrap()).unwrap(), process::plain_path(&executable));
    assert_eq!(mcp_executable("thinkrail-surely-missing-mcp-command").unwrap_err().kind(), Some(ErrorKind::Invalid));
}

#[test]
fn merging_dsh_routes_keeps_user_values_out_of_generated_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("cordis.patch.yml"), concat!(
        "- id: llm-pi-ai\n  config:\n    providers:\n      mine:\n",
        "        headers: { X-Private: sensitive-header-value }\n",
        "        displayName: !!js '\"Mine\"'\n",
    )).unwrap();
    let patch = projection_engine::dsh_patch(false, &dsh_routing()).unwrap().unwrap();
    let mut env = Vec::new();
    let output = preserve_user_routes(dir.path(), patch, &mut env).unwrap();
    assert!(!output.contains("sensitive-header-value"));
    let config: Value = serde_json::from_str(env[0].1.to_str().unwrap()).unwrap();
    assert_eq!(config["base"]["providers"]["mine"]["headers"]["X-Private"], "sensitive-header-value");
    assert_eq!(config["base"]["providers"]["mine"]["displayName"]["__jsExpr"], "\"Mine\"");
    assert_eq!(config["projected"]["providers"]["thinkrail-1a2b3c4d-am"]["models"][0]["id"], "claude-x");
}
