//! Log setup (04 §3): daily-rolling `<data>/logs/thinkrail-native.YYYY-MM-DD.log` through a
//! non-blocking writer, `THINKRAIL_LOG` as the `EnvFilter` (default `info`), stderr too in debug.

use std::path::Path;

use anyhow::Context as _;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _};

/// Installs the global subscriber. Keep the guard alive until exit or buffered lines are lost.
pub fn init(data_dir: &Path) -> anyhow::Result<WorkerGuard> {
    let dir = data_dir.join("logs");
    std::fs::create_dir_all(&dir).with_context(|| format!("无法创建日志目录 {}", dir.display()))?;
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("thinkrail-native")
        .filename_suffix("log")
        .build(&dir)
        .context("无法打开日志文件")?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("THINKRAIL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let file = fmt::layer().with_writer(writer).with_ansi(false).with_target(true);
    let registry = tracing_subscriber::registry().with(filter).with(file);
    #[cfg(debug_assertions)]
    let registry = registry.with(fmt::layer().with_writer(std::io::stderr).with_target(true));
    registry.try_init().context("日志已经初始化")?;
    Ok(guard)
}
