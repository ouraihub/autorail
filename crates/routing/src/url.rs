//! API address normalization, after Cherry `formatApiHost` / `withoutTrailingApiVersion`
//! (agent-settings 04 §4.2).

use thinkrail_core::Protocol;

/// The base address an engine SDK is given for `protocol`.
///
/// OpenAI-style addresses end with an API version (`/v1` added when missing); Anthropic addresses
/// end without one, because the SDK appends `/v1/messages` itself. A trailing `#` disables both
/// rules and returns the address as written.
pub(crate) fn base_url(raw: &str, protocol: Protocol) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if let Some(verbatim) = trimmed.strip_suffix('#') {
        return verbatim.to_owned();
    }
    match protocol {
        Protocol::OpenaiChat | Protocol::OpenaiResponses => {
            if has_version_segment(trimmed) {
                trimmed.to_owned()
            } else {
                format!("{trimmed}/v1")
            }
        }
        Protocol::AnthropicMessages => without_trailing_version(trimmed).to_owned(),
    }
}

/// The full request address the engine SDK derives from its normalized base address.
/// A trailing `#` suppresses base-version normalization, not the SDK resource suffix.
pub fn preview(raw: &str, protocol: Protocol) -> String {
    let base = base_url(raw, protocol);
    match protocol {
        Protocol::OpenaiChat => format!("{base}/chat/completions"),
        Protocol::OpenaiResponses => format!("{base}/responses"),
        Protocol::AnthropicMessages => format!("{base}/v1/messages"),
    }
}

/// Path the model list is read from, appended to [`base_url`].
pub(crate) fn models_path(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat | Protocol::OpenaiResponses => "/models",
        Protocol::AnthropicMessages => "/v1/models",
    }
}

/// The address "同步模型" / "检测" request with `GET`; the same one [`crate::Probe`] uses.
pub fn models_url(raw: &str, protocol: Protocol) -> String {
    format!("{}{}", base_url(raw, protocol), models_path(protocol))
}

/// `v<digits>` optionally followed by `alpha` / `beta`.
fn is_version(segment: &str) -> bool {
    let Some(rest) = segment.strip_prefix(['v', 'V']) else { return false };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    let suffix = rest[digits..].to_ascii_lowercase();
    suffix.is_empty() || suffix == "alpha" || suffix == "beta"
}

/// Path segments only: the scheme and host are skipped.
fn path_segments(url: &str) -> impl Iterator<Item = &str> {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    after_scheme.split('/').skip(1)
}

fn has_version_segment(url: &str) -> bool {
    path_segments(url).any(is_version)
}

fn without_trailing_version(url: &str) -> &str {
    match url.rsplit_once('/') {
        Some((head, last)) if is_version(last) && path_segments(url).next().is_some() => head,
        _ => url,
    }
}
