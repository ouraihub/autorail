//! The user's theme and font folders under the data dir (native-multi-agent 08 A5, A13): scans
//! with the original's caps, the change signature the poller compares, and the font copy that
//! never replaces a good file with a bad one. Only the global data dir, never a workspace.

use std::{
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const THEME_SUFFIX: &str = ".theme.json";
pub const MAX_THEME_BYTES: u64 = 256 * 1024;
pub const MAX_THEME_FILES: usize = 64;
pub const MAX_FONT_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_FONT_FILES: usize = 128;

/// Why a listed file was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileProblem {
    TooLarge,
    /// Past the per-folder cap; the first files by name are read.
    TooMany,
    Unreadable(String),
    /// `.ttc` / `.otc`: DirectWrite takes single faces only.
    Collection,
    /// `.woff` / `.woff2`: DirectWrite cannot load them (08 A13).
    Unsupported,
}

/// One theme file: its text, or why it was not read.
#[derive(Debug)]
pub struct ThemeFile {
    pub file_name: String,
    pub contents: Result<String, FileProblem>,
}

/// One font file: its bytes, or why it was not read.
#[derive(Debug)]
pub struct FontFile {
    pub file_name: String,
    pub bytes: Result<Vec<u8>, FileProblem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontKind {
    Face,
    Collection,
    Unsupported,
}

fn font_kind(name: &str) -> Option<FontKind> {
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext)?;
    match extension {
        "ttf" | "otf" => Some(FontKind::Face),
        "ttc" | "otc" => Some(FontKind::Collection),
        "woff" | "woff2" => Some(FontKind::Unsupported),
        _ => None,
    }
}

/// Visible entry names in `dir` that `keep` accepts, sorted; a missing folder is empty.
fn names(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.') && keep(name))
        .collect();
    names.sort();
    names
}

fn unreadable(error: &io::Error) -> FileProblem {
    FileProblem::Unreadable(error.to_string())
}

/// Reads `path` unless it is over `max` bytes.
fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, FileProblem> {
    let size = fs::metadata(path).map_err(|e| unreadable(&e))?.len();
    if size > max {
        return Err(FileProblem::TooLarge);
    }
    fs::read(path).map_err(|e| unreadable(&e))
}

pub fn read_themes(dir: &Path) -> Vec<ThemeFile> {
    names(dir, |name| name.ends_with(THEME_SUFFIX))
        .into_iter()
        .enumerate()
        .map(|(index, file_name)| {
            let contents = if index >= MAX_THEME_FILES {
                Err(FileProblem::TooMany)
            } else {
                read_capped(&dir.join(&file_name), MAX_THEME_BYTES).and_then(|bytes| {
                    String::from_utf8(bytes)
                        .map_err(|_| FileProblem::Unreadable("不是 UTF-8 文本".to_owned()))
                })
            };
            ThemeFile {
                file_name,
                contents,
            }
        })
        .collect()
}

pub fn read_fonts(dir: &Path) -> Vec<FontFile> {
    names(dir, |name| font_kind(name).is_some())
        .into_iter()
        .enumerate()
        .map(|(index, file_name)| {
            let bytes = match font_kind(&file_name) {
                _ if index >= MAX_FONT_FILES => Err(FileProblem::TooMany),
                Some(FontKind::Collection) => Err(FileProblem::Collection),
                Some(FontKind::Unsupported) => Err(FileProblem::Unsupported),
                Some(FontKind::Face) | None => read_capped(&dir.join(&file_name), MAX_FONT_BYTES),
            };
            FontFile { file_name, bytes }
        })
        .collect()
}

/// Changes when a scanned file is added, removed, resized or rewritten (name, size, mtime).
pub fn signature(dir: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut rows: Vec<(String, u64, u128)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            Some((name, meta.len(), mtime))
        })
        .collect();
    rows.sort();
    rows.hash(&mut hasher);
    hasher.finish()
}

/// Why a picked font could not be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyProblem {
    File(FileProblem),
    /// The validator's reason, shown as detail.
    Invalid(String),
}

/// Copies `source` into `dir` under its own name: reads it once, validates the bytes, writes a
/// dot-prefixed temp file beside the target and renames it over, so a rejected file never
/// touches an existing one and the scan never sees a partial copy. Replacing a name is allowed
/// when the folder is full; adding one is not.
pub fn copy_font(
    source: &Path,
    dir: &Path,
    validate: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<String, CopyProblem> {
    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| CopyProblem::File(FileProblem::Unreadable("文件名无效".to_owned())))?;
    match font_kind(&file_name) {
        Some(FontKind::Face) => {}
        Some(FontKind::Collection) => return Err(CopyProblem::File(FileProblem::Collection)),
        Some(FontKind::Unsupported) | None => {
            return Err(CopyProblem::File(FileProblem::Unsupported));
        }
    }
    let bytes = read_capped(source, MAX_FONT_BYTES).map_err(CopyProblem::File)?;
    validate(&bytes).map_err(CopyProblem::Invalid)?;
    fs::create_dir_all(dir).map_err(|e| CopyProblem::File(unreadable(&e)))?;
    let full = |dir: &Path| {
        let existing = names(dir, |name| font_kind(name).is_some());
        existing.len() >= MAX_FONT_FILES && !existing.contains(&file_name)
    };
    if full(dir) {
        return Err(CopyProblem::File(FileProblem::TooMany));
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp: PathBuf = dir.join(format!(".upload-{}-{nonce}.tmp", std::process::id()));
    let result = fs::write(&temp, &bytes)
        .map_err(|e| CopyProblem::File(unreadable(&e)))
        .and_then(|()| {
            // Checked again right before the rename: another copy may have filled the folder.
            if full(dir) {
                return Err(CopyProblem::File(FileProblem::TooMany));
            }
            fs::rename(&temp, dir.join(&file_name)).map_err(|e| CopyProblem::File(unreadable(&e)))
        });
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map(|()| file_name)
}

#[cfg(test)]
mod tests;
