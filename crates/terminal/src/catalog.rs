use std::{
    collections::HashSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use thinkrail_core::{Error, IoContext, Result, WorkspaceId};

const FILE_NAME: &str = "terminals.json";
const LOCK_NAME: &str = "terminals.json.lock";
const MAX_BYTES: u64 = 1_048_576;
const SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_ID_BYTES: usize = 500;
pub(crate) const MAX_TITLE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredTab {
    pub workspace: WorkspaceId,
    pub tab_key: String,
    pub title: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    tabs: Vec<StoredTab>,
}

/// `terminals.json`: every mutation re-reads the file under an fs2 exclusive lock.
#[derive(Debug)]
pub(crate) struct Catalog {
    dir: PathBuf,
    local: Mutex<()>,
}

/// Holds both the in-process and the cross-process lock; the file is re-read on every lock.
#[derive(Debug)]
pub(crate) struct Locked<'a> {
    dir: &'a Path,
    _file: File,
    _local: MutexGuard<'a, ()>,
}

impl Catalog {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            local: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<Locked<'_>> {
        let local = self.local.lock();
        if let Some(metadata) = safe_metadata(&self.dir)?
            && !metadata.is_dir()
        {
            return Err(Error::corrupt("终端配置目录不是文件夹"));
        }
        fs::create_dir_all(&self.dir).context("创建终端配置目录")?;
        let path = self.dir.join(LOCK_NAME);
        if let Some(metadata) = safe_metadata(&path)?
            && !metadata.is_file()
        {
            return Err(Error::corrupt("终端配置锁不是普通文件"));
        }
        let file = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .context("打开终端配置锁")?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            tracing::debug!(%error, "terminal catalog lock contended");
            Error::busy("终端配置正被其他进程占用，请稍后重试")
        })?;
        safe_metadata(&path)?;
        Ok(Locked {
            dir: &self.dir,
            _file: file,
            _local: local,
        })
    }
}

impl Locked<'_> {
    pub(crate) fn read(&self) -> Result<Vec<StoredTab>> {
        let path = self.dir.join(FILE_NAME);
        let Some(metadata) = safe_metadata(&path)? else {
            return Ok(Vec::new());
        };
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err(Error::corrupt("终端配置不是普通文件或超过 1 MiB"));
        }
        let mut bytes = Vec::new();
        File::open(&path)
            .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
            .context("读取终端配置")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::corrupt("终端配置超过 1 MiB"));
        }
        let file: CatalogFile = serde_json::from_slice(&bytes)
            .map_err(|error| Error::corrupt(format!("终端配置无法解析：{error}")))?;
        if file.schema_version != SCHEMA_VERSION {
            return Err(Error::corrupt(format!(
                "终端配置版本 {} 不受支持",
                file.schema_version
            )));
        }
        let mut seen = HashSet::new();
        for tab in &file.tabs {
            validate_ids(&tab.workspace, &tab.tab_key)
                .map_err(|_| Error::corrupt("终端配置包含无效标签标识"))?;
            if tab.title.len() > MAX_TITLE_BYTES {
                return Err(Error::corrupt("终端配置标题过长"));
            }
            if !seen.insert((&tab.workspace, &tab.tab_key)) {
                return Err(Error::corrupt("终端配置包含重复标签"));
            }
        }
        Ok(file.tabs)
    }

    pub(crate) fn save(&self, tabs: &[StoredTab]) -> Result<()> {
        let file = CatalogFile {
            schema_version: SCHEMA_VERSION,
            tabs: tabs.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|error| Error::invalid(format!("终端配置无法序列化：{error}")))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(Error::invalid("终端标签过多，配置将超过 1 MiB"));
        }
        let target = self.dir.join(FILE_NAME);
        safe_metadata(&target)?;
        let temporary = self
            .dir
            .join(format!("terminals.{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = write_new(&temporary, &bytes)
            .and_then(|()| fs::rename(&temporary, &target))
            .context("保存终端配置");
        if result.is_err()
            && let Err(error) = fs::remove_file(&temporary)
        {
            tracing::debug!(%error, "terminal catalog temporary cleanup failed");
        }
        result?;
        #[cfg(unix)]
        File::open(self.dir)
            .and_then(|directory| directory.sync_all())
            .context("同步终端配置目录")?;
        Ok(())
    }
}

/// Workspace and tab identifiers must be non-empty and at most 500 bytes.
pub(crate) fn validate_ids(workspace: &WorkspaceId, tab_key: &str) -> Result<()> {
    let workspace = workspace.as_str();
    if workspace.is_empty()
        || tab_key.is_empty()
        || workspace.len() > MAX_ID_BYTES
        || tab_key.len() > MAX_ID_BYTES
    {
        return Err(Error::invalid("终端标识须为 1..500 字节"));
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = private_options().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn private_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Refuses symlinks and Windows reparse points (junctions) so the catalog never escapes `dir`.
fn safe_metadata(path: &Path) -> Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[allow(unused_mut)]
            let mut linked = metadata.file_type().is_symlink();
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                linked |= metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
            }
            if linked {
                return Err(Error::forbidden("终端配置路径不能是符号链接或目录联接"));
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("检查终端配置路径"),
    }
}
