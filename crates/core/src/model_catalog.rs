//! Pure rules for a provider's model list (agent-settings 04 §7.6): grouping, capability hints
//! inferred from a model id, and merging a "同步模型" selection. No I/O.

use std::collections::{BTreeSet, HashSet};

use crate::{DiscoveredModel, ModelType, Protocol, ProviderModel};

/// Group shown in the model list: the id up to the first `-`, `/` or `:` (`gpt-4o` → `gpt`,
/// `Qwen/Qwen3` → `Qwen`). Empty when the id starts with a separator; callers show "其他".
pub fn model_group(model_id: &str) -> &str {
    model_id.split(['-', '/', ':']).next().unwrap_or_default()
}

/// What a model id alone suggests. Unknown families get no limits; reasoning is never guessed
/// because a wrong guess sends thinking parameters a model rejects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelHints {
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub vision: bool,
    /// Chat models can call tools; embedding, image, audio and rerank models cannot.
    pub tools: bool,
}

/// Type keywords, checked in order; the first hit wins (Aether `infer.rs`).
const TYPES: &[(&[&str], ModelType)] = &[
    (&["embed"], ModelType::Embedding),
    (&["rerank"], ModelType::Rerank),
    (&["tts", "speech"], ModelType::Tts),
    (&["whisper", "stt", "transcri"], ModelType::Stt),
    (&["dall-e", "image", "flux", "imagen"], ModelType::Image),
    (&["video", "sora", "veo"], ModelType::Video),
];

/// The type an id suggests; anything without a type keyword is `Chat`.
pub fn model_type(model_id: &str) -> ModelType {
    let id = model_id.to_ascii_lowercase();
    TYPES
        .iter()
        .find(|(words, _)| words.iter().any(|w| id.contains(w)))
        .map_or(ModelType::Chat, |&(_, t)| t)
}

/// `(prefix, context window, max output, vision)`; known variants before their family default.
const FAMILIES: &[(&str, u32, u32, bool)] = &[
    ("claude", 200_000, 8_192, true),
    ("gpt-3.5", 16_385, 4_096, false),
    ("gpt", 128_000, 16_384, true),
    ("gemini", 1_048_576, 8_192, true),
    ("deepseek", 128_000, 8_192, false),
    ("qwen", 131_072, 8_192, false),
    ("glm", 128_000, 4_096, false),
    ("kimi", 131_072, 8_192, false),
    ("moonshot-v1-8k", 8_192, 4_096, false),
    ("moonshot-v1-32k", 32_768, 4_096, false),
    ("moonshot", 131_072, 4_096, false),
];

/// OpenAI `o<digit>…` reasoning models; `omni-…` is not one.
const O_SERIES: (u32, u32, bool) = (200_000, 100_000, true);

/// Image-input markers that override a family's non-vision default (`qwen-vl-max`, `glm-4v`).
const VISION_MARKERS: &[&str] = &["vision", "-vl", "vl-", "4v"];

pub fn model_hints(model_id: &str) -> ModelHints {
    let id = model_id.to_ascii_lowercase();
    let chat = model_type(&id) == ModelType::Chat;
    let name = id.rsplit('/').next().unwrap_or_default();
    let marked = VISION_MARKERS.iter().any(|marker| name.contains(marker));
    let family = FAMILIES
        .iter()
        .find(|(prefix, ..)| name.starts_with(prefix))
        .map(|&(_, context, output, vision)| (context, output, vision))
        .or_else(|| {
            let mut chars = name.chars();
            (chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit()))
                .then_some(O_SERIES)
        });
    match family {
        Some((context, output, vision)) if chat => ModelHints {
            context_window: Some(context),
            max_tokens: Some(output),
            vision: vision || marked,
            tools: true,
        },
        _ => ModelHints {
            context_window: None,
            max_tokens: None,
            vision: chat && marked,
            tools: chat,
        },
    }
}

/// A new, enabled model for `found` on the line `protocol`, filled from the probe and the hints.
pub fn model_from_discovery(found: &DiscoveredModel, protocol: Protocol) -> ProviderModel {
    let hints = model_hints(&found.id);
    let name = found
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    ProviderModel {
        model_id: found.id.clone(),
        model_type: model_type(&found.id),
        name: name.unwrap_or(&found.id).to_owned(),
        protocols: vec![protocol],
        context_window: found.context_window.or(hints.context_window),
        max_tokens: hints.max_tokens,
        vision: hints.vision,
        reasoning: false,
        tools: hints.tools,
        efforts: Vec::new(),
        enabled: true,
    }
}

/// The model list after confirming a "同步模型" selection made on the line `protocol`.
///
/// - A model the probe did not list is kept unchanged.
/// - A listed model already on this line stays when picked (its own fields untouched; a missing
///   context window is filled from the probe) and leaves this line when not picked; it is
///   dropped once it is on no line.
/// - A listed model only on other lines joins this line when picked, otherwise is unchanged.
/// - Picked models not present yet are appended in probe order.
///
/// Duplicate ids in `discovered` count once, so confirming twice never duplicates a model.
pub fn merge_picked(
    existing: &[ProviderModel],
    discovered: &[DiscoveredModel],
    picked: &BTreeSet<String>,
    protocol: Protocol,
) -> Vec<ProviderModel> {
    let listed: HashSet<&str> = discovered.iter().map(|found| found.id.as_str()).collect();
    let mut out = Vec::with_capacity(existing.len() + picked.len());
    for model in existing {
        if !listed.contains(model.model_id.as_str()) {
            out.push(model.clone());
            continue;
        }
        let on_line = model.protocols.contains(&protocol);
        let keep = picked.contains(&model.model_id);
        let mut model = model.clone();
        if keep {
            if !on_line {
                model.protocols.push(protocol);
            }
            if model.context_window.is_none() {
                model.context_window = discovered
                    .iter()
                    .find(|f| f.id == model.model_id)
                    .and_then(|f| f.context_window);
            }
        } else if on_line {
            model.protocols.retain(|item| *item != protocol);
            if model.protocols.is_empty() {
                continue;
            }
        }
        out.push(model);
    }
    let mut seen: HashSet<String> = out.iter().map(|model| model.model_id.clone()).collect();
    for found in discovered {
        if picked.contains(&found.id) && seen.insert(found.id.clone()) {
            out.push(model_from_discovery(found, protocol));
        }
    }
    out
}
