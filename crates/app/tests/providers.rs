//! Provider integration contracts live outside `src/` production modules.

use std::{fs, time::{SystemTime, UNIX_EPOCH}};

use thinkrail_core::{Access, Endpoint, Header, Protocol, ProviderInput, ProviderModel, ProviderService};
use thinkrail_settings_store::SqliteSettingsStore;

fn temp_db() -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("thinkrail-provider-test-{nonce}.db"))
}
fn input(url: &str) -> ProviderInput {
    ProviderInput {
        name: "Relay".into(),
        endpoints: vec![Endpoint { protocol: Protocol::OpenaiChat, base_url: url.into() }],
        default_protocol: Protocol::OpenaiChat,
        headers: vec![Header { name: "X-Trace".into(), value: "trace".into() }],
        access: Access::Direct,
        enabled: true,
        models: vec![ProviderModel {
            model_id: "model".into(),
            name: "Model".into(),
            protocols: vec![Protocol::OpenaiChat],
            context_window: None,
            max_tokens: None,
            vision: false,
            reasoning: false,
            efforts: Vec::new(),
            tools: true,
            enabled: true,
        }],
    }
}

#[test]
fn provider_service_rejects_invalid_url_and_preserves_valid_contract() {
    let path = temp_db();
    let store = SqliteSettingsStore::open(&path).unwrap();
    assert!(store.create(input("ftp://relay.example")).is_err());
    let provider = store.create(input("https://relay.example/v1")).unwrap();
    assert_eq!(provider.input.endpoints[0].base_url, "https://relay.example/v1");
    assert_eq!(provider.input.models[0].model_id, "model");
    drop(store);
    let _ = fs::remove_file(path);
}
