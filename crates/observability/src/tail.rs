//! The bounded read-back the log page uses (after AutoNote / Aether `tail`): the last 256 KiB of
//! the newest `thinkrail-native.*.log`, at most 2000 lines. Each line is parsed with LumaKit's
//! `parse_line`, the same contract `GrepFormat` writes, so the page never keeps its own parser.

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
    /// The `TraceId` segment, when the line was written inside an operation.
    pub trace_id: Option<String>,
    /// The `ME` segment: the stable operation name.
    pub operation: Option<String>,
    /// The `EV` segment of lifecycle lines: `ENTRY`, `EXIT`, `ERROR`, `CANCEL`, `TIMEOUT`…
    pub event: Option<String>,
    /// The `RT` segment: elapsed milliseconds of a finished operation.
    pub duration_ms: Option<u64>,
    /// The `Layer` segment: the layer of the operation the line was written in (`ui`, `review`…).
    pub layer: Option<String>,
    /// The event's module (`Target`), or the `Layer` of a lifecycle line; empty when neither.
    pub component: String,
    /// The `Msg` segment; the whole line when it does not follow the format.
    pub message: String,
    pub raw: String,
}

fn parse(seq: usize, line: &str) -> LogEntry {
    let parsed = lumakit_observability::parse_line(line);
    LogEntry {
        seq,
        time: parsed.time,
        level: parsed.level,
        trace_id: parsed.trace_id,
        operation: parsed.operation,
        event: parsed.event,
        duration_ms: parsed.duration_ms,
        component: parsed
            .target
            .or_else(|| parsed.layer.clone())
            .unwrap_or_default(),
        layer: parsed.layer,
        message: parsed.message,
        raw: parsed.raw,
    }
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
        let prefixed = name
            .strip_prefix(crate::FILE_PREFIX)
            .is_some_and(|rest| rest.starts_with('.'));
        if prefixed
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
