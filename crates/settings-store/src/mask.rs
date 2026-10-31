//! Key masking for display, same rule as Cherry `maskApiKey` (agent-settings 04 §2.2).

/// Longer than 8 characters: first 2 + `****` + last 4; otherwise a fixed `••••••••`.
pub(crate) fn mask(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".to_owned();
    }
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}****{tail}")
}
