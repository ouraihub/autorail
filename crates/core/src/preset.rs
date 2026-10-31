//! Built-in provider presets: public, non-secret templates the host seeds as disabled providers
//! (agent-settings 04 §7.6). Mirrors Aether `packages/presets/src/china.ts`.

use crate::Protocol;

/// One preset. It carries no key, no models and no enabled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    /// Stable identity stored in `providers.preset_id`; never shown.
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub protocol: Protocol,
    /// Where the user signs up for a key; always `https://`.
    pub api_key_url: &'static str,
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "qiniu",
        name: "七牛云",
        base_url: "https://api.qnaigc.com",
        protocol: Protocol::OpenaiChat,
        api_key_url: "https://s.qiniu.com/BfyeU3",
    },
    ProviderPreset {
        id: "siliconflow",
        name: "硅基流动",
        base_url: "https://api.siliconflow.cn/v1",
        protocol: Protocol::OpenaiChat,
        api_key_url: "https://cloud.siliconflow.cn/i/ieP7r6Bx",
    },
    ProviderPreset {
        id: "fenno",
        name: "Fenno",
        base_url: "https://api.fenno.ai/v1",
        protocol: Protocol::OpenaiChat,
        api_key_url: "https://api.fenno.ai/s/vWbM",
    },
];

/// The preset with `id`, if it is still built in.
pub fn provider_preset(id: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|preset| preset.id == id)
}
