//! The filesystem side (original `core/store.ts`): the spec glob and the indexable-path rule.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::model::{Frontmatter, is_spec, parse_frontmatter};

const IGNORED_DIRS: [&str; 9] =
    ["node_modules", ".git", "dist", "build", ".claude", ".superpowers", ".gsd", ".mockups", ".playwright-mcp"];
const EXTENSION: &str = ".md";

#[derive(Debug)]
pub(super) struct SpecFile {
    pub rel: String,
    pub abs: PathBuf,
    pub content: String,
    pub fm: Frontmatter,
}

/// Every spec under `root`, in walk order: each directory's `.md` files and non-ignored
/// subdirectories sorted together by name, symlinks never followed.
// ponytail: fresh scan + parse per tool call, O(markdown files) each time; add a per-root
// (mtime, size) parse cache like the original `SpecIndex` if large repos make calls slow.
pub(super) fn scan(root: &Path) -> Vec<SpecFile> {
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<SpecFile>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut candidates: Vec<(String, bool)> = read
        .flatten()
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            let name = entry.file_name().into_string().ok()?;
            if kind.is_dir() && !IGNORED_DIRS.contains(&name.as_str()) {
                Some((name, true))
            } else if kind.is_file() && name.ends_with(EXTENSION) {
                Some((name, false))
            } else {
                None
            }
        })
        .collect();
    // ponytail: byte-order name sort; the original compares NFC-normalized names first, which only
    // differs for canonically-equivalent spellings. Add NFC keys if such names ever matter.
    candidates.sort();
    for (name, directory) in candidates {
        let abs = dir.join(&name);
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if directory {
            walk(&abs, &rel, out);
        } else if let Ok(content) = fs::read_to_string(&abs)
            && let Some(fm) = parse_frontmatter(&content).filter(is_spec)
        {
            out.push(SpecFile { rel, abs, content, fm });
        }
    }
}

fn fold(name: &str) -> String {
    name.to_lowercase()
}

fn resolve_segment(entries: &[String], segment: &str, exists: bool) -> Result<String, String> {
    let mut name = segment.to_owned();
    if exists && !entries.iter().any(|e| e == segment) {
        let folded = fold(segment);
        let matches: Vec<&String> = entries.iter().filter(|e| fold(e) == folded).collect();
        match matches.as_slice() {
            [] => return Err(format!("Path component \"{segment}\" resolves to no entry its parent directory lists")),
            [only] => name = (*only).clone(),
            [first, rest @ ..] => {
                let rest: Vec<&str> = rest.iter().map(|s| s.as_str()).collect();
                return Err(format!(
                    "Path component \"{segment}\" matches more than one entry on this filesystem (\"{first}\", \"{}\")",
                    rest.join("\", \"")
                ));
            }
        }
    }
    if IGNORED_DIRS.contains(&fold(&name).as_str()) {
        return Err(format!("Path is inside an ignored directory (\"{name}\") and would not be indexed"));
    }
    Ok(name)
}

/// Could the glob ever see `path`? Answers with the canonical `(rel, abs)` the index will report.
/// Refuses absolute, drive/stream (`:`) and escaping paths, non-`.md` files, ignored directories
/// (`.git` included, in any case), and any symlink/junction component beneath the root.
pub(super) fn resolve_spec_path(root: &Path, path: &str) -> Result<(String, PathBuf), String> {
    if path.trim().is_empty() {
        return Err("Path must not be empty.".into());
    }
    if Path::new(path).has_root() || path.starts_with(['/', '\\']) {
        return Err(format!("Path must be root-relative, not absolute: {path}"));
    }
    if cfg!(windows) && path.contains(':') {
        return Err(format!("Path must not use Windows drive or stream syntax: {path}"));
    }
    if !path.ends_with(EXTENSION) {
        return Err(format!("Spec files must end in {EXTENSION}: {path}"));
    }
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split(separators) {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|s| *s != "..") => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    if segments.contains(&"..") || segments.is_empty() {
        return Err(format!("Path must stay inside the project root: {path}"));
    }
    // Win32 silently strips a trailing dot/space, so `dir./SPEC.md` would land in `dir/`.
    if cfg!(windows) && segments.iter().any(|s| s.ends_with(['.', ' '])) {
        return Err(format!("Path components must not end in a dot or space on Windows: {path}"));
    }
    if !root.is_dir() {
        return Err(format!("Project root does not exist: {}", root.display()));
    }

    let mut walked = root.to_path_buf();
    let mut walked_exists = true;
    let mut canonical = Vec::with_capacity(segments.len());
    for segment in segments {
        let mut entries = Vec::new();
        if walked_exists {
            let Ok(read) = fs::read_dir(&walked) else {
                return Err(format!("Path passes through a directory the index cannot list: {path}"));
            };
            entries = read.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
        }
        let exists = walked_exists && fs::symlink_metadata(walked.join(segment)).is_ok();
        let name = resolve_segment(&entries, segment, exists).map_err(|error| format!("{error}: {path}"))?;
        walked.push(&name);
        // `is_symlink` also covers Windows junctions (name-surrogate reparse points).
        if fs::symlink_metadata(&walked).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("Path passes through a symlink, which the index never follows: {path}"));
        }
        canonical.push(name);
        walked_exists = exists;
    }

    let rel = canonical.join("/");
    if !rel.ends_with(EXTENSION) {
        return Err(format!("Spec files must end in {EXTENSION}: {path}"));
    }
    if !walked.starts_with(root) {
        return Err(format!("Path must stay inside the project root: {path}"));
    }
    Ok((rel, walked))
}
