//! Line edits keep models and the default protocol consistent.

use thinkrail_core::{
    Access, Endpoint, ModelType, Protocol, ProviderInput, ProviderModel, free_protocols,
    line_models, remove_line, retarget_line, set_line,
};

const CHAT: Protocol = Protocol::OpenaiChat;
const CLAUDE: Protocol = Protocol::AnthropicMessages;
const RESPONSES: Protocol = Protocol::OpenaiResponses;

fn model(id: &str, protocols: &[Protocol]) -> ProviderModel {
    ProviderModel {
        model_id: id.into(),
        model_type: ModelType::Chat,
        name: id.into(),
        protocols: protocols.to_vec(),
        context_window: None,
        max_tokens: None,
        vision: false,
        reasoning: false,
        tools: true,
        efforts: Vec::new(),
        enabled: true,
    }
}

fn two_lines() -> ProviderInput {
    ProviderInput {
        name: "Relay".into(),
        endpoints: vec![
            Endpoint {
                protocol: CHAT,
                base_url: "https://relay.example/v1".into(),
            },
            Endpoint {
                protocol: CLAUDE,
                base_url: "https://relay.example".into(),
            },
        ],
        default_protocol: CHAT,
        headers: Vec::new(),
        access: Access::Direct,
        enabled: true,
        models: vec![model("gpt-6", &[CHAT]), model("claude-x", &[CLAUDE, CHAT])],
    }
}

#[test]
fn free_protocols_and_set_line_add_or_replace_one_address_per_protocol() {
    let mut input = two_lines();
    assert_eq!(free_protocols(&input), [RESPONSES]);
    set_line(&mut input, CHAT, "https://other.example/v1".into());
    assert_eq!(input.endpoints.len(), 2);
    assert_eq!(input.endpoints[0].base_url, "https://other.example/v1");
    set_line(&mut input, RESPONSES, "https://relay.example/v1".into());
    assert!(free_protocols(&input).is_empty());
}

#[test]
fn removing_a_line_drops_models_left_without_one_and_moves_the_default() {
    let mut input = two_lines();
    assert!(remove_line(&mut input, CHAT));
    assert_eq!(
        input
            .endpoints
            .iter()
            .map(|e| e.protocol)
            .collect::<Vec<_>>(),
        [CLAUDE]
    );
    assert_eq!(input.default_protocol, CLAUDE);
    assert_eq!(input.models, [model("claude-x", &[CLAUDE])]);
    assert!(!remove_line(&mut input, CLAUDE), "the last line stays");
    assert!(
        !remove_line(&mut input, RESPONSES),
        "a missing line is not an edit"
    );
}

#[test]
fn retargeting_moves_models_and_default_and_refuses_taken_protocols() {
    let mut input = two_lines();
    let before = input.clone();
    assert!(!retarget_line(&mut input, CHAT, CLAUDE));
    assert_eq!(input, before, "a refused retarget changes nothing");
    assert!(retarget_line(&mut input, CHAT, RESPONSES));
    assert_eq!(input.default_protocol, RESPONSES);
    assert_eq!(input.models[0].protocols, [RESPONSES]);
    assert_eq!(
        input.models[1].protocols,
        [CLAUDE, RESPONSES],
        "order (and so each model's default) is kept"
    );
}

#[test]
fn line_models_lists_only_models_on_that_line() {
    let input = two_lines();
    let on_claude: Vec<_> = line_models(&input, CLAUDE)
        .map(|(index, m)| (index, m.model_id.as_str()))
        .collect();
    assert_eq!(on_claude, [(1, "claude-x")]);
    assert_eq!(line_models(&input, CHAT).count(), 2);
}
