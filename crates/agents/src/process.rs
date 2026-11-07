//! Engine child process: isolated launch environment, stdin writer, stderr tail, exit tracking and
//! PID-tree termination.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{ChildStdout, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use thinkrail_core::{EngineRouting, Error, ErrorKind, Launch, Result};
use tracing::{info, warn};

mod redaction;
mod streams;

use crate::trace::SessionTrace;
use streams::{read_stderr, wait_child, write_stdin};

pub(crate) use redaction::redact;

const STDERR_TAIL: usize = 64 * 1024;
const STDERR_SUMMARY: usize = 2 * 1024;

/// Inherited variables that could carry the user's own credentials or engine configuration.
pub(crate) fn is_isolated(name: &str) -> bool {
    const PREFIXES: [&str; 18] = [
        "PI_",
        "OMP_",
        "DSH_",
        "THINKRAIL_",
        "AWS_",
        "OPENAI_",
        "ANTHROPIC_",
        "AZURE_",
        "GOOGLE_",
        "GEMINI_",
        "VERTEX_",
        "DEEPSEEK_",
        "GROQ_",
        "MISTRAL_",
        "XAI_",
        "COHERE_",
        "OPENROUTER_",
        "BEDROCK_",
    ];
    const PARTS: [&str; 7] = [
        "API_KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "CREDENTIAL",
        "ACCESS_KEY",
        "PRIVATE_KEY",
    ];
    let name = name.to_ascii_uppercase();
    PREFIXES.iter().any(|p| name.starts_with(p))
        || PARTS.iter().any(|p| name.contains(p))
        || name.ends_with("_BASE_URL")
}

/// Path as engines expect it: Windows verbatim prefixes (`\\?\`) stripped.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// `config_root`, else the engine's default `<home>/<default>` (USERPROFILE, then HOME).
pub(crate) fn config_dir(config_root: Option<&Path>, default: &str) -> Result<PathBuf> {
    match config_root {
        Some(root) => Ok(root.to_path_buf()),
        None => std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join(default))
            .ok_or_else(|| {
                Error::invalid("找不到用户主目录（USERPROFILE / HOME），无法定位引擎配置目录")
            }),
    }
}

pub(crate) struct LaunchSpec<'a> {
    pub launch: &'a Launch,
    /// Inserted right after the launch's own arguments.
    pub host_args: Vec<String>,
    pub cwd: PathBuf,
    /// The engine config dir; `None` leaves the engine on its own default.
    pub config_root: Option<&'a Path>,
    /// Variable that carries `config_root` (`PI_CODING_AGENT_DIR` / `DSH_HOME`).
    pub config_var: &'static str,
    /// Extra variables set last (host tool pipe and token, then `EngineRouting.env`); never logged.
    pub host_env: Vec<(String, OsString)>,
}

/// `EngineRouting.env` as launch variables, appended after the host tool variables. `command`
/// clears inherited `THINKRAIL_*` first, so these values always win.
pub(crate) fn routing_env(
    routing: &EngineRouting,
) -> impl Iterator<Item = (String, OsString)> + '_ {
    routing
        .env
        .iter()
        .map(|(name, value)| (name.clone(), OsString::from(value.expose())))
}

pub(crate) fn command(spec: &LaunchSpec<'_>) -> Result<Command> {
    let Launch::External { executable, args } = spec.launch;
    if !executable.is_absolute() || !executable.is_file() {
        return Err(Error::new(
            ErrorKind::BackendUnavailable,
            format!("可执行文件不存在：{}", executable.display()),
        ));
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .args(&spec.host_args)
        .current_dir(plain_path(&spec.cwd))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars_os() {
        if is_isolated(&name.to_string_lossy()) {
            command.env_remove(&name);
        }
    }
    if let Some(root) = spec.config_root {
        command.env(spec.config_var, plain_path(root));
    }
    for (name, value) in &spec.host_env {
        command.env(name, value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(command)
}

#[derive(Default)]
struct Life {
    exited: bool,
    code: Option<i32>,
    stderr_closed: bool,
}

/// A running engine process. Dropping it does not kill the process; callers dispose explicitly.
pub(crate) struct Process {
    pid: u32,
    stdin: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    life: Arc<(Mutex<Life>, Condvar)>,
    stderr: Arc<Mutex<Vec<u8>>>,
    frame_limit: usize,
    pub(crate) trace: Arc<SessionTrace>,
}

impl Process {
    /// Spawns the engine plus its stdin writer, stderr tail and exit waiter threads.
    pub(crate) fn spawn(
        mut command: Command,
        frame_limit: usize,
        trace: Arc<SessionTrace>,
    ) -> Result<(Arc<Self>, ChildStdout)> {
        let _entered = trace.enter("agent.process.start");
        info!("agent.process.start_requested");
        let started = Instant::now();
        let mut child = command.spawn().map_err(|error| {
            Error::new(
                ErrorKind::BackendUnavailable,
                format!("无法启动进程：{error}"),
            )
        })?;
        let pid = child.id();
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            terminate(pid);
            return Err(Error::new(
                ErrorKind::BackendUnavailable,
                "无法连接进程的标准输入输出",
            ));
        };
        let process = Arc::new(Self {
            pid,
            stdin: Mutex::new(None),
            life: Arc::new((Mutex::new(Life::default()), Condvar::new())),
            stderr: Arc::new(Mutex::new(Vec::new())),
            frame_limit,
            trace: trace.clone(),
        });
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        *process.stdin.lock() = Some(sender);
        let started_threads = spawn_thread("agent.stdin", trace.clone(), move || {
            write_stdin(stdin, receiver)
        })
        .and_then(|()| {
            let (tail, life) = (process.stderr.clone(), process.life.clone());
            spawn_thread("agent.stderr", trace.clone(), move || {
                read_stderr(stderr, &tail, &life)
            })
        })
        .and_then(|()| {
            let life = process.life.clone();
            spawn_thread("agent.wait", trace.clone(), move || {
                wait_child(child, &life, started)
            })
        });
        if let Err(error) = started_threads {
            terminate(pid);
            return Err(error);
        }
        info!(pid, "agent process started");
        Ok((process, stdout))
    }

    pub(crate) fn send(&self, frame: &Value) -> Result<()> {
        let _entered = self.trace.enter("agent.protocol.send");
        info!(
            request_kind = frame["type"]
                .as_str()
                .filter(|kind| kind.len() <= 64)
                .or(frame["method"].as_str().filter(|kind| kind.len() <= 64))
                .unwrap_or("reply"),
            request_id = frame["id"].as_str().filter(|id| id.len() <= 128),
            "agent.protocol.send"
        );
        let mut bytes = serde_json::to_vec(frame)
            .map_err(|e| Error::protocol(format!("无法编码发送帧：{e}")))?;
        bytes.push(b'\n');
        if bytes.len() > self.frame_limit {
            return Err(Error::invalid("发送给 Agent 的内容超过协议单帧上限"));
        }
        let stdin = self.stdin.lock();
        let closed = || Error::new(ErrorKind::SessionClosed, "Agent 进程的输入已关闭");
        stdin
            .as_ref()
            .ok_or_else(closed)?
            .send(bytes)
            .map_err(|_| closed())
    }

    /// Ends stdin once queued frames are written; engines treat EOF as a request to exit.
    pub(crate) fn close_stdin(&self) {
        let _entered = self.trace.enter("agent.process.stdin_close");
        self.stdin.lock().take();
    }

    /// Waits until the process has exited; returns its exit code if it did within `timeout`.
    pub(crate) fn wait_exit(&self, timeout: Duration) -> Option<Option<i32>> {
        let (lock, condvar) = &*self.life;
        let mut life = lock.lock();
        condvar.wait_while_for(&mut life, |life| !life.exited, timeout);
        life.exited.then_some(life.code)
    }

    /// Terminates the whole process tree by PID. On Windows the waiter keeps the child handle open
    /// until it records the exit under this lock, so the PID cannot have been reused here.
    pub(crate) fn kill_tree(&self) {
        let _entered = self.trace.enter("agent.process.terminate");
        let life = self.life.0.lock();
        if !life.exited {
            terminate(self.pid);
        }
    }

    /// The last 2 KiB of stderr with credential-looking values masked. Waits briefly for the
    /// stderr pipe to drain when the process has already exited.
    pub(crate) fn stderr_summary(&self) -> String {
        {
            let (lock, condvar) = &*self.life;
            let mut life = lock.lock();
            if life.exited {
                condvar.wait_while_for(
                    &mut life,
                    |life| !life.stderr_closed,
                    Duration::from_secs(2),
                );
            }
        }
        let tail = self.stderr.lock();
        let start = tail.len().saturating_sub(STDERR_SUMMARY);
        redact(String::from_utf8_lossy(&tail[start..]).trim())
    }

    pub(crate) fn stderr_contains(&self, needle: &str) -> bool {
        String::from_utf8_lossy(&self.stderr.lock()).contains(needle)
    }
}

fn spawn_thread(
    operation: &'static str,
    trace: Arc<SessionTrace>,
    body: impl FnOnce() + Send + 'static,
) -> Result<()> {
    thread::Builder::new()
        .name(operation.into())
        .spawn(move || {
            let _entered = trace.enter(operation);
            body();
        })
        .map(drop)
        .map_err(|error| {
            Error::new(
                ErrorKind::BackendUnavailable,
                format!("无法创建线程：{error}"),
            )
        })
}

fn terminate(pid: u32) {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut command = Command::new(PathBuf::from(root).join(r"System32\taskkill.exe"));
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("kill");
        command.args(["-KILL", &format!("-{pid}")]);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.status() {
        Ok(status) if status.success() => info!(pid, "agent process tree terminated"),
        Ok(status) => warn!(
            pid,
            code = status.code(),
            "agent process tree termination failed"
        ),
        Err(error) => warn!(pid, %error, "cannot run process tree termination"),
    }
}

#[cfg(test)]
mod tests;
