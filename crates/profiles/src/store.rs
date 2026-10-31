use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use thinkrail_core::{
    ChatBinding, ChatId, Error, Profile, ProfileCatalog, ProfileId, ProfileInput, ProfileService, Result,
    WorkspaceId,
};
use tracing::info;

use crate::{
    state::{State, ensure_enabled},
    storage::{self, WriterLock},
    validate::{self, MAX_REVISION},
};

/// [`ProfileService`] backed by `<data>/agent-profiles.json`.
///
/// Every call rereads the file, so writers in other processes are always observed. Mutations take
/// an exclusive cross-process lock, reread under it, and atomically replace the file.
#[derive(Debug)]
pub struct JsonProfileStore {
    root: PathBuf,
    /// Serializes writers in this process so they queue instead of reporting `Busy` to each other.
    writer: Mutex<()>,
}

impl JsonProfileStore {
    /// Open the store in `data_dir`. A missing file is an empty catalog; nothing is created until
    /// the first write. Damaged storage fails with `Corrupt`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        storage::read(data_dir)?;
        Ok(Self { root: data_dir.to_owned(), writer: Mutex::new(()) })
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let _writer = self.writer.lock();
        let lock = WriterLock::acquire(&self.root)?;
        let mut state = storage::read(&self.root)?;
        let result = change(&mut state)?;
        storage::write(&self.root, &lock, &state)?;
        Ok(result)
    }
}

impl ProfileService for JsonProfileStore {
    fn catalog(&self) -> Result<ProfileCatalog> {
        Ok(storage::read(&self.root)?.catalog())
    }

    fn create(&self, input: ProfileInput) -> Result<Profile> {
        validate::input(&input)?;
        let profile = self.mutate(|state| {
            let profile = Profile { id: ProfileId::generate(), config_revision: 1, input };
            state.revisions.push(profile.clone());
            Ok(profile)
        })?;
        info!(backend = profile.input.backend_id.label(), "agent profile created");
        Ok(profile)
    }

    fn update(&self, id: &ProfileId, expect_revision: u64, input: ProfileInput) -> Result<Profile> {
        validate::profile_id(id)?;
        validate::revision(expect_revision)?;
        validate::input(&input)?;
        let profile = self.mutate(|state| {
            state.current(id, Some(expect_revision))?;
            if expect_revision == MAX_REVISION {
                return Err(Error::invalid("配置版本号已用尽"));
            }
            if !input.enabled && state.default_profile_id.as_ref() == Some(id) {
                state.default_profile_id = None;
            }
            let profile = Profile { id: id.clone(), config_revision: expect_revision + 1, input };
            state.revisions.push(profile.clone());
            Ok(profile)
        })?;
        info!(backend = profile.input.backend_id.label(), revision = profile.config_revision, "agent profile updated");
        Ok(profile)
    }

    fn delete(&self, id: &ProfileId, expect_revision: u64) -> Result<()> {
        validate::profile_id(id)?;
        validate::revision(expect_revision)?;
        let backend = self.mutate(|state| {
            let backend = state.current(id, Some(expect_revision))?.input.backend_id;
            if state.bindings.iter().any(|binding| &binding.profile_id == id) {
                return Err(Error::forbidden("该档案已有聊天在使用，不能删除；可以改为禁用"));
            }
            state.revisions.retain(|profile| &profile.id != id);
            if state.default_profile_id.as_ref() == Some(id) {
                state.default_profile_id = None;
            }
            Ok(backend)
        })?;
        info!(backend = backend.label(), "agent profile deleted");
        Ok(())
    }

    fn set_default(&self, id: Option<&ProfileId>) -> Result<ProfileCatalog> {
        if let Some(id) = id {
            validate::profile_id(id)?;
        }
        let catalog = self.mutate(|state| {
            if let Some(id) = id {
                ensure_enabled(state.current(id, None)?)?;
            }
            state.default_profile_id = id.cloned();
            Ok(state.catalog())
        })?;
        info!(cleared = id.is_none(), "default agent profile changed");
        Ok(catalog)
    }

    fn revision(&self, id: &ProfileId, revision: u64) -> Result<Profile> {
        validate::profile_id(id)?;
        validate::revision(revision)?;
        storage::read(&self.root)?.revision(id, revision).cloned()
    }

    fn bind(&self, binding: ChatBinding) -> Result<()> {
        validate::binding(&binding)?;
        let (chat, workspace) = (binding.chat_id.clone(), binding.workspace_id.clone());
        let created = self.mutate(|state| {
            if let Some(existing) = state.bindings.iter().find(|entry| entry.chat_id == binding.chat_id) {
                return if existing == &binding {
                    Ok(false)
                } else {
                    Err(Error::conflict("该聊天已绑定到另一个 Agent 会话，绑定不可修改"))
                };
            }
            if state.bindings.iter().any(|entry| {
                entry.profile_id == binding.profile_id
                    && entry.config_revision == binding.config_revision
                    && entry.native_session_key == binding.native_session_key
            }) {
                return Err(Error::conflict("该原生会话已被同一档案版本下的另一个聊天绑定"));
            }
            ensure_enabled(state.current(&binding.profile_id, Some(binding.config_revision))?)?;
            state.bindings.push(binding);
            Ok(true)
        })?;
        if created {
            info!(chat = %chat, workspace = %workspace, "chat bound to agent profile revision");
        }
        Ok(())
    }

    fn binding(&self, chat: &ChatId) -> Result<Option<ChatBinding>> {
        let state = storage::read(&self.root)?;
        Ok(state.bindings.into_iter().find(|binding| &binding.chat_id == chat))
    }

    fn bindings(&self, workspace: &WorkspaceId) -> Result<Vec<ChatBinding>> {
        let state = storage::read(&self.root)?;
        Ok(state.bindings.into_iter().filter(|binding| &binding.workspace_id == workspace).collect())
    }

    fn unbind(&self, chat: &ChatId) -> Result<()> {
        validate::chat_id(chat)?;
        let removed = self.mutate(|state| {
            let before = state.bindings.len();
            state.bindings.retain(|binding| &binding.chat_id != chat);
            Ok(state.bindings.len() != before)
        })?;
        if removed {
            info!(chat = %chat, "chat binding removed");
        }
        Ok(())
    }
}
