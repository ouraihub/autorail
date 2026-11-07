//! 模型设置页文案与显示格式。

use thinkrail_core::{BackendKind, EngineNote, ModelStatus, ThinkingLevel};

pub const TITLE: &str = "模型";
pub const DEFAULT: &str = "默认模型";
pub const SESSION_MODEL: &str = "会话模型";
pub const VISIBLE: &str = "可见模型";
pub const SEARCH: &str = "搜索模型";
pub const EMPTY: &str = "没有可用模型。请先在“提供方”中添加并启用模型。";
pub const NO_MATCHES: &str = "没有匹配的模型。";
pub const NEW_MODELS_VISIBLE: &str = "新模型会自动显示。";
pub const RETRY: &str = "重试";
/// The empty default: nothing is passed, so each engine picks with its own rules (08 §4.3).
pub const NONE: &str = "引擎默认";
pub const SAVING: &str = "正在保存模型设置…";
/// Effort select when nothing is saved or the model declares no levels.
pub const EFFORT_DEFAULT: &str = "力度 默认";
pub const EFFORT_NONE: &str = "力度 —";
/// The effort list's first item: clears the saved effort.
pub const EFFORT_CLEAR: &str = "默认";
/// Prefixed to the selected effort on the trigger ("力度 高").
pub const EFFORT_PREFIX: &str = "力度 ";
const NEW_SESSIONS: &str = "，新会话将使用各引擎自己的默认模型。";

pub fn effort_label(effort: ThinkingLevel) -> &'static str {
    match effort {
        ThinkingLevel::Off => "关闭",
        ThinkingLevel::Minimal => "最低",
        ThinkingLevel::Low => "低",
        ThinkingLevel::Medium => "中",
        ThinkingLevel::High => "高",
        ThinkingLevel::Xhigh => "极高",
        ThinkingLevel::Max => "最高",
    }
}

/// Token counts as the original shows them (`chat/modelPicker.ts`): whole millions as `1M`,
/// everything else in thousands.
pub fn format_context(tokens: u32) -> String {
    if tokens >= 1_000_000 && tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else {
        format!("{}K", tokens / 1000)
    }
}

pub fn model_label(provider: &str, name: &str, id: &str) -> String {
    if name == id {
        format!("{provider} · {name}")
    } else {
        format!("{provider} · {name} ({id})")
    }
}

/// A visible-models row's metadata: "{提供方} · {上下文}", plus " · 推理" for reasoning models.
/// `None` means the provider left the window empty and routing uses its default.
pub fn model_meta(provider: &str, context: Option<u32>, reasoning: bool) -> String {
    let context = context.map_or_else(|| "默认上下文".to_owned(), format_context);
    let mut meta = format!("{provider} · {context}");
    if reasoning {
        meta.push_str(" · 推理");
    }
    meta
}

pub fn visibility_summary(hidden: usize) -> String {
    if hidden == 0 {
        NEW_MODELS_VISIBLE.to_owned()
    } else {
        format!("已隐藏 {hidden} 个模型。{NEW_MODELS_VISIBLE}")
    }
}

/// Why the saved default cannot be used; `None` when it can.
pub fn unavailable(status: &ModelStatus) -> Option<String> {
    Some(match status {
        ModelStatus::Available => return None,
        ModelStatus::ProviderMissing => format!("默认模型所属的提供方已删除{NEW_SESSIONS}"),
        ModelStatus::ProviderDisabled { provider } => {
            format!("默认模型所属的提供方「{provider}」已停用{NEW_SESSIONS}")
        }
        ModelStatus::ModelMissing => format!("默认模型已从提供方中删除{NEW_SESSIONS}"),
        ModelStatus::ModelUnusable { model } => {
            format!("默认模型「{model}」已停用或不是聊天模型{NEW_SESSIONS}")
        }
    })
}

/// How one engine's 编码搭档 choice affects these settings.
pub fn engine_note(backend: BackendKind, note: &EngineNote) -> String {
    let engine = backend.label();
    match note {
        EngineNote::Official => format!("{engine} 使用自己的配置，不使用这里的默认模型和力度。"),
        EngineNote::OnlyProvider { provider } => {
            format!("{engine} 只使用提供方「{provider}」的模型。")
        }
        EngineNote::ProviderUnavailable => {
            format!("{engine} 在编码搭档中选择的提供方已删除或停用，启动会失败。")
        }
    }
}

pub const CONTEXT_TITLE: &str = "GPT 上下文窗口";
pub const CONTEXT_ALL: &str = "全局";
pub const CONTEXT_NONE: &str = "没有符合条件的模型";
pub const CONTEXT_EACH: &str = "按模型分别设置";
pub const CONTEXT_MIXED: &str = "按模型分别设置";
/// The three choices of every context control, in order.
pub const CONTEXT_CHOICES: [&str; 3] = ["默认", "1M", "自定义"];
pub const CONTEXT_PLACEHOLDER: &str = "272,000–1,000,000";
pub const CONTEXT_INPUT: &str = "自定义上下文窗口（token）";
pub const CONTEXT_INVALID: &str = "请输入 272,000–1,000,000 之间的 token 数。";
pub const CONTEXT_NOTE: &str = "只对新会话生效；运行中的会话保持原来的上下文。";

pub fn context_available(count: usize) -> String {
    format!("{count} 个可用模型")
}

/// A per-model row's description; `tokens` is the provider setting or the routing default.
pub fn context_model(provider: &str, tokens: u32) -> String {
    format!(
        "{provider} · {} token（提供方设置）",
        format_context(tokens)
    )
}

pub fn load_error(error: &str) -> String {
    format!("无法加载模型设置：{error}")
}

pub fn save_error(error: &str) -> String {
    format!("无法保存模型设置：{error}")
}
