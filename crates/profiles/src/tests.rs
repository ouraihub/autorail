use std::{fmt::Debug, fs, path::Path};

use serde_json::Value;
use tempfile::TempDir;
use thinkrail_core::{
    BackendKind, ChatBinding, ChatId, EnvRef, EnvSource, ErrorKind, Launch, Profile, ProfileCatalog, ProfileInput,
    ProfileService, Result, WorkspaceId,
};

use crate::{JsonProfileStore, storage::FILE_NAME};

fn input(backend: BackendKind) -> ProfileInput {
    ProfileInput {
        backend_id: backend,
        label: "Personal".into(),
        enabled: true,
        config_root: None,
        launch: Launch::External { executable: std::env::current_exe().unwrap(), args: Vec::new() },
        env_refs: Vec::new(),
    }
}

fn binding(profile: &Profile, chat: &str) -> ChatBinding {
    ChatBinding {
        chat_id: chat.into(),
        workspace_id: "workspace-1".into(),
        profile_id: profile.id.clone(),
        config_revision: profile.config_revision,
        native_session_key: "../../opaque/native.json".into(),
    }
}

fn env(name: &str, reference: &str) -> EnvRef {
    EnvRef { name: name.into(), source: EnvSource::Launch, reference: reference.into() }
}

#[track_caller]
fn kind<T: Debug>(result: Result<T>, expected: ErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), Some(expected), "{error}");
}

fn bytes(root: &Path) -> Vec<u8> {
    fs::read(root.join(FILE_NAME)).unwrap()
}

fn entries(root: &Path) -> usize {
    fs::read_dir(root).unwrap().count()
}

fn empty() -> ProfileCatalog {
    ProfileCatalog { profiles: Vec::new(), default_profile_id: None }
}

#[test]
fn empty_catalog_and_durable_crud_with_revision_conflicts() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    assert_eq!(store.catalog().unwrap(), empty());
    assert_eq!(entries(directory.path()), 0);
    for backend in BackendKind::ALL {
        let original = store.create(input(backend)).unwrap();
        assert_eq!(original.config_revision, 1);
        let replacement = ProfileInput { label: "Revised".into(), ..input(backend) };
        let revised = store.update(&original.id, 1, replacement).unwrap();
        assert_eq!(revised.config_revision, 2);
        assert_eq!(store.revision(&original.id, 1).unwrap(), original);
        assert_eq!(store.catalog().unwrap().profiles, vec![revised.clone()]);
        let before = bytes(directory.path());
        kind(store.update(&original.id, 1, input(backend)), ErrorKind::Conflict);
        kind(store.delete(&original.id, 1), ErrorKind::Conflict);
        assert_eq!(bytes(directory.path()), before);

        let reopened = JsonProfileStore::open(directory.path()).unwrap();
        assert_eq!(reopened.revision(&original.id, 1).unwrap(), original);
        reopened.set_default(Some(&original.id)).unwrap();
        reopened.delete(&original.id, 2).unwrap();
        kind(reopened.revision(&original.id, 1), ErrorKind::NotFound);
        assert_eq!(reopened.catalog().unwrap(), empty());
        assert_eq!(store.catalog().unwrap(), empty());
    }
    assert_eq!(entries(directory.path()), 2);
}

#[test]
fn defaults_and_profile_changes_never_rebind_existing_chats() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    let first = store.create(input(BackendKind::Pi)).unwrap();
    let second = store.create(input(BackendKind::Omp)).unwrap();
    assert_eq!(store.catalog().unwrap().default_profile_id, None);
    let catalog = store.set_default(Some(&first.id)).unwrap();
    assert_eq!(catalog.default_profile_id, Some(first.id.clone()));
    assert_eq!(catalog.profiles, vec![first.clone(), second.clone()]);

    let bound = binding(&first, "chat-1");
    store.bind(bound.clone()).unwrap();
    store.bind(bound.clone()).unwrap();
    store.set_default(Some(&second.id)).unwrap();
    assert_eq!(store.catalog().unwrap().default_profile_id, Some(second.id.clone()));
    assert_eq!(store.binding(&ChatId::from("chat-1")).unwrap(), Some(bound.clone()));

    store.set_default(Some(&first.id)).unwrap();
    let disabled = store.update(&first.id, 1, ProfileInput { enabled: false, ..input(BackendKind::Pi) }).unwrap();
    assert_eq!(store.catalog().unwrap().default_profile_id, None);
    store.bind(bound.clone()).unwrap();
    kind(store.bind(binding(&disabled, "chat-2")), ErrorKind::ProfileDisabled);
    kind(store.set_default(Some(&first.id)), ErrorKind::ProfileDisabled);
    kind(store.delete(&first.id, 2), ErrorKind::Forbidden);
    store.set_default(None).unwrap();

    assert_eq!(store.bindings(&WorkspaceId::from("workspace-1")).unwrap(), vec![bound.clone()]);
    assert!(store.bindings(&WorkspaceId::from("workspace-2")).unwrap().is_empty());
    let reopened = JsonProfileStore::open(directory.path()).unwrap();
    assert_eq!(reopened.binding(&ChatId::from("chat-1")).unwrap(), Some(bound));
    assert_eq!(reopened.binding(&ChatId::from("chat-2")).unwrap(), None);
    assert_eq!(reopened.revision(&first.id, 1).unwrap(), first);
}

#[test]
fn binding_identity_is_scoped_but_chat_identity_is_global() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    let first = store.create(input(BackendKind::Pi)).unwrap();
    let second = store.create(input(BackendKind::Omp)).unwrap();
    let bound = binding(&first, "chat-1");
    store.bind(bound.clone()).unwrap();
    store.bind(binding(&second, "chat-2")).unwrap();
    kind(store.bind(binding(&first, "chat-3")), ErrorKind::Conflict);
    kind(store.bind(ChatBinding { workspace_id: "workspace-2".into(), ..bound }), ErrorKind::Conflict);
    kind(store.bind(binding(&second, "chat-1")), ErrorKind::Conflict);

    let revised = store.update(&first.id, 1, input(BackendKind::Pi)).unwrap();
    let stale = ChatBinding { native_session_key: "different-native".into(), ..binding(&first, "chat-3") };
    kind(store.bind(stale), ErrorKind::Conflict);
    store.bind(binding(&revised, "chat-3")).unwrap();
    assert_eq!(store.bindings(&WorkspaceId::from("workspace-1")).unwrap().len(), 3);
    kind(store.bind(ChatBinding { chat_id: "chat 4".into(), ..binding(&revised, "x") }), ErrorKind::Invalid);
    assert_eq!(entries(directory.path()), 2);
}

#[test]
fn a_closed_chat_is_unbound_and_its_session_can_be_bound_again() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    let profile = store.create(input(BackendKind::Pi)).unwrap();
    store.bind(binding(&profile, "chat-1")).unwrap();
    kind(store.bind(binding(&profile, "chat-2")), ErrorKind::Conflict);
    store.unbind(&ChatId::from("chat-1")).unwrap();
    store.unbind(&ChatId::from("chat-1")).unwrap();
    let reopened = JsonProfileStore::open(directory.path()).unwrap();
    assert_eq!(reopened.binding(&ChatId::from("chat-1")).unwrap(), None);
    reopened.bind(binding(&profile, "chat-3")).unwrap();
    kind(reopened.unbind(&ChatId::from("chat 4")), ErrorKind::Invalid);
}

#[test]
fn validation_rejects_unsafe_fields_without_writing() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    let mut relative = input(BackendKind::Pi);
    relative.launch = Launch::External { executable: "node.exe".into(), args: Vec::new() };
    let mut control = input(BackendKind::Pi);
    control.launch = Launch::External { executable: std::env::current_exe().unwrap(), args: vec!["a\nb".into()] };
    let mut reserved = input(BackendKind::Pi);
    reserved.env_refs = vec![env("PATH", "PATH")];
    let mut prefixed = input(BackendKind::Pi);
    prefixed.env_refs = vec![env("THINKRAIL_HOST_TOKEN", "X")];
    let mut duplicate = input(BackendKind::Pi);
    duplicate.env_refs = vec![env("API_KEY", "A"), env("api_key", "B")];
    let mut root = input(BackendKind::Dsh);
    root.config_root = Some("relative/home".into());
    for bad in [relative, control, reserved, prefixed, duplicate, root] {
        kind(store.create(bad), ErrorKind::Invalid);
    }
    assert_eq!(entries(directory.path()), 0);
    let mut good = input(BackendKind::Omp);
    good.env_refs = vec![env("OPENAI_API_KEY", "MY_OPENAI_KEY")];
    let saved = store.create(good).unwrap();
    let stored: Value = serde_json::from_slice(&bytes(directory.path())).unwrap();
    assert_eq!(stored.to_string().matches("MY_OPENAI_KEY").count(), 1);
    kind(store.delete(&saved.id, 0), ErrorKind::Invalid);
}

#[test]
fn corrupt_storage_and_held_lock_never_lose_data() {
    let directory = TempDir::new().unwrap();
    let store = JsonProfileStore::open(directory.path()).unwrap();
    store.create(input(BackendKind::Pi)).unwrap();
    let valid = bytes(directory.path());
    for damaged in [b"{broken".to_vec(), br#"{"schemaVersion":99}"#.to_vec()] {
        fs::write(directory.path().join(FILE_NAME), &damaged).unwrap();
        kind(JsonProfileStore::open(directory.path()), ErrorKind::Corrupt);
        kind(store.catalog(), ErrorKind::Corrupt);
        kind(store.create(input(BackendKind::Pi)), ErrorKind::Corrupt);
        assert_eq!(bytes(directory.path()), damaged);
    }
    fs::write(directory.path().join(FILE_NAME), &valid).unwrap();
    let lock = crate::storage::WriterLock::acquire(directory.path()).unwrap();
    kind(store.create(input(BackendKind::Omp)), ErrorKind::Busy);
    assert_eq!(bytes(directory.path()), valid);
    drop(lock);
    store.create(input(BackendKind::Dsh)).unwrap();
    assert_eq!(store.catalog().unwrap().profiles.len(), 2);
}
