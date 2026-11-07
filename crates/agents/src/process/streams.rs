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
pub(super) fn write_stdin(mut stdin: impl Write, receiver: mpsc::Receiver<Vec<u8>>) {
    info!("agent.stdin.started");
    for bytes in receiver {
        if let Err(error) = stdin.write_all(&bytes).and_then(|()| stdin.flush()) {
            debug!(%error, "agent stdin closed");
            return;
        }
    }
}

pub(super) fn read_stderr(
    mut stderr: impl Read,
    tail: &Mutex<Vec<u8>>,
    life: &(Mutex<Life>, Condvar),
) {
    info!("agent.stderr.started");
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
    info!("agent.stderr.closed");
    life.0.lock().stderr_closed = true;
    life.1.notify_all();
}

pub(super) fn wait_child(mut child: Child, life: &(Mutex<Life>, Condvar), started: Instant) {
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
    info!(
        code,
        duration_ms = started.elapsed().as_millis() as u64,
        "agent process exited"
    );
    drop(state);
    life.1.notify_all();
}
