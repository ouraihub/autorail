//! Contained, bounded reads of workspace directories and files.

use std::fs::{self, File};

use thinkrail_core::{DirEntry, EntryKind, Error, FileContent, IoContext, Result, WorkspaceId};

use super::GitWorkspaceService;
use crate::{
    git::MAX_OUTPUT,
    paths::{bounded_read, forbidden_component, is_link, resolve_contained, utf8},
};

impl GitWorkspaceService {
    pub(super) fn read_dir(&self, id: &WorkspaceId, path: &str) -> Result<Vec<DirEntry>> {
        let root = self.root(id)?;
        let directory = resolve_contained(&root, path)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(directory).context("读取目录")? {
            let entry = entry.context("读取目录")?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if forbidden_component(&name) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| Error::forbidden("路径超出工作区"))?
                .to_path_buf();
            let relative = utf8(&relative)?.replace('\\', "/");
            // Escaping links are not navigable entries.
            let Ok(resolved) = resolve_contained(&root, &relative) else {
                continue;
            };
            let metadata = fs::metadata(&resolved).context("读取目录项")?;
            let kind = if metadata.is_dir() {
                EntryKind::Dir
            } else if metadata.is_file() {
                EntryKind::File
            } else {
                continue;
            };
            entries.push(DirEntry {
                name,
                path: relative,
                kind,
            });
        }
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Dir)
                .cmp(&(b.kind != EntryKind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub(super) fn read_file(&self, id: &WorkspaceId, path: &str) -> Result<FileContent> {
        let root = self.root(id)?;
        let file = resolve_contained(&root, path)?;
        let metadata = fs::symlink_metadata(&file).context("读取文件")?;
        if !metadata.is_file() || is_link(&metadata) {
            return Err(Error::invalid("不是普通文件"));
        }
        let bytes = bounded_read(File::open(&file).context("读取文件")?, MAX_OUTPUT)?;
        if bytes.contains(&0) {
            return Ok(FileContent::Binary {
                byte_len: bytes.len() as u64,
            });
        }
        Ok(match String::from_utf8(bytes) {
            Ok(text) => FileContent::Text(text),
            Err(error) => FileContent::Binary {
                byte_len: error.as_bytes().len() as u64,
            },
        })
    }
}
