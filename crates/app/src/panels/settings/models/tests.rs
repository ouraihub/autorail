//! Unit tests for model visibility and the default group's effort rule.

use super::*;
use thinkrail_core::{Access, ProjectedModel, Protocol, ProviderId, ProviderInput};

fn provider(enabled: bool) -> Provider {
    Provider {
        id: ProviderId::from("provider"),
        revision: 0,
        input: ProviderInput {
            name: "Provider".into(),
            endpoints: Vec::new(),
            default_protocol: Protocol::OpenaiChat,
            headers: Vec::new(),
            access: Access::Direct,
            enabled,
            models: vec![ProviderModel {
                model_id: "hidden-model-id".into(),
                model_type: thinkrail_core::ModelType::Chat,
                name: "Hidden Model".into(),
                protocols: vec![Protocol::OpenaiChat],
                context_window: None,
                max_tokens: None,
                vision: false,
                reasoning: false,
                tools: true,
                efforts: Vec::new(),
                enabled: true,
            }],
        },
        keys: Vec::new(),
        preset_id: None,
    }
}

#[test]
fn visibility_uses_enabled_provider_models_not_the_catalog() {
    let mut disabled_model = provider(true);
    disabled_model.input.models[0].enabled = false;
    let models = visibility_models(vec![provider(true), provider(false), disabled_model]);
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].reference.model_id, "hidden-model-id");
    assert!(models[0].matches("hidden-model-id"));
    assert!(models[0].matches("hidden model"));
    assert!(!models[0].matches("missing"));
}

#[test]
fn visibility_search_also_matches_the_provider_name() {
    let models = visibility_models(vec![provider(true)]);
    assert!(models[0].matches("provider"));
}

#[test]
fn changing_model_keeps_the_effort_only_where_the_new_model_declares_it() {
    use ThinkingLevel::*;
    let choice = |efforts: Vec<ThinkingLevel>| ModelChoice {
        provider: ProviderId::from("p"),
        provider_name: "P".into(),
        route: "r".into(),
        model: ProjectedModel {
            id: "m".into(),
            name: "M".into(),
            vision: false,
            reasoning: !efforts.is_empty(),
            efforts,
            context_window: 128_000,
            max_tokens: 16_384,
        },
    };
    let kept = model_select::kept_effort;
    assert_eq!(kept(Some(High), Some(&choice(vec![Low, High]))), Some(High));
    assert_eq!(kept(Some(High), Some(&choice(vec![Low, Medium]))), None);
    assert_eq!(kept(Some(High), Some(&choice(Vec::new()))), None);
    assert_eq!(
        kept(Some(High), None),
        None,
        "engine default keeps no effort"
    );
    assert_eq!(kept(None, Some(&choice(vec![Low]))), None);
}

#[test]
fn context_sizes_print_whole_millions_as_m_and_the_rest_in_thousands() {
    assert_eq!(s::format_context(272_000), "272K");
    assert_eq!(s::format_context(128_000), "128K");
    assert_eq!(s::format_context(1_000_000), "1M");
    assert_eq!(s::format_context(2_000_000), "2M");
    assert_eq!(s::format_context(1_050_000), "1050K");
}

#[test]
fn context_controls_select_the_shared_choice_or_nothing_when_mixed() {
    use context_group::{Choice, shared_choice};
    assert_eq!(shared_choice(&[None, None]), Some(Choice::Default));
    assert_eq!(shared_choice(&[Some(1_000_000); 2]), Some(Choice::Million));
    assert_eq!(
        shared_choice(&[Some(400_000); 3]),
        Some(Choice::Custom(400_000))
    );
    assert_eq!(shared_choice(&[None, Some(1_000_000)]), None);
    assert_eq!(shared_choice(&[Some(400_000), Some(500_000)]), None);
    assert_eq!(shared_choice(&[]), None);
}

#[test]
fn custom_context_accepts_whole_numbers_in_range_only() {
    use context_group::parse_tokens;
    assert_eq!(parse_tokens("272000"), Some(272_000));
    assert_eq!(parse_tokens(" 1,000,000 "), Some(1_000_000));
    for bad in [
        "",
        "271999",
        "1000001",
        "500k",
        "4e5",
        "-300000",
        "三十万",
        "300000.5",
    ] {
        assert_eq!(parse_tokens(bad), None, "{bad:?}");
    }
}
