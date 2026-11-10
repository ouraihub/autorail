//! What every backend session shares: request/reply correlation, the stdout reader thread, start
//! validation, startup failure reporting and graceful shutdown.

use std::{
    collections::HashMap,
    io::BufReader,
    process::ChildStdout,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::Value;
use thinkrail_core::{
    BackendKind, Error, ErrorCode, ErrorKind, Result, SessionError, SessionRole, StartSession,
};
use tracing::{debug, info, warn};

use crate::{
    framing,
    process::{self, Process, redact},
    projection::{Identity, Projection},
};
mod requests;
use requests::Waiter;

pub(crate) const HANDSHAKE: Duration = Duration::from_secs(30);
pub(crate) const COMMAND: Duration = Duration::from_secs(30);
const EXIT_GRACE: Duration = Duration::from_secs(3);
const ENGINE_MESSAGE: usize = 2 * 1024;

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) struct Live {
    pub(crate) process: Arc<Process>,
    pub(crate) state: Mutex<Projection>,
    pending: Mutex<HashMap<String, Waiter>>,
    closed: AtomicBool,
    disposing: AtomicBool,
    backend: BackendKind,
}

impl Live {
    pub(crate) fn new(
        process: Arc<Process>,
        mut projection: Projection,
        backend: BackendKind,
    ) -> Arc<Self> {
        projection.attach_trace(process.trace.clone());
        Arc::new(Self {
            process,
            state: Mutex::new(projection),
            pending: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            disposing: AtomicBool::new(false),
            backend,
        })
    }

    /// Fails with `SessionClosed` or `SessionBusy` unless a new prompt may start.
    pub(crate) fn ensure_idle(&self) -> Result<()> {
        let state = self.state.lock();
        if state.is_closed() || self.disposing.load(Ordering::Acquire) {
            return Err(closed());
        }
        if state.is_running() || state.has_interactions() {
            return Err(Error::new(
                ErrorKind::SessionBusy,
                "请等待当前回合结束或先停止",
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) || self.disposing.load(Ordering::Acquire) {
            return Err(closed());
        }
        Ok(())
    }

    pub(crate) fn begin_dispose(&self) -> bool {
        !self.disposing.swap(true, Ordering::AcqRel)
    }

    /// Final state change: publish `Closed` and fail every waiter.
    pub(crate) fn close(&self, error: Option<SessionError>) {
        let _entered = self.process.trace.enter("agent.session.close");
        self.state.lock().close(error);
        let waiters: Vec<Waiter> = {
            let mut pending = self.pending.lock();
            self.closed.store(true, Ordering::Release);
            pending.drain().map(|(_, waiter)| waiter).collect()
        };
        for waiter in waiters {
            waiter.close();
        }
    }

    /// Ends stdin, waits for a voluntary exit, then terminates the tree.
    pub(crate) fn shutdown(&self) {
        let _entered = self.process.trace.enter("agent.session.dispose");
        self.disposing.store(true, Ordering::Release);
        self.process.close_stdin();
        if self.process.wait_exit(EXIT_GRACE).is_none() {
            warn!(
                limit_ms = EXIT_GRACE.as_millis() as u64,
                "agent.process.shutdown_timeout"
            );
            self.process.kill_tree();
            if self.process.wait_exit(EXIT_GRACE).is_none() {
                warn!(
                    limit_ms = EXIT_GRACE.as_millis() as u64,
                    "agent.process.termination_unconfirmed"
                );
            }
        }
        self.close(None);
        info!(
            backend = self.backend.label(),
            exit_confirmed = self.process.wait_exit(Duration::ZERO).is_some(),
            "agent.session.disposed"
        );
    }

    /// Kills a session whose handshake failed and builds the start error, with the redacted
    /// stderr summary. `hint` replaces the generic message when stderr matches its needle.
    pub(crate) fn fail_start(&self, error: Error, hint: Option<(&str, &str)>) -> Error {
        let _entered = self.process.trace.enter("agent.session.start_failed");
        self.disposing.store(true, Ordering::Release);
        self.process.kill_tree();
        if self.process.wait_exit(EXIT_GRACE).is_none() {
            warn!(
                limit_ms = EXIT_GRACE.as_millis() as u64,
                "agent.process.termination_unconfirmed"
            );
        }
        self.close(None);
        warn!(
            backend = self.backend.label(),
            code = error.code(),
            "agent handshake failed"
        );
        if let Some((needle, message)) = hint
            && self.process.stderr_contains(needle)
        {
            return Error::new(ErrorKind::BackendUnavailable, message);
        }
        let summary = self.process.stderr_summary();
        let reason = if summary.is_empty() {
            error.to_string()
        } else {
            format!("{error}\n{summary}")
        };
        Error::new(ErrorKind::BackendUnavailable, reason)
    }

    fn finish(&self, failure: Option<Error>) {
        let disposing = self.disposing.load(Ordering::Acquire);
        if failure.is_some() {
            self.process.kill_tree();
        }
        let mut code = self.process.wait_exit(EXIT_GRACE);
        if code.is_none() {
            self.process.kill_tree();
            code = self.process.wait_exit(EXIT_GRACE);
        }
        let error = match (disposing, failure) {
            (true, _) => None,
            (false, Some(error)) => {
                warn!(
                    backend = self.backend.label(),
                    code = error.code(),
                    "agent protocol failure"
                );
                Some(session_error(&error))
            }
            (false, None) => {
                let exit = code
                    .flatten()
                    .map_or_else(|| "未知".to_owned(), |c| c.to_string());
                let summary = self.process.stderr_summary();
                warn!(backend = self.backend.label(), exit = %exit, "agent process exited unexpectedly");
                let message = if summary.is_empty() {
                    format!("Agent 进程意外退出（退出码 {exit}）")
                } else {
                    format!("Agent 进程意外退出（退出码 {exit}）：\n{summary}")
                };
                Some(SessionError {
                    code: ErrorKind::SessionClosed.code().into(),
                    message,
                    trace_id: None,
                })
            }
        };
        self.close(error);
    }
}

pub(crate) fn closed() -> Error {
    Error::new(
        ErrorKind::SessionClosed,
        "Agent 进程已结束，请重新发送以恢复会话",
    )
}

pub(crate) fn session_error(error: &Error) -> SessionError {
    SessionError {
        code: error.code().into(),
        message: error.to_string(),
        trace_id: None,
    }
}

/// An engine-reported failure: bounded and redacted, never parsed.
pub(crate) fn engine_error(message: &str) -> Error {
    let mut text = redact(message.trim());
    if text.len() > ENGINE_MESSAGE {
        let mut end = ENGINE_MESSAGE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Error::external(format!("Agent 报告：{text}"))
}

/// Runs `handle` for every stdout frame until EOF or the first error, then closes the session.
pub(crate) fn spawn_reader(
    live: Arc<Live>,
    stdout: ChildStdout,
    limit: usize,
    mut handle: impl FnMut(Value) -> Result<()> + Send + 'static,
) -> Result<()> {
    thread::Builder::new()
        .name("agent-stdout".into())
        .spawn(move || {
            let _entered = live.process.trace.enter("agent.stdout");
            debug!("agent.stdout.started");
            let mut reader = BufReader::with_capacity(64 * 1024, stdout);
            let failure = loop {
                match framing::read_frame(&mut reader, limit) {
                    Ok(Some(frame)) => {
                        debug!(
                            backend = live.backend.label(),
                            is_rpc = frame.get("jsonrpc").is_some(),
                            has_id = frame.get("id").is_some(),
                            "agent.protocol.received"
                        );
                        if let Err(error) = handle(frame) {
                            break Some(error);
                        }
                    }
                    Ok(None) => break None,
                    Err(error) => break Some(error),
                }
            };
            debug!(is_error = failure.is_some(), "agent.stdout.closed");
            live.finish(failure);
        })
        .map(drop)
        .map_err(|error| {
            Error::new(
                ErrorKind::BackendUnavailable,
                format!("无法创建读取线程：{error}"),
            )
        })
}

/// Checks that `request` is for `backend` and runs in an existing `request.cwd`.
pub(crate) fn validate(request: &StartSession, backend: BackendKind) -> Result<()> {
    if request.backend != backend {
        return Err(Error::invalid(format!(
            "会话的后端是 {}，不是 {}",
            request.backend.label(),
            backend.label()
        )));
    }
    if !request.cwd.is_absolute() || !request.cwd.is_dir() {
        return Err(Error::invalid(format!(
            "工作目录不存在：{}",
            request.cwd.display()
        )));
    }
    if matches!(request.role, SessionRole::Reviewer { .. })
        && (request.resume.is_some() || request.host_tools.is_some())
    {
        return Err(Error::invalid("评审会话不能恢复旧会话或使用宿主工具"));
    }
    Ok(())
}

pub(crate) fn identity(
    request: &StartSession,
    capabilities: thinkrail_core::Capabilities,
) -> Identity {
    Identity {
        chat: request.chat.clone(),
        backend: request.backend,
        cwd: process::plain_path(&request.cwd),
        capabilities,
    }
}

/// Time left until `deadline`, at least one millisecond so a late step still gets a fair poll.
pub(crate) fn left(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(1))
}
