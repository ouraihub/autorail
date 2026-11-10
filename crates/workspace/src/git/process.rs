//! Git child ownership, bounded pipe draining, deadlines and cleanup.
use crate::paths::bounded_read;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use thinkrail_core::{Error, IoContext, Result};
use tracing::{debug, warn};

pub(crate) const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// A host commit runs the user's hooks, which may run checks.
const HOOK_TIMEOUT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(crate) struct Output {
    pub success: bool,
    pub stdout: String,
}

/// Runs `git` without a shell, with hooks/fsmonitor disabled, `GIT_*` env removed, a 30 s timeout
/// and 16 MiB output limit. Exit codes in `allowed` are returned as `success: false`.
pub(crate) fn run(root: &Path, args: &[&str], allowed: &[i32]) -> Result<Output> {
    spawn(root, args, allowed, false)
}

/// [`run`] with the user's hooks enabled and a 10 minute timeout, for the host's commit.
pub(crate) fn run_with_hooks(root: &Path, args: &[&str], allowed: &[i32]) -> Result<Output> {
    spawn(root, args, allowed, true)
}

fn spawn(root: &Path, args: &[&str], allowed: &[i32], hooks: bool) -> Result<Output> {
    let started = Instant::now();
    let timeout = if hooks { HOOK_TIMEOUT } else { TIMEOUT };
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["--no-optional-locks", "--literal-pathspecs"])
        .args(if hooks {
            &[][..]
        } else {
            &["-c", "core.hooksPath="][..]
        })
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .context("无法启动 git，请确认已安装 Git 并在 PATH 中")?;
    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(out), Some(err)) => (out, err),
        _ => {
            terminate(&mut child);
            return Err(Error::external("git 输出管道不可用"));
        }
    };
    let out_rx = drain(stdout);
    let err_rx = drain(stderr);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(source) => {
                terminate(&mut child);
                return Err(Error::Io {
                    context: "等待 git".into(),
                    source,
                });
            }
        }
        if started.elapsed() > timeout {
            terminate(&mut child);
            return Err(Error::timeout(format!(
                "git {} 超过 {} 秒未完成",
                args.first().copied().unwrap_or(""),
                timeout.as_secs()
            )));
        }
        thread::sleep(POLL);
    };
    let stdout = received(&out_rx)?;
    let _stderr = received(&err_rx)?;
    debug!(
        command = args.first().copied().unwrap_or(""),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "workspace.git.finished"
    );
    if !status.success() && !status.code().is_some_and(|code| allowed.contains(&code)) {
        return Err(Error::external(format!(
            "git {} 失败（退出码 {:?}）",
            args.first().copied().unwrap_or(""),
            status.code()
        )));
    }
    let stdout = String::from_utf8(stdout).map_err(|_| Error::external("git 输出不是 UTF-8"))?;
    Ok(Output {
        success: status.success(),
        stdout,
    })
}

fn drain(reader: impl Read + Send + 'static) -> mpsc::Receiver<Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    let span = tracing::Span::current();
    thread::spawn(move || {
        let _guard = span.enter();
        // The caller owns the result; after a timeout its receiver is intentionally gone.
        if tx.send(bounded_read(reader, MAX_OUTPUT)).is_err() {
            debug!(code = "SESSION_CLOSED", "workspace.git.output_discarded");
        }
    });
    rx
}

fn received(rx: &mpsc::Receiver<Result<Vec<u8>>>) -> Result<Vec<u8>> {
    rx.recv_timeout(TIMEOUT).map_err(|error| match error {
        mpsc::RecvTimeoutError::Timeout => Error::timeout("读取 git 输出超时"),
        mpsc::RecvTimeoutError::Disconnected => Error::external("读取 git 输出的后台线程异常结束"),
    })?
}

fn terminate(child: &mut std::process::Child) {
    if let Err(error) = child.kill() {
        warn!(code = "IO", io_kind = ?error.kind(), "workspace.git.kill_failed");
    }
    if let Err(error) = child.wait() {
        warn!(code = "IO", io_kind = ?error.kind(), "workspace.git.reap_failed");
    }
}
