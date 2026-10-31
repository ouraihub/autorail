//! Built-in provider presets: seeding, identity and delete protection through the public
//! `ProviderService` contract.

use tempfile::TempDir;
use thinkrail_core::{
    Access, Endpoint, ErrorKind, PROVIDER_PRESETS, Protocol, Provider, ProviderInput, ProviderService, provider_preset,
};
use thinkrail_settings_store::SqliteSettingsStore;

fn open(directory: &TempDir) -> SqliteSettingsStore {
    SqliteSettingsStore::open(&directory.path().join("thinkrail.db")).unwrap()
}

fn user_provider(name: &str) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        endpoints: vec![Endpoint { protocol: Protocol::OpenaiChat, base_url: "https://relay.example/v1".into() }],
        default_protocol: Protocol::OpenaiChat,
        headers: Vec::new(),
        access: Access::Direct,
        enabled: true,
        models: Vec::new(),
    }
}

fn by_preset<'a>(providers: &'a [Provider], id: &str) -> &'a Provider {
    providers.iter().find(|p| p.preset_id.as_deref() == Some(id)).unwrap()
}

#[test]
fn seeding_creates_each_preset_once_as_a_disabled_template() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let seeded = store.seed_presets().unwrap();
    assert_eq!(seeded.len(), PROVIDER_PRESETS.len());
    for preset in PROVIDER_PRESETS {
        let provider = by_preset(&seeded, preset.id);
        assert_eq!(provider.input.name, preset.name);
        assert!(!provider.input.enabled, "a preset never starts enabled");
        assert_eq!(provider.input.endpoints, [Endpoint { protocol: preset.protocol, base_url: preset.base_url.into() }]);
        assert!(provider.input.models.is_empty() && provider.keys.is_empty());
    }
    let again = store.seed_presets().unwrap();
    assert_eq!(again, seeded, "seeding twice changes nothing");
}

#[test]
fn user_edits_survive_reseeding_and_keep_the_preset_identity() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let fenno = by_preset(&store.seed_presets().unwrap(), "fenno").clone();
    let mut input = fenno.input.clone();
    input.name = "我的 Fenno".into();
    input.enabled = true;
    let renamed = store.update(&fenno.id, fenno.revision, input).unwrap();
    assert_eq!(renamed.preset_id.as_deref(), Some("fenno"), "update keeps preset_id");
    drop(store);

    let store = open(&directory);
    let providers = store.seed_presets().unwrap();
    assert_eq!(providers.len(), PROVIDER_PRESETS.len(), "a renamed preset is not seeded again");
    assert_eq!(by_preset(&providers, "fenno").input.name, "我的 Fenno");
}

#[test]
fn a_user_provider_with_a_preset_name_blocks_only_that_preset() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let mine = store.create(user_provider("Fenno")).unwrap();
    assert_eq!(mine.preset_id, None, "create never sets preset_id");
    let providers = store.seed_presets().unwrap();
    assert_eq!(providers.len(), PROVIDER_PRESETS.len());
    assert!(providers.iter().all(|p| p.preset_id.as_deref() != Some("fenno")));
    assert_eq!(providers.iter().find(|p| p.input.name == "Fenno").unwrap().id, mine.id);
}

#[test]
fn preset_providers_cannot_be_deleted_but_user_providers_can() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let qiniu = by_preset(&store.seed_presets().unwrap(), "qiniu").clone();
    let error = store.delete(&qiniu.id, qiniu.revision).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Forbidden), "{error}");
    assert!(store.provider(&qiniu.id).is_ok(), "the row is still there");

    let mine = store.create(user_provider("Relay")).unwrap();
    store.delete(&mine.id, mine.revision).unwrap();
    assert_eq!(store.provider(&mine.id).unwrap_err().kind(), Some(ErrorKind::NotFound));
}

#[test]
fn preset_lookup_matches_the_table_and_key_links_are_https() {
    for preset in PROVIDER_PRESETS {
        assert_eq!(provider_preset(preset.id), Some(preset));
        assert!(preset.api_key_url.starts_with("https://"), "{}", preset.id);
    }
    assert_eq!(provider_preset("unknown"), None);
}

#[test]
fn an_earlier_development_schema_is_refused_with_a_recreate_hint() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("thinkrail.db");
    drop(SqliteSettingsStore::open(&path).unwrap());
    pollster::block_on(async {
        let db = turso::Builder::new_local(path.to_str().unwrap()).build().await.unwrap();
        db.connect().unwrap().execute("UPDATE schema_meta SET version = 1", ()).await.unwrap();
    });
    let error = SqliteSettingsStore::open(&path).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Corrupt), "{error}");
    assert!(error.to_string().contains("删除 thinkrail.db"), "{error}");
}
