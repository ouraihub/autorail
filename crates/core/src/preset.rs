//! Built-in provider presets: public, non-secret templates the host seeds as disabled providers
//! (agent-settings 04 §7.6).
//!
//! The table itself lives in `lumakit_support::presets` so every LumaKit host shares one copy;
//! this module only maps its protocol onto [`Protocol`].

use crate::Protocol;
use lumakit_support::presets::PresetProtocol;
pub use lumakit_support::presets::{PROVIDER_PRESETS, ProviderPreset, provider_preset};

impl From<PresetProtocol> for Protocol {
    fn from(protocol: PresetProtocol) -> Self {
        match protocol {
            PresetProtocol::OpenaiChat => Self::OpenaiChat,
            PresetProtocol::OpenaiResponses => Self::OpenaiResponses,
            PresetProtocol::AnthropicMessages => Self::AnthropicMessages,
        }
    }
}
