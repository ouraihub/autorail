//! Chat bindings and the default Agent through the public contracts (agent-settings 07 §4).

use tempfile::TempDir;
use thinkrail_core::{BackendKind, ChatBinding, ChatBindingService, ModelPrefService};
use thinkrail_settings_store::SqliteSettingsStore;

fn binding(chat: &str, workspace: &str, backend: BackendKind) -> ChatBinding {
    ChatBinding { chat_id: chat.into(), workspace_id: workspace.into(), backend, native_session_key: format!("{chat}.jsonl").into() }
}

#[test]
fn bindings_list_per_workspace_in_creation_order_and_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("thinkrail.db");
    let store = SqliteSettingsStore::open(&path).unwrap();
    for (chat, workspace) in [("c3", "w1"), ("c1", "w1"), ("other", "w2"), ("c2", "w1")] {
        store.bind(binding(chat, workspace, BackendKind::Pi)).unwrap();
    }
    store.bind(binding("c1", "w1", BackendKind::Dsh)).unwrap();
    drop(store);

    let store = SqliteSettingsStore::open(&path).unwrap();
    let w1 = store.bindings(&"w1".into()).unwrap();
    let ids: Vec<&str> = w1.iter().map(|b| b.chat_id.as_str()).collect();
    assert_eq!(ids, ["c3", "c1", "c2"], "rebinding keeps a chat's tab position");
    assert_eq!(w1[1], binding("c1", "w1", BackendKind::Dsh));

    store.unbind(&"c1".into()).unwrap();
    store.unbind(&"c1".into()).unwrap();
    assert_eq!(store.bindings(&"w1".into()).unwrap().len(), 2);
    assert_eq!(store.bindings(&"w2".into()).unwrap(), [binding("other", "w2", BackendKind::Pi)]);
}

#[test]
fn default_agent_is_stored_and_cleared() {
    let dir = TempDir::new().unwrap();
    let store = SqliteSettingsStore::open(&dir.path().join("thinkrail.db")).unwrap();
    assert_eq!(store.prefs().unwrap().default_agent, None);
    assert_eq!(store.set_default_agent(Some(BackendKind::Omp)).unwrap().default_agent, Some(BackendKind::Omp));
    assert_eq!(store.prefs().unwrap().default_agent, Some(BackendKind::Omp));
    assert_eq!(store.set_default_agent(None).unwrap().default_agent, None);
}
