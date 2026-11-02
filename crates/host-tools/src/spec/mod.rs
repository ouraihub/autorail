//! The seven `spec_*` tools (original `packages/spec-graph`). Names, schemas and model-facing wording
//! are the original's (`specs/spec-tools.json`); the spec root of a call is the chat's worktree.
//! `model` parses frontmatter, `edit` rewrites it in place, `store` globs and resolves paths,
//! `graph` derives links/slices/validation, `tools` formats each tool's result.

mod edit;
mod graph;
mod model;
mod store;
mod tools;

use std::{io::Write, path::Path};

use serde_json::Value;
use thinkrail_core::HostToolResult;

pub(crate) const SPEC_SPECS: &str = include_str!("../../specs/spec-tools.json");
const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;
/// Byte ceiling of one `spec_grep` page, leaving room for the header and continuation.
const GREP_PAGE_BYTES: usize = MAX_BYTES - 2048;

pub(crate) fn is_spec_tool(name: &str) -> bool {
    tools::NAMES.contains(&name)
}

/// Runs one spec tool against `root`; failures are `is_error` results.
pub(crate) fn call(root: &Path, name: &str, args: &Value) -> HostToolResult {
    match tools::run(root, name, args) {
        Ok((text, continuation)) => HostToolResult {
            text: bounded(text, &continuation),
            is_error: false,
            structured: None,
        },
        Err(message) => {
            let inline = format!("Error: {message}");
            let text = if fits(&inline) {
                inline
            } else {
                format!("Error:\n{message}")
            };
            HostToolResult {
                text: bounded(text, ""),
                is_error: true,
                structured: None,
            }
        }
    }
}

fn fits(text: &str) -> bool {
    text.len() <= MAX_BYTES && text.split('\n').count() <= MAX_LINES
}

/// Keeps model text within 2,000 lines / 50 KiB; the full text goes to a private temporary file.
fn bounded(mut text: String, continuation: &str) -> String {
    if !continuation.is_empty() {
        text.push('\n');
        text.push_str(continuation);
    }
    if fits(&text) {
        return text;
    }
    let notice = match capture(&text) {
        Ok(path) => format!(
            "Full output saved to: {}\nTemporary file; may expire. Read only relevant offset/limit ranges; use shell byte ranges for oversized single lines.",
            path.display()
        ),
        Err(error) => {
            tracing::warn!(%error, "spec tool output capture failed");
            "Full output unavailable: temporary-file capture failed. Narrow a read query; do not repeat a mutation to retrieve its output.".to_owned()
        }
    };
    let mut footer = format!(
        "\n\n[Output truncated: {} lines, {} bytes total. {notice}]",
        text.split('\n').count(),
        text.len()
    );
    if !continuation.is_empty() {
        footer.push('\n');
        footer.push_str(continuation);
    }
    let max_bytes = MAX_BYTES.saturating_sub(footer.len());
    let max_lines = MAX_LINES.saturating_sub(footer.split('\n').count());
    let mut head = String::new();
    for (i, line) in text.split('\n').take(max_lines).enumerate() {
        let needed = line.len() + usize::from(i > 0);
        if head.len() + needed > max_bytes {
            if i == 0 {
                head.push_str(&line[..line.floor_char_boundary(max_bytes)]);
            }
            break;
        }
        if i > 0 {
            head.push('\n');
        }
        head.push_str(line);
    }
    head + &footer
}

/// An owner-only (`0600`) file in the OS temporary directory, kept after return.
fn capture(text: &str) -> std::io::Result<std::path::PathBuf> {
    let mut file = tempfile::Builder::new()
        .prefix("pi-spec-output-")
        .suffix(".txt")
        .tempfile()?;
    file.write_all(text.as_bytes())?;
    let (_, path) = file.keep().map_err(|e| e.error)?;
    Ok(path)
}
