//! 模型设置页文案与显示格式。

use thinkrail_core::ThinkingLevel;

pub const TITLE: &str = "模型";
pub const DEFAULT: &str = "默认模型";
pub const SESSION_MODEL: &str = "会话模型";
pub const EFFORT: &str = "力度";
pub const VISIBLE: &str = "可见模型";
pub const SHOW: &str = "展开";
pub const HIDE: &str = "收起";
pub const SEARCH: &str = "搜索模型 ID 或名称";
pub const EMPTY: &str = "没有可用模型。请先在“提供方”中添加并启用模型。";
pub const NO_MATCHES: &str = "没有匹配的模型。";
pub const NEW_MODELS_VISIBLE: &str = "新模型会自动显示。";
pub const RETRY: &str = "重试";
pub const NONE: &str = "首个可用";
pub const LOADING: &str = "正在加载模型设置…";
pub const SAVING: &str = "正在保存模型设置…";
pub const NO_EFFORT: &str = "无可用力度";
pub const DEFAULT_EFFORT: &str = "模型默认力度";
pub const UNAVAILABLE_DEFAULT: &str = "已保存的默认模型当前不可用，将使用首个可用模型。可重新选择或清除默认值。";

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

pub fn model_label(provider: &str, name: &str, id: &str) -> String {
    if name == id {
        format!("{provider} · {name}")
    } else {
        format!("{provider} · {name} ({id})")
    }
}

pub fn model_details(context: Option<u32>, reasoning: bool, vision: bool) -> String {
    let mut details = match context {
        Some(tokens) => format!("上下文：{tokens} tokens"),
        None => "默认上下文".to_owned(),
    };
    if reasoning {
        details.push_str(" · 推理");
    }
    if vision {
        details.push_str(" · 视觉");
    }
    details
}

pub fn visibility_summary(hidden: usize) -> String {
    if hidden == 0 {
        NEW_MODELS_VISIBLE.to_owned()
    } else {
        format!("已隐藏模型：{hidden}。{NEW_MODELS_VISIBLE}")
    }
}

pub fn load_error(error: &str) -> String {
    format!("无法加载模型设置：{error}")
}

pub fn save_error(error: &str) -> String {
    format!("无法保存模型设置：{error}")
}
