use std::{
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use portable_pty::{ChildKiller, MasterPty, PtySize, native_pty_system};
use thinkrail_core::{
    Attached, Error, ErrorKind, GridSize, Result, Shell, TerminalEvent, TerminalRuntimeId,
    TerminalSink, WorkspaceId,
};
use tracing::{info, warn};

use crate::{decode::decode_utf8, shell};

const HISTORY_BYTES: usize = 131_072;
const READ_BUFFER: usize = 8192;
const MAX_INPUT: usize = 65_536;
/// Chunks buffered between the reader and the sink; when full the reader stops reading the PTY.
const QUEUE_CHUNKS: usize = 64;
const POLL: Duration = Duration::from_millis(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct History {
    text: String,
    truncated: bool,
    end: u64,
}

impl History {
    /// Assigns the chunk's byte offset and appends it in one step, keeping the newest 128 KiB.
    fn append(&mut self, data: &str) -> u64 {
        let offset = self.end;
        self.end += data.len() as u64;
        self.text.push_str(data);
        if self.text.len() > HISTORY_BYTES {
            let mut cut = self.text.len() - HISTORY_BYTES;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
            self.truncated = true;
        }
        offset
    }
}

struct Shared {
    id: TerminalRuntimeId,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    history: Mutex<History>,
    exit_code: Mutex<Option<u32>>,
    exited: AtomicBool,
    stopped: AtomicBool,
}

/// One live shell behind a PTY, with its waiter, reader and publisher threads.
pub(crate) struct Runtime {
    pub(crate) id: TerminalRuntimeId,
    pub(crate) workspace: WorkspaceId,
    pub(crate) tab_key: String,
    shared: Arc<Shared>,
    size: Mutex<GridSize>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    waiter: Mutex<Option<JoinHandle<()>>>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

pub(crate) struct SpawnRequest<'a> {
    pub workspace: WorkspaceId,
    pub tab_key: String,
    pub shell: Shell,
    pub size: GridSize,
    pub cwd: &'a Path,
    pub sink: Arc<dyn TerminalSink>,
}

pub(crate) fn validate_size(size: GridSize) -> Result<()> {
    if size.cols == 0 || size.rows == 0 || size.cols > 32_767 || size.rows > 32_767 {
        return Err(Error::invalid("终端尺寸须为 1..32767"));
    }
    Ok(())
}

fn pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn pty_error(what: &str, _error: impl std::fmt::Display) -> Error {
    Error::external(what)
}

/// The worktree as a shell expects it: Windows verbatim prefixes (`\\?\`) stripped, else shells
/// show and inherit the provider-qualified path.
fn plain_path(path: &Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}").into()
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.into()
    } else {
        path.to_path_buf()
    }
}

impl Runtime {
    pub(crate) fn spawn(request: SpawnRequest<'_>) -> Result<Self> {
        validate_size(request.size)?;
        let program = shell::program(request.shell)?;
        let pair = native_pty_system()
            .openpty(pty_size(request.size))
            .map_err(|e| pty_error("无法创建终端", e))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| pty_error("无法读取终端", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| pty_error("无法写入终端", e))?;
        let mut command = shell::command(request.shell, program);
        command.cwd(plain_path(request.cwd));
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| pty_error("无法启动终端 shell", e))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let id = TerminalRuntimeId::generate();
        let shared = Arc::new(Shared {
            id: id.clone(),
            master: Mutex::new(Some(pair.master)),
            history: Mutex::new(History::default()),
            exit_code: Mutex::new(None),
            exited: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        });
        let waiter = {
            let shared = shared.clone();
            let span = tracing::Span::current();
            thread::spawn(move || {
                let _guard = span.enter();
                let status = child.wait();
                let code = status
                    .as_ref()
                    .ok()
                    .map(portable_pty::ExitStatus::exit_code);
                *shared.exit_code.lock() = code;
                shared.exited.store(true, Ordering::Release);
                // Closing the master is what ends the reader on Windows ConPTY.
                drop(shared.master.lock().take());
                match status {
                    Ok(_) => {
                        info!(runtime = %shared.id, exit_code = ?code, "terminal.runtime.exited")
                    }
                    Err(_) => {
                        warn!(runtime = %shared.id, code = "EXTERNAL", "terminal.runtime.wait_failed")
                    }
                }
            })
        };
        let (tx, rx) = mpsc::sync_channel::<TerminalEvent>(QUEUE_CHUNKS);
        let sink = request.sink;
        // Detached: a sink that never returns must not be able to stall shutdown.
        let span = tracing::Span::current();
        thread::spawn(move || {
            let _guard = span.enter();
            for event in rx {
                sink.publish(event);
            }
        });
        let reader = {
            let shared = shared.clone();
            let span = tracing::Span::current();
            thread::spawn(move || span.in_scope(|| read_loop(reader, &shared, &tx)))
        };
        info!(runtime = %id, workspace = %request.workspace, shell = ?request.shell, "terminal.runtime.started");
        Ok(Self {
            id,
            workspace: request.workspace,
            tab_key: request.tab_key,
            shared,
            size: Mutex::new(request.size),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(killer),
            waiter: Mutex::new(Some(waiter)),
            reader: Mutex::new(Some(reader)),
        })
    }

    pub(crate) fn is_exited(&self) -> bool {
        self.shared.exited.load(Ordering::Acquire)
    }

    /// Replay and its end offset come from the same lock the reader assigns offsets under.
    pub(crate) fn attached(&self, created: bool) -> Attached {
        let history = self.shared.history.lock();
        Attached {
            runtime: self.id.clone(),
            created,
            replay: history.text.clone(),
            replay_truncated: history.truncated,
            replay_end_offset: history.end,
            size: *self.size.lock(),
        }
    }

    pub(crate) fn write(&self, data: &str) -> Result<()> {
        if data.len() > MAX_INPUT {
            return Err(Error::invalid("终端输入超过 64 KiB"));
        }
        let mut writer = self.writer.lock();
        let writer = writer
            .as_mut()
            .filter(|_| !self.is_exited())
            .ok_or_else(closed)?;
        writer
            .write_all(data.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|e| pty_error("终端写入失败", e))
    }

    pub(crate) fn resize(&self, size: GridSize) -> Result<()> {
        validate_size(size)?;
        let mut current = self.size.lock();
        if *current == size {
            return Ok(());
        }
        let master = self.shared.master.lock();
        master
            .as_ref()
            .ok_or_else(closed)?
            .resize(pty_size(size))
            .map_err(|e| pty_error("终端调整尺寸失败", e))?;
        *current = size;
        Ok(())
    }

    /// Ends the shell and joins its threads. Success means the process was observed to exit:
    /// portable-pty 0.9.0's Windows `kill` reports success as an error, so its result is not proof.
    pub(crate) fn shutdown(&self) -> Result<()> {
        self.shared.stopped.store(true, Ordering::Release);
        let kill_error = if self.is_exited() {
            None
        } else {
            self.killer.lock().kill().err()
        };
        self.writer.lock().take();
        let deadline = Instant::now() + EXIT_TIMEOUT;
        while !self.is_exited() && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        if !self.is_exited() {
            return Err(match kill_error {
                Some(error) => pty_error("无法结束终端进程", error),
                None => Error::timeout("终端进程未在 5 秒内退出"),
            });
        }
        join(&self.waiter);
        drop(self.shared.master.lock().take());
        join(&self.reader);
        if self.shared.exit_code.lock().is_none() {
            return Err(Error::external("无法确认终端进程退出状态"));
        }
        Ok(())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            warn!(runtime = %self.id, code = %thinkrail_core::ErrorCode::code(&error), "terminal.runtime.drop_cleanup_failed");
        }
    }
}

fn closed() -> Error {
    Error::new(ErrorKind::SessionClosed, "终端已退出，请重新连接")
}

fn join(handle: &Mutex<Option<JoinHandle<()>>>) {
    if let Some(handle) = handle.lock().take()
        && let Err(payload) = handle.join()
    {
        std::panic::resume_unwind(payload);
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, shared: &Shared, tx: &SyncSender<TerminalEvent>) {
    let mut buffer = [0u8; READ_BUFFER];
    let mut pending = Vec::with_capacity(READ_BUFFER + 4);
    loop {
        let (count, end) = match reader.read(&mut buffer) {
            Ok(0) => (0, true),
            Ok(count) => (count, false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                if !shared.exited.load(Ordering::Acquire) && !shared.stopped.load(Ordering::Acquire)
                {
                    warn!(runtime = %shared.id, code = "IO", io_kind = ?error.kind(), "terminal.runtime.read_failed");
                }
                (0, true)
            }
        };
        if shared.stopped.load(Ordering::Acquire) {
            if end {
                return;
            }
            continue;
        }
        pending.extend_from_slice(&buffer[..count]);
        let data = decode_utf8(&mut pending, end);
        if !data.is_empty() {
            let byte_offset = shared.history.lock().append(&data);
            if !send(
                tx,
                &shared.stopped,
                TerminalEvent::Data {
                    runtime: shared.id.clone(),
                    byte_offset,
                    data,
                },
            ) {
                return;
            }
        }
        if end {
            break;
        }
    }
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while !shared.exited.load(Ordering::Acquire)
        && !shared.stopped.load(Ordering::Acquire)
        && Instant::now() < deadline
    {
        thread::sleep(POLL);
    }
    if !shared.exited.load(Ordering::Acquire) && !shared.stopped.load(Ordering::Acquire) {
        warn!(runtime = %shared.id, code = "TIMEOUT", limit_ms = EXIT_TIMEOUT.as_millis() as u64, "terminal.runtime.exit_unconfirmed");
    }
    let exit_code = *shared.exit_code.lock();
    send(
        tx,
        &shared.stopped,
        TerminalEvent::Exit {
            runtime: shared.id.clone(),
            exit_code,
        },
    );
}

/// Blocks while the queue is full (backpressure) but gives up as soon as the runtime is stopped.
fn send(tx: &SyncSender<TerminalEvent>, stopped: &AtomicBool, mut event: TerminalEvent) -> bool {
    loop {
        match tx.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(back)) => {
                if stopped.load(Ordering::Acquire) {
                    return false;
                }
                event = back;
                thread::sleep(POLL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_offsets_are_contiguous_and_trim_on_char_boundaries() {
        let mut history = History::default();
        assert_eq!(history.append("ab"), 0);
        assert_eq!(history.append("中"), 2);
        let big = "中".repeat(HISTORY_BYTES / 3 + 10);
        assert_eq!(history.append(&big), 5);
        assert!(history.truncated);
        assert!(history.text.len() <= HISTORY_BYTES);
        assert!(history.text.chars().all(|c| c == '中'));
        assert_eq!(history.end, 5 + big.len() as u64);
    }
}
