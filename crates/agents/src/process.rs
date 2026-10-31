//! Engine child process: isolated launch environment, stdin writer, stderr tail, exit tracking and
//! PID-tree termination.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use thinkrail_core::{EnvSource, Error, ErrorKind, Launch, Profile, Result};
use tracing::{debug, info, warn};

const STDERR_TAIL: usize = 64 * 1024;
const STDERR_SUMMARY: usize = 2 * 1024;
const SENSITIVE: [&str; 5] = ["key", "token", "secret", "password", "authorization"];

/// Inherited variables that could carry another profile's credentials or engine configuration.
pub(crate) fn is_isolated(name: &str) -> bool {
    const PREFIXES: [&str; 18] = [
        "PI_", "OMP_", "DSH_", "THINKRAIL_", "AWS_", "OPENAI_", "ANTHROPIC_", "AZURE_", "GOOGLE_", "GEMINI_",
        "VERTEX_", "DEEPSEEK_", "GROQ_", "MISTRAL_", "XAI_", "COHERE_", "OPENROUTER_", "BEDROCK_",
    ];
    const PARTS: [&str; 7] = ["API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "ACCESS_KEY", "PRIVATE_KEY"];
    let name = name.to_ascii_uppercase();
    PREFIXES.iter().any(|p| name.starts_with(p)) || PARTS.iter().any(|p| name.contains(p)) || name.ends_with("_BASE_URL")
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

pub(crate) struct LaunchSpec<'a> {
    pub profile: &'a Profile,
    /// Inserted right after the profile's own arguments.
    pub host_args: Vec<String>,
    pub cwd: PathBuf,
    /// Variable that carries the profile's config root (`PI_CODING_AGENT_DIR` / `DSH_HOME`).
    pub config_var: &'static str,
    /// Extra variables set last (the host tool pipe and token); never logged.
    pub host_env: Vec<(&'static str, std::ffi::OsString)>,
}

pub(crate) fn command(spec: &LaunchSpec<'_>) -> Result<Command> {
    let Launch::External { executable, args } = &spec.profile.input.launch;
    if !executable.is_absolute() || !executable.is_file() {
        return Err(Error::new(ErrorKind::BackendUnavailable, format!("可执行文件不存在：{}", executable.display())));
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
    if let Some(root) = &spec.profile.input.config_root {
        command.env(spec.config_var, plain_path(root));
    }
    for entry in &spec.profile.input.env_refs {
        if entry.name.eq_ignore_ascii_case(spec.config_var) {
            return Err(Error::invalid(format!("请用档案的配置目录设置 {}，不要通过环境变量引用", spec.config_var)));
        }
        let value = match entry.source {
            EnvSource::Launch => std::env::var_os(&entry.reference),
        }
        .ok_or_else(|| Error::invalid(format!("启动环境中没有变量 {}", entry.reference)))?;
        command.env(&entry.name, value);
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
}

impl Process {
    /// Spawns the engine plus its stdin writer, stderr tail and exit waiter threads.
    pub(crate) fn spawn(mut command: Command, frame_limit: usize) -> Result<(Arc<Self>, ChildStdout)> {
        let mut child = command
            .spawn()
            .map_err(|error| Error::new(ErrorKind::BackendUnavailable, format!("无法启动进程：{error}")))?;
        let pid = child.id();
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            terminate(pid);
            return Err(Error::new(ErrorKind::BackendUnavailable, "无法连接进程的标准输入输出"));
        };
        let process = Arc::new(Self {
            pid,
            stdin: Mutex::new(None),
            life: Arc::new((Mutex::new(Life::default()), Condvar::new())),
            stderr: Arc::new(Mutex::new(Vec::new())),
            frame_limit,
        });
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        *process.stdin.lock() = Some(sender);
        let started = spawn_thread("agent-stdin", move || write_stdin(stdin, receiver))
            .and_then(|()| {
                let (tail, life) = (process.stderr.clone(), process.life.clone());
                spawn_thread("agent-stderr", move || read_stderr(stderr, &tail, &life))
            })
            .and_then(|()| {
                let life = process.life.clone();
                spawn_thread("agent-wait", move || wait_child(child, &life))
            });
        if let Err(error) = started {
            terminate(pid);
            return Err(error);
        }
        info!(pid, "agent process started");
        Ok((process, stdout))
    }

    pub(crate) fn send(&self, frame: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(frame).map_err(|e| Error::protocol(format!("无法编码发送帧：{e}")))?;
        bytes.push(b'\n');
        if bytes.len() > self.frame_limit {
            return Err(Error::invalid("发送给 Agent 的内容超过协议单帧上限"));
        }
        let stdin = self.stdin.lock();
        let closed = || Error::new(ErrorKind::SessionClosed, "Agent 进程的输入已关闭");
        stdin.as_ref().ok_or_else(closed)?.send(bytes).map_err(|_| closed())
    }

    /// Ends stdin once queued frames are written; engines treat EOF as a request to exit.
    pub(crate) fn close_stdin(&self) {
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
                condvar.wait_while_for(&mut life, |life| !life.stderr_closed, Duration::from_secs(2));
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

fn spawn_thread(name: &str, body: impl FnOnce() + Send + 'static) -> Result<()> {
    thread::Builder::new()
        .name(name.into())
        .spawn(body)
        .map(drop)
        .map_err(|error| Error::new(ErrorKind::BackendUnavailable, format!("无法创建线程：{error}")))
}

fn write_stdin(mut stdin: impl Write, receiver: mpsc::Receiver<Vec<u8>>) {
    for bytes in receiver {
        if let Err(error) = stdin.write_all(&bytes).and_then(|()| stdin.flush()) {
            debug!(%error, "agent stdin closed");
            return;
        }
    }
}

fn read_stderr(mut stderr: impl Read, tail: &Mutex<Vec<u8>>, life: &(Mutex<Life>, Condvar)) {
    let mut buffer = [0; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let mut tail = tail.lock();
                tail.extend_from_slice(&buffer[..count]);
                if tail.len() > STDERR_TAIL {
                    let excess = tail.len() - STDERR_TAIL;
                    tail.drain(..excess);
                }
            }
        }
    }
    life.0.lock().stderr_closed = true;
    life.1.notify_all();
}

fn wait_child(mut child: Child, life: &(Mutex<Life>, Condvar)) {
    let code = match child.wait() {
        Ok(status) => status.code(),
        Err(error) => {
            warn!(%error, "cannot wait for agent process");
            None
        }
    };
    let mut state = life.0.lock();
    state.exited = true;
    state.code = code;
    drop(child);
    drop(state);
    life.1.notify_all();
    info!(code, "agent process exited");
}

fn terminate(pid: u32) {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut command = Command::new(PathBuf::from(root).join(r"System32\taskkill.exe"));
        command.args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("kill");
        command.args(["-KILL", &format!("-{pid}")]);
        command
    };
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match command.status() {
        Ok(status) if status.success() => info!(pid, "agent process tree terminated"),
        Ok(status) => warn!(pid, code = status.code(), "agent process tree termination failed"),
        Err(error) => warn!(pid, %error, "cannot run process tree termination"),
    }
}

/// Masks credentials in engine output: the value after a `NAME=` / `NAME:` whose name contains
/// key|token|secret|password|authorization, and secret-shaped words on other lines that mention
/// such a word. Prose like "No API key found" stays readable.
pub(crate) fn redact(text: &str) -> String {
    text.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str) -> String {
    if !is_sensitive(line) {
        return line.to_owned();
    }
    for (at, separator) in line.char_indices().filter(|(_, c)| matches!(c, '=' | ':' | '：')) {
        let before = line[..at].trim_end();
        let start = before
            .char_indices()
            .rfind(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-')))
            .map_or(0, |(i, c)| i + c.len_utf8());
        if is_sensitive(&before[start..]) {
            return format!("{}{separator} ***", &line[..at]);
        }
    }
    line.split(' ').map(|word| if secret_shaped(word) { "***" } else { word }).collect::<Vec<_>>().join(" ")
}

fn is_sensitive(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SENSITIVE.iter().any(|word| lower.contains(word))
}

fn secret_shaped(word: &str) -> bool {
    let word = word.trim_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '-' | '_'));
    word.len() >= 16
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_engine_config_are_never_inherited() {
        for name in ["OPENAI_API_KEY", "my_token", "PI_CODING_AGENT_DIR", "dsh_home", "AWS_PROFILE", "ANTHROPIC_BASE_URL", "GITHUB_TOKEN", "CUSTOM_BASE_URL", "THINKRAIL_HOST_TOKEN"] {
            assert!(is_isolated(name), "{name}");
        }
        for name in ["PATH", "SystemRoot", "USERPROFILE", "APPDATA", "TEMP", "NODE_OPTIONS"] {
            assert!(!is_isolated(name), "{name}");
        }
    }

    #[test]
    fn stderr_values_after_sensitive_words_are_masked() {
        let text = "boot ok\nOPENAI_API_KEY=sk-live-1\nAuthorization: Bearer abc\nInvalid secret sk-proj-0123456789abcdef.\n中文 password：x";
        let masked = redact(text);
        assert!(!masked.contains("sk-live-1") && !masked.contains("abc") && !masked.contains("0123456789"), "{masked}");
        assert!(!masked.ends_with('x'), "{masked}");
        assert!(masked.starts_with("boot ok\nOPENAI_API_KEY= ***"), "{masked}");
        assert_eq!(masked.lines().count(), 5);
    }

    #[test]
    fn credential_prose_and_links_stay_readable() {
        let text = "No API key found for unknown.\nUse /login into a provider via OAuth or API key. See: D:\\pi\\docs\\providers.md";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn verbatim_prefixes_are_stripped_for_engines() {
        assert_eq!(plain_path(Path::new(r"\\?\C:\work\a")), PathBuf::from(r"C:\work\a"));
        assert_eq!(plain_path(Path::new(r"\\?\UNC\srv\share")), PathBuf::from(r"\\srv\share"));
        assert_eq!(plain_path(Path::new(r"C:\work")), PathBuf::from(r"C:\work"));
    }
}
