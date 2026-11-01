//! Spec frontmatter (original `core/parse.ts`): the is-a-spec rule, the lossy scalar/string-list read
//! dialect, the `FIELDS` registry and finite vocabularies, and canonical serialization.

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

pub(super) const ID: &str = "id";
pub(super) const TYPE: &str = "type";
pub(super) const STATUS: &str = "status";
pub(super) const TITLE: &str = "title";
pub(super) const PARENT: &str = "parent";
pub(super) const DEPENDS_ON: &str = "depends-on";
pub(super) const REFERENCES: &str = "references";
pub(super) const IMPLEMENTS: &str = "implements";
pub(super) const COVERS: &str = "covers";
pub(super) const TAGS: &str = "tags";

pub(super) const IDENTITY_FIELDS: [&str; 2] = [ID, TYPE];
pub(super) const LIST_FIELDS: [&str; 5] = [DEPENDS_ON, REFERENCES, IMPLEMENTS, COVERS, TAGS];
pub(super) const LINK_KINDS: [&str; 4] = [PARENT, DEPENDS_ON, REFERENCES, IMPLEMENTS];
pub(super) const SLICE_DIRECTIONS: [&str; 3] = ["subtree", "ancestors", "neighbors"];
pub(super) const BOM: char = '\u{feff}';
const FENCE: &str = "---";

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

pub(super) type Frontmatter = BTreeMap<String, FmValue>;

/// The fence-interior text (each line's trailing `\r` dropped) and the body after the closing fence.
pub(super) struct Split<'a> {
    pub fm_text: String,
    pub body: &'a str,
}

pub(super) fn split_frontmatter(content: &str) -> Option<Split<'_>> {
    let text = content.strip_prefix(BOM).unwrap_or(content);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim() != FENCE {
        return None;
    }
    let mut offset = first.len();
    let mut interior = Vec::new();
    for line in lines {
        offset += line.len();
        if line.trim() == FENCE {
            return Some(Split { fm_text: interior.join("\n"), body: &text[offset..] });
        }
        let line = line.strip_suffix('\n').unwrap_or(line);
        interior.push(line.strip_suffix('\r').unwrap_or(line));
    }
    None
}

/// `None` when the file has no frontmatter block or the block is not a YAML mapping.
pub(super) fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    let split = split_frontmatter(content)?;
    if split.fm_text.trim().is_empty() {
        return Some(Frontmatter::new());
    }
    let Value::Mapping(map) = serde_yaml_ng::from_str::<Value>(&split.fm_text).ok()? else { return None };
    let mut fm = Frontmatter::new();
    for (key, value) in &map {
        let Some(key) = scalar_text(key) else { continue };
        let value = match value {
            Value::Sequence(items) => FmValue::List(items.iter().filter_map(scalar_text).collect()),
            other => match scalar_text(other) {
                Some(text) => FmValue::Scalar(text),
                None => continue,
            },
        };
        fm.insert(key, value);
    }
    Some(fm)
}

pub(super) fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub(super) fn scalar<'a>(fm: &'a Frontmatter, key: &str) -> Option<&'a str> {
    match fm.get(key) {
        Some(FmValue::Scalar(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

pub(super) fn list<'a>(fm: &'a Frontmatter, key: &str) -> Vec<&'a str> {
    match fm.get(key) {
        Some(FmValue::List(items)) => items.iter().map(String::as_str).collect(),
        Some(FmValue::Scalar(s)) if !s.is_empty() => vec![s.as_str()],
        _ => Vec::new(),
    }
}

pub(super) fn is_spec(fm: &Frontmatter) -> bool {
    IDENTITY_FIELDS.iter().all(|field| scalar(fm, field).is_some())
}

/// Canonical new frontmatter in the given field order, lists inline, empty values dropped.
pub(super) fn serialize_frontmatter(fields: &[(&str, FmValue)]) -> String {
    let mut out = format!("{FENCE}\n");
    for (key, value) in fields {
        let empty = match value {
            FmValue::Scalar(s) => s.is_empty(),
            FmValue::List(items) => items.is_empty(),
        };
        if !empty {
            out.push_str(&field_line(key, value));
            out.push('\n');
        }
    }
    out.push_str(FENCE);
    out.push('\n');
    out
}

/// One top-level `key: value` line; lists render as an inline flow sequence.
pub(super) fn field_line(key: &str, value: &FmValue) -> String {
    let key = if plain_key(key) { key.to_owned() } else { quoted(key) };
    match value {
        FmValue::Scalar(s) => format!("{key}: {}", encode(s, false)),
        FmValue::List(items) => {
            let items: Vec<String> = items.iter().map(|item| encode(item, true)).collect();
            format!("{key}: [{}]", items.join(", "))
        }
    }
}

/// Plain when the plain spelling reads back as exactly this string, else a double-quoted scalar.
fn encode(text: &str, in_flow: bool) -> String {
    let plain = !text.is_empty() && !text.contains(['\n', '\r']) && text.trim() == text && {
        let doc = if in_flow { format!("k: [{text}]") } else { format!("k: {text}") };
        match serde_yaml_ng::from_str::<Value>(&doc).ok().as_ref().and_then(|v| v.get("k")) {
            Some(Value::String(s)) => !in_flow && s == text,
            Some(Value::Sequence(items)) => in_flow && matches!(items.as_slice(), [Value::String(s)] if s == text),
            _ => false,
        }
    };
    if plain { text.to_owned() } else { quoted(text) }
}

fn plain_key(key: &str) -> bool {
    !key.is_empty() && !key.contains(['\n', '\r']) && key.trim() == key && {
        match serde_yaml_ng::from_str::<Value>(&format!("{key}: x")) {
            Ok(Value::Mapping(map)) => map.len() == 1 && map.contains_key(key),
            _ => false,
        }
    }
}

/// A JSON string literal is a valid YAML double-quoted scalar.
fn quoted(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}
