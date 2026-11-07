//! Model providers, their API keys and the user's model preferences (agent-settings 04 §2.2, §2.4).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ProviderId, ProviderKeyId, Result};

/// Wire protocol of one provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
}

/// `Direct`: engines call the upstream themselves; `Gateway`: through the host loopback gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Direct,
    Gateway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// The effort a launch uses for a saved preference: the highest level the model declares that is
/// not above `saved`. `None` when nothing is saved, the model declares no levels, or every declared
/// level is above `saved`. Routing and the settings page share this one rule.
pub fn clamp_effort(
    saved: Option<ThinkingLevel>,
    efforts: &[ThinkingLevel],
) -> Option<ThinkingLevel> {
    let saved = saved?;
    efforts.iter().copied().filter(|e| *e <= saved).max()
}

/// What a model does (Aether `ModelType`). Only `Chat` models are offered to engines.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    #[default]
    Chat,
    Image,
    Video,
    Tts,
    Stt,
    Embedding,
    Rerank,
    Unknown,
}

impl ModelType {
    pub const ALL: [ModelType; 8] = [
        Self::Chat,
        Self::Image,
        Self::Video,
        Self::Tts,
        Self::Stt,
        Self::Embedding,
        Self::Rerank,
        Self::Unknown,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub protocol: Protocol,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    pub model_id: String,
    /// Inferred from the id when added; the user can change it.
    pub model_type: ModelType,
    pub name: String,
    /// Ordered, non-empty; the first is the default.
    pub protocols: Vec<Protocol>,
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub vision: bool,
    pub reasoning: bool,
    pub tools: bool,
    /// Levels a reasoning model declares; empty when it does not reason.
    pub efforts: Vec<ThinkingLevel>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub name: String,
    pub endpoints: Vec<Endpoint>,
    pub default_protocol: Protocol,
    pub headers: Vec<Header>,
    pub access: Access,
    pub enabled: bool,
    pub models: Vec<ProviderModel>,
}

/// A key as the UI sees it: never the plaintext, only `masked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKeySummary {
    pub id: ProviderKeyId,
    pub label: Option<String>,
    pub masked: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: ProviderId,
    pub revision: u64,
    pub input: ProviderInput,
    pub keys: Vec<ProviderKeySummary>,
    /// `Some` for a provider seeded from a built-in preset ([`crate::PROVIDER_PRESETS`]); set
    /// only at seeding, never by `create` / `update`. Preset providers cannot be deleted.
    pub preset_id: Option<String>,
}

/// Plaintext secret. `Debug` prints `Secret(***)`; no `Display` / `Serialize`; `expose` is the
/// only way to read the value.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Clone)]
pub struct ProviderKeyInput {
    pub label: Option<String>,
    pub key: Secret,
    pub enabled: bool,
}

impl fmt::Debug for ProviderKeyInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderKeyInput")
            .field("label", &self.label)
            .field("key", &self.key)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// Edit a key: with `key` = None only the label and enabled state change.
#[derive(Clone)]
pub struct ProviderKeyUpdate {
    pub label: Option<String>,
    pub key: Option<Secret>,
    pub enabled: bool,
}

impl fmt::Debug for ProviderKeyUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderKeyUpdate")
            .field("label", &self.label)
            .field("key", &self.key)
            .field("enabled", &self.enabled)
            .finish()
    }
}

/// The selected key; its value is used only inside the host process.
#[derive(Clone)]
pub struct SelectedKey {
    pub id: ProviderKeyId,
    pub label: Option<String>,
    pub value: Secret,
}

impl fmt::Debug for SelectedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectedKey")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("value", &self.value)
            .finish()
    }
}

pub trait ProviderService: Send + Sync {
    /// Sorted by `sort`; no plaintext.
    fn providers(&self) -> Result<Vec<Provider>>;
    fn provider(&self, id: &ProviderId) -> Result<Provider>;
    fn create(&self, input: ProviderInput) -> Result<Provider>;
    fn update(
        &self,
        id: &ProviderId,
        expect_revision: u64,
        input: ProviderInput,
    ) -> Result<Provider>;
    /// Cascades to endpoints, models and keys.
    fn delete(&self, id: &ProviderId, expect_revision: u64) -> Result<()>;
    fn add_key(&self, id: &ProviderId, key: ProviderKeyInput) -> Result<Provider>;
    fn update_key(
        &self,
        id: &ProviderId,
        key_id: &ProviderKeyId,
        key: ProviderKeyUpdate,
    ) -> Result<Provider>;
    fn delete_key(&self, id: &ProviderId, key_id: &ProviderKeyId) -> Result<Provider>;
    /// Show / copy: the plaintext of one key; does not advance rotation.
    fn reveal_key(&self, key_id: &ProviderKeyId) -> Result<Secret>;
    /// Round-robin to the next enabled key; advances rotation. None when no key is enabled.
    fn next_key(&self, id: &ProviderId) -> Result<Option<SelectedKey>>;
    /// Creates each missing built-in preset as a disabled provider with one endpoint, no models
    /// and no keys. A preset whose name is already taken by a user provider is skipped.
    /// Returns every provider afterwards.
    fn seed_presets(&self) -> Result<Vec<Provider>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: ProviderId,
    pub model_id: String,
}

/// Only hidden models are recorded; new models are visible by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrefs {
    pub default_model: Option<ModelRef>,
    pub default_effort: Option<ThinkingLevel>,
    pub hidden: Vec<ModelRef>,
    /// 编码搭档 choice per engine; an engine missing here has no choice.
    pub engine_providers: Vec<(crate::BackendKind, crate::EngineProvider)>,
    /// 编码搭档 设为默认: the engine new chats run (agent-settings 07).
    pub default_agent: Option<crate::BackendKind>,
    /// GPT 上下文窗口: per-model context windows that replace the provider setting when routing
    /// (agent-settings 08 §6). The provider's own `context_window` is never rewritten.
    pub context_overrides: Vec<(ModelRef, u32)>,
}

/// The range a context-window override may take (the original's `MODEL_CONTEXT_WINDOW_LIMITS`).
pub const CONTEXT_OVERRIDE_RANGE: std::ops::RangeInclusive<u32> = 272_000..=1_000_000;

impl ModelPrefs {
    pub fn engine_provider(&self, backend: crate::BackendKind) -> Option<&crate::EngineProvider> {
        self.engine_providers
            .iter()
            .find(|(b, _)| *b == backend)
            .map(|(_, p)| p)
    }

    pub fn context_override(&self, model: &ModelRef) -> Option<u32> {
        self.context_overrides
            .iter()
            .find(|(m, _)| m == model)
            .map(|(_, tokens)| *tokens)
    }
}

pub trait ModelPrefService: Send + Sync {
    fn prefs(&self) -> Result<ModelPrefs>;
    fn set_default(
        &self,
        model: Option<ModelRef>,
        effort: Option<ThinkingLevel>,
    ) -> Result<ModelPrefs>;
    fn set_hidden(&self, model: &ModelRef, hidden: bool) -> Result<ModelPrefs>;
    /// 编码搭档: which provider row `backend` uses.
    fn set_engine_provider(
        &self,
        backend: crate::BackendKind,
        choice: crate::EngineProvider,
    ) -> Result<ModelPrefs>;
    fn set_default_agent(&self, backend: Option<crate::BackendKind>) -> Result<ModelPrefs>;
    /// Sets (`Some`) or removes (`None`) context-window overrides in one write. A value outside
    /// [`CONTEXT_OVERRIDE_RANGE`] or an unknown model rejects the whole batch.
    fn set_context_overrides(&self, changes: &[(ModelRef, Option<u32>)]) -> Result<ModelPrefs>;
}

#[cfg(test)]
mod tests;
