use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thinkrail_core::{ChatBinding, Error, ErrorKind, Profile, ProfileCatalog, ProfileId, Result};

use crate::validate;

const SCHEMA_VERSION: u32 = 1;

/// On-disk document. `revisions` is append-only history; the last entry per id is current.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct State {
    schema_version: u32,
    pub(crate) default_profile_id: Option<ProfileId>,
    pub(crate) revisions: Vec<Profile>,
    pub(crate) bindings: Vec<ChatBinding>,
}

impl State {
    pub(crate) fn empty() -> Self {
        Self { schema_version: SCHEMA_VERSION, ..Self::default() }
    }

    /// Decode and check every invariant; any violation is `Corrupt`.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        let state: Self = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
        state.check().map_err(|_| corrupt())?;
        Ok(state)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|cause| Error::invalid(format!("无法序列化 Agent 配置：{cause}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn check(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::invalid("不支持的 Agent 配置版本"));
        }
        let mut latest: HashMap<&ProfileId, u64> = HashMap::new();
        for profile in &self.revisions {
            validate::profile_id(&profile.id)?;
            validate::revision(profile.config_revision)?;
            validate::input(&profile.input)?;
            let previous = latest.get(&profile.id).copied().unwrap_or(0);
            if profile.config_revision != previous + 1 {
                return Err(Error::invalid("档案版本历史不连续"));
            }
            latest.insert(&profile.id, profile.config_revision);
        }
        let mut chats = HashSet::new();
        let mut natives = HashSet::new();
        for binding in &self.bindings {
            validate::binding(binding)?;
            if latest.get(&binding.profile_id).copied().unwrap_or(0) < binding.config_revision {
                return Err(Error::invalid("绑定引用的档案版本不存在"));
            }
            let native = (&binding.profile_id, binding.config_revision, &binding.native_session_key);
            if !chats.insert(&binding.chat_id) || !natives.insert(native) {
                return Err(Error::invalid("重复的聊天绑定"));
            }
        }
        if let Some(id) = &self.default_profile_id {
            ensure_enabled(self.current(id, None)?)?;
        }
        Ok(())
    }

    /// Latest revision of `id`; `expected` turns a stale base revision into `Conflict`.
    pub(crate) fn current(&self, id: &ProfileId, expected: Option<u64>) -> Result<&Profile> {
        let profile = self
            .revisions
            .iter()
            .rev()
            .find(|profile| &profile.id == id)
            .ok_or_else(|| Error::not_found(format!("Agent 档案 {id}")))?;
        match expected {
            Some(expected) if profile.config_revision != expected => Err(Error::conflict(format!(
                "Agent 档案 {id} 当前是版本 {}，不是版本 {expected}",
                profile.config_revision
            ))),
            _ => Ok(profile),
        }
    }

    pub(crate) fn revision(&self, id: &ProfileId, revision: u64) -> Result<&Profile> {
        self.revisions
            .iter()
            .find(|profile| &profile.id == id && profile.config_revision == revision)
            .ok_or_else(|| Error::not_found(format!("Agent 档案 {id} 的版本 {revision}")))
    }

    pub(crate) fn catalog(&self) -> ProfileCatalog {
        let mut profiles: Vec<Profile> = Vec::new();
        let mut positions: HashMap<&ProfileId, usize> = HashMap::new();
        for profile in &self.revisions {
            if let Some(&index) = positions.get(&profile.id) {
                profiles[index] = profile.clone();
            } else {
                positions.insert(&profile.id, profiles.len());
                profiles.push(profile.clone());
            }
        }
        ProfileCatalog { profiles, default_profile_id: self.default_profile_id.clone() }
    }
}

pub(crate) fn ensure_enabled(profile: &Profile) -> Result<()> {
    if profile.input.enabled {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::ProfileDisabled, format!("{} 不能用于新聊天或设为默认", profile.input.label)))
    }
}

fn corrupt() -> Error {
    Error::corrupt("agent-profiles.json 无法读取、已损坏或版本不受支持，请修复或移走该文件")
}
