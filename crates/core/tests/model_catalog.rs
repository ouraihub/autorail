//! Model list rules: grouping, id hints and the "同步模型" merge.

use std::collections::BTreeSet;

use thinkrail_core::{
    DiscoveredModel, ModelType, Protocol, ProviderModel, ThinkingLevel, merge_picked, model_group, model_hints, model_type,
};

const CHAT: Protocol = Protocol::OpenaiChat;
const CLAUDE: Protocol = Protocol::AnthropicMessages;

fn found(id: &str, context: Option<u32>) -> DiscoveredModel {
    DiscoveredModel { id: id.into(), name: None, context_window: context }
}

fn edited(id: &str, protocols: &[Protocol]) -> ProviderModel {
    ProviderModel {
        model_id: id.into(),
        model_type: ModelType::Chat,
        name: "手工名称".into(),
        protocols: protocols.to_vec(),
        context_window: None,
        max_tokens: Some(1234),
        vision: false,
        reasoning: true,
        tools: false,
        efforts: vec![ThinkingLevel::High],
        enabled: false,
    }
}

fn picked(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn ids(models: &[ProviderModel]) -> Vec<&str> {
    models.iter().map(|model| model.model_id.as_str()).collect()
}

#[test]
fn groups_are_the_id_prefix() {
    assert_eq!(model_group("gpt-6-astra"), "gpt");
    assert_eq!(model_group("Qwen/Qwen3-235B"), "Qwen");
    assert_eq!(model_group("deepseek:chat"), "deepseek");
    assert_eq!(model_group("o3"), "o3");
    assert_eq!(model_group("-odd"), "");
}

#[test]
fn hints_come_from_known_families_and_markers_only() {
    let gpt = model_hints("gpt-6-sol");
    assert_eq!((gpt.context_window, gpt.vision, gpt.tools), (Some(128_000), true, true));
    assert!(!model_hints("gpt-3.5-turbo").vision, "known variant wins over the family default");
    assert!(model_hints("qwen-vl-max").vision, "marker overrides a non-vision family");
    assert_eq!(model_hints("o3-mini").context_window, Some(200_000));
    assert_eq!(model_hints("omni-moderation").context_window, None, "omni is not the o series");
    let image = model_hints("gpt-image-2");
    assert_eq!((image.tools, image.vision, image.context_window), (false, false, None), "non-chat models get no chat limits");
    let unknown = model_hints("my-relay-model");
    assert_eq!((unknown.context_window, unknown.vision, unknown.tools), (None, false, true), "unknown chat ids get no limits but keep tools");
}

#[test]
fn types_come_from_id_keywords_in_order() {
    let cases = [
        ("gpt-6-sol", ModelType::Chat), ("grok-2-image", ModelType::Image), ("text-embedding-3-small", ModelType::Embedding),
        ("bge-reranker", ModelType::Rerank), ("tts-1", ModelType::Tts), ("whisper-1", ModelType::Stt),
        ("sora-2", ModelType::Video), ("Flux.1-dev", ModelType::Image),
    ];
    for (id, expected) in cases {
        assert_eq!(model_type(id), expected, "{id}");
    }
}

#[test]
fn only_picked_models_are_added_and_reconfirming_adds_nothing() {
    let discovered = [found("gpt-6-sol", Some(400_000)), found("gpt-image-2", None), found("gpt-6-sol", None)];
    let first = merge_picked(&[], &discovered, &picked(&["gpt-6-sol"]), CHAT);
    assert_eq!(ids(&first), ["gpt-6-sol"], "duplicate probe ids count once; unpicked ones stay out");
    let added = &first[0];
    assert_eq!((added.enabled, added.protocols.as_slice(), added.context_window), (true, &[CHAT][..], Some(400_000)));
    assert_eq!(added.name, "gpt-6-sol", "no probe name falls back to the id");
    assert_eq!(merge_picked(&first, &discovered, &picked(&["gpt-6-sol"]), CHAT), first);
}

#[test]
fn an_empty_selection_and_unlisted_models_leave_the_list_alone() {
    let mine = edited("my-finetune", &[CHAT]);
    let merged = merge_picked(std::slice::from_ref(&mine), &[found("gpt-6-sol", None)], &picked(&[]), CHAT);
    assert_eq!(merged, [mine], "a model the probe did not list is never removed");
}

#[test]
fn kept_models_keep_their_manual_fields_and_only_gain_a_missing_context_window() {
    let existing = [edited("claude-x", &[CHAT])];
    let merged = merge_picked(&existing, &[found("claude-x", Some(200_000))], &picked(&["claude-x"]), CHAT);
    let expected = ProviderModel { context_window: Some(200_000), ..existing[0].clone() };
    assert_eq!(merged, [expected]);

    let sized = [ProviderModel { context_window: Some(64_000), ..existing[0].clone() }];
    let merged = merge_picked(&sized, &[found("claude-x", Some(200_000))], &picked(&["claude-x"]), CHAT);
    assert_eq!(merged[0].context_window, Some(64_000), "a context window the user set is not overwritten");
}

#[test]
fn unpicking_removes_the_model_from_this_line_only() {
    let both = [edited("claude-x", &[CLAUDE, CHAT]), edited("gpt-6-sol", &[CHAT])];
    let listed = [found("claude-x", None), found("gpt-6-sol", None)];
    let merged = merge_picked(&both, &listed, &picked(&[]), CHAT);
    assert_eq!(ids(&merged), ["claude-x"], "a model on no line is dropped");
    assert_eq!(merged[0].protocols, [CLAUDE], "the other line keeps it");
}

#[test]
fn picking_a_model_from_another_line_adds_this_line_and_keeps_its_default() {
    let existing = [edited("claude-x", &[CLAUDE])];
    let merged = merge_picked(&existing, &[found("claude-x", None)], &picked(&["claude-x"]), CHAT);
    assert_eq!(merged[0].protocols, [CLAUDE, CHAT], "the first protocol stays the default");
    assert_eq!(merged[0].name, "手工名称");
}
