use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thinkrail_core::{Error, IoContext, Result, ReviewComment};

const MAX_FILE: u64 = 16 * 1024 * 1024;

pub(crate) fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(hash.len() * 2);
    for byte in hash.iter() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Workspace-relative, `/`-separated, inside `root` after canonicalization (symlinks/junctions that
/// leave the worktree are rejected).
fn contained(root: &Path, relative: &str) -> Result<(String, PathBuf)> {
    let normalized = relative.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || path.is_absolute()
        || normalized.contains(':')
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::forbidden("评审路径必须是工作区内的相对路径"));
    }
    let root = fs::canonicalize(root).context("解析工作区")?;
    let full = fs::canonicalize(root.join(path))
        .map_err(|_| Error::not_found(format!("文件 {normalized}")))?;
    if !full.starts_with(&root) {
        return Err(Error::forbidden("评审文件不在工作区内"));
    }
    Ok((normalized, full))
}

fn read_text(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(MAX_FILE + 1).read_to_end(&mut bytes))
        .context("读取评审文件")?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::invalid("评审文件超过 16 MiB 上限"));
    }
    String::from_utf8(bytes).map_err(|_| Error::invalid("评审文件不是 UTF-8 文本"))
}

#[derive(Debug)]
pub(crate) struct Anchor {
    pub path: String,
    pub line: u32,
    pub quote: String,
    pub line_hash: String,
    pub file_hash: String,
}

/// Captures an anchor; `quote` must be non-blank and occur on `line` (1-based).
pub(crate) fn capture(
    root: &Path,
    relative: &str,
    line: u32,
    quote: Option<&str>,
) -> Result<Anchor> {
    let (path, full) = contained(root, relative)?;
    let text = read_text(&full)?;
    let lines: Vec<&str> = text.lines().collect();
    let line_text = usize::try_from(line)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| lines.get(i).copied())
        .ok_or_else(|| Error::invalid("评审行号超出文件范围"))?;
    let quote = quote.filter(|q| !q.trim().is_empty()).unwrap_or(line_text);
    if quote.trim().is_empty() || !line_text.contains(quote) {
        return Err(Error::invalid("引用文字必须出现在指定行，且不能为空"));
    }
    Ok(Anchor {
        path,
        line,
        quote: quote.to_owned(),
        line_hash: digest(line_text.as_bytes()),
        file_hash: digest(text.as_bytes()),
    })
}

/// Re-locates `comment` against the current file: original line hash first, then a unique quote
/// match. Anything else (missing file, ambiguous quote) marks it stale.
pub(crate) fn refresh(root: &Path, comment: &mut ReviewComment) {
    let located = contained(root, &comment.path)
        .and_then(|(_, full)| read_text(&full))
        .ok()
        .and_then(|text| {
            let lines: Vec<&str> = text.lines().collect();
            let original = comment.line as usize;
            if original >= 1
                && lines
                    .get(original - 1)
                    .is_some_and(|l| digest(l.as_bytes()) == comment.line_hash)
            {
                return Some(comment.line);
            }
            let mut matches = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains(comment.text_quote.as_str()));
            match (matches.next(), matches.next()) {
                (Some((index, l)), None) => {
                    comment.line_hash = digest(l.as_bytes());
                    u32::try_from(index + 1).ok()
                }
                _ => None,
            }
        });
    match located {
        Some(line) => {
            comment.line = line;
            comment.stale = false;
        }
        None => comment.stale = true,
    }
}

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
