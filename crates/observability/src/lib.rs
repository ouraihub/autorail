//! Logging (native-multi-agent 06): the global subscriber, daily files under `<data>/logs` with
//! Beijing-time stamps, a runtime-switchable level, the bounded tail the log page reads and the
//! `op` span that ties one user action's lines together.

mod op;
mod tail;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thinkrail_core::LogLevel;
use time::{UtcOffset, format_description::well_known::Rfc3339};
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    EnvFilter, Registry, fmt, fmt::time::OffsetTime, layer::SubscriberExt as _, reload,
    util::SubscriberInitExt as _,
};

pub use op::{finish, op};
pub use tail::{LogEntry, tail};

/// File names are `thinkrail-native.YYYY-MM-DD.log` (the appender's UTC date).
pub const FILE_PREFIX: &str = "thinkrail-native";
/// Daily files kept; older ones are deleted on rotation.
const KEEP_FILES: usize = 14;
/// Overrides the stored level when set (`EnvFilter` syntax, e.g. `thinkrail_agents=debug`).
const ENV_VAR: &str = "THINKRAIL_LOG";
/// China Standard Time; fixed, so the stamp never depends on the machine's zone settings.
const BEIJING: UtcOffset = match UtcOffset::from_hms(8, 0, 0) {
    Ok(offset) => offset,
    Err(_) => UtcOffset::UTC,
};

/// `debug` raises only this app's crates (`thinkrail_*`, the `thinkrail-native` binary); the
/// database and renderer libraries stay at `info`, or their internals would flood the page.
fn filter(level: LogLevel) -> EnvFilter {
    match level {
        LogLevel::Info => EnvFilter::new("info"),
        LogLevel::Debug => EnvFilter::new("info,thinkrail=debug"),
    }
}

/// The installed logger. Keep it alive until exit: dropping it flushes buffered lines.
pub struct LogHandle {
    filter: reload::Handle<EnvFilter, Registry>,
    dir: PathBuf,
    pinned: bool,
    _guard: WorkerGuard,
}

impl std::fmt::Debug for LogHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogHandle")
            .field("dir", &self.dir)
            .field("pinned", &self.pinned)
            .finish_non_exhaustive()
    }
}

impl LogHandle {
    /// `<data>/logs`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether `THINKRAIL_LOG` fixes the level, so [`Self::set_level`] has no effect.
    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// Takes effect immediately, unless the environment variable pins the level.
    pub fn set_level(&self, level: LogLevel) -> anyhow::Result<()> {
        if self.pinned {
            return Ok(());
        }
        self.filter
            .reload(filter(level))
            .context("无法切换日志级别")?;
        tracing::info!(level = level.as_str(), "log level changed");
        Ok(())
    }

    /// The newest `limit` lines of the newest file (see [`tail`]).
    pub fn tail(&self, limit: usize) -> std::io::Result<Vec<LogEntry>> {
        tail(&self.dir, limit)
    }
}

/// Installs the global subscriber at `info` (or `THINKRAIL_LOG`); the stored level is applied
/// later with [`LogHandle::set_level`], once the settings database is open. Debug builds also
/// write to stderr.
pub fn init(data_dir: &Path) -> anyhow::Result<LogHandle> {
    let dir = data_dir.join("logs");
    std::fs::create_dir_all(&dir).with_context(|| format!("无法创建日志目录 {}", dir.display()))?;
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix(FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(&dir)
        .context("无法打开日志文件")?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let env = EnvFilter::try_from_env(ENV_VAR).ok();
    let pinned = env.is_some();
    let (level_layer, handle) = reload::Layer::new(env.unwrap_or_else(|| filter(LogLevel::Info)));
    let timer = OffsetTime::new(BEIJING, Rfc3339);
    let file = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .with_timer(timer.clone());
    let registry = tracing_subscriber::registry().with(level_layer).with(file);
    #[cfg(debug_assertions)]
    let registry = registry.with(
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_timer(timer),
    );
    registry.try_init().context("日志已经初始化")?;
    Ok(LogHandle {
        filter: handle,
        dir,
        pinned,
        _guard: guard,
    })
}
