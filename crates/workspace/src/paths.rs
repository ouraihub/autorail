use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use thinkrail_core::{Error, ErrorKind, IoContext, Result};

/// `.git` in any spelling Windows resolves to the same entry (trailing dots/spaces, 8.3 alias).
pub(crate) fn forbidden_component(component: &str) -> bool {
    let component = component.trim_end_matches(['.', ' ']);
    [".git", "git~1", ".git~1"]
        .iter()
        .any(|name| component.eq_ignore_ascii_case(name))
}

/// Normalizes a workspace-relative input to `a/b/c`; rejects absolute paths, `..`, NUL, NTFS
/// streams (`:`), Git metadata and names Windows would silently trim.
pub(crate) fn safe_relative(input: &str) -> Result<String> {
    if input.contains(['\0', ':']) || input.starts_with(['/', '\\']) {
        return Err(Error::forbidden("只允许工作区内的相对路径"));
    }
    let input = input.replace('\\', "/");
    let mut parts = Vec::new();
    for part in input.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || forbidden_component(part) || part.ends_with(['.', ' ']) {
            return Err(Error::forbidden("不允许访问上级目录或 Git 元数据"));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// Resolves `input` under `root` one component at a time so a symlink or junction that points
/// outside the worktree (or into `.git`) is rejected even if the final target looks harmless.
pub(crate) fn resolve_contained(root: &Path, input: &str) -> Result<PathBuf> {
    let relative = safe_relative(input)?;
    let mut current = root.to_path_buf();
    for part in relative.split('/').filter(|p| !p.is_empty()) {
        current = fs::canonicalize(current.join(part)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::not_found(format!("路径 {relative}")),
            _ => Error::Io {
                context: "解析路径".into(),
                source: e,
            },
        })?;
        let escaped = current
            .strip_prefix(root)
            .map(|inner| inner.components().any(|c| matches!(c, Component::Normal(name) if name.to_str().is_none_or(forbidden_component))))
            .unwrap_or(true);
        if escaped {
            return Err(Error::forbidden("路径超出工作区或指向 Git 元数据"));
        }
    }
    Ok(current)
}

/// For paths that may not exist (deleted files in a diff): the nearest existing ancestor must be
/// contained.
pub(crate) fn validate_missing_leaf(root: &Path, relative: &str) -> Result<()> {
    let mut ancestor = root.join(relative);
    while !ancestor.try_exists().context("检查路径")? {
        if !ancestor.pop() {
            return Err(Error::forbidden("路径没有位于工作区内的上级目录"));
        }
    }
    let inner = ancestor
        .strip_prefix(root)
        .map_err(|_| Error::forbidden("路径超出工作区"))?;
    let inner = inner
        .to_str()
        .ok_or_else(|| Error::invalid("路径不是 UTF-8"))?;
    match resolve_contained(root, inner) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == Some(ErrorKind::NotFound) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Every component below `root` up to `path` must be a real directory, not a symlink, junction or
/// other reparse point. Guards the managed-worktree tree before anything is removed.
pub(crate) fn reject_links(root: &Path, path: &Path) -> Result<()> {
    let inner = path
        .strip_prefix(root)
        .map_err(|_| Error::forbidden("路径不在数据目录内"))?;
    let mut current = root.to_path_buf();
    for component in inner.components() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current).context("检查托管目录")?;
        if is_link(&metadata) || fs::canonicalize(&current).context("检查托管目录")? != current
        {
            return Err(Error::forbidden("托管目录中存在符号链接或 junction"));
        }
    }
    Ok(())
}

pub(crate) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub(crate) fn bounded_read(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .context("读取内容")?;
    if bytes.len() > limit {
        return Err(Error::invalid(format!(
            "内容超过 {} MiB 上限",
            limit / 1024 / 1024
        )));
    }
    Ok(bytes)
}

pub(crate) fn utf8(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::invalid("路径不是 UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_reject_escape_and_git_metadata() {
        for bad in [
            "../x",
            "/etc",
            "C:\\x",
            "a/../b",
            ".git/config",
            ".GIT/config",
            ".git./x",
            "git~1/x",
            "a/b.",
            "f:s",
        ] {
            assert!(safe_relative(bad).is_err(), "{bad}");
        }
        assert_eq!(
            safe_relative("目录\\中文 空格.txt").unwrap(),
            "目录/中文 空格.txt"
        );
        assert_eq!(safe_relative("./a//b").unwrap(), "a/b");
        assert_eq!(safe_relative("").unwrap(), "");
    }
}
