//! Provider draft value helpers. UI state remains owned by `mod.rs`.

use gpui_kit::{component::{input::InputState, select::SelectItem}, Context, Entity, SharedString, Window};
use thinkrail_core::{Header, Protocol, ThinkingLevel};
use url::Url;

use super::{HeaderFields, ProvidersPage};
use crate::strings::settings_providers as s;

pub(crate) const PROTOCOLS: [Protocol; 3] = [
    Protocol::OpenaiChat,
    Protocol::AnthropicMessages,
    Protocol::OpenaiResponses,
];
pub(crate) const EFFORTS: [ThinkingLevel; 7] = [
    ThinkingLevel::Off, ThinkingLevel::Minimal, ThinkingLevel::Low, ThinkingLevel::Medium,
    ThinkingLevel::High, ThinkingLevel::Xhigh, ThinkingLevel::Max,
];

#[derive(Clone)]
pub(crate) struct ProtocolOption { pub(crate) title: SharedString, pub(crate) value: Protocol }
impl SelectItem for ProtocolOption {
    type Value = Protocol;
    fn title(&self) -> SharedString { self.title.clone() }
    fn value(&self) -> &Protocol { &self.value }
}
pub(crate) type ProtocolOptions = gpui_kit::component::select::SearchableVec<ProtocolOption>;

pub(crate) fn protocol_options() -> ProtocolOptions {
    ProtocolOptions::new(PROTOCOLS.into_iter().map(|protocol| ProtocolOption { title: protocol_label(protocol).into(), value: protocol }).collect::<Vec<_>>())
}
pub(crate) fn protocol_label(protocol: Protocol) -> &'static str { match protocol {
    Protocol::OpenaiChat => s::CHAT,
    Protocol::OpenaiResponses => s::RESPONSES,
    Protocol::AnthropicMessages => s::ANTHROPIC,
} }
pub(crate) fn protocol_index(protocol: Protocol) -> usize { PROTOCOLS.iter().position(|item| *item == protocol).unwrap_or(0) }
pub(crate) fn effort_label(effort: ThinkingLevel) -> &'static str { match effort {
    ThinkingLevel::Off => "关闭", ThinkingLevel::Minimal => "minimal", ThinkingLevel::Low => "low",
    ThinkingLevel::Medium => "medium", ThinkingLevel::High => "high", ThinkingLevel::Xhigh => "xhigh", ThinkingLevel::Max => "max",
} }
pub(crate) fn parse_url(raw: &str) -> Result<String, &'static str> {
    let value = raw.trim();
    if value.is_empty() { return Ok(String::new()); }
    let parsed = Url::parse(value).map_err(|_| s::URL_REQUIRED)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(s::URL_REQUIRED);
    }
    Ok(value.to_owned())
}
pub(crate) fn set_input(field: &Entity<InputState>, value: String, window: &mut Window, cx: &mut Context<impl Sized>) { field.update(cx, |input, cx| input.set_value(value, window, cx)); }
pub(crate) fn parse_headers(fields: &[HeaderFields], cx: &Context<ProvidersPage>) -> Result<Vec<Header>, &'static str> {
    let mut headers = Vec::new();
    for field in fields {
        let name = field.name.read(cx).value().trim().to_owned();
        let value = field.value.read(cx).value().trim().to_owned();
        if name.is_empty() && value.is_empty() { continue; }
        if name.is_empty() || value.is_empty() { return Err(s::HEADER_INVALID); }
        headers.push(Header { name, value });
    }
    Ok(headers)
}
