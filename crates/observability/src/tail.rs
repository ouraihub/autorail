//! The bounded read-back the log page uses (after AutoNote / Aether `tail`): the last 256 KiB of
//! the newest `thinkrail-native.*.log`, at most 2000 lines, each parsed into an entry.

use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const TAIL_BYTES: u64 = 256 * 1024;
const MAX_LIMIT: usize = 2000;

/// One parsed line. `raw` is the line as written, for copying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Position in the returned list, oldest first.
    pub seq: usize,
    /// RFC 3339 with the `+08:00` offset as written; empty for an unparsable line.
    pub time: String,
    /// `ERROR` | `WARN` | `INFO` | `DEBUG` | `TRACE`.
    pub level: String,
    /// `op{id=… at=…}` and other span prefixes, outermost first.
    pub spans: String,
    /// The innermost structured `trace_id` field, when present.
    pub trace_id: Option<String>,
    /// The innermost structured `operation` field, when present.
    pub operation: Option<String>,
    /// Module path of the event.
    pub target: String,
    /// Message followed by its event fields.
    pub message: String,
    pub raw: String,
}

/// Parses one tracing line, retaining optional structured trace fields.
fn parse(seq: usize, line: &str) -> LogEntry {
    parse_fixed(seq, line).unwrap_or_else(|| LogEntry {
        seq,
        time: String::new(),
        level: "INFO".to_owned(),
        spans: String::new(),
        trace_id: None,
        operation: None,
        target: String::new(),
        message: line.to_owned(),
        raw: line.to_owned(),
    })
}

fn parse_fixed(seq: usize, line: &str) -> Option<LogEntry> {
    let (time, rest) = line.split_once(' ')?;
    if !time.contains('T') {
        return None;
    }
    let (level, rest) = rest.trim_start().split_once(' ')?;
    if !matches!(level, "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE") {
        return None;
    }
    let mut rest = rest.trim_start();
    let mut span_text = Vec::new();
    let mut trace_id = None;
    let mut operation = None;
    while let Some((end, _name, fields)) = take_span(rest) {
        span_text.push(rest[..end].trim_end_matches(':'));
        if let Some(value) = field_value(fields, "trace_id") {
            trace_id = Some(value);
        }
        if let Some(value) = field_value(fields, "operation") {
            operation = Some(value);
        }
        rest = rest[end..].trim_start();
    }
    let spans = span_text.join(": ");
    let (target, message) = rest
        .split_once(": ")
        .or_else(|| rest.strip_suffix(':').map(|t| (t, "")))?;
    if target.is_empty() || target.contains(' ') {
        return None;
    }
    Some(LogEntry {
        seq,
        time: time.to_owned(),
        level: level.to_owned(),
        spans,
        trace_id,
        operation,
        target: target.to_owned(),
        message: message.to_owned(),
        raw: line.to_owned(),
    })
}

/// Reads one tracing span prefix, respecting quoted field values containing braces or colons.
fn take_span(input: &str) -> Option<(usize, &str, &str)> {
    let open = input.find('{')?;
    let name = &input[..open];
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return None;
    }
    let bytes = input.as_bytes();
    let mut quoted = false;
    let mut escaped = false;
    let mut close = None;
    for index in open + 1..bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' && quoted {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if byte == b'}' && !quoted && bytes.get(index + 1) == Some(&b':') {
            close = Some(index);
            break;
        }
    }
    let close = close?;
    Some((close + 2, name, &input[open + 1..close]))
}

fn field_value(fields: &str, wanted: &str) -> Option<String> {
    let mut cursor = 0;
    while cursor < fields.len() {
        let rest = fields[cursor..].trim_start();
        let skipped = fields[cursor..].len() - rest.len();
        cursor += skipped;
        let key_end = rest.find(|c: char| c == '=' || c.is_whitespace())?;
        if rest.as_bytes().get(key_end) != Some(&b'=') {
            return None;
        }
        let key = &rest[..key_end];
        let value = &rest[key_end + 1..];
        let (value, consumed) = field_token(value);
        if key == wanted {
            return Some(value.to_owned());
        }
        cursor += key_end + 1 + consumed;
    }
    None
}

fn field_token(value: &str) -> (&str, usize) {
    if value.starts_with('"') {
        let bytes = value.as_bytes();
        let mut escaped = false;
        for index in 1..bytes.len() {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                return (&value[1..index], index + 1);
            }
        }
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    (&value[..end], end)
}

/// The newest file in `dir` (names sort by date); `None` when there is none.
fn newest(dir: &Path) -> io::Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut newest: Option<PathBuf> = None;
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with(&format!("{}.", crate::FILE_PREFIX))
            && name.ends_with(".log")
            && newest.as_ref().is_none_or(|current| path > *current)
        {
            newest = Some(path);
        }
    }
    Ok(newest)
}

/// Up to `limit` (at most 2000) complete lines, oldest first, from the end of the newest log in
/// `dir`. No log yet: an empty list.
pub fn tail(dir: &Path, limit: usize) -> io::Result<Vec<LogEntry>> {
    let Some(path) = newest(dir)? else {
        return Ok(Vec::new());
    };
    let mut file = File::open(&path)?;
    let size = file.metadata()?.len();
    let from = size.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    // The snapshot length bounds the read while the writer keeps appending.
    file.take(size - from).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    lines.pop(); // the empty tail after the last `\n`, or a line still being written
    if from > 0 && !lines.is_empty() {
        lines.remove(0); // cut mid-line
    }
    lines.retain(|line| !line.trim().is_empty());
    let start = lines.len().saturating_sub(limit.min(MAX_LIMIT));
    Ok(lines[start..]
        .iter()
        .enumerate()
        .map(|(seq, line)| parse(seq, line.trim_end_matches('\r')))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_nested_trace_fields() {
        let entry = parse(
            0,
            "2026-11-07T00:00:00+08:00 INFO operation{trace_id=abc operation=agent.session}: target: hello",
        );
        assert_eq!(entry.trace_id.as_deref(), Some("abc"));
        assert_eq!(entry.operation.as_deref(), Some("agent.session"));
    }
}
