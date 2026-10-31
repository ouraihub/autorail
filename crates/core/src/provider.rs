//! Model providers, their API keys and the user's model preferences (agent-settings 04 §2.2, §2.4).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ProviderId, ProviderKeyId, Result};

/// Wire protocol of one provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol { OpenaiChat, OpenaiResponses, AnthropicMessages }

/// `Direct`: engines call the upstream themselves; `Gateway`: through the host loopback gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access { Direct, Gateway }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel { Off, Minimal, Low, Medium, High, Xhigh, Max }

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
    pub fn new(value: impl Into<String>) -> Self { Self(value.into()) }
    pub fn expose(&self) -> &str { &self.0 }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("Secret(***)") }
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
    fn update(&self, id: &ProviderId, expect_revision: u64, input: ProviderInput) -> Result<Provider>;
    /// Cascades to endpoints, models and keys.
    fn delete(&self, id: &ProviderId, expect_revision: u64) -> Result<()>;
    fn add_key(&self, id: &ProviderId, key: ProviderKeyInput) -> Result<Provider>;
    fn update_key(&self, id: &ProviderId, key_id: &ProviderKeyId, key: ProviderKeyUpdate) -> Result<Provider>;
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
}

pub trait ModelPrefService: Send + Sync {
    fn prefs(&self) -> Result<ModelPrefs>;
    fn set_default(&self, model: Option<ModelRef>, effort: Option<ThinkingLevel>) -> Result<ModelPrefs>;
    fn set_hidden(&self, model: &ModelRef, hidden: bool) -> Result<ModelPrefs>;
}

#[cfg(test)]
mod tests;
