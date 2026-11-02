//! Credential masking of engine output (stderr summaries and engine error messages).

const SENSITIVE: [&str; 5] = ["key", "token", "secret", "password", "authorization"];

/// Masks credentials in engine output: the value after a `NAME=` / `NAME:` whose name contains
/// key|token|secret|password|authorization, and secret-shaped words on other lines that mention
/// such a word. Prose like "No API key found" stays readable.
pub(crate) fn redact(text: &str) -> String {
    text.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str) -> String {
    if !is_sensitive(line) {
        return line.to_owned();
    }
    for (at, separator) in line
        .char_indices()
        .filter(|(_, c)| matches!(c, '=' | ':' | '：'))
    {
        let before = line[..at].trim_end();
        let start = before
            .char_indices()
            .rfind(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-')))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if is_sensitive(&before[start..]) {
            return format!("{}{separator} ***", &line[..at]);
        }
    }
    line.split(' ')
        .map(|word| if secret_shaped(word) { "***" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_sensitive(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SENSITIVE.iter().any(|word| lower.contains(word))
}

fn secret_shaped(word: &str) -> bool {
    let word = word.trim_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '-' | '_'));
    word.len() >= 16
        && word.chars().any(|c| c.is_ascii_digit())
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}
