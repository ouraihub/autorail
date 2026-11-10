//! Dedicated process stdin, stderr-tail and exit-wait workers.

use std::{
    io::{Read, Write},
    process::Child,
    sync::mpsc,
    time::Instant,
};

use parking_lot::{Condvar, Mutex};
use tracing::{debug, info, warn};

use super::{Life, STDERR_TAIL};
use crate::trace::SessionTrace;
pub(super) fn write_stdin(mut stdin: impl Write, receiver: mpsc::Receiver<Vec<u8>>) {
    debug!("agent.stdin.started");
    for bytes in receiver {
        if let Err(error) = stdin.write_all(&bytes).and_then(|()| stdin.flush()) {
            warn!(error_kind = ?error.kind(), "agent.stdin.write_failed");
            return;
        }
    }
}

pub(super) fn read_stderr(
    mut stderr: impl Read,
    tail: &Mutex<Vec<u8>>,
    life: &(Mutex<Life>, Condvar),
) {
    debug!("agent.stderr.started");
    let mut buffer = [0; 4096];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                warn!(error_kind = ?error.kind(), "agent.stderr.read_failed");
                break;
            }
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
    debug!("agent.stderr.closed");
    life.0.lock().stderr_closed = true;
    life.1.notify_all();
}

pub(super) fn wait_child(
    mut child: Child,
    life: &(Mutex<Life>, Condvar),
    started: Instant,
    trace: std::sync::Arc<SessionTrace>,
) {
    let operation = trace.operation("agent.process.exit", "external");
    let (wait_result, code) = match child.wait() {
        Ok(status) => {
            let code = status.code();
            let result = if status.success() {
                Ok(())
            } else {
                Err("PROCESS_EXIT_FAILED")
            };
            (result, code)
        }
        Err(_) => (Err("PROCESS_WAIT_FAILED"), None),
    };
    let mut state = life.0.lock();
    state.exited = wait_result != Err("PROCESS_WAIT_FAILED");
    state.code = code;
    drop(child);
    if state.exited {
        let _entered = trace.enter_operation(&operation);
        info!(
            code,
            duration_ms = started.elapsed().as_millis() as u64,
            "agent process exited"
        );
    }
    drop(state);
    life.1.notify_all();
    match wait_result {
        Ok(()) => operation.finish("SUCCESS"),
        Err(code) => operation.fail(code),
    }
}
