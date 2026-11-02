//! Unit tests for model visibility.

use super::*;
use thinkrail_core::{Access, Protocol, ProviderId, ProviderInput};

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
