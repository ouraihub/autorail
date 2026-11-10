//! Process-wide Rust panic diagnostics; arbitrary payloads are hashed, never logged verbatim.
use std::{backtrace::Backtrace, io::Write, sync::Arc};

use tracing_appender::rolling::RollingFileAppender;
use tracing_subscriber::fmt::MakeWriter as _;

pub(super) const TARGET: &str = "thinkrail_crash";

/// Both normal queued logs and synchronous panic logs use the same rolling file and lock.
#[derive(Clone)]
pub(super) struct FileWriter(pub Arc<RollingFileAppender>);

impl Write for FileWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.make_writer().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.make_writer().flush()
    }
}

pub(super) fn install() {
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str));
        let summary = if payload.is_some() {
            "redacted_string_payload"
        } else {
            "non_string_payload"
        };
        let location = info.location();
        let file = location
            .map(|at| source_path(at.file()))
            .unwrap_or_default();
        let backtrace = Backtrace::force_capture().to_string();
        // Backtrace symbol names contain code identities, not runtime values. Remove host paths.
        let backtrace = backtrace
            .lines()
            .map(|line| match line.trim().strip_prefix("at ") {
                Some(path) => format!("at {}", source_path(path)),
                None => line.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        tracing::error!(target: "thinkrail_crash",
            code = "RUST_PANIC", summary,
            payload_hash = payload.map(lumakit_observability::diagnostic_hash),
            panic_file = file, panic_line = location.map(|at| at.line()),
            panic_column = location.map(|at| at.column()),
            thread_id = ?std::thread::current().id(),
            backtrace = %backtrace,
            "app.panic");
        // Do not invoke the default stderr hook: it prints arbitrary payloads, including secrets.
        // Returning preserves Rust's unwind/abort behavior; this hook does not recover the task.
    }));
}

fn source_path(path: &str) -> &str {
    path.find("crates/")
        .or_else(|| path.find("crates\\"))
        .map(|start| &path[start..])
        .unwrap_or_else(|| path.rsplit(['/', '\\']).next().unwrap_or(path))
}
