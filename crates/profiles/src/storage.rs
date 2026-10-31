use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use thinkrail_core::{Error, IoContext, Result};
use tracing::debug;

use crate::state::State;

pub(crate) const FILE_NAME: &str = "agent-profiles.json";
const LOCK_NAME: &str = "agent-profiles.json.lock";

/// Missing file is an empty store; anything that is not a regular, valid document is `Corrupt`.
pub(crate) fn read(root: &Path) -> Result<State> {
    let path = root.join(FILE_NAME);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(Error::corrupt("agent-profiles.json 不是普通文件，未做任何修改")),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(State::empty()),
        Err(cause) => return Err(cause).context("读取 Agent 配置"),
    }
    let bytes = fs::read(&path).context("读取 Agent 配置")?;
    State::decode(&bytes)
}

/// Exclusive cross-process writer lock, released on drop.
///
/// The lock file is never deleted: removing a released lock would let two writers lock two
/// different inodes. A stale file left by a dead process is not locked and does not block.
#[derive(Debug)]
pub(crate) struct WriterLock {
    _file: File,
}

impl WriterLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self> {
        let started = Instant::now();
        fs::create_dir_all(root).context("创建数据目录")?;
        let file = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LOCK_NAME))
            .context("打开 Agent 配置锁文件")?;
        if let Err(cause) = fs2::FileExt::try_lock_exclusive(&file) {
            if cause.kind() == io::ErrorKind::WouldBlock
                || cause.raw_os_error() == fs2::lock_contended_error().raw_os_error()
            {
                debug!(code = "BUSY", "agent profile lock contended");
                return Err(Error::busy("另一个进程正在修改 Agent 配置，请稍后重试"));
            }
            return Err(cause).context("锁定 Agent 配置");
        }
        debug!(elapsed_ms = started.elapsed().as_millis() as u64, "agent profile lock acquired");
        Ok(Self { _file: file })
    }
}

/// Write to a private temporary file, fsync, close, then atomically replace the document.
pub(crate) fn write(root: &Path, _lock: &WriterLock, state: &State) -> Result<()> {
    let bytes = state.encode()?;
    let temporary = Temporary { path: root.join(format!("{FILE_NAME}.{}.tmp", uuid::Uuid::new_v4())), persisted: false };
    let mut file = private_options().write(true).create_new(true).open(&temporary.path).context("创建 Agent 配置临时文件")?;
    file.write_all(&bytes).context("写入 Agent 配置临时文件")?;
    file.sync_all().context("同步 Agent 配置临时文件")?;
    drop(file);
    temporary.persist(&root.join(FILE_NAME))
}

fn private_options() -> OpenOptions {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Removes the temporary file unless it was renamed into place.
struct Temporary {
    path: PathBuf,
    persisted: bool,
}

impl Temporary {
    fn persist(mut self, target: &Path) -> Result<()> {
        fs::rename(&self.path, target).context("替换 Agent 配置")?;
        self.persisted = true;
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.persisted
            && let Err(cause) = fs::remove_file(&self.path)
        {
            debug!(error = %cause, "failed to remove agent profile temporary file");
        }
    }
}
