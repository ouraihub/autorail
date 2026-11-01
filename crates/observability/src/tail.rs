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
    /// `op{id=… at=…}` prefixes (outermost first), empty outside an operation.
    pub spans: String,
    /// Module path of the event.
    pub target: String,
    /// Message followed by its `k=v` fields.
    pub message: String,
    pub raw: String,
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
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.starts_with(&format!("{}.", crate::FILE_PREFIX)) && name.ends_with(".log")
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
    let Some(path) = newest(dir)? else { return Ok(Vec::new()) };
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
    Ok(lines[start..].iter().enumerate().map(|(seq, line)| parse(seq, line.trim_end_matches('\r'))).collect())
}

/// `<time> <LEVEL> [<span>{fields}:]* <target>: <message>`; anything else is an INFO line with
/// the whole text as its message.
fn parse(seq: usize, line: &str) -> LogEntry {
    parse_fixed(seq, line).unwrap_or_else(|| LogEntry {
        seq,
        time: String::new(),
        level: "INFO".to_owned(),
        spans: String::new(),
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
    let rest = rest.trim_start();
    // Spans are `name{fields}:` written back to back; fields may hold spaces, never `}:`.
    let mut spans_end = 0;
    while let Some(open) = rest[spans_end..].find('{') {
        let name = &rest[spans_end..spans_end + open];
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            break;
        }
        let Some(close) = rest[spans_end + open..].find("}:") else { break };
        spans_end += open + close + 2;
    }
    let spans = rest[..spans_end].trim_end_matches(':');
    let rest = rest[spans_end..].trim_start();
    let (target, message) = rest.split_once(": ").or_else(|| rest.strip_suffix(':').map(|t| (t, "")))?;
    if target.contains(' ') {
        return None;
    }
    Some(LogEntry {
        seq,
        time: time.to_owned(),
        level: level.to_owned(),
        spans: spans.to_owned(),
        target: target.to_owned(),
        message: message.to_owned(),
        raw: line.to_owned(),
    })
}
